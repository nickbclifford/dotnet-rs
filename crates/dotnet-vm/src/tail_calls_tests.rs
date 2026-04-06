use crate::{
    StepResult,
    dispatch::ExecutionEngine,
    stack::{CallStack, GCArena, ops::CallOps},
    state::{ArenaLocalState, SharedGlobalState},
    sync::Arc,
};
use dotnet_assemblies::AssemblyLoader;
use dotnet_types::{TypeDescription, generics::GenericLookup, members::MethodDescription};
use dotnet_utils::gc::GCHandle;
use dotnetdll::{
    binary::signature::kinds::CallingConvention,
    prelude::*,
    resolved::{
        assembly::ExternalAssemblyReference,
        members::{Method, MethodSource, UserMethod},
        signature::ReturnType,
        types::{ExternalTypeReference, ResolutionScope, TypeDefinition, UserType},
    },
};

fn get_mock_loader() -> Arc<AssemblyLoader> {
    thread_local! {
        static MOCK_LOADER: Arc<AssemblyLoader> = {
            let loader = AssemblyLoader::new_bare("mock_root".to_string())
                .expect("Failed to create mock AssemblyLoader");

            let mut corlib = Resolution::new(Module::new("System.Private.CoreLib.dll"));
            corlib.assembly = Some(Assembly::new("System.Private.CoreLib"));

            let mut object_type = TypeDefinition::new(None, "Object");
            object_type.namespace = Some("System".into());
            let object_idx = corlib.push_type_definition(object_type);

            let mut value_type = TypeDefinition::new(None, "ValueType");
            value_type.namespace = Some("System".into());
            value_type.extends = Some(UserType::Definition(object_idx).into());
            let value_type_idx = corlib.push_type_definition(value_type);

            let mut enum_type = TypeDefinition::new(None, "Enum");
            enum_type.namespace = Some("System".into());
            enum_type.extends = Some(UserType::Definition(value_type_idx).into());
            corlib.push_type_definition(enum_type);

            let mut type_type = TypeDefinition::new(None, "Type");
            type_type.namespace = Some("System".into());
            type_type.extends = Some(UserType::Definition(object_idx).into());
            corlib.push_type_definition(type_type);

            let mut array_type = TypeDefinition::new(None, "Array");
            array_type.namespace = Some("System".into());
            array_type.extends = Some(UserType::Definition(object_idx).into());
            corlib.push_type_definition(array_type);

            let mut string_type = TypeDefinition::new(None, "String");
            string_type.namespace = Some("System".into());
            string_type.extends = Some(UserType::Definition(object_idx).into());
            corlib.push_type_definition(string_type);

            loader.register_owned_assembly(corlib.clone());

            let mut mscorlib = corlib.clone();
            mscorlib.assembly = Some(Assembly::new("mscorlib"));
            loader.register_owned_assembly(mscorlib);

            let mut system_runtime = corlib;
            system_runtime.assembly = Some(Assembly::new("System.Runtime"));
            loader.register_owned_assembly(system_runtime);
            Arc::new(loader)
        };
    }
    MOCK_LOADER.with(|l| l.clone())
}

fn run_tail_chain_and_measure_max_depth(tail_call: bool, chain_len: usize) -> usize {
    let loader = get_mock_loader();
    let shared = Arc::new(SharedGlobalState::new(loader.clone()));

    // Build a minimal Resolution with one type, an entrypoint, and a chain of methods.
    // Each chain method tail-calls (or normal-calls) the next and then returns.
    let mut res = Resolution::new(Module::new("TailChain.dll"));
    res.assembly = Some(Assembly::new("TailChainAssembly"));

    // Add references for inheritance and basic types.
    let system_runtime =
        res.push_assembly_reference(ExternalAssemblyReference::new("System.Runtime"));
    let object_type_ref = res.push_type_reference(ExternalTypeReference::new(
        Some("System".into()),
        "Object",
        ResolutionScope::Assembly(system_runtime),
    ));

    let mut type_def = TypeDefinition::new(None, "TailChainType");
    type_def.extends = Some(object_type_ref.into());
    let type_idx = res.push_type_definition(type_def);

    let void_sig = MethodSignature {
        instance: false,
        explicit_this: false,
        calling_convention: CallingConvention::Default,
        parameters: vec![],
        return_type: ReturnType(vec![], None),
        varargs: None,
    };

    let end_body = body::Method {
        header: body::Header {
            maximum_stack_size: 1,
            local_variables: vec![],
            initialize_locals: true,
        },
        instructions: vec![Instruction::Return],
        data_sections: vec![],
    };

    let mut next_method_idx = res.push_method(
        type_idx,
        Method::new(
            Accessibility::Public,
            void_sig.clone(),
            "ChainEnd",
            Some(end_body),
        ),
    );

    for i in (0..chain_len).rev() {
        let body = body::Method {
            header: body::Header {
                maximum_stack_size: 1,
                local_variables: vec![],
                initialize_locals: true,
            },
            instructions: vec![
                Instruction::Call {
                    tail_call,
                    param0: MethodSource::User(UserMethod::Definition(next_method_idx)),
                },
                Instruction::Return,
            ],
            data_sections: vec![],
        };

        next_method_idx = res.push_method(
            type_idx,
            Method::new(
                Accessibility::Public,
                void_sig.clone(),
                format!("Chain{i}"),
                Some(body),
            ),
        );
    }

    let main_sig = MethodSignature {
        instance: false,
        explicit_this: false,
        calling_convention: CallingConvention::Default,
        parameters: vec![],
        return_type: ReturnType(vec![], None),
        varargs: None,
    };
    let main_body = body::Method {
        header: body::Header {
            maximum_stack_size: 1,
            local_variables: vec![],
            initialize_locals: true,
        },
        instructions: vec![
            Instruction::Call {
                tail_call: false,
                param0: MethodSource::User(UserMethod::Definition(next_method_idx)),
            },
            Instruction::Return,
        ],
        data_sections: vec![],
    };
    let main_idx = res.push_method(
        type_idx,
        Method::new(Accessibility::Public, main_sig, "Main", Some(main_body)),
    );

    let res_s = loader.register_owned_assembly(res);
    let _typedef = &res_s.definition()[type_idx];
    let method_def = &res_s.definition()[main_idx];
    let td = TypeDescription::new(res_s.clone(), type_idx);
    let method_index = td
        .definition()
        .methods
        .iter()
        .position(|m| std::ptr::eq(m, method_def))
        .unwrap();
    let entrypoint = MethodDescription::new(
        td,
        GenericLookup::default(),
        res_s,
        MethodMemberIndex::Method(method_index),
    );

    // Run by directly stepping the execution engine so we can observe frame depth.
    let mut arena = GCArena::new(|_| {
        let local = ArenaLocalState::new(shared.statics.clone());
        ExecutionEngine::new(CallStack::new(shared.clone(), local))
    });

    #[cfg(feature = "memory-validation")]
    let thread_id = dotnet_utils::sync::get_current_thread_id();

    arena.mutate_root(|gc, engine| {
        let gc_handle = GCHandle::new(
            gc,
            #[cfg(feature = "multithreading")]
            unsafe {
                engine.stack.arena_inner_gc()
            },
            #[cfg(feature = "memory-validation")]
            thread_id,
        );

        let info = shared
            .caches
            .get_method_info(entrypoint, &Default::default(), shared.clone())
            .expect("Failed to resolve entrypoint");
        engine
            .ves_context(gc_handle)
            .entrypoint_frame(info, Default::default(), vec![])
            .expect("Failed to set up entrypoint frame");
    });

    let mut max_depth = 0usize;
    for _ in 0..(chain_len.saturating_mul(5) + 10_000) {
        let step_res = arena.mutate_root(|gc, engine| {
            let gc_handle = GCHandle::new(
                gc,
                #[cfg(feature = "multithreading")]
                unsafe {
                    engine.stack.arena_inner_gc()
                },
                #[cfg(feature = "memory-validation")]
                thread_id,
            );

            max_depth = max_depth.max(engine.stack.execution.frame_stack.len());
            engine.step(gc_handle)
        });

        match step_res {
            StepResult::Return => break,
            StepResult::Error(e) => panic!(
                "Execution error: {e}\nLast instructions:\n{}",
                shared.last_instructions.lock().dump()
            ),
            StepResult::MethodThrew(exc) => panic!(
                "Unexpected managed exception: {exc}\nLast instructions:\n{}",
                shared.last_instructions.lock().dump()
            ),
            _ => {}
        }
    }

    max_depth
}

#[test]
fn tail_call_chain_keeps_frame_depth_bounded() {
    let max_depth = run_tail_chain_and_measure_max_depth(true, 200);
    assert!(
        max_depth <= 3,
        "tail-call chain should not grow the call stack; observed max depth {max_depth}"
    );
}

#[test]
fn non_tail_call_chain_grows_frame_depth() {
    let max_depth = run_tail_chain_and_measure_max_depth(false, 200);
    assert!(
        max_depth >= 50,
        "non-tail call chain should grow the call stack; observed max depth {max_depth}"
    );
}

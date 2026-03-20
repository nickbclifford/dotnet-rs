#[cfg(test)]
mod tests {
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
            self as resolved_mod,
            assembly::ExternalAssemblyReference,
            members::{ExternalMethodReference, Method, MethodReferenceParent, UserMethod},
            signature::ReturnType,
            types::{
                BaseType, ExternalTypeReference, LocalVariable, MethodType, ResolutionScope,
                TypeDefinition,
            },
        },
    };
    use std::sync::OnceLock;
    static MOCK_LOADER: OnceLock<Arc<AssemblyLoader>> = OnceLock::new();
    fn get_mock_loader() -> Arc<AssemblyLoader> {
        MOCK_LOADER
            .get_or_init(|| {
                let loader = AssemblyLoader::new_bare("mock_root_fault".to_string())
                    .expect("Failed to create mock AssemblyLoader");
                let mut system_runtime = Resolution::new(Module::new("System.Private.CoreLib.dll"));
                system_runtime.assembly = Some(Assembly::new("System.Private.CoreLib"));
                let mut obj2 = TypeDefinition::new(None, "Object");
                obj2.namespace = Some("System".into());
                let obj_idx = system_runtime.push_type_definition(obj2);
                let void_sig = MethodSignature {
                    instance: true,
                    explicit_this: false,
                    calling_convention: CallingConvention::Default,
                    parameters: vec![],
                    return_type: ReturnType(vec![], None),
                    varargs: None,
                };
                system_runtime.push_method(
                    obj_idx,
                    Method::new(
                        resolved_mod::Accessibility::Public,
                        void_sig.clone(),
                        ".ctor",
                        Some(body::Method {
                            header: body::Header {
                                maximum_stack_size: 1,
                                local_variables: vec![],
                                initialize_locals: true,
                            },
                            instructions: vec![Instruction::Return],
                            data_sections: vec![],
                        }),
                    ),
                );
                let mut exc = TypeDefinition::new(None, "Exception");
                exc.namespace = Some("System".into());
                let exc_idx = system_runtime.push_type_definition(exc);
                system_runtime.push_method(
                    exc_idx,
                    Method::new(
                        resolved_mod::Accessibility::Public,
                        void_sig,
                        ".ctor",
                        Some(body::Method {
                            header: body::Header {
                                maximum_stack_size: 1,
                                local_variables: vec![],
                                initialize_locals: true,
                            },
                            instructions: vec![Instruction::Return],
                            data_sections: vec![],
                        }),
                    ),
                );
                let mut val_type = TypeDefinition::new(None, "ValueType");
                val_type.namespace = Some("System".into());
                system_runtime.push_type_definition(val_type);
                let mut enum_type = TypeDefinition::new(None, "Enum");
                enum_type.namespace = Some("System".into());
                system_runtime.push_type_definition(enum_type);
                let mut array_type = TypeDefinition::new(None, "Array");
                array_type.namespace = Some("System".into());
                system_runtime.push_type_definition(array_type);
                let mut string_type = TypeDefinition::new(None, "String");
                string_type.namespace = Some("System".into());
                system_runtime.push_type_definition(string_type);
                loader.register_owned_assembly(system_runtime.clone());
                let mut mscorlib = system_runtime.clone();
                mscorlib.assembly = Some(Assembly::new("mscorlib"));
                loader.register_owned_assembly(mscorlib);
                let mut system_runtime_compat = system_runtime;
                system_runtime_compat.assembly = Some(Assembly::new("System.Runtime"));
                loader.register_owned_assembly(system_runtime_compat);
                Arc::new(loader)
            })
            .clone()
    }
    #[test]
    fn test_fault_handler_skipped_on_normal_exit() {
        let loader = get_mock_loader();
        let shared = Arc::new(SharedGlobalState::new(loader.clone()));
        let mut res = Resolution::new(Module::new("FaultTest.dll"));
        res.assembly = Some(Assembly::new("FaultTestAssembly"));
        let system_runtime =
            res.push_assembly_reference(ExternalAssemblyReference::new("System.Runtime"));
        let object_type_ref = res.push_type_reference(ExternalTypeReference::new(
            Some("System".into()),
            "Object",
            ResolutionScope::Assembly(system_runtime),
        ));
        let mut type_def = TypeDefinition::new(None, "FaultType");
        type_def.extends = Some(object_type_ref.into());
        let type_idx = res.push_type_definition(type_def);
        let sig = MethodSignature {
            instance: false,
            explicit_this: false,
            calling_convention: CallingConvention::Default,
            parameters: vec![],
            return_type: ReturnType(
                vec![],
                Some(dotnetdll::prelude::ParameterType::Value(MethodType::Base(
                    Box::new(BaseType::Int32),
                ))),
            ),
            varargs: None,
        };
        let body = body::Method {
            header: body::Header {
                maximum_stack_size: 1,
                local_variables: vec![LocalVariable::Variable {
                    custom_modifiers: vec![],
                    pinned: false,
                    by_ref: false,
                    var_type: MethodType::Base(Box::new(BaseType::Int32)),
                }],
                initialize_locals: true,
            },
            instructions: vec![
                Instruction::LoadConstantInt32(42),
                Instruction::StoreLocal(0),
                Instruction::Leave(6),
                Instruction::LoadConstantInt32(100),
                Instruction::StoreLocal(0),
                Instruction::EndFinally,
                Instruction::LoadLocal(0),
                Instruction::Return,
            ],
            data_sections: vec![body::DataSection::ExceptionHandlers(vec![
                body::Exception {
                    kind: body::ExceptionKind::Fault,
                    try_offset: 0,
                    try_length: 3,
                    handler_offset: 3,
                    handler_length: 3,
                },
            ])],
        };
        let method_idx = res.push_method(
            type_idx,
            Method::new(
                resolved_mod::Accessibility::Public,
                sig.clone(),
                "TestMethod",
                Some(body),
            ),
        );
        let res_s = loader.register_owned_assembly(res);
        let _typedef = &res_s.definition()[type_idx];
        let method_def = &res_s.definition()[method_idx];
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
        let mut final_int = None;
        for _ in 0..100 {
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
                let res = engine.step(gc_handle);
                if let StepResult::Return = res {
                    let val = engine.stack.execution.evaluation_stack.pop();
                    if let dotnet_value::StackValue::Int32(v) = val {
                        final_int = Some(v);
                    }
                }
                res
            });
            match step_res {
                StepResult::Return => break,
                StepResult::Error(e) => panic!("Execution error: {e}"),
                StepResult::MethodThrew(exc) => {
                    panic!("Unexpected managed exception: {exc}")
                }
                _ => {}
            }
        }
        assert_eq!(final_int, Some(42), "Fault handler should NOT have run");
    }
    #[test]
    fn test_fault_handler_executed_on_exception() {
        let loader = get_mock_loader();
        let shared = Arc::new(SharedGlobalState::new(loader.clone()));
        let mut res = Resolution::new(Module::new("FaultTestExc.dll"));
        res.assembly = Some(Assembly::new("FaultTestExcAssembly"));
        let system_runtime =
            res.push_assembly_reference(ExternalAssemblyReference::new("System.Runtime"));
        let object_type_ref = res.push_type_reference(ExternalTypeReference::new(
            Some("System".into()),
            "Object",
            ResolutionScope::Assembly(system_runtime),
        ));
        let exception_type_ref = res.push_type_reference(ExternalTypeReference::new(
            Some("System".into()),
            "Exception",
            ResolutionScope::Assembly(system_runtime),
        ));
        let mut type_def = TypeDefinition::new(None, "FaultTypeExc");
        type_def.extends = Some(object_type_ref.into());
        let type_idx = res.push_type_definition(type_def);
        let sig = MethodSignature {
            instance: false,
            explicit_this: false,
            calling_convention: CallingConvention::Default,
            parameters: vec![],
            return_type: ReturnType(
                vec![],
                Some(dotnetdll::prelude::ParameterType::Value(MethodType::Base(
                    Box::new(BaseType::Int32),
                ))),
            ),
            varargs: None,
        };
        let ctor_sig = MethodSignature {
            instance: true,
            explicit_this: false,
            calling_convention: CallingConvention::Default,
            parameters: vec![],
            return_type: ReturnType(vec![], None),
            varargs: None,
        };
        let exc_ctor = res.push_method_reference(ExternalMethodReference {
            parent: MethodReferenceParent::Type(MethodType::Base(Box::new(BaseType::class(
                exception_type_ref,
            )))),
            name: ".ctor".into(),
            signature: ctor_sig,
            attributes: vec![],
        });
        let body = body::Method {
            header: body::Header {
                maximum_stack_size: 2,
                local_variables: vec![LocalVariable::Variable {
                    custom_modifiers: vec![],
                    pinned: false,
                    by_ref: false,
                    var_type: MethodType::Base(Box::new(BaseType::Int32)),
                }],
                initialize_locals: true,
            },
            instructions: vec![
                Instruction::NewObject(UserMethod::Reference(exc_ctor)),
                Instruction::Throw,
                Instruction::LoadConstantInt32(100),
                Instruction::StoreLocal(0),
                Instruction::EndFinally,
                Instruction::Pop,
                Instruction::LoadLocal(0),
                Instruction::Return,
            ],
            data_sections: vec![body::DataSection::ExceptionHandlers(vec![
                body::Exception {
                    kind: body::ExceptionKind::Fault,
                    try_offset: 0,
                    try_length: 2,
                    handler_offset: 2,
                    handler_length: 3,
                },
                body::Exception {
                    kind: body::ExceptionKind::TypedException(MethodType::Base(Box::new(
                        BaseType::class(exception_type_ref),
                    ))),
                    try_offset: 0,
                    try_length: 5,
                    handler_offset: 5,
                    handler_length: 3,
                },
            ])],
        };
        let method_idx = res.push_method(
            type_idx,
            Method::new(
                resolved_mod::Accessibility::Public,
                sig.clone(),
                "TestMethodExc",
                Some(body),
            ),
        );
        let res_s = loader.register_owned_assembly(res);
        let _typedef = &res_s.definition()[type_idx];
        let method_def = &res_s.definition()[method_idx];
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
        let mut final_int = None;
        for _ in 0..100 {
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
                let res = engine.step(gc_handle);
                if let StepResult::Return = res {
                    let val = engine.stack.execution.evaluation_stack.pop();
                    if let dotnet_value::StackValue::Int32(v) = val {
                        final_int = Some(v);
                    }
                }
                res
            });
            match step_res {
                StepResult::Return => break,
                StepResult::Error(e) => panic!("Execution error: {e}"),
                StepResult::MethodThrew(exc) => {
                    panic!("Unexpected managed exception: {exc}")
                }
                _ => {}
            }
        }
        assert_eq!(final_int, Some(100), "Fault handler SHOULD have run");
    }
}

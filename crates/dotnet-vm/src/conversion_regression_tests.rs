#[cfg(test)]
mod tests {
    use crate::{
        StepResult,
        stack::ops::VmCallOps,
        state::SharedGlobalState,
        sync::Arc,
        test_utils::{new_test_arena, parameterless_i32_signature, should_break_after_step},
    };
    use dotnet_assemblies::AssemblyLoader;
    use dotnet_types::{TypeDescription, generics::GenericLookup, members::MethodDescription};
    use dotnet_utils::gc::GCHandle;
    use dotnet_value::StackValue;
    use dotnetdll::{
        prelude::*,
        resolved::{
            self as resolved_mod,
            members::{Field, Method},
            types::{BaseType, TypeDefinition},
        },
    };

    fn get_mock_loader() -> Arc<AssemblyLoader> {
        thread_local! {
            static MOCK_LOADER : Arc < AssemblyLoader > = {
                let loader = AssemblyLoader::new_bare("mock_root_conversion_regression".to_string())
                    .expect("Failed to create mock AssemblyLoader");

                let mut mscorlib = Resolution::new(Module::new("mscorlib.dll"));
                mscorlib.assembly = Some(Assembly::new("mscorlib"));
                let mut obj = TypeDefinition::new(None, "Object");
                obj.namespace = Some("System".into());
                mscorlib.push_type_definition(obj);
                loader.register_owned_assembly(mscorlib);

                let mut system_runtime = Resolution::new(Module::new("System.Runtime.dll"));
                system_runtime.assembly = Some(Assembly::new("System.Runtime"));
                let mut obj2 = TypeDefinition::new(None, "Object");
                obj2.namespace = Some("System".into());
                system_runtime.push_type_definition(obj2);
                loader.register_owned_assembly(system_runtime);

                Arc::new(loader)
            };
        }
        MOCK_LOADER.with(|l| l.clone())
    }

    fn run_i32_entrypoint(shared: Arc<SharedGlobalState>, entrypoint: MethodDescription) -> i32 {
        let mut arena = new_test_arena(&shared);

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
                .get_method_info(entrypoint.clone(), &Default::default(), shared.clone())
                .expect("Failed to resolve entrypoint");
            engine
                .ves_context(gc_handle)
                .entrypoint_frame(info, Default::default(), vec![])
                .expect("Failed to set up entrypoint frame");
        });

        let mut final_int = None;
        for _ in 0..1000 {
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
                    match val {
                        StackValue::Int32(v) => final_int = Some(v),
                        other => panic!("Expected Int32 return, got {:?}", other),
                    }
                }
                res
            });

            if should_break_after_step(step_res) {
                break;
            }
        }

        final_int.expect("entrypoint did not produce an Int32 return value")
    }

    fn new_static_field(name: &str, return_type: MemberType) -> Field<'static> {
        Field {
            attributes: vec![],
            name: name.to_string().into(),
            type_modifiers: vec![],
            by_ref: false,
            return_type,
            accessibility: resolved_mod::members::Accessibility::Access(
                resolved_mod::Accessibility::Public,
            ),
            static_member: true,
            init_only: false,
            literal: false,
            default: None,
            not_serialized: false,
            special_name: false,
            pinvoke: None,
            runtime_special_name: false,
            offset: None,
            marshal: None,
            initial_value: None,
        }
    }

    #[test]
    fn static_fields_preserve_small_integer_sign_and_zero_extension() {
        let loader = get_mock_loader();
        let shared = Arc::new(SharedGlobalState::new(loader.clone()));

        let (mut res, _, type_idx) = make_test_assembly!(
            "StaticConvSmallInts.dll",
            "StaticConvSmallIntsAssembly",
            "StaticConvSmallIntsType"
        );

        let i8_field = FieldSource::Definition(res.push_field(
            type_idx,
            new_static_field("I8", MemberType::Base(Box::new(BaseType::Int8))),
        ));
        let u8_field = FieldSource::Definition(res.push_field(
            type_idx,
            new_static_field("U8", MemberType::Base(Box::new(BaseType::UInt8))),
        ));
        let i16_field = FieldSource::Definition(res.push_field(
            type_idx,
            new_static_field("I16", MemberType::Base(Box::new(BaseType::Int16))),
        ));
        let u16_field = FieldSource::Definition(res.push_field(
            type_idx,
            new_static_field("U16", MemberType::Base(Box::new(BaseType::UInt16))),
        ));

        let sig = parameterless_i32_signature();

        let body = make_test_method!(
            max_stack: 4,
            instructions: vec![
                Instruction::LoadConstantInt32(-1),
                Instruction::Convert(ConversionType::Int8),
                Instruction::StoreStaticField {
                    param0: i8_field,
                    volatile: false,
                },
                Instruction::LoadConstantInt32(-1),
                Instruction::Convert(ConversionType::UInt8),
                Instruction::StoreStaticField {
                    param0: u8_field,
                    volatile: false,
                },
                Instruction::LoadConstantInt32(-2),
                Instruction::Convert(ConversionType::Int16),
                Instruction::StoreStaticField {
                    param0: i16_field,
                    volatile: false,
                },
                Instruction::LoadConstantInt32(-2),
                Instruction::Convert(ConversionType::UInt16),
                Instruction::StoreStaticField {
                    param0: u16_field,
                    volatile: false,
                },
                Instruction::LoadStaticField {
                    param0: i8_field,
                    volatile: false,
                },
                Instruction::LoadStaticField {
                    param0: u8_field,
                    volatile: false,
                },
                Instruction::Add,
                Instruction::LoadStaticField {
                    param0: i16_field,
                    volatile: false,
                },
                Instruction::Add,
                Instruction::LoadStaticField {
                    param0: u16_field,
                    volatile: false,
                },
                Instruction::Add,
                Instruction::Return,
            ],
        );

        let method_idx = res.push_method(
            type_idx,
            Method::new(Accessibility::Public, sig, "Run", Some(body)),
        );

        let res_s = loader.register_owned_assembly(res);
        let method_def = &res_s.definition()[method_idx];
        let td = TypeDescription::new(res_s.clone(), type_idx);
        let method_index = td
            .definition()
            .methods
            .iter()
            .position(|m| std::ptr::eq(m, method_def))
            .expect("method must exist in type definition");
        let entrypoint = MethodDescription::new(
            td,
            GenericLookup::default(),
            res_s,
            MethodMemberIndex::Method(method_index),
        );

        let result = run_i32_entrypoint(shared, entrypoint);
        assert_eq!(result, 65786);
    }

    #[test]
    fn static_fields_round_trip_object_references() {
        let loader = get_mock_loader();
        let shared = Arc::new(SharedGlobalState::new(loader.clone()));

        let (mut res, _, type_idx) = make_test_assembly!(
            "StaticConvRefs.dll",
            "StaticConvRefsAssembly",
            "StaticConvRefsType"
        );

        let object_field = FieldSource::Definition(res.push_field(
            type_idx,
            new_static_field("Obj", MemberType::Base(Box::new(BaseType::Object))),
        ));

        let sig = parameterless_i32_signature();

        let body = make_test_method!(
            max_stack: 2,
            instructions: vec![
                Instruction::LoadNull,
                Instruction::StoreStaticField {
                    param0: object_field,
                    volatile: false,
                },
                Instruction::LoadStaticField {
                    param0: object_field,
                    volatile: false,
                },
                Instruction::LoadNull,
                Instruction::CompareEqual,
                Instruction::Return,
            ],
        );

        let method_idx = res.push_method(
            type_idx,
            Method::new(Accessibility::Public, sig, "Run", Some(body)),
        );

        let res_s = loader.register_owned_assembly(res);
        let method_def = &res_s.definition()[method_idx];
        let td = TypeDescription::new(res_s.clone(), type_idx);
        let method_index = td
            .definition()
            .methods
            .iter()
            .position(|m| std::ptr::eq(m, method_def))
            .expect("method must exist in type definition");
        let entrypoint = MethodDescription::new(
            td,
            GenericLookup::default(),
            res_s,
            MethodMemberIndex::Method(method_index),
        );

        let result = run_i32_entrypoint(shared, entrypoint);
        assert_eq!(result, 1);
    }
}

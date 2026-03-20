#[cfg(test)]
mod tests {
    use crate::{
        StepResult,
        dispatch::ExecutionEngine,
        stack::ops::CallOps,
        stack::{CallStack, GCArena},
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
            members::{Method, MethodSource, UserMethod},
            signature::ReturnType,
            types::{ExternalTypeReference, ResolutionScope, TypeDefinition},
        },
    };
    use std::sync::OnceLock;
    static MOCK_LOADER: OnceLock<Arc<AssemblyLoader>> = OnceLock::new();
    fn get_mock_loader() -> Arc<AssemblyLoader> {
        MOCK_LOADER
            .get_or_init(|| {
                let loader = AssemblyLoader::new_bare("mock_root_jmp".to_string())
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
            })
            .clone()
    }
    #[test]
    fn test_jmp_instruction() {
        let loader = get_mock_loader();
        let shared = Arc::new(SharedGlobalState::new(loader.clone()));
        let mut res = Resolution::new(Module::new("JmpTest.dll"));
        res.assembly = Some(Assembly::new("JmpTestAssembly"));
        let system_runtime =
            res.push_assembly_reference(ExternalAssemblyReference::new("System.Runtime"));
        let object_type_ref = res.push_type_reference(ExternalTypeReference::new(
            Some("System".into()),
            "Object",
            ResolutionScope::Assembly(system_runtime),
        ));
        let mut type_def = TypeDefinition::new(None, "JmpType");
        type_def.extends = Some(object_type_ref.into());
        let type_idx = res.push_type_definition(type_def);
        let sig = MethodSignature {
            instance: false,
            explicit_this: false,
            calling_convention: CallingConvention::Default,
            parameters: vec![dotnetdll::resolved::signature::Parameter(
                vec![],
                dotnetdll::prelude::ParameterType::Value(
                    dotnetdll::resolved::types::MethodType::Base(Box::new(
                        dotnetdll::resolved::types::BaseType::Int32,
                    )),
                ),
            )],
            return_type: ReturnType(
                vec![],
                Some(dotnetdll::prelude::ParameterType::Value(
                    dotnetdll::resolved::types::MethodType::Base(Box::new(
                        dotnetdll::resolved::types::BaseType::Int32,
                    )),
                )),
            ),
            varargs: None,
        };
        let target_body = body::Method {
            header: body::Header {
                maximum_stack_size: 2,
                local_variables: vec![],
                initialize_locals: true,
            },
            instructions: vec![
                Instruction::LoadArgument(0),
                Instruction::LoadConstantInt32(10),
                Instruction::Add,
                Instruction::Return,
            ],
            data_sections: vec![],
        };
        let target_idx = res.push_method(
            type_idx,
            Method::new(
                resolved_mod::Accessibility::Public,
                sig.clone(),
                "Target",
                Some(target_body),
            ),
        );
        let jumper_body = body::Method {
            header: body::Header {
                maximum_stack_size: 0,
                local_variables: vec![],
                initialize_locals: true,
            },
            instructions: vec![Instruction::Jump(MethodSource::User(
                UserMethod::Definition(target_idx),
            ))],
            data_sections: vec![],
        };
        let jumper_idx = res.push_method(
            type_idx,
            Method::new(
                resolved_mod::Accessibility::Public,
                sig.clone(),
                "Jumper",
                Some(jumper_body),
            ),
        );
        let res_s = loader.register_owned_assembly(res);
        let _typedef = &res_s.definition()[type_idx];
        let method_def = &res_s.definition()[jumper_idx];
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
                .entrypoint_frame(
                    info,
                    Default::default(),
                    vec![dotnet_value::StackValue::Int32(42)],
                )
                .expect("Failed to set up entrypoint frame");
        });
        let mut max_depth = 0usize;
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
                max_depth = max_depth.max(engine.stack.execution.frame_stack.len());
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
        assert_eq!(final_int, Some(52));
        assert_eq!(max_depth, 1);
    }
    #[test]
    fn test_jmp_invalid_stack() {
        let loader = get_mock_loader();
        let shared = Arc::new(SharedGlobalState::new(loader.clone()));
        let mut res = Resolution::new(Module::new("JmpInvalid.dll"));
        res.assembly = Some(Assembly::new("JmpInvalidAssembly"));
        let system_runtime =
            res.push_assembly_reference(ExternalAssemblyReference::new("System.Runtime"));
        let object_type_ref = res.push_type_reference(ExternalTypeReference::new(
            Some("System".into()),
            "Object",
            ResolutionScope::Assembly(system_runtime),
        ));
        let mut type_def = TypeDefinition::new(None, "JmpInvalidType");
        type_def.extends = Some(object_type_ref.into());
        let type_idx = res.push_type_definition(type_def);
        let sig = MethodSignature {
            instance: false,
            explicit_this: false,
            calling_convention: CallingConvention::Default,
            parameters: vec![],
            return_type: ReturnType(vec![], None),
            varargs: None,
        };
        let target_idx = res.push_method(
            type_idx,
            Method::new(
                resolved_mod::Accessibility::Public,
                sig.clone(),
                "Target",
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
        let jumper_body = body::Method {
            header: body::Header {
                maximum_stack_size: 1,
                local_variables: vec![],
                initialize_locals: true,
            },
            instructions: vec![
                Instruction::LoadConstantInt32(42),
                Instruction::Jump(MethodSource::User(UserMethod::Definition(target_idx))),
            ],
            data_sections: vec![],
        };
        let jumper_idx = res.push_method(
            type_idx,
            Method::new(
                resolved_mod::Accessibility::Public,
                sig.clone(),
                "Jumper",
                Some(jumper_body),
            ),
        );
        let res_s = loader.register_owned_assembly(res);
        let _typedef = &res_s.definition()[type_idx];
        let method_def = &res_s.definition()[jumper_idx];
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
                .unwrap();
        });
        let mut step_res = StepResult::Continue;
        for _ in 0..10 {
            step_res = arena.mutate_root(|gc, engine| {
                let gc_handle = GCHandle::new(
                    gc,
                    #[cfg(feature = "multithreading")]
                    unsafe {
                        engine.stack.arena_inner_gc()
                    },
                    #[cfg(feature = "memory-validation")]
                    thread_id,
                );
                engine.step(gc_handle)
            });
            if matches!(step_res, StepResult::Error(_)) {
                break;
            }
        }
        match step_res {
            StepResult::Error(crate::error::VmError::Execution(
                crate::error::ExecutionError::Aborted(msg),
            )) => {
                assert!(msg.contains("jmp requires empty evaluation stack"));
            }
            _ => panic!("Expected stack height mismatch error, got {:?}", step_res),
        }
    }
    #[test]
    fn test_jmp_inside_try() {
        let loader = get_mock_loader();
        let shared = Arc::new(SharedGlobalState::new(loader.clone()));
        let mut res = Resolution::new(Module::new("JmpTry.dll"));
        res.assembly = Some(Assembly::new("JmpTryAssembly"));
        let system_runtime =
            res.push_assembly_reference(ExternalAssemblyReference::new("System.Runtime"));
        let object_type_ref = res.push_type_reference(ExternalTypeReference::new(
            Some("System".into()),
            "Object",
            ResolutionScope::Assembly(system_runtime),
        ));
        let mut type_def = TypeDefinition::new(None, "JmpTryType");
        type_def.extends = Some(object_type_ref.into());
        let type_idx = res.push_type_definition(type_def);
        let sig = MethodSignature {
            instance: false,
            explicit_this: false,
            calling_convention: CallingConvention::Default,
            parameters: vec![],
            return_type: ReturnType(vec![], None),
            varargs: None,
        };
        let target_idx = res.push_method(
            type_idx,
            Method::new(
                resolved_mod::Accessibility::Public,
                sig.clone(),
                "Target",
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
        let jumper_body = body::Method {
            header: body::Header {
                maximum_stack_size: 1,
                local_variables: vec![],
                initialize_locals: true,
            },
            instructions: vec![
                Instruction::Jump(MethodSource::User(UserMethod::Definition(target_idx))),
                Instruction::Return,
                Instruction::Return,
            ],
            data_sections: vec![body::DataSection::ExceptionHandlers(vec![
                body::Exception {
                    kind: body::ExceptionKind::Finally,
                    try_offset: 0,
                    try_length: 1,
                    handler_offset: 2,
                    handler_length: 1,
                },
            ])],
        };
        let jumper_idx = res.push_method(
            type_idx,
            Method::new(
                resolved_mod::Accessibility::Public,
                sig.clone(),
                "Jumper",
                Some(jumper_body),
            ),
        );
        let res_s = loader.register_owned_assembly(res);
        let _typedef = &res_s.definition()[type_idx];
        let method_def = &res_s.definition()[jumper_idx];
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
                .unwrap();
        });
        let mut step_res = StepResult::Continue;
        for _ in 0..10 {
            step_res = arena.mutate_root(|gc, engine| {
                let gc_handle = GCHandle::new(
                    gc,
                    #[cfg(feature = "multithreading")]
                    unsafe {
                        engine.stack.arena_inner_gc()
                    },
                    #[cfg(feature = "memory-validation")]
                    thread_id,
                );
                engine.step(gc_handle)
            });
            if matches!(step_res, StepResult::Error(_)) {
                break;
            }
        }
        match step_res {
            StepResult::Error(crate::error::VmError::Execution(
                crate::error::ExecutionError::Aborted(msg),
            )) => {
                assert!(msg.contains("jmp out of try/catch/finally block"));
            }
            _ => panic!(
                "Expected invalid control transfer error, got {:?}",
                step_res
            ),
        }
    }
    #[test]
    fn test_jmp_signature_mismatch() {
        let loader = get_mock_loader();
        let shared = Arc::new(SharedGlobalState::new(loader.clone()));
        let mut res = Resolution::new(Module::new("JmpSig.dll"));
        res.assembly = Some(Assembly::new("JmpSigAssembly"));
        let system_runtime =
            res.push_assembly_reference(ExternalAssemblyReference::new("System.Runtime"));
        let object_type_ref = res.push_type_reference(ExternalTypeReference::new(
            Some("System".into()),
            "Object",
            ResolutionScope::Assembly(system_runtime),
        ));
        let mut type_def = TypeDefinition::new(None, "JmpSigType");
        type_def.extends = Some(object_type_ref.into());
        let type_idx = res.push_type_definition(type_def);
        let sig_void = MethodSignature {
            instance: false,
            explicit_this: false,
            calling_convention: CallingConvention::Default,
            parameters: vec![],
            return_type: ReturnType(vec![], None),
            varargs: None,
        };
        let sig_int = MethodSignature {
            instance: false,
            explicit_this: false,
            calling_convention: CallingConvention::Default,
            parameters: vec![dotnetdll::resolved::signature::Parameter(
                vec![],
                dotnetdll::prelude::ParameterType::Value(
                    dotnetdll::resolved::types::MethodType::Base(Box::new(
                        dotnetdll::resolved::types::BaseType::Int32,
                    )),
                ),
            )],
            return_type: ReturnType(
                vec![],
                Some(dotnetdll::prelude::ParameterType::Value(
                    dotnetdll::resolved::types::MethodType::Base(Box::new(
                        dotnetdll::resolved::types::BaseType::Int32,
                    )),
                )),
            ),
            varargs: None,
        };
        let target_idx = res.push_method(
            type_idx,
            Method::new(
                resolved_mod::Accessibility::Public,
                sig_int,
                "Target",
                Some(body::Method {
                    header: body::Header {
                        maximum_stack_size: 1,
                        local_variables: vec![],
                        initialize_locals: true,
                    },
                    instructions: vec![Instruction::LoadConstantInt32(1), Instruction::Return],
                    data_sections: vec![],
                }),
            ),
        );
        let jumper_body = body::Method {
            header: body::Header {
                maximum_stack_size: 0,
                local_variables: vec![],
                initialize_locals: true,
            },
            instructions: vec![Instruction::Jump(MethodSource::User(
                UserMethod::Definition(target_idx),
            ))],
            data_sections: vec![],
        };
        let jumper_idx = res.push_method(
            type_idx,
            Method::new(
                resolved_mod::Accessibility::Public,
                sig_void,
                "Jumper",
                Some(jumper_body),
            ),
        );
        let res_s = loader.register_owned_assembly(res);
        let _typedef = &res_s.definition()[type_idx];
        let method_def = &res_s.definition()[jumper_idx];
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
                .unwrap();
        });
        let mut step_res = StepResult::Continue;
        for _ in 0..10 {
            step_res = arena.mutate_root(|gc, engine| {
                let gc_handle = GCHandle::new(
                    gc,
                    #[cfg(feature = "multithreading")]
                    unsafe {
                        engine.stack.arena_inner_gc()
                    },
                    #[cfg(feature = "memory-validation")]
                    thread_id,
                );
                engine.step(gc_handle)
            });
            if matches!(step_res, StepResult::Error(_)) {
                break;
            }
        }
        match step_res {
            StepResult::Error(crate::error::VmError::Execution(
                crate::error::ExecutionError::Aborted(msg),
            )) => {
                assert!(msg.contains("jmp signature mismatch"));
            }
            _ => panic!("Expected signature mismatch error, got {:?}", step_res),
        }
    }
}

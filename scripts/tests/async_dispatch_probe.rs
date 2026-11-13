#[test]
fn async_probe_dispatch_modes() {
    use crate::stack::ops::{UnifiedDispatchTarget, VmCallOps};
    for name in ["First", "Unselected", "Nested", "BodyOnly", "SelectorOnly"] {
        for mode in ["inline", "vm-context", "tail"] {
            let mut executor = executor();
            let mut res = Resolution::new(Module::new("AsyncProbe.dll"));
            res.assembly = Some(Assembly::new("AsyncProbe"));
            let mut ty = TypeDefinition::new(None, "AsyncProbe");
            ty.namespace = Some("DotnetRs".into());
            ty.flags.before_field_init = true;
            let index = res.push_type_definition(ty);
            let mut signature = parameterless_i32_signature();
            signature.return_type = dotnetdll::resolved::signature::ReturnType(vec![], None);
            if name == "Nested" {
                res.push_method(
                    index,
                    Method::new(Accessibility::Public, signature.clone(), "Unselected", None),
                );
            }
            let method_index = res.push_method(
                index,
                Method::new(Accessibility::Public, signature, name, None),
            );
            let res = executor.shared.loader.register_owned_assembly(res);
            let ty = TypeDescription::new(res.clone(), index);
            let index = ty
                .definition()
                .methods
                .iter()
                .position(|m| std::ptr::eq(m, &res.definition()[method_index]))
                .unwrap();
            let method = MethodDescription::new(
                ty,
                GenericLookup::default(),
                res,
                MethodMemberIndex::Method(index),
            );
            let result = executor
                .with_arena(|arena| {
                    arena.mutate_root(|gc, engine| {
                        // SAFETY: F1.GcHandleRooted — the engine is the live arena root for this mutation.
                        let gc = unsafe { test_gc_handle(gc, engine) };
                        let mut ctx = engine.ves_context(gc);
                        let target = UnifiedDispatchTarget::Resolved(
                            method.clone(),
                            GenericLookup::default(),
                        );
                        match mode {
                            "inline" => ctx.unified_dispatch(target, None, None),
                            "vm-context" => {
                                ctx.dispatch_method(method.clone(), GenericLookup::default())
                            }
                            "tail" => ctx.unified_dispatch_tail(target, None, None),
                            _ => unreachable!("unexpected dispatch mode {mode}"),
                        }
                    })
                })
                .unwrap();
            let non_inline_async =
                matches!(name, "BodyOnly" | "SelectorOnly") && mode == "vm-context";
            let async_started = matches!(mode, "inline" | "tail") && name != "Nested";
            assert_eq!(
                result,
                if non_inline_async {
                    StepResult::not_implemented(format!(
                        "async intrinsic {method:?} reached through non-inline dispatch"
                    ))
                } else if async_started {
                    StepResult::Yield
                } else {
                    StepResult::Continue
                },
                "{name} {mode}"
            );
            executor
                .with_arena(|arena| {
                    arena.mutate(|_, engine| {
                        assert_eq!(
                            !engine.stack.execution.call_state.native.is_empty(),
                            async_started
                        );
                        assert!(engine.stack.execution.evaluation_stack.stack.is_empty());
                    })
                })
                .unwrap();
        }
    }
}

#[test]
fn async_probe_il_routes() {
    use crate::stack::ops::VmCallOps;
    use dotnet_vm_ops::MemoryOps;
    use dotnetdll::resolved::{
        members::{MethodSource, UserMethod},
        types::{TypeSource, UserType, ValueKind},
    };
    for route in ["constrained", "constrained_virtual", "tail", "abstract"] {
        let (mut executor, _) = boundary_executor(false);
        let mut res = Resolution::new(Module::new("AsyncProbeRoutes.dll"));
        res.assembly = Some(Assembly::new("AsyncProbeRoutes"));
        let mut base = TypeDefinition::new(None, "Base");
        base.flags.before_field_init = true;
        let base_idx = res.push_type_definition(base);
        let mut ty = TypeDefinition::new(None, "AsyncProbe");
        ty.namespace = Some("DotnetRs".into());
        ty.flags.before_field_init = true;
        ty.extends = Some(base_idx.into());
        let ty_idx = res.push_type_definition(ty);
        let mut sig = parameterless_i32_signature();
        sig.return_type = ReturnType(vec![], None);
        sig.parameters.push(Parameter(
            vec![],
            ParameterType::Value(BaseType::Int32.into()),
        ));
        let static_idx = res.push_method(
            ty_idx,
            Method::new(Accessibility::Public, sig.clone(), "WithArg", None),
        );
        sig.instance = true;
        let mut abstract_method = Method::new(Accessibility::Public, sig.clone(), "Virtual", None);
        abstract_method.virtual_member = true;
        abstract_method.abstract_member = true;
        let abstract_idx = res.push_method(base_idx, abstract_method);
        let mut implementation = Method::new(Accessibility::Public, sig, "Virtual", None);
        implementation.virtual_member = true;
        let virtual_idx = res.push_method(ty_idx, implementation);
        let constraint: MethodType = BaseType::Type {
            source: TypeSource::User(UserType::Definition(ty_idx)),
            value_kind: Some(ValueKind::Class),
        }
        .into();
        let source = |idx| MethodSource::User(UserMethod::Definition(idx));
        let instruction = match route {
            "constrained" => Instruction::CallConstrained(constraint, source(static_idx)),
            "constrained_virtual" => {
                Instruction::CallVirtualConstrained(constraint, source(virtual_idx))
            }
            "tail" => Instruction::Call {
                tail_call: true,
                param0: source(static_idx),
            },
            _ => Instruction::Call {
                tail_call: false,
                param0: source(abstract_idx),
            },
        };
        let mut caller_sig = parameterless_i32_signature();
        caller_sig.return_type = ReturnType(vec![], None);
        let caller_idx = res.push_method(ty_idx, Method::new(Accessibility::Public, caller_sig, "Caller", Some(make_test_method!(max_stack: 4, instructions: vec![instruction, Instruction::NoOperation, Instruction::Return]))));
        let res = executor.shared.loader.register_owned_assembly(res);
        let ty = TypeDescription::new(res.clone(), ty_idx);
        let idx = ty
            .definition()
            .methods
            .iter()
            .position(|m| std::ptr::eq(m, &res.definition()[caller_idx]))
            .unwrap();
        let caller = MethodDescription::new(
            ty.clone(),
            GenericLookup::default(),
            res,
            MethodMemberIndex::Method(idx),
        );
        let info = executor
            .shared
            .caches
            .get_method_info(caller, &GenericLookup::default(), executor.shared.clone())
            .unwrap();
        executor
            .with_arena(|arena| {
                arena.mutate_root(|gc, engine| {
                    // SAFETY: F1.GcHandleRooted — the engine remains the live arena root.
                    let gc = unsafe { test_gc_handle(gc, engine) };
                    let mut ctx = engine.ves_context(gc);
                    ctx.entrypoint_frame(info, GenericLookup::default(), vec![])
                        .unwrap();
                    ctx.push(StackValue::Int32(77));
                    if matches!(route, "constrained_virtual" | "abstract") {
                        let object = ctx.new_object(ty).unwrap();
                        let object = ctx.alloc_obj_ref(gc, object);
                        ctx.push(StackValue::ObjectRef(object));
                    }
                    ctx.push(StackValue::Int32(42));
                })
            })
            .unwrap();
        assert_eq!(resume(&mut executor), StepResult::Yield, "{route}");
        executor
            .with_arena(|arena| {
                arena.mutate(|_, engine| {
                    let execution = &engine.stack.execution;
                    assert_eq!(execution.frame_stack.len(), 2, "{route}");
                    assert_eq!(execution.frame_stack.current_frame().state.ip, 0, "{route}");
                    assert_eq!(execution.call_state.native.len(), 1, "{route}");
                    let native = &execution.call_state.native[0];
                    assert_eq!(native.caller_depth, 2, "{route}");
                    assert_eq!(native.caller_ip, 0, "{route}");
                    assert_eq!(
                        native.operands.len(),
                        if matches!(route, "constrained_virtual" | "abstract") {
                            2
                        } else {
                            1
                        },
                        "{route}"
                    );
                    assert!(
                        matches!(native.operands.last(), Some(StackValue::Int32(42))),
                        "{route}"
                    );
                    assert!(
                        matches!(
                            execution.evaluation_stack.stack.last(),
                            Some(StackValue::Int32(77))
                        ),
                        "{route}"
                    );
                    assert_eq!(
                        native.stack_height,
                        execution.evaluation_stack.top_of_stack(),
                        "{route}"
                    );
                })
            })
            .unwrap();
        assert_eq!(resume(&mut executor), StepResult::Continue, "{route}");
        executor
            .with_arena(|arena| {
                arena.mutate(|_, engine| {
                    let execution = &engine.stack.execution;
                    assert_eq!(execution.frame_stack.len(), 2, "{route}");
                    assert_eq!(execution.frame_stack.current_frame().state.ip, 1, "{route}");
                    assert!(execution.call_state.native.is_empty(), "{route}");
                    assert!(
                        matches!(
                            execution.evaluation_stack.stack.last(),
                            Some(StackValue::Int32(77))
                        ),
                        "{route}"
                    );
                })
            })
            .unwrap();
        assert_eq!(resume(&mut executor), StepResult::Continue, "{route}");
        executor
            .with_arena(|arena| {
                arena.mutate(|_, engine| {
                    let execution = &engine.stack.execution;
                    assert_eq!(execution.frame_stack.current_frame().state.ip, 2, "{route}");
                    assert!(execution.call_state.native.is_empty(), "{route}");
                    assert!(
                        matches!(
                            execution.evaluation_stack.stack.last(),
                            Some(StackValue::Int32(77))
                        ),
                        "{route}"
                    );
                })
            })
            .unwrap();
    }
}

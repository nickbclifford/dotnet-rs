
#[test]
fn async_probe_dispatch_modes() {
    use crate::stack::ops::{UnifiedDispatchTarget, VmCallOps};
    for name in ["First", "Unselected", "Nested"] {
        for mode in ["direct", "ordinary", "tail", "engine"] {
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
                res.push_method(index, Method::new(Accessibility::Public, signature.clone(), "Unselected", None));
            }
            let method_index = res.push_method(index, Method::new(Accessibility::Public, signature, name, None));
            let res = executor.shared.loader.register_owned_assembly(res);
            let ty = TypeDescription::new(res.clone(), index);
            let index = ty.definition().methods.iter().position(|m| std::ptr::eq(m, &res.definition()[method_index])).unwrap();
            let method = MethodDescription::new(ty, GenericLookup::default(), res, MethodMemberIndex::Method(index));
            let result = executor.with_arena(|arena| arena.mutate_root(|gc, engine| {
                // SAFETY: F1.GcHandleRooted — the engine is the live arena root for this mutation.
                let gc = unsafe { test_gc_handle(gc, engine) };
                let mut ctx = engine.ves_context(gc);
                let target = UnifiedDispatchTarget::Resolved(method.clone(), GenericLookup::default());
                match mode {
                    "direct" => ctx.unified_dispatch(target, None, None),
                    "ordinary" => ctx.unified_dispatch_for_engine_api(target, None, None),
                    "tail" => ctx.unified_dispatch_tail(target, None, None),
                    _ => ctx.dispatch_resolved_method_for_engine_api(method, GenericLookup::default()),
                }
            })).unwrap();
            assert_eq!(result, if mode == "direct" && name != "Nested" { StepResult::Yield } else { StepResult::Continue }, "{name} {mode}");
            executor.with_arena(|arena| arena.mutate(|_, engine| {
                assert_eq!(!engine.stack.execution.call_state.native.is_empty(), mode == "direct" && name != "Nested");
                assert!(engine.stack.execution.evaluation_stack.stack.is_empty());
            })).unwrap();
        }
    }
}

use super::*;
use effect_intrinsics::{
    InvokeSpec, ReturnKind, region_checkpoint, region_raii, region_vm_await, region_with_vm,
};
use std::{cell::Cell, rc::Rc};

#[test]
fn region_api_candidates_keep_chunk_progress() {
    for variant in 0..4 {
        let mut vm = Vm::new();
        vm.arena.mutate_root(|_, r| r.backend.force_stop = true);
        let count = 65536 + 3;
        vm.spawn(move |mut fx| {
            Box::pin(async move {
                let source = fx.enter(|cx| cx.string(vec![65; count + 2]));
                match variant {
                    0 => region_with_vm::substring(fx, source, 1, Some(count as i32)).await,
                    1 => region_vm_await::substring(fx, source, 1, Some(count as i32)).await,
                    2 => region_checkpoint::substring(fx, source, 1, Some(count as i32)).await,
                    _ => region_raii::substring(fx, source, 1, Some(count as i32)).await,
                }
            })
        });
        vm.run();
        assert_eq!(vm.result_string(), vec![65; count]);
        vm.arena.mutate(|_, r| assert_eq!(r.backend.copied, count));
        // Explicit checkpoints run between chunks; automatic checkpoints run before
        // validation, each chunk, and allocation. No copied prefix is repeated.
        assert_eq!(vm.collections, if variant >= 2 { 4 } else { 1 });
    }
}

#[test]
fn region_requests_complete_inline_without_a_stop() {
    let arena = Arena::<Rootable![Root<'_>]>::new(|_| Root::new());
    for variant in 0..4 {
        arena.mutate(|mc, _| {
            let mut backend = Backend::default();
            let mut frame = NativeFrame::new(mc, move |mut fx| {
                Box::pin(async move {
                    let n = match variant {
                        0 => fx.with_vm(|_| 42),
                        1 => fx.vm_ready(|_| 42).await,
                        2 => fx.checkpoint_vm(|_| Ok(42)).await?,
                        _ => {
                            let mut vm = fx.vm().await?;
                            vm.with(|_| 42)
                        }
                    };
                    Ok(Stashed::Int(n))
                })
            });
            assert!(matches!(
                frame.poll(mc, &mut backend),
                Poll::Ready(Ok(Stashed::Int(42)))
            ));
        });
    }
}

#[test]
fn checkpoint_defers_callback_until_after_collection() {
    let mut vm = Vm::new();
    vm.arena.mutate_root(|_, r| r.backend.force_stop = true);
    let called = Rc::new(Cell::new(false));
    let observed = called.clone();
    vm.spawn(move |mut fx| {
        Box::pin(async move {
            fx.checkpoint_vm(|cx| {
                called.set(true);
                Ok(cx.string(vec![90]).0)
            })
            .await
        })
    });
    assert!(vm.step());
    assert_eq!(vm.collections, 1);
    assert!(
        !observed.get(),
        "callback must not run before honoring the stop"
    );
    vm.run();
    assert!(observed.get());
    assert_eq!(vm.result_string(), vec![90]);
}

#[test]
fn async_closure_can_lend_a_guard_to_an_immediately_ready_helper() {
    let mut vm = Vm::new();
    vm.spawn(|mut fx| {
        Box::pin(async move {
            let source = fx.enter(|cx| cx.string(vec![65]));
            let count = fx
                .vm_async(async |cx| {
                    let _scope = cx.scope();
                    let source = source.get(cx);
                    let source = source.borrow();
                    let ObjectData::String(chars) = &*source else {
                        panic!()
                    };
                    std::future::ready(chars.len()).await
                })
                .await;
            fx.safe_point().await?;
            Ok(Stashed::Int(count as i64))
        })
    });
    vm.run();
    assert_eq!(vm.result_int(), Ok(1));
}

#[test]
fn async_closure_pending_rejects_suspension_and_drops_real_scope_guard() {
    let arena = Arena::<Rootable![Root<'_>]>::new(|_| Root::new());
    arena.mutate(|mc, _| {
        let mut backend = Backend::default();
        let mut frame = NativeFrame::new(mc, |mut fx| {
            Box::pin(async move {
                fx.vm_async(async |cx| {
                    let _scope = cx.scope();
                    std::future::pending::<()>().await;
                })
                .await;
                Ok(Stashed::Void)
            })
        });
        let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = frame.poll(mc, &mut backend);
        }))
        .expect_err("pending callback must panic before leaving the region");
        assert!(
            panic
                .downcast_ref::<&str>()
                .is_some_and(|s| s.contains("async VM region attempted to suspend"))
        );
        assert_eq!(backend.scopes.get(), 0, "scope must drop on rejection");
    });
}

#[test]
fn reflection_completion_and_throw_work_with_each_region_api() {
    for variant in 0..4 {
        for method in [3, 5] {
            let mut vm = Vm::new();
            vm.arena.mutate_root(|_, r| r.backend.force_stop = true);
            vm.spawn(move |mut fx| {
                Box::pin(async move {
                    let parameters = fx.enter(|cx| cx.array(vec![Value::Int(1)]));
                    let spec = InvokeSpec {
                        method,
                        receiver: Stashed::Null,
                        parameters,
                        byref_indices: vec![0],
                        reference_byref_indices: vec![],
                        return_kind: ReturnKind::Value,
                    };
                    match variant {
                        0 => region_with_vm::reflection_invoke(fx, spec).await,
                        1 => region_vm_await::reflection_invoke(fx, spec).await,
                        2 => region_checkpoint::reflection_invoke(fx, spec).await,
                        _ => region_raii::reflection_invoke(fx, spec).await,
                    }
                })
            });
            vm.run();
            vm.arena.mutate(|_, root| {
                if method == 5 {
                    assert!(matches!(
                        root.result,
                        Some(Err(Failure::Managed("callee failure")))
                    ));
                } else {
                    let Some(Ok(Value::Object(o))) = root.result else {
                        panic!("boxed return")
                    };
                    assert!(matches!(*o.borrow(), ObjectData::Boxed(7)));
                }
            });
        }
    }
}

#[test]
fn raii_access_can_span_a_ready_await_then_release_before_a_vm_effect() {
    let mut vm = Vm::new();
    vm.spawn(|mut fx| {
        Box::pin(async move {
            let source = fx.enter(|cx| cx.string(vec![65, 66]));
            let count = {
                let mut vm = fx.vm().await?;
                let mut source = vm.string(&source);
                std::future::ready(()).await;
                source.substring_len(0, None)?
            };
            fx.safe_point().await?;
            Ok(Stashed::Int(count as i64))
        })
    });
    vm.run();
    assert_eq!(vm.result_int(), Ok(2));
}

#[test]
fn forgetting_the_access_guard_does_not_forget_a_heap_borrow() {
    let mut vm = Vm::new();
    vm.spawn(|mut fx| {
        Box::pin(async move {
            let mut access = fx.borrow_vm();
            let source = access.new_string(vec![65]);
            std::mem::forget(access);
            // No Gc or lock is stored in the forgotten guard: it was an effects loan.
            fx.safe_point().await?;
            Ok(source)
        })
    });
    vm.run();
    assert_eq!(vm.result_string(), vec![65]);
}

#[test]
fn escaped_access_capability_cannot_touch_a_destroyed_arena() {
    let escaped = Rc::new(std::cell::RefCell::new(None));
    let source = Rc::new(std::cell::RefCell::new(None));
    {
        let mut vm = Vm::new();
        let saved_fx = escaped.clone();
        let saved_source = source.clone();
        vm.spawn(move |mut fx| {
            Box::pin(async move {
                *saved_source.borrow_mut() = Some(StringRoot(fx.borrow_vm().new_string(vec![65])));
                *saved_fx.borrow_mut() = Some(fx);
                Ok(Stashed::Void)
            })
        });
        vm.run();
        vm.full_gc();
    }
    let mut escaped = escaped.borrow_mut();
    let fx = escaped.as_mut().unwrap();
    let source = source.borrow();
    let mut access = fx.borrow_vm();
    let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = access
            .string(source.as_ref().unwrap())
            .substring_len(0, None);
    }))
    .expect_err("a view cannot access heap state outside a poll");
    assert!(
        panic
            .downcast_ref::<&str>()
            .is_some_and(|s| s.contains("region outside poll"))
    );
}

#[test]
fn cancelled_guard_acquisition_releases_roots_without_entering_the_scope() {
    for reason in [Failure::Aborted, Failure::Budget] {
        let mut vm = Vm::new();
        vm.arena.mutate_root(|_, r| r.backend.force_stop = true);
        let before = vm.arena.metrics().total_gc_allocation();
        let entered = Rc::new(Cell::new(false));
        let observed = entered.clone();
        vm.spawn(move |mut fx| {
            Box::pin(async move {
                let source = fx.enter(|cx| cx.string(vec![65]));
                let mut access = fx.vm().await?;
                entered.set(true);
                access.string(&source).substring_len(0, None)?;
                Ok(Stashed::Void)
            })
        });
        assert!(vm.step());
        assert!(!observed.get());
        vm.cancel(reason);
        vm.full_gc();
        assert!(!observed.get());
        assert_eq!(vm.arena.metrics().total_gc_allocation(), before);
    }
}

#[test]
fn ready_accesses_yield_a_bounded_quantum_without_a_gc_request() {
    for reason in [Failure::Aborted, Failure::Budget] {
        let mut vm = Vm::new();
        let completed = Rc::new(Cell::new(0));
        let observed = completed.clone();
        vm.spawn(move |mut fx| {
            Box::pin(async move {
                for _ in 0..VM_ACCESS_QUANTUM + 1 {
                    let mut access = fx.vm().await?;
                    access.with(|_| ());
                    completed.set(completed.get() + 1);
                }
                Ok(Stashed::Void)
            })
        });
        assert!(vm.step());
        assert_eq!(observed.get(), VM_ACCESS_QUANTUM);
        assert!(matches!(vm.waits.as_slice(), [Wait::SafePoint]));
        vm.cancel(reason);
        assert_eq!(observed.get(), VM_ACCESS_QUANTUM);
    }
    let mut vm = Vm::new();
    vm.spawn(|mut fx| {
        Box::pin(async move {
            for _ in 0..VM_ACCESS_QUANTUM + 1 {
                let _access = fx.vm().await?;
            }
            Ok(Stashed::Int(1))
        })
    });
    vm.run();
    assert_eq!(vm.result_int(), Ok(1));
    assert_eq!(vm.waits.len(), 1);
}

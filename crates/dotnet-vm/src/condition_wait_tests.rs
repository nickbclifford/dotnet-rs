use super::*;

fn condition_wait_method(executor: &Executor, object: bool) -> MethodDescription {
    use dotnetdll::{binary::signature::kinds::CallingConvention, resolved::types::TypeDefinition};
    let mut res = Resolution::new(Module::new("ConditionWait.dll"));
    res.assembly = Some(Assembly::new("ConditionWait"));
    let ty_index = res.push_type_definition(TypeDefinition::new(None, "ConditionWait"));
    let signature = MethodSignature {
        instance: false,
        explicit_this: false,
        calling_convention: CallingConvention::Default,
        parameters: if object {
            vec![Parameter(
                vec![],
                ParameterType::Value(MethodType::Base(Box::new(BaseType::String))),
            )]
        } else {
            vec![]
        },
        return_type: ReturnType(vec![], None),
        varargs: None,
    };
    let index = res.push_method(
        ty_index,
        Method::new(Accessibility::Public, signature, "Wait", None),
    );
    let res = executor.shared.loader.register_owned_assembly(res);
    let ty = TypeDescription::new(res.clone(), ty_index);
    let method = ty
        .definition()
        .methods
        .iter()
        .position(|m| std::ptr::eq(m, &res.definition()[index]))
        .unwrap();
    MethodDescription::new(
        ty,
        GenericLookup::default(),
        res,
        MethodMemberIndex::Method(method),
    )
}

fn condition_monitor_body(
    mut fx: dotnet_vm_ops::effects::VmEffects,
) -> dotnet_vm_ops::effects::NativeFuture {
    Box::pin(async move {
        let object = fx.vm().await?.pop_object()?;
        assert!(fx.monitor_enter(object, None).await?);
        Ok(())
    })
}

fn condition_finalizer_body(
    mut fx: dotnet_vm_ops::effects::VmEffects,
) -> dotnet_vm_ops::effects::NativeFuture {
    Box::pin(async move { fx.wait_finalizers().await })
}

#[test]
fn condition_wait_ready_monitor_and_finalizers_use_driver() {
    use crate::sync::SyncBlockOps;
    use dotnet_intrinsics_threading::MonitorHost;
    use dotnet_vm_ops::effects::{NativeDispatchMode, NativeIntrinsicHost};
    let mut executor = executor();
    let monitor_method = condition_wait_method(&executor, true);
    let finalizers_method = condition_wait_method(&executor, false);
    let thread = executor.thread_id;
    executor
        .with_arena(|arena| {
            arena.mutate_root(|gc, engine| {
                // SAFETY: F1.GcHandleRooted — the engine is the live arena root for this mutation.
                let gc = unsafe { test_gc_handle(gc, engine) };
                let source = ObjectRef::new(gc, HeapStorage::Str(CLRString::from("monitor")));
                let mut ctx = engine.ves_context(gc);
                let block = ctx.monitor_get_or_create_sync_block_for_object(source, gc);
                assert!(block.try_enter(thread));
                ctx.push(StackValue::ObjectRef(source));
                assert_eq!(
                    ctx.start_native(
                        monitor_method,
                        NativeDispatchMode::InlineCall,
                        &Default::default(),
                        None,
                        native_test_fallback,
                        condition_monitor_body
                    ),
                    StepResult::Jump(1)
                );
                assert!(block.exit(thread));
                assert!(block.exit(thread));
                assert!(!block.exit(thread));
                assert_eq!(
                    ctx.start_native(
                        finalizers_method,
                        NativeDispatchMode::InlineCall,
                        &Default::default(),
                        None,
                        native_test_fallback,
                        condition_finalizer_body
                    ),
                    StepResult::Jump(1)
                );
                assert!(ctx.call_state.native.is_empty());
            })
        })
        .unwrap();
}

#[cfg(feature = "multithreading")]
#[test]
fn condition_wait_release_before_recheck_keeps_ownership() {
    use crate::sync::SyncBlockOps;
    use dotnet_intrinsics_threading::MonitorHost;
    use dotnet_vm_ops::effects::{NativeDispatchMode, NativeIntrinsicHost};
    let mut executor = executor();
    let method = condition_wait_method(&executor, true);
    let owner = dotnet_utils::ArenaId::new(9999);
    let thread = executor.thread_id;
    let block = executor
        .with_arena(|arena| {
            arena.mutate_root(|gc, engine| {
                // SAFETY: F1.GcHandleRooted — the engine is the live arena root for this mutation.
                let gc = unsafe { test_gc_handle(gc, engine) };
                let source = ObjectRef::new(gc, HeapStorage::Str(CLRString::from("monitor")));
                let mut ctx = engine.ves_context(gc);
                let block = ctx.monitor_get_or_create_sync_block_for_object(source, gc);
                assert!(block.try_enter(owner));
                ctx.push(StackValue::ObjectRef(source));
                assert_eq!(
                    ctx.start_native(
                        method,
                        NativeDispatchMode::InlineCall,
                        &Default::default(),
                        None,
                        native_test_fallback,
                        condition_monitor_body
                    ),
                    StepResult::Yield
                );
                block
            })
        })
        .unwrap();
    collect(&mut executor);
    assert!(block.exit(owner));
    assert_eq!(resume(&mut executor), StepResult::Continue);
    assert!(!block.exit(owner));
    assert!(block.exit(thread));
    assert!(!block.exit(thread));
}

fn finalizer_wait_dispatch<'gc>(
    ctx: &mut crate::stack::VesContext<'_, 'gc>,
    method: MethodDescription,
    mode: dotnet_vm_ops::effects::NativeDispatchMode,
) -> StepResult {
    crate::intrinsics::dispatch_method_intrinsic(
        crate::intrinsics::MethodIntrinsicId::System_GC_WaitForPendingFinalizers_5109cdb4b3e16bc1,
        ctx,
        method,
        &Default::default(),
        mode,
    )
}

#[test]
fn finalizer_wait_ready_and_non_inline_dispatch() {
    use dotnet_vm_ops::effects::NativeDispatchMode;
    let mut executor = executor();
    let method = condition_wait_method(&executor, false);
    executor
        .with_arena(|arena| {
            arena.mutate_root(|gc, engine| {
                // SAFETY: F1.GcHandleRooted — the engine is the live arena root for this mutation.
                let gc = unsafe { test_gc_handle(gc, engine) };
                assert_eq!(
                    finalizer_wait_dispatch(
                        &mut engine.ves_context(gc),
                        method.clone(),
                        NativeDispatchMode::Other
                    ),
                    StepResult::Continue
                );
                assert_eq!(
                    finalizer_wait_dispatch(
                        &mut engine.ves_context(gc),
                        method,
                        NativeDispatchMode::InlineCall
                    ),
                    StepResult::Jump(1)
                );
                assert!(engine.stack.execution.call_state.native.is_empty());
                assert!(!engine.stack.local.heap.processing_finalizer.get());
            })
        })
        .unwrap();
}

#[test]
fn condition_wait_executor_runs_pending_finalizer_before_resuming() {
    finalizer_wait_executor(false);
}

#[test]
fn finalizer_wait_from_finalizer_returns_immediately() {
    finalizer_wait_executor(true);
}

fn finalizer_wait_executor(test_self_wait: bool) {
    use dotnet_vm_ops::effects::NativeDispatchMode;
    let mut executor = executor();
    let method = condition_wait_method(&executor, false);
    let mut res = Resolution::new(Module::new("mscorlib.dll"));
    res.assembly = Some(Assembly::new("mscorlib"));
    let mut object = TypeDefinition::new(None, "Object");
    object.namespace = Some("System".into());
    let ty_index = res.push_type_definition(object);
    for name in ["Enum", "ValueType"] {
        let mut stub = TypeDefinition::new(None, name);
        stub.namespace = Some("System".into());
        res.push_type_definition(stub);
    }
    let mut signature = parameterless_i32_signature();
    signature.instance = true;
    signature.return_type = ReturnType(vec![], None);
    let mut finalizer = Method::new(
        Accessibility::Public,
        signature,
        "Finalize",
        Some(
            make_test_method!(max_stack: 1, instructions: vec![Instruction::LoadConstantInt32(42), Instruction::Pop, Instruction::Return]),
        ),
    );
    finalizer.virtual_member = true;
    res.push_method(ty_index, finalizer);
    let res = executor.shared.loader.register_owned_assembly(res);
    let ty = TypeDescription::new(res, ty_index);
    executor
        .with_arena(|arena| {
            arena.mutate_root(|gc, engine| {
                // SAFETY: F1.GcHandleRooted — the engine is the live arena root for this mutation.
                let gc = unsafe { test_gc_handle(gc, engine) };
                let layout = dotnet_value::layout::FieldLayoutManager {
                    fields: Default::default(),
                    total_size: 0,
                    alignment: 1,
                    gc_desc: Default::default(),
                    has_ref_fields: false,
                };
                let queued = ObjectRef::new(
                    gc,
                    HeapStorage::Obj(Box::new(dotnet_value::object::Object::new(
                        ty,
                        GenericLookup::default(),
                        dotnet_value::storage::FieldStorage::new(Arc::new(layout), vec![]),
                    ))),
                );
                engine
                    .stack
                    .local
                    .heap
                    .pending_finalization
                    .borrow_mut()
                    .push(queued);
                engine.ves_context(gc).push(StackValue::Int32(0));
                assert_eq!(
                    finalizer_wait_dispatch(
                        &mut engine.ves_context(gc),
                        method.clone(),
                        NativeDispatchMode::Other
                    ),
                    StepResult::Yield
                );
                assert!(engine.stack.execution.call_state.native.is_empty());
                assert_eq!(
                    finalizer_wait_dispatch(
                        &mut engine.ves_context(gc),
                        method.clone(),
                        NativeDispatchMode::InlineCall
                    ),
                    StepResult::Yield
                );
                assert!(matches!(
                    engine
                        .stack
                        .execution
                        .call_state
                        .native
                        .last()
                        .unwrap()
                        .wait,
                    Some(crate::stack::native_effects::PendingWait::Finalizers(_))
                ));
                if test_self_wait {
                    assert_eq!(
                        engine.process_pending_finalizers(gc),
                        StepResult::FramePushed
                    );
                    assert!(engine.stack.local.heap.processing_finalizer.get());
                    assert!(engine.stack.parked_finalizer_caller.is_some());
                    let ip = engine.stack.execution.frame_stack.current_frame().state.ip;
                    for mode in [NativeDispatchMode::InlineCall, NativeDispatchMode::Other] {
                        assert_eq!(
                            finalizer_wait_dispatch(
                                &mut engine.ves_context(gc),
                                method.clone(),
                                mode
                            ),
                            StepResult::Continue
                        );
                        assert!(engine.stack.local.heap.processing_finalizer.get());
                        assert!(engine.stack.execution.call_state.native.is_empty());
                        assert_eq!(
                            engine.stack.execution.frame_stack.current_frame().state.ip,
                            ip
                        );
                        assert!(
                            engine
                                .ves_context(gc)
                                .prepare_pending_finalizer()
                                .unwrap()
                                .is_none()
                        );
                    }
                    // Additional work queued while the caller is parked must also finish.
                    engine
                        .stack
                        .local
                        .heap
                        .pending_finalization
                        .borrow_mut()
                        .push(queued);
                }
            })
        })
        .unwrap();
    crate::executor::WAIT_TEST_HOOK.with(|hook| {
        *hook.borrow_mut() = Some(Box::new(|| panic!("ready finalizer wait must not park")));
    });
    assert!(matches!(executor.run(), ExecutorResult::Exited(0)));
    crate::executor::WAIT_TEST_HOOK.with(|hook| drop(hook.borrow_mut().take()));
    executor
        .with_arena(|arena| {
            arena.mutate(|_, engine| {
                assert!(
                    engine
                        .stack
                        .local
                        .heap
                        .pending_finalization
                        .borrow()
                        .is_empty()
                );
                assert!(!engine.stack.local.heap.processing_finalizer.get());
                assert!(engine.stack.parked_finalizer_caller.is_none());
                assert!(engine.stack.execution.call_state.native.is_empty());
            })
        })
        .unwrap();
}

#[cfg(feature = "multithreading")]
fn start_condition_monitor(
    executor: &mut Executor,
    install: impl FnOnce(Arc<crate::sync::SyncBlock>),
    body: fn(dotnet_vm_ops::effects::VmEffects) -> dotnet_vm_ops::effects::NativeFuture,
) -> (StepResult, Arc<crate::sync::SyncBlock>) {
    use crate::sync::SyncBlockOps;
    use dotnet_intrinsics_threading::MonitorHost;
    use dotnet_vm_ops::effects::{NativeDispatchMode, NativeIntrinsicHost};
    let method = condition_wait_method(executor, true);
    executor
        .with_arena(|arena| {
            arena.mutate_root(|gc, engine| {
                // SAFETY: F1.GcHandleRooted — the engine is the live arena root for this mutation.
                let gc = unsafe { test_gc_handle(gc, engine) };
                let source = ObjectRef::new(gc, HeapStorage::Str(CLRString::from("monitor")));
                engine
                    .stack
                    .local
                    .heap
                    .gchandles
                    .borrow_mut()
                    .push(Some((source, GCHandleType::Weak)));
                let mut ctx = engine.ves_context(gc);
                let block = ctx.monitor_get_or_create_sync_block_for_object(source, gc);
                assert!(block.try_enter(dotnet_utils::ArenaId::new(9999)));
                ctx.push(StackValue::Int32(0));
                ctx.push(StackValue::ObjectRef(source));
                install(block.clone());
                let result = ctx.start_native(
                    method,
                    NativeDispatchMode::InlineCall,
                    &Default::default(),
                    None,
                    native_test_fallback,
                    body,
                );
                (result, block)
            })
        })
        .unwrap()
}

#[cfg(feature = "multithreading")]
#[test]
fn condition_wait_registration_schedules_do_not_lose_release() {
    use crate::stack::native_effects::{WAIT_TEST_HOOK, WaitTestPoint};
    use crate::sync::SyncBlockOps;
    let owner = dotnet_utils::ArenaId::new(9999);
    for release_after_registration in [false, true] {
        let mut executor = executor();
        let thread = executor.thread_id;
        let (result, block) = start_condition_monitor(
            &mut executor,
            |block| {
                let mut fired = false;
                WAIT_TEST_HOOK.with(|hook| {
                    *hook.borrow_mut() = Some(Box::new(move |point| {
                        if !fired
                            && matches!(
                                (release_after_registration, point),
                                (false, WaitTestPoint::Probed) | (true, WaitTestPoint::Registered)
                            )
                        {
                            fired = true;
                            assert!(block.exit(owner));
                        }
                    }))
                });
            },
            condition_monitor_body,
        );
        WAIT_TEST_HOOK.with(|hook| drop(hook.borrow_mut().take()));
        assert_eq!(result, StepResult::Jump(1));
        assert!(block.exit(thread));
        assert!(!block.exit(thread));
    }
}

#[cfg(feature = "multithreading")]
#[test]
fn condition_wait_stale_release_never_grants_ownership() {
    use crate::stack::native_effects::{WAIT_TEST_HOOK, WaitTestPoint};
    use crate::sync::SyncBlockOps;
    let owner = dotnet_utils::ArenaId::new(9999);
    let mut executor = executor();
    let thread = executor.thread_id;
    let (result, block) = start_condition_monitor(
        &mut executor,
        |block| {
            let mut fired = false;
            WAIT_TEST_HOOK.with(|hook| {
                *hook.borrow_mut() = Some(Box::new(move |point| {
                    if !fired && matches!(point, WaitTestPoint::Registered) {
                        fired = true;
                        assert!(block.exit(owner));
                        assert!(block.try_enter(owner));
                    }
                }))
            });
        },
        condition_monitor_body,
    );
    WAIT_TEST_HOOK.with(|hook| drop(hook.borrow_mut().take()));
    assert_eq!(result, StepResult::Yield);
    assert_eq!(resume(&mut executor), StepResult::Yield);
    assert!(!block.exit(thread));
    assert!(block.exit(owner));
    assert_eq!(resume(&mut executor), StepResult::Continue);
    assert!(block.exit(thread));
}

#[cfg(feature = "multithreading")]
#[test]
fn condition_wait_executor_parks_outside_mutation_and_wakes_on_release() {
    use crate::{sync::SyncBlockOps, test_utils::worker::Worker};
    use std::{sync::mpsc, thread, time::Duration};
    let owner = dotnet_utils::ArenaId::new(9999);
    let mut executor = executor();
    let thread = executor.thread_id;
    let (result, block) = start_condition_monitor(&mut executor, |_| {}, condition_monitor_body);
    assert_eq!(result, StepResult::Yield);
    collect(&mut executor);
    let (parked_tx, parked_rx) = mpsc::channel();
    let released_block = block.clone();
    let worker = Worker::new(
        thread::spawn(move || {
            let parked = parked_rx.recv_timeout(Duration::from_secs(5)).is_ok();
            assert!(released_block.exit(owner));
            parked
        }),
        (),
    );
    crate::sync::WAIT_TEST_HOOK.with(|hook| {
        *hook.borrow_mut() = Some(Box::new(move || {
            let _ = parked_tx.send(());
        }))
    });
    assert!(matches!(executor.run(), ExecutorResult::Exited(0)));
    crate::sync::WAIT_TEST_HOOK.with(|hook| drop(hook.borrow_mut().take()));
    assert!(worker.finish().unwrap(), "executor did not park");
    assert!(block.exit(thread));
    assert!(!block.exit(thread));
}

#[test]
fn condition_wait_cancel_does_not_manufacture_finalizer_readiness() {
    use dotnet_vm_ops::effects::{NativeDispatchMode, NativeIntrinsicHost};
    let mut executor = executor();
    let method = condition_wait_method(&executor, false);
    executor
        .with_arena(|arena| {
            arena.mutate_root(|gc, engine| {
                // SAFETY: F1.GcHandleRooted — the engine is the live arena root for this mutation.
                let gc = unsafe { test_gc_handle(gc, engine) };
                engine.stack.local.heap.processing_finalizer.set(true);
                assert_eq!(
                    engine.ves_context(gc).start_native(
                        method,
                        NativeDispatchMode::InlineCall,
                        &Default::default(),
                        None,
                        native_test_fallback,
                        condition_finalizer_body
                    ),
                    StepResult::Yield
                );
            })
        })
        .unwrap();
    assert_eq!(resume(&mut executor), StepResult::Yield);
    let abort = executor.shared.abort_signal();
    crate::executor::WAIT_TEST_HOOK
        .with(|hook| *hook.borrow_mut() = Some(Box::new(move || abort.request_abort())));
    assert!(matches!(executor.run(), ExecutorResult::Error(_)));
    crate::executor::WAIT_TEST_HOOK.with(|hook| assert!(hook.borrow().is_none()));
    executor
        .with_arena(|arena| {
            arena.mutate(|_, engine| {
                assert!(engine.stack.local.heap.processing_finalizer.get());
                assert!(engine.stack.execution.call_state.native.is_empty());
            })
        })
        .unwrap();
}

#[cfg(feature = "multithreading")]
fn condition_timeout_body(
    mut fx: dotnet_vm_ops::effects::VmEffects,
) -> dotnet_vm_ops::effects::NativeFuture {
    Box::pin(async move {
        let object = fx.vm().await?.pop_object()?;
        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(200);
        assert!(!fx.monitor_enter(object, Some(deadline)).await?);
        Ok(())
    })
}

#[cfg(feature = "multithreading")]
#[test]
fn condition_wait_timeout_does_not_acquire_or_release_monitor() {
    use crate::sync::SyncBlockOps;
    let owner = dotnet_utils::ArenaId::new(9999);
    let mut executor = executor();
    let thread = executor.thread_id;
    let (result, block) = start_condition_monitor(&mut executor, |_| {}, condition_timeout_body);
    assert_eq!(result, StepResult::Yield);
    assert!(matches!(executor.run(), ExecutorResult::Exited(0)));
    assert!(!block.exit(thread));
    assert!(block.exit(owner));
    assert!(!block.exit(owner));
}

#[cfg(feature = "multithreading")]
#[test]
fn condition_wait_executor_participates_in_real_gc_stop() {
    use crate::{sync::SyncBlockOps, test_utils::worker::Worker};
    use std::{sync::mpsc, thread, time::Duration};
    let owner = dotnet_utils::ArenaId::new(9999);
    let mut executor = executor();
    let thread = executor.thread_id;
    let (result, block) = start_condition_monitor(&mut executor, |_| {}, condition_monitor_body);
    assert_eq!(result, StepResult::Yield);
    let shared = executor.shared.clone();
    let released_block = block.clone();
    let (ready_tx, ready_rx) = mpsc::channel();
    let (start_tx, start_rx) = mpsc::channel();
    let worker = Worker::new(
        thread::spawn(move || {
            let mut collector = Executor::new(shared);
            ready_tx.send(()).unwrap();
            let started = start_rx.recv_timeout(Duration::from_secs(5)).is_ok();
            let collected = started && collector.perform_full_gc().is_ok();
            assert!(released_block.exit(owner));
            collected
        }),
        (),
    );
    ready_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    crate::sync::WAIT_TEST_HOOK.with(|hook| {
        *hook.borrow_mut() = Some(Box::new(move || {
            let _ = start_tx.send(());
        }))
    });
    assert!(matches!(executor.run(), ExecutorResult::Exited(0)));
    crate::sync::WAIT_TEST_HOOK.with(|hook| drop(hook.borrow_mut().take()));
    assert!(worker.finish().unwrap(), "real collection did not complete");
    assert!(block.exit(thread));
}

#[cfg(feature = "multithreading")]
#[test]
fn condition_wait_spurious_and_duplicate_wakes_only_trigger_rechecks() {
    use crate::{sync::SyncBlockOps, test_utils::worker::Worker};
    use std::{sync::mpsc, thread, time::Duration};
    let owner = dotnet_utils::ArenaId::new(9999);
    let mut executor = executor();
    let thread = executor.thread_id;
    let (result, block) = start_condition_monitor(&mut executor, |_| {}, condition_monitor_body);
    assert_eq!(result, StepResult::Yield);
    let (parked_tx, parked_rx) = mpsc::channel();
    let released_block = block.clone();
    let worker = Worker::new(
        thread::spawn(move || {
            let first = parked_rx.recv_timeout(Duration::from_secs(5)).is_ok();
            if first {
                released_block.notify_spurious();
                released_block.notify_spurious();
            }
            let second = parked_rx.recv_timeout(Duration::from_secs(5)).is_ok();
            assert!(released_block.exit(owner));
            first && second
        }),
        (),
    );
    crate::sync::WAIT_TEST_HOOK.with(|hook| {
        *hook.borrow_mut() = Some(Box::new(move || {
            let _ = parked_tx.send(());
        }))
    });
    assert!(matches!(executor.run(), ExecutorResult::Exited(0)));
    crate::sync::WAIT_TEST_HOOK.with(|hook| drop(hook.borrow_mut().take()));
    assert!(
        worker.finish().unwrap(),
        "wait did not re-park after spurious notification"
    );
    assert!(block.exit(thread));
}

#[cfg(feature = "multithreading")]
#[test]
fn condition_wait_cancellation_releases_registration_without_releasing_monitor() {
    use crate::sync::SyncBlockOps;
    let owner = dotnet_utils::ArenaId::new(9999);
    let mut executor = executor();
    let thread = executor.thread_id;
    let (result, block) = start_condition_monitor(&mut executor, |_| {}, condition_monitor_body);
    assert_eq!(result, StepResult::Yield);
    let abort = executor.shared.abort_signal();
    crate::executor::WAIT_TEST_HOOK
        .with(|hook| *hook.borrow_mut() = Some(Box::new(move || abort.request_abort())));
    assert!(matches!(executor.run(), ExecutorResult::Error(_)));
    executor
        .with_arena(|arena| {
            arena.mutate(|_, engine| assert!(engine.stack.execution.call_state.native.is_empty()))
        })
        .unwrap();
    assert!(!block.exit(thread));
    assert!(block.exit(owner));
}

#[cfg(feature = "multithreading")]
fn condition_long_deadline_body(
    mut fx: dotnet_vm_ops::effects::VmEffects,
) -> dotnet_vm_ops::effects::NativeFuture {
    Box::pin(async move {
        let object = fx.vm().await?.pop_object()?;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        let _ = fx.monitor_enter(object, Some(deadline)).await?;
        Ok(())
    })
}

#[cfg(feature = "multithreading")]
#[test]
fn condition_wait_deadline_survives_real_collection() {
    use crate::{stack::native_effects::PendingWait, sync::SyncBlockOps};
    let owner = dotnet_utils::ArenaId::new(9999);
    let mut executor = executor();
    let (result, block) =
        start_condition_monitor(&mut executor, |_| {}, condition_long_deadline_body);
    assert_eq!(result, StepResult::Yield);
    let deadline = |executor: &mut Executor| {
        executor
            .with_arena(|arena| {
                arena.mutate(|_, engine| {
                    let Some(PendingWait::Monitor(wait)) = &engine
                        .stack
                        .execution
                        .call_state
                        .native
                        .last()
                        .unwrap()
                        .wait
                    else {
                        panic!("missing monitor wait")
                    };
                    wait.deadline.unwrap()
                })
            })
            .unwrap()
    };
    let original = deadline(&mut executor);
    collect(&mut executor);
    executor
        .with_arena(|arena| {
            arena.mutate(|_, engine| {
                assert!(
                    engine.stack.local.heap.gchandles.borrow()[0]
                        .as_ref()
                        .unwrap()
                        .0
                        .0
                        .is_some()
                );
            })
        })
        .unwrap();
    assert_eq!(resume(&mut executor), StepResult::Yield);
    assert_eq!(deadline(&mut executor), original);
    executor.shared.abort_signal().request_abort();
    assert!(matches!(executor.run(), ExecutorResult::Error(_)));
    assert!(block.exit(owner));
}

#[cfg(feature = "multithreading")]
#[test]
fn condition_wait_deadlines_and_notifications_are_activation_local() {
    use crate::{stack::native_effects::PendingWait, sync::SyncBlockOps};
    let owner = dotnet_utils::ArenaId::new(9999);
    let mut executor = executor();
    let (result, first_block) =
        start_condition_monitor(&mut executor, |_| {}, condition_timeout_body);
    assert_eq!(result, StepResult::Yield);
    let snapshot = |executor: &mut Executor| {
        executor
            .with_arena(|arena| {
                arena.mutate(|_, engine| {
                    let work = engine.stack.execution.call_state.native.last().unwrap();
                    let Some(PendingWait::Monitor(wait)) = &work.wait else {
                        panic!("missing monitor wait")
                    };
                    (work.id, wait.deadline.unwrap())
                })
            })
            .unwrap()
    };
    let (first_id, first_deadline) = snapshot(&mut executor);
    std::thread::sleep(
        first_deadline.saturating_duration_since(std::time::Instant::now())
            + std::time::Duration::from_millis(1),
    );
    assert_eq!(resume(&mut executor), StepResult::Continue);
    let (result, second_block) =
        start_condition_monitor(&mut executor, |_| {}, condition_long_deadline_body);
    assert_eq!(result, StepResult::Yield);
    let (second_id, second_deadline) = snapshot(&mut executor);
    assert_ne!(first_id, second_id);
    assert!(second_deadline > first_deadline);
    assert!(first_block.exit(owner));
    assert_eq!(resume(&mut executor), StepResult::Yield);
    executor.shared.abort_signal().request_abort();
    assert!(matches!(executor.run(), ExecutorResult::Error(_)));
    assert!(second_block.exit(owner));
}

#[cfg(feature = "multithreading")]
fn condition_error_after_acquire_body(
    mut fx: dotnet_vm_ops::effects::VmEffects,
) -> dotnet_vm_ops::effects::NativeFuture {
    Box::pin(async move {
        let object = fx.vm().await?.pop_object()?;
        assert!(fx.monitor_enter(object, None).await?);
        let error: dotnet_types::error::VmError =
            dotnet_types::error::ExecutionError::InternalError("injected native failure".into())
                .into();
        Err(error.into())
    })
}

#[cfg(feature = "multithreading")]
#[test]
fn condition_wait_acquisition_survives_later_native_failure() {
    use crate::sync::SyncBlockOps;
    let owner = dotnet_utils::ArenaId::new(9999);
    let mut executor = executor();
    let thread = executor.thread_id;
    let (result, block) =
        start_condition_monitor(&mut executor, |_| {}, condition_error_after_acquire_body);
    assert_eq!(result, StepResult::Yield);
    assert!(block.exit(owner));
    assert!(matches!(resume(&mut executor), StepResult::Error(_)));
    assert!(block.exit(thread));
    assert!(!block.exit(thread));
}

#[cfg(feature = "multithreading")]
#[test]
fn condition_wait_foreign_monitor_owner_survives_collection() {
    use crate::{
        sync::SyncBlockOps,
        test_utils::{
            foreign_reference::{decode_reference, encode_reference},
            worker::Worker,
        },
        threading::ThreadManagerOps,
    };
    use dotnet_intrinsics_threading::MonitorHost;
    use dotnet_vm_ops::effects::{NativeDispatchMode, NativeIntrinsicHost};
    use std::{sync::mpsc, thread, time::Duration};
    let mut executor = executor();
    let method = condition_wait_method(&executor, true);
    let shared = executor.shared.clone();
    let owner_shared = shared.clone();
    let (source_tx, source_rx) = mpsc::channel();
    let (command_tx, command_rx) = mpsc::channel();
    let (witness_tx, witness_rx) = mpsc::channel();
    let owner = Worker::new(
        thread::spawn(move || {
            let mut owner = Executor::new(owner_shared.clone());
            let id = owner.thread_id;
            let storage = owner
                .with_arena(|arena| {
                    arena.mutate_root(|gc, engine| {
                        // SAFETY: F1.GcHandleRooted — the engine is the live arena root for this mutation.
                        let gc = unsafe { test_gc_handle(gc, engine) };
                        let source = ObjectRef::new(
                            gc,
                            HeapStorage::Str(CLRString::from("foreign monitor")),
                        );
                        engine
                            .stack
                            .local
                            .heap
                            .gchandles
                            .borrow_mut()
                            .push(Some((source, GCHandleType::Weak)));
                        encode_reference(source)
                    })
                })
                .unwrap();
            source_tx.send(storage).unwrap();
            loop {
                owner_shared
                    .thread_manager
                    .safe_point(id, &owner_shared.gc_coordinator);
                match command_rx.recv_timeout(Duration::from_millis(1)) {
                    Ok(()) => {
                        let alive = owner
                            .with_arena(|arena| {
                                arena.mutate(|_, engine| {
                                    engine.stack.local.heap.gchandles.borrow()[0]
                                        .as_ref()
                                        .unwrap()
                                        .0
                                        .0
                                        .is_some()
                                })
                            })
                            .unwrap();
                        if witness_tx.send(alive).is_err() {
                            break;
                        }
                    }
                    Err(mpsc::RecvTimeoutError::Timeout) => {}
                    Err(mpsc::RecvTimeoutError::Disconnected) => break,
                }
            }
        }),
        command_tx,
    );
    let storage = source_rx.recv_timeout(Duration::from_secs(30)).unwrap();
    let fake_owner = dotnet_utils::ArenaId::new(9999);
    let waiter = executor.thread_id;
    let block = executor
        .with_arena(|arena| {
            arena.mutate_root(|gc, engine| {
                // SAFETY: F1.GcHandleRooted — the engine is the live arena root for this mutation.
                let gc = unsafe { test_gc_handle(gc, engine) };
                let source = decode_reference(storage, gc);
                let mut ctx = engine.ves_context(gc);
                let block = ctx.monitor_get_or_create_sync_block_for_object(source, gc);
                assert!(block.try_enter(fake_owner));
                ctx.push(StackValue::ObjectRef(source));
                assert_eq!(
                    ctx.start_native(
                        method,
                        NativeDispatchMode::InlineCall,
                        &Default::default(),
                        None,
                        native_test_fallback,
                        condition_monitor_body
                    ),
                    StepResult::Yield
                );
                block
            })
        })
        .unwrap();
    collect(&mut executor);
    owner.resources().send(()).unwrap();
    assert!(
        witness_rx.recv_timeout(Duration::from_secs(30)).unwrap(),
        "pending monitor did not retain foreign owner"
    );
    assert!(block.exit(fake_owner));
    assert_eq!(resume(&mut executor), StepResult::Continue);
    assert!(block.exit(waiter));
    collect(&mut executor);
    owner.resources().send(()).unwrap();
    assert!(
        !witness_rx.recv_timeout(Duration::from_secs(30)).unwrap(),
        "retired monitor retained foreign owner"
    );
    owner.finish().unwrap();
}

#[cfg(feature = "multithreading")]
#[test]
fn condition_wait_unprobed_monitor_rechecks_before_parking() {
    use crate::{stack::native_effects::PendingWait, sync::SyncBlockOps};
    let owner = dotnet_utils::ArenaId::new(9999);
    let mut executor = executor();
    let thread = executor.thread_id;
    let (result, block) = start_condition_monitor(&mut executor, |_| {}, condition_monitor_body);
    assert_eq!(result, StepResult::Yield);
    executor
        .with_arena(|arena| {
            arena.mutate_root(|_, engine| {
                let Some(PendingWait::Monitor(wait)) = &mut engine
                    .stack
                    .execution
                    .call_state
                    .native
                    .last_mut()
                    .unwrap()
                    .wait
                else {
                    panic!("missing monitor wait")
                };
                wait.epoch = None;
            })
        })
        .unwrap();
    assert!(block.exit(owner));
    crate::executor::WAIT_TEST_HOOK.with(|hook| {
        *hook.borrow_mut() = Some(Box::new(|| panic!("unprobed monitor must not park")))
    });
    assert!(matches!(executor.run(), ExecutorResult::Exited(0)));
    crate::executor::WAIT_TEST_HOOK.with(|hook| drop(hook.borrow_mut().take()));
    assert!(block.exit(thread));
}

fn monitor_byref_method(executor: &Executor) -> MethodDescription {
    use dotnetdll::{binary::signature::kinds::CallingConvention, resolved::types::TypeDefinition};
    let mut res = Resolution::new(Module::new("MonitorTest.dll"));
    res.assembly = Some(Assembly::new("MonitorTest"));
    let ty_index = res.push_type_definition(TypeDefinition::new(None, "MonitorTest"));
    let signature = MethodSignature {
        instance: false,
        explicit_this: false,
        calling_convention: CallingConvention::Default,
        parameters: vec![
            Parameter(
                vec![],
                ParameterType::Value(MethodType::Base(Box::new(BaseType::Object))),
            ),
            Parameter(
                vec![],
                ParameterType::Ref(MethodType::Base(Box::new(BaseType::Boolean))),
            ),
        ],
        return_type: ReturnType(vec![], None),
        varargs: None,
    };
    let index = res.push_method(
        ty_index,
        Method::new(Accessibility::Public, signature, "Enter", None),
    );
    let res = executor.shared.loader.register_owned_assembly(res);
    let ty = TypeDescription::new(res.clone(), ty_index);
    let method = ty
        .definition()
        .methods
        .iter()
        .position(|m| std::ptr::eq(m, &res.definition()[index]))
        .unwrap();
    MethodDescription::new(
        ty,
        GenericLookup::default(),
        res,
        MethodMemberIndex::Method(method),
    )
}

#[test]
fn monitor_byref_dispatch_writes_frame_home() {
    use crate::sync::SyncBlockOps;
    use dotnet_intrinsics_threading::MonitorHost;
    use dotnet_value::pointer::ManagedPtr;
    use dotnet_vm_ops::effects::NativeDispatchMode;
    let mut executor = executor();
    let method = monitor_byref_method(&executor);
    let thread = executor.thread_id;
    executor
        .with_arena(|arena| {
            arena.mutate_root(|gc, engine| {
                // SAFETY: F1.GcHandleRooted — the engine is the live arena root for this mutation.
                let gc = unsafe { test_gc_handle(gc, engine) };
                let frames = &mut engine.stack.execution.frame_stack;
                let serial = frames.issue_home_serial();
                let layout = dotnet_value::layout::FieldLayoutManager {
                    fields: Default::default(),
                    total_size: 1,
                    alignment: 1,
                    gc_desc: Default::default(),
                    has_ref_fields: false,
                };
                let layout = dotnet_vm_data::frame_homes::FrameHomeLayout::new(
                    layout,
                    Box::new([]),
                    Box::new([]),
                );
                frames.current_frame_mut().homes =
                    dotnet_vm_data::frame_homes::FrameHomes::allocate(Arc::new(layout), serial)
                        .unwrap();
                let (address, _) = frames.home_block(serial).unwrap();
                let pointer = ManagedPtr::new_frame_home(
                    address,
                    TypeDescription::NULL,
                    serial,
                    dotnet_utils::ByteOffset::new(0),
                    false,
                );
                let object = ObjectRef::new(gc, HeapStorage::Str(CLRString::from("monitor")));
                let mut ctx = engine.ves_context(gc);
                let block = ctx.monitor_get_or_create_sync_block_for_object(object, gc);
                ctx.push(StackValue::ObjectRef(object));
                ctx.push(StackValue::ManagedPtr(pointer.into()));
                let result = crate::intrinsics::dispatch_method_intrinsic(
            crate::intrinsics::MethodIntrinsicId::System_Threading_Monitor_Enter_cf1115d21714f8ee,
            &mut ctx, method, &Default::default(), NativeDispatchMode::InlineCall,
        );
                assert_eq!(result, StepResult::Jump(1));
                // SAFETY: F10.FrameHomeChecked — this live one-byte block was allocated above and remains frame-owned.
                assert_eq!(unsafe { address.as_ptr().read() }, 1);
                assert!(block.exit(thread));
                assert!(!block.exit(thread));
            })
        })
        .unwrap();
}

#[cfg(feature = "multithreading")]
#[test]
fn monitor_byref_contended_dispatch_survives_collection() {
    use crate::sync::SyncBlockOps;
    use dotnet_intrinsics_threading::MonitorHost;
    use dotnet_value::pointer::ManagedPtr;
    use dotnet_vm_ops::effects::NativeDispatchMode;
    let mut executor = executor();
    let method = monitor_byref_method(&executor);
    let owner = dotnet_utils::ArenaId::new(9999);
    let thread = executor.thread_id;
    let (block, serial) = executor
        .with_arena(|arena| {
            arena.mutate_root(|gc, engine| {
                // SAFETY: F1.GcHandleRooted — the engine is the live arena root for this mutation.
                let gc = unsafe { test_gc_handle(gc, engine) };
                let frames = &mut engine.stack.execution.frame_stack;
                let serial = frames.issue_home_serial();
                let layout = dotnet_value::layout::FieldLayoutManager {
                    fields: Default::default(),
                    total_size: 1,
                    alignment: 1,
                    gc_desc: Default::default(),
                    has_ref_fields: false,
                };
                let layout = dotnet_vm_data::frame_homes::FrameHomeLayout::new(
                    layout,
                    Box::new([]),
                    Box::new([]),
                );
                frames.current_frame_mut().homes =
                    dotnet_vm_data::frame_homes::FrameHomes::allocate(Arc::new(layout), serial)
                        .unwrap();
                let (address, _) = frames.home_block(serial).unwrap();
                let pointer = ManagedPtr::new_frame_home(
                    address,
                    TypeDescription::NULL,
                    serial,
                    dotnet_utils::ByteOffset::new(0),
                    false,
                );
                let object = ObjectRef::new(gc, HeapStorage::Str(CLRString::from("monitor")));
                let mut ctx = engine.ves_context(gc);
                let block = ctx.monitor_get_or_create_sync_block_for_object(object, gc);
                assert!(block.try_enter(owner));
                ctx.push(StackValue::ObjectRef(object));
                ctx.push(StackValue::ManagedPtr(pointer.into()));
                assert_eq!(crate::intrinsics::dispatch_method_intrinsic(
            crate::intrinsics::MethodIntrinsicId::System_Threading_Monitor_Enter_cf1115d21714f8ee,
            &mut ctx, method, &Default::default(), NativeDispatchMode::InlineCall,
        ), StepResult::Yield);
                (block, serial)
            })
        })
        .unwrap();
    collect(&mut executor);
    executor
        .with_arena(|arena| {
            arena.mutate(|_, engine| {
                let (address, _) = engine
                    .stack
                    .execution
                    .frame_stack
                    .home_block(serial)
                    .unwrap();
                // SAFETY: F10.FrameHomeChecked — the frame still owns this live one-byte block.
                assert_eq!(unsafe { address.as_ptr().read() }, 0);
            })
        })
        .unwrap();
    assert!(block.exit(owner));
    assert_eq!(resume(&mut executor), StepResult::Continue);
    executor
        .with_arena(|arena| {
            arena.mutate(|_, engine| {
                let (address, _) = engine
                    .stack
                    .execution
                    .frame_stack
                    .home_block(serial)
                    .unwrap();
                // SAFETY: F10.FrameHomeChecked — the frame still owns this live one-byte block.
                assert_eq!(unsafe { address.as_ptr().read() }, 1);
            })
        })
        .unwrap();
    assert!(block.exit(thread));
    assert!(!block.exit(thread));
}

fn timed_monitor_method(executor: &Executor) -> MethodDescription {
    use dotnetdll::{binary::signature::kinds::CallingConvention, resolved::types::TypeDefinition};
    let mut res = Resolution::new(Module::new("TimedMonitor.dll"));
    res.assembly = Some(Assembly::new("TimedMonitor"));
    let mut monitor = TypeDefinition::new(None, "Monitor");
    monitor.namespace = Some("System.Threading".into());
    monitor.flags.before_field_init = true;
    let ty_index = res.push_type_definition(monitor);
    res.push_field(
        ty_index,
        Field {
            attributes: vec![],
            name: "flag".into(),
            type_modifiers: vec![],
            by_ref: false,
            return_type: MemberType::Base(Box::new(BaseType::Boolean)),
            accessibility: dotnetdll::resolved::members::Accessibility::Access(
                Accessibility::Public,
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
        },
    );
    let signature = MethodSignature {
        instance: false,
        explicit_this: false,
        calling_convention: CallingConvention::Default,
        parameters: vec![
            Parameter(
                vec![],
                ParameterType::Value(MethodType::Base(Box::new(BaseType::Object))),
            ),
            Parameter(
                vec![],
                ParameterType::Value(MethodType::Base(Box::new(BaseType::Int32))),
            ),
            Parameter(
                vec![],
                ParameterType::Ref(MethodType::Base(Box::new(BaseType::Boolean))),
            ),
        ],
        return_type: ReturnType(vec![], None),
        varargs: None,
    };
    let index = res.push_method(
        ty_index,
        Method::new(Accessibility::Public, signature, "TryEnter", None),
    );
    let res = executor.shared.loader.register_owned_assembly(res);
    let ty = TypeDescription::new(res.clone(), ty_index);
    let index = ty
        .definition()
        .methods
        .iter()
        .position(|m| std::ptr::eq(m, &res.definition()[index]))
        .unwrap();
    MethodDescription::new(
        ty,
        GenericLookup::default(),
        res,
        MethodMemberIndex::Method(index),
    )
}

#[cfg(feature = "multithreading")]
#[test]
fn monitor_legacy_static_timeout_retains_deadline_across_stop() {
    use dotnet_vm_ops::effects::NativeDispatchMode;
    for mode in [NativeDispatchMode::InlineCall, NativeDispatchMode::Other] {
        check_legacy_static_timeout(mode);
    }
}

#[cfg(feature = "multithreading")]
fn check_legacy_static_timeout(mode: dotnet_vm_ops::effects::NativeDispatchMode) {
    use crate::{
        stack::ops::{VmCallOps, VmResolutionOps},
        sync::SyncBlockOps,
        threading::ThreadManagerOps,
    };
    use dotnet_intrinsics_threading::MonitorHost;
    use dotnet_value::pointer::ManagedPtr;
    let (mut executor, _) = boundary_executor(false);
    let method = timed_monitor_method(&executor);
    let owner = dotnet_utils::ArenaId::new(9999);
    let shared = executor.shared.clone();
    let guard = shared.thread_manager.request_stop_the_world();
    let (block, deadline) = executor.with_arena(|arena| arena.mutate_root(|gc, engine| {
        // SAFETY: F1.GcHandleRooted — the engine is the live arena root for this mutation.
        let gc = unsafe { test_gc_handle(gc, engine) };
        let mut ctx = engine.ves_context(gc);
        let generics = GenericLookup::default();
        ctx.shared.statics.init(method.parent.clone(), &ctx.with_generics(&generics), ctx.thread_id.get()).unwrap();
        let ptr = ManagedPtr::new_static(None, TypeDescription::NULL, method.parent.clone(), generics, false, dotnet_utils::ByteOffset::new(0));
        let object = ObjectRef::new(gc, HeapStorage::Str(CLRString::from("monitor")));
        let block = ctx.monitor_get_or_create_sync_block_for_object(object, gc);
        assert!(block.try_enter(owner));
        ctx.push(StackValue::ObjectRef(object));
        ctx.push(StackValue::Int32(10_000));
        ctx.push(StackValue::ManagedPtr(ptr.into()));
        let result = if mode == dotnet_vm_ops::effects::NativeDispatchMode::Other {
            ctx.dispatch_method(method.clone(), Default::default())
        } else {
            crate::intrinsics::dispatch_method_intrinsic(
                crate::intrinsics::MethodIntrinsicId::System_Threading_Monitor_TryEnter_a8f16726fa8c35c7,
                &mut ctx, method.clone(), &Default::default(), mode,
            )
        };
        assert_eq!(result, StepResult::Yield);
        assert!(ctx.call_state.native.is_empty());
        assert_eq!(ctx.frame_stack.current_frame().stack_height.as_usize(), 3);
        (block, ctx.frame_stack.current_frame().monitor_retry.as_ref().unwrap().deadline)
    })).unwrap();
    drop(guard);
    enqueue_test_finalizer(&mut executor);
    executor
        .with_arena(|arena| {
            arena.mutate_root(|gc, engine| {
                // SAFETY: F1.GcHandleRooted — the engine is the live arena root for this mutation.
                let gc = unsafe { test_gc_handle(gc, engine) };
                assert_eq!(
                    engine.process_pending_finalizers(gc),
                    StepResult::FramePushed
                );
                let caller = engine.stack.parked_finalizer_caller.as_ref().unwrap();
                assert_eq!(
                    caller
                        .frame_stack
                        .current_frame()
                        .monitor_retry
                        .as_ref()
                        .unwrap()
                        .deadline,
                    deadline
                );
                let mut ctx = engine.ves_context(gc);
                let ptr = ManagedPtr::new_static(
                    None,
                    TypeDescription::NULL,
                    method.parent.clone(),
                    Default::default(),
                    false,
                    dotnet_utils::ByteOffset::new(0),
                );
                let object =
                    ObjectRef::new(gc, HeapStorage::Str(CLRString::from("finalizer monitor")));
                let nested = ctx.monitor_get_or_create_sync_block_for_object(object, gc);
                // The nested zero-timeout call must not inherit the parked caller's longer
                // deadline. Holding this lock makes an accidentally shared deadline observable.
                assert!(nested.try_enter(owner));
                ctx.push(StackValue::ObjectRef(object));
                ctx.push(StackValue::Int32(0));
                ctx.push(StackValue::ManagedPtr(ptr.into()));
                let nested_deadline = ctx.monitor_retry_deadline(&method, 0);
                assert!(nested_deadline <= std::time::Instant::now());
                assert!(nested_deadline < deadline);
                assert_eq!(
                    ctx.dispatch_method(method.clone(), Default::default()),
                    StepResult::Continue
                );
                assert!(ctx.frame_stack.current_frame().monitor_retry.is_none());
                assert_eq!(
                    ctx.shared
                        .statics
                        .get(method.parent.clone(), &Default::default())
                        .storage
                        .with_data(|data| data[0]),
                    0
                );
                assert!(nested.exit(owner));
                let caller = engine.stack.parked_finalizer_caller.as_ref().unwrap();
                assert_eq!(
                    caller
                        .frame_stack
                        .current_frame()
                        .monitor_retry
                        .as_ref()
                        .unwrap()
                        .deadline,
                    deadline
                );
                assert_eq!(engine.step(gc), StepResult::Continue);
                assert!(engine.stack.parked_finalizer_caller.is_none());
            })
        })
        .unwrap();
    executor.with_arena(|arena| arena.mutate_root(|gc, engine| {
        // SAFETY: F1.GcHandleRooted — the engine is the live arena root for this mutation.
        let gc = unsafe { test_gc_handle(gc, engine) };
        let frame = engine.stack.execution.frame_stack.current_frame_mut();
        let record = frame.monitor_retry.as_mut().unwrap();
        assert_eq!(record.deadline, deadline);
        record.deadline = std::time::Instant::now() - std::time::Duration::from_millis(1);
        let mut ctx = engine.ves_context(gc);
        let result = if mode == dotnet_vm_ops::effects::NativeDispatchMode::Other {
            ctx.dispatch_method(method.clone(), Default::default())
        } else {
            crate::intrinsics::dispatch_method_intrinsic(
                crate::intrinsics::MethodIntrinsicId::System_Threading_Monitor_TryEnter_a8f16726fa8c35c7,
                &mut ctx, method.clone(), &Default::default(), mode,
            )
        };
        assert_eq!(result, StepResult::Continue);
        assert!(ctx.frame_stack.current_frame().monitor_retry.is_none());
        assert_eq!(ctx.frame_stack.current_frame().stack_height.as_usize(), 0);
        assert_eq!(ctx.shared.statics.get(method.parent.clone(), &Default::default()).storage.with_data(|data| data[0]), 0);
    })).unwrap();
    assert!(block.exit(owner));
}

#[test]
fn monitor_retry_record_is_removed_on_executor_abort_and_frame_reuse() {
    use dotnet_intrinsics_threading::MonitorHost;
    let mut executor = executor();
    let method = monitor_byref_method(&executor);
    executor
        .with_arena(|arena| {
            arena.mutate_root(|gc, engine| {
                // SAFETY: F1.GcHandleRooted — the engine is the live arena root for this mutation.
                let gc = unsafe { test_gc_handle(gc, engine) };
                let mut ctx = engine.ves_context(gc);
                let first = ctx.monitor_retry_deadline(&method, 1000);
                assert_eq!(ctx.monitor_retry_deadline(&method, 0), first);
                let frame = ctx.frame_stack.current_frame_mut();
                let info = frame.state.info_handle.clone();
                frame.prepare_for_pool();
                assert!(frame.monitor_retry.is_none());
                frame.reset_for_call(
                    BasePointer::default(),
                    info,
                    Default::default(),
                    Default::default(),
                );
                assert!(frame.monitor_retry.is_none());
                let second = ctx.monitor_retry_deadline(&method, 10_000);
                assert!(second > first);
            })
        })
        .unwrap();
    executor.shared.abort_signal().request_abort();
    assert!(matches!(executor.run(), ExecutorResult::Error(_)));
    executor
        .with_arena(|arena| {
            arena.mutate(|_, engine| {
                assert!(
                    engine
                        .stack
                        .execution
                        .frame_stack
                        .frames
                        .iter()
                        .all(|frame| frame.monitor_retry.is_none())
                );
                assert!(
                    engine
                        .stack
                        .execution
                        .frame_stack
                        .suspended_frames
                        .iter()
                        .all(|frame| frame.monitor_retry.is_none())
                );
            })
        })
        .unwrap();
}

#[cfg(not(feature = "multithreading"))]
struct SimulatedMonitorContention;

#[cfg(not(feature = "multithreading"))]
impl Drop for SimulatedMonitorContention {
    fn drop(&mut self) {
        crate::sync::MONITOR_TEST_CONTENDED.with(|contended| contended.set(false));
    }
}

#[cfg(not(feature = "multithreading"))]
fn hold_monitor_for_timeout(
    _: &crate::sync::SyncBlock,
    _: dotnet_utils::ArenaId,
) -> SimulatedMonitorContention {
    // A single-threaded sync block always acquires, so emulate a failed probe while the
    // real driver handles the expired deadline and replies to the actual intrinsic.
    crate::sync::MONITOR_TEST_CONTENDED.with(|contended| {
        assert!(!contended.replace(true));
    });
    SimulatedMonitorContention
}

#[cfg(not(feature = "multithreading"))]
fn release_monitor_for_test(
    _: &crate::sync::SyncBlock,
    _: dotnet_utils::ArenaId,
    contention: SimulatedMonitorContention,
) {
    drop(contention);
}

#[cfg(feature = "multithreading")]
fn release_monitor_for_test(block: &crate::sync::SyncBlock, owner: dotnet_utils::ArenaId, _: bool) {
    use crate::sync::SyncBlockOps;
    assert!(block.exit(owner));
}

#[cfg(feature = "multithreading")]
fn hold_monitor_for_timeout(block: &crate::sync::SyncBlock, owner: dotnet_utils::ArenaId) -> bool {
    use crate::sync::SyncBlockOps;
    assert!(block.try_enter(owner));
    true
}

#[test]
fn monitor_timed_byref_alias_writes_once_for_timeout_and_acquisition() {
    use crate::{stack::native_effects::MONITOR_FLAG_WRITES, sync::SyncBlockOps};
    use dotnet_intrinsics_threading::MonitorHost;
    use dotnet_value::{
        object::Object,
        pointer::{ManagedPtr, PointerOrigin},
        storage::FieldStorage,
    };
    use dotnet_vm_ops::effects::NativeDispatchMode;

    let mut executor = executor();
    let method = timed_monitor_method(&executor);
    let owner = dotnet_utils::ArenaId::new(9999);
    let thread = executor.thread_id;
    for (index, acquired) in [false, true].into_iter().enumerate() {
        let (block, contention) = executor.with_arena(|arena| arena.mutate_root(|gc, engine| {
            // SAFETY: F1.GcHandleRooted — the engine is the live arena root for this mutation.
            let gc = unsafe { test_gc_handle(gc, engine) };
            let layout = dotnet_value::layout::FieldLayoutManager {
                fields: Default::default(), total_size: 1, alignment: 1,
                gc_desc: Default::default(), has_ref_fields: false,
            };
            let flag = ObjectRef::new(gc, HeapStorage::Obj(Box::new(Object::new(
                method.parent.clone(), Default::default(), FieldStorage::new(Arc::new(layout), vec![0]),
            ))));
            let address = flag.as_heap_storage(|storage| match storage {
                HeapStorage::Obj(obj) => obj.instance_storage.with_data(|data| std::ptr::NonNull::new(data.as_ptr().cast_mut())),
                _ => unreachable!(),
            });
            let pointer = ManagedPtr::new(address, TypeDescription::NULL, Some(flag), false, Some(dotnet_utils::ByteOffset::new(0)));
            engine.stack.local.heap.gchandles.borrow_mut().push(Some((flag, GCHandleType::Weak)));
            let object = ObjectRef::new(gc, HeapStorage::Str(CLRString::from("monitor")));
            let mut ctx = engine.ves_context(gc);
            let block = ctx.monitor_get_or_create_sync_block_for_object(object, gc);
            let contention = hold_monitor_for_timeout(&block, owner);

            // Keep one byref on the caller's stack while its alias is passed to TryEnter.
            ctx.push(StackValue::ManagedPtr(pointer.clone().into()));
            ctx.push(StackValue::ObjectRef(object));
            ctx.push(StackValue::Int32(i32::MAX));
            ctx.push(StackValue::ManagedPtr(pointer.into()));
            MONITOR_FLAG_WRITES.with(|writes| assert!(writes.borrow_mut().replace(Vec::new()).is_none()));
            assert_eq!(crate::intrinsics::dispatch_method_intrinsic(
                crate::intrinsics::MethodIntrinsicId::System_Threading_Monitor_TryEnter_a8f16726fa8c35c7,
                &mut ctx, method.clone(), &Default::default(), NativeDispatchMode::InlineCall,
            ), StepResult::Yield);
            (block, contention)
        })).unwrap();
        let mut contention = Some(contention);
        MONITOR_FLAG_WRITES.with(|writes| assert!(writes.borrow().as_ref().unwrap().is_empty()));
        collect(&mut executor);
        if acquired {
            release_monitor_for_test(&block, owner, contention.take().unwrap());
        } else {
            executor
                .with_arena(|arena| {
                    arena.mutate_root(|_, engine| {
                        let Some(crate::stack::native_effects::PendingWait::Monitor(wait)) =
                            &mut engine
                                .stack
                                .execution
                                .call_state
                                .native
                                .last_mut()
                                .unwrap()
                                .wait
                        else {
                            panic!("missing monitor wait");
                        };
                        // Expire the actual activation deadline without a wall-clock sleep.
                        wait.deadline = Some(std::time::Instant::now());
                    })
                })
                .unwrap();
        }
        assert_eq!(resume(&mut executor), StepResult::Continue);
        let writes = MONITOR_FLAG_WRITES.with(|writes| writes.borrow_mut().take().unwrap());
        assert_eq!(writes, vec![acquired]);
        executor
            .with_arena(|arena| {
                arena.mutate_root(|gc, engine| {
                    // SAFETY: F1.GcHandleRooted — the engine is the live arena root for this mutation.
                    let gc = unsafe { test_gc_handle(gc, engine) };
                    let flag = engine.stack.local.heap.gchandles.borrow()[index]
                        .as_ref()
                        .unwrap()
                        .0;
                    let mut ctx = engine.ves_context(gc);
                    let alias = ctx.pop().as_managed_ptr();
                    let PointerOrigin::Heap(alias_owner) = alias.origin() else {
                        panic!("alias lost heap owner")
                    };
                    assert_eq!(*alias_owner, flag);
                    assert_eq!(
                        alias_owner.as_heap_storage(|storage| match storage {
                            HeapStorage::Obj(obj) => obj.instance_storage.with_data(|data| data[0]),
                            _ => unreachable!(),
                        }),
                        u8::from(acquired)
                    );
                    assert_eq!(ctx.frame_stack.current_frame().stack_height.as_usize(), 0);
                })
            })
            .unwrap();
        if acquired {
            assert!(block.exit(thread));
            assert!(!block.exit(thread));
        } else {
            assert!(!block.exit(thread));
            release_monitor_for_test(&block, owner, contention.take().unwrap());
        }
    }
}

#[cfg(feature = "multithreading")]
#[test]
fn monitor_timed_byref_writes_result_and_preserves_ownership() {
    use crate::sync::SyncBlockOps;
    use dotnet_intrinsics_threading::MonitorHost;
    use dotnet_value::pointer::ManagedPtr;
    use dotnet_vm_ops::effects::NativeDispatchMode;
    let mut executor = executor();
    let method = timed_monitor_method(&executor);
    let owner = dotnet_utils::ArenaId::new(9999);
    let thread = executor.thread_id;
    executor.with_arena(|arena| arena.mutate_root(|gc, engine| {
        // SAFETY: F1.GcHandleRooted — the engine is the live arena root for this mutation.
        let gc = unsafe { test_gc_handle(gc, engine) };
        let frames = &mut engine.stack.execution.frame_stack;
        let serial = frames.issue_home_serial();
        let layout = dotnet_value::layout::FieldLayoutManager { fields: Default::default(), total_size: 1, alignment: 1, gc_desc: Default::default(), has_ref_fields: false };
        let layout = dotnet_vm_data::frame_homes::FrameHomeLayout::new(layout, Box::new([]), Box::new([]));
        frames.current_frame_mut().homes = dotnet_vm_data::frame_homes::FrameHomes::allocate(Arc::new(layout), serial).unwrap();
        let (address, _) = frames.home_block(serial).unwrap();
        let pointer = ManagedPtr::new_frame_home(address, TypeDescription::NULL, serial, dotnet_utils::ByteOffset::new(0), false);
        let object = ObjectRef::new(gc, HeapStorage::Str(CLRString::from("monitor")));
        let mut ctx = engine.ves_context(gc);
        let block = ctx.monitor_get_or_create_sync_block_for_object(object, gc);
        assert!(block.try_enter(owner));
        for (index, should_acquire) in [false, true, true].into_iter().enumerate() {
            if index == 1 {
                assert!(block.exit(owner));
            }
            // SAFETY: F10.FrameHomeChecked — this live one-byte block remains owned by the current frame.
            unsafe { address.as_ptr().write(0) };
            ctx.push(StackValue::ObjectRef(object));
            ctx.push(StackValue::Int32(0));
            ctx.push(StackValue::ManagedPtr(pointer.clone().into()));
            let result = crate::intrinsics::dispatch_method_intrinsic(
                crate::intrinsics::MethodIntrinsicId::System_Threading_Monitor_TryEnter_a8f16726fa8c35c7,
                &mut ctx, method.clone(), &Default::default(), NativeDispatchMode::InlineCall,
            );
            assert_eq!(result, StepResult::Jump(1));
            // SAFETY: F10.FrameHomeChecked — this live one-byte block remains owned by the current frame.
            assert_eq!(unsafe { address.as_ptr().read() }, u8::from(should_acquire));
        }
        assert!(block.exit(thread));
        assert!(block.exit(thread));
        assert!(!block.exit(thread));
    })).unwrap();
}

#[cfg(feature = "multithreading")]
#[test]
fn monitor_heap_byref_survives_collection_and_writes_owner() {
    use crate::sync::SyncBlockOps;
    use dotnet_intrinsics_threading::MonitorHost;
    use dotnet_value::{object::Object, pointer::ManagedPtr, storage::FieldStorage};
    use dotnet_vm_ops::effects::NativeDispatchMode;
    let mut executor = executor();
    let method = monitor_byref_method(&executor);
    let owner = dotnet_utils::ArenaId::new(9999);
    let thread = executor.thread_id;
    let block = executor
        .with_arena(|arena| {
            arena.mutate_root(|gc, engine| {
                // SAFETY: F1.GcHandleRooted — the engine is the live arena root for this mutation.
                let gc = unsafe { test_gc_handle(gc, engine) };
                let layout = dotnet_value::layout::FieldLayoutManager {
                    fields: Default::default(),
                    total_size: 1,
                    alignment: 1,
                    gc_desc: Default::default(),
                    has_ref_fields: false,
                };
                let flag = ObjectRef::new(
                    gc,
                    HeapStorage::Obj(Box::new(Object::new(
                        method.parent.clone(),
                        Default::default(),
                        FieldStorage::new(Arc::new(layout), vec![0]),
                    ))),
                );
                let address = flag.as_heap_storage(|storage| match storage {
                    HeapStorage::Obj(obj) => obj
                        .instance_storage
                        .with_data(|data| std::ptr::NonNull::new(data.as_ptr().cast_mut())),
                    _ => unreachable!(),
                });
                let pointer = ManagedPtr::new(
                    address,
                    TypeDescription::NULL,
                    Some(flag),
                    false,
                    Some(dotnet_utils::ByteOffset::new(0)),
                );
                engine
                    .stack
                    .local
                    .heap
                    .gchandles
                    .borrow_mut()
                    .push(Some((flag, GCHandleType::Weak)));
                let object = ObjectRef::new(gc, HeapStorage::Str(CLRString::from("monitor")));
                let mut ctx = engine.ves_context(gc);
                let block = ctx.monitor_get_or_create_sync_block_for_object(object, gc);
                assert!(block.try_enter(owner));
                ctx.push(StackValue::ObjectRef(object));
                ctx.push(StackValue::ManagedPtr(pointer.into()));
                assert_eq!(crate::intrinsics::dispatch_method_intrinsic(
            crate::intrinsics::MethodIntrinsicId::System_Threading_Monitor_Enter_cf1115d21714f8ee,
            &mut ctx, method, &Default::default(), NativeDispatchMode::InlineCall,
        ), StepResult::Yield);
                block
            })
        })
        .unwrap();
    collect(&mut executor);
    executor
        .with_arena(|arena| {
            arena.mutate(|_, engine| {
                let handles = engine.stack.local.heap.gchandles.borrow();
                assert!(handles[0].as_ref().unwrap().0.0.is_some());
            })
        })
        .unwrap();
    assert!(block.exit(owner));
    assert_eq!(resume(&mut executor), StepResult::Continue);
    executor
        .with_arena(|arena| {
            arena.mutate(|_, engine| {
                let handles = engine.stack.local.heap.gchandles.borrow();
                let flag = handles[0].as_ref().unwrap().0;
                assert_eq!(
                    flag.as_heap_storage(|storage| match storage {
                        HeapStorage::Obj(obj) => obj.instance_storage.with_data(|data| data[0]),
                        _ => unreachable!(),
                    }),
                    1
                );
            })
        })
        .unwrap();
    assert!(block.exit(thread));
    collect(&mut executor);
    executor
        .with_arena(|arena| {
            arena.mutate(|_, engine| {
                let handles = engine.stack.local.heap.gchandles.borrow();
                assert!(handles[0].as_ref().unwrap().0.0.is_none());
            })
        })
        .unwrap();
}

#[test]
fn monitor_byref_rejects_out_of_range_before_acquisition() {
    use crate::sync::SyncBlockOps;
    use dotnet_intrinsics_threading::MonitorHost;
    use dotnet_value::pointer::ManagedPtr;
    use dotnet_vm_ops::effects::NativeDispatchMode;
    let mut executor = executor();
    let method = monitor_byref_method(&executor);
    let thread = executor.thread_id;
    executor
        .with_arena(|arena| {
            arena.mutate_root(|gc, engine| {
                // SAFETY: F1.GcHandleRooted — the engine is the live arena root for this mutation.
                let gc = unsafe { test_gc_handle(gc, engine) };
                let frames = &mut engine.stack.execution.frame_stack;
                let serial = frames.issue_home_serial();
                let layout = dotnet_value::layout::FieldLayoutManager {
                    fields: Default::default(),
                    total_size: 1,
                    alignment: 1,
                    gc_desc: Default::default(),
                    has_ref_fields: false,
                };
                let layout = dotnet_vm_data::frame_homes::FrameHomeLayout::new(
                    layout,
                    Box::new([]),
                    Box::new([]),
                );
                frames.current_frame_mut().homes =
                    dotnet_vm_data::frame_homes::FrameHomes::allocate(Arc::new(layout), serial)
                        .unwrap();
                let (address, _) = frames.home_block(serial).unwrap();
                let one_past = std::ptr::NonNull::new(address.as_ptr().wrapping_add(1)).unwrap();
                let pointer = ManagedPtr::new_frame_home(
                    one_past,
                    TypeDescription::NULL,
                    serial,
                    dotnet_utils::ByteOffset::new(1),
                    false,
                );
                let object = ObjectRef::new(gc, HeapStorage::Str(CLRString::from("monitor")));
                let mut ctx = engine.ves_context(gc);
                let block = ctx.monitor_get_or_create_sync_block_for_object(object, gc);
                ctx.push(StackValue::ObjectRef(object));
                ctx.push(StackValue::ManagedPtr(pointer.into()));
                assert!(matches!(crate::intrinsics::dispatch_method_intrinsic(
            crate::intrinsics::MethodIntrinsicId::System_Threading_Monitor_Enter_cf1115d21714f8ee,
            &mut ctx, method, &Default::default(), NativeDispatchMode::InlineCall,
        ), StepResult::Error(_)));
                assert!(ctx.call_state.native.is_empty());
                assert!(!block.exit(thread));
            })
        })
        .unwrap();
}

#[test]
fn monitor_heap_byref_survives_stop_before_dispatch() {
    use crate::sync::SyncBlockOps;
    use dotnet_intrinsics_threading::MonitorHost;
    use dotnet_value::{object::Object, pointer::ManagedPtr, storage::FieldStorage};
    use dotnet_vm_ops::effects::NativeDispatchMode;
    let mut executor = executor();
    let method = monitor_byref_method(&executor);
    let thread = executor.thread_id;
    let block = executor
        .with_arena(|arena| {
            arena.mutate_root(|gc, engine| {
                // SAFETY: F1.GcHandleRooted — the engine is the live arena root for this mutation.
                let gc = unsafe { test_gc_handle(gc, engine) };
                let layout = dotnet_value::layout::FieldLayoutManager {
                    fields: Default::default(),
                    total_size: 1,
                    alignment: 1,
                    gc_desc: Default::default(),
                    has_ref_fields: false,
                };
                let flag = ObjectRef::new(
                    gc,
                    HeapStorage::Obj(Box::new(Object::new(
                        method.parent.clone(),
                        Default::default(),
                        FieldStorage::new(Arc::new(layout), vec![0]),
                    ))),
                );
                let address = flag.as_heap_storage(|storage| match storage {
                    HeapStorage::Obj(obj) => obj
                        .instance_storage
                        .with_data(|data| std::ptr::NonNull::new(data.as_ptr().cast_mut())),
                    _ => unreachable!(),
                });
                let pointer = ManagedPtr::new(
                    address,
                    TypeDescription::NULL,
                    Some(flag),
                    false,
                    Some(dotnet_utils::ByteOffset::new(0)),
                );
                engine
                    .stack
                    .local
                    .heap
                    .gchandles
                    .borrow_mut()
                    .push(Some((flag, GCHandleType::Weak)));
                let object = ObjectRef::new(gc, HeapStorage::Str(CLRString::from("monitor")));
                engine.stack.local.heap.needs_full_collect.set(true);
                let mut ctx = engine.ves_context(gc);
                let block = ctx.monitor_get_or_create_sync_block_for_object(object, gc);
                ctx.push(StackValue::ObjectRef(object));
                ctx.push(StackValue::ManagedPtr(pointer.into()));
                assert_eq!(crate::intrinsics::dispatch_method_intrinsic(
            crate::intrinsics::MethodIntrinsicId::System_Threading_Monitor_Enter_cf1115d21714f8ee,
            &mut ctx, method, &Default::default(), NativeDispatchMode::InlineCall,
        ), StepResult::Yield);
                block
            })
        })
        .unwrap();
    collect(&mut executor);
    assert_eq!(resume(&mut executor), StepResult::Continue);
    executor
        .with_arena(|arena| {
            arena.mutate(|_, engine| {
                let handles = engine.stack.local.heap.gchandles.borrow();
                let flag = handles[0].as_ref().unwrap().0;
                assert_eq!(
                    flag.as_heap_storage(|storage| match storage {
                        HeapStorage::Obj(obj) => obj.instance_storage.with_data(|data| data[0]),
                        _ => unreachable!(),
                    }),
                    1
                );
            })
        })
        .unwrap();
    assert!(block.exit(thread));
}

#[cfg(feature = "multithreading")]
#[test]
fn monitor_wait_rejects_retired_frame_home_without_unlocking() {
    use crate::sync::SyncBlockOps;
    use dotnet_intrinsics_threading::MonitorHost;
    use dotnet_value::pointer::ManagedPtr;
    use dotnet_vm_ops::effects::NativeDispatchMode;
    let mut executor = executor();
    let method = monitor_byref_method(&executor);
    let owner = dotnet_utils::ArenaId::new(9999);
    let thread = executor.thread_id;
    let block = executor
        .with_arena(|arena| {
            arena.mutate_root(|gc, engine| {
                // SAFETY: F1.GcHandleRooted — the engine is the live arena root for this mutation.
                let gc = unsafe { test_gc_handle(gc, engine) };
                let frames = &mut engine.stack.execution.frame_stack;
                let serial = frames.issue_home_serial();
                let layout = dotnet_value::layout::FieldLayoutManager {
                    fields: Default::default(),
                    total_size: 1,
                    alignment: 1,
                    gc_desc: Default::default(),
                    has_ref_fields: false,
                };
                let layout = dotnet_vm_data::frame_homes::FrameHomeLayout::new(
                    layout,
                    Box::new([]),
                    Box::new([]),
                );
                frames.current_frame_mut().homes =
                    dotnet_vm_data::frame_homes::FrameHomes::allocate(Arc::new(layout), serial)
                        .unwrap();
                let (address, _) = frames.home_block(serial).unwrap();
                let pointer = ManagedPtr::new_frame_home(
                    address,
                    TypeDescription::NULL,
                    serial,
                    dotnet_utils::ByteOffset::new(0),
                    false,
                );
                let object = ObjectRef::new(gc, HeapStorage::Str(CLRString::from("monitor")));
                let mut ctx = engine.ves_context(gc);
                let block = ctx.monitor_get_or_create_sync_block_for_object(object, gc);
                assert!(block.try_enter(owner));
                ctx.push(StackValue::ObjectRef(object));
                ctx.push(StackValue::ManagedPtr(pointer.into()));
                assert_eq!(crate::intrinsics::dispatch_method_intrinsic(
            crate::intrinsics::MethodIntrinsicId::System_Threading_Monitor_Enter_cf1115d21714f8ee,
            &mut ctx, method, &Default::default(), NativeDispatchMode::InlineCall,
        ), StepResult::Yield);
                ctx.frame_stack.current_frame_mut().homes.release();
                assert!(ctx.frame_stack.home_block(serial).is_none());
                block
            })
        })
        .unwrap();
    assert!(block.exit(owner));
    assert!(matches!(resume(&mut executor), StepResult::Error(_)));
    assert!(block.exit(thread));
    assert!(!block.exit(thread));
}

#[cfg(feature = "multithreading")]
#[test]
fn monitor_timed_dispatch_keeps_deadline_across_collection() {
    use crate::{stack::native_effects::PendingWait, sync::SyncBlockOps};
    use dotnet_intrinsics_threading::MonitorHost;
    use dotnet_value::pointer::ManagedPtr;
    use dotnet_vm_ops::effects::NativeDispatchMode;
    let mut executor = executor();
    let method = timed_monitor_method(&executor);
    let owner = dotnet_utils::ArenaId::new(9999);
    let thread = executor.thread_id;
    let (block, serial, deadline) = executor.with_arena(|arena| arena.mutate_root(|gc, engine| {
        // SAFETY: F1.GcHandleRooted — the engine is the live arena root for this mutation.
        let gc = unsafe { test_gc_handle(gc, engine) };
        let frames = &mut engine.stack.execution.frame_stack;
        let serial = frames.issue_home_serial();
        let layout = dotnet_value::layout::FieldLayoutManager { fields: Default::default(), total_size: 1, alignment: 1, gc_desc: Default::default(), has_ref_fields: false };
        let layout = dotnet_vm_data::frame_homes::FrameHomeLayout::new(layout, Box::new([]), Box::new([]));
        frames.current_frame_mut().homes = dotnet_vm_data::frame_homes::FrameHomes::allocate(Arc::new(layout), serial).unwrap();
        let (address, _) = frames.home_block(serial).unwrap();
        let pointer = ManagedPtr::new_frame_home(address, TypeDescription::NULL, serial, dotnet_utils::ByteOffset::new(0), false);
        let object = ObjectRef::new(gc, HeapStorage::Str(CLRString::from("monitor")));
        let mut ctx = engine.ves_context(gc);
        let block = ctx.monitor_get_or_create_sync_block_for_object(object, gc);
        assert!(block.try_enter(owner));
        ctx.push(StackValue::ObjectRef(object));
        ctx.push(StackValue::Int32(10_000));
        ctx.push(StackValue::ManagedPtr(pointer.into()));
        assert_eq!(crate::intrinsics::dispatch_method_intrinsic(
            crate::intrinsics::MethodIntrinsicId::System_Threading_Monitor_TryEnter_a8f16726fa8c35c7,
            &mut ctx, method, &Default::default(), NativeDispatchMode::InlineCall,
        ), StepResult::Yield);
        let Some(PendingWait::Monitor(wait)) = &ctx.call_state.native.last().unwrap().wait else { panic!("monitor wait missing") };
        (block, serial, wait.deadline.unwrap())
    })).unwrap();
    collect(&mut executor);
    executor
        .with_arena(|arena| {
            arena.mutate_root(|_, engine| {
                let Some(PendingWait::Monitor(wait)) = &mut engine
                    .stack
                    .execution
                    .call_state
                    .native
                    .last_mut()
                    .unwrap()
                    .wait
                else {
                    panic!("monitor wait missing")
                };
                assert_eq!(wait.deadline, Some(deadline));
                wait.deadline =
                    Some(std::time::Instant::now() - std::time::Duration::from_millis(1));
            })
        })
        .unwrap();
    assert_eq!(resume(&mut executor), StepResult::Continue);
    executor
        .with_arena(|arena| {
            arena.mutate(|_, engine| {
                let (address, _) = engine
                    .stack
                    .execution
                    .frame_stack
                    .home_block(serial)
                    .unwrap();
                // SAFETY: F10.FrameHomeChecked — the frame still owns this live one-byte block.
                assert_eq!(unsafe { address.as_ptr().read() }, 0);
            })
        })
        .unwrap();
    assert!(!block.exit(thread));
    assert!(block.exit(owner));
}

#[test]
fn monitor_retry_record_is_removed_on_managed_throw() {
    use crate::stack::ops::ExceptionOps;
    use dotnet_intrinsics_threading::MonitorHost;
    use dotnet_vm_ops::MemoryOps;
    let (mut executor, exception_type) = boundary_executor(false);
    let method = monitor_byref_method(&executor);
    executor
        .with_arena(|arena| {
            arena.mutate_root(|gc, engine| {
                // SAFETY: F1.GcHandleRooted — the engine is the live arena root for this mutation.
                let gc = unsafe { test_gc_handle(gc, engine) };
                let mut ctx = engine.ves_context(gc);
                let object = ctx.new_object(exception_type).unwrap();
                let exception = ctx.alloc_obj_ref(gc, object);
                ctx.monitor_retry_deadline(&method, 1000);
                assert!(ctx.frame_stack.current_frame().monitor_retry.is_some());
                let _ = ctx.throw(exception);
                assert!(
                    ctx.frame_stack
                        .frames
                        .iter()
                        .all(|frame| frame.monitor_retry.is_none())
                );
            })
        })
        .unwrap();
}

#[test]
fn sync_block_is_assigned_once_per_object() {
    use crate::sync::SyncManagerOps;
    use dotnet_intrinsics_threading::MonitorHost;
    let mut executor = executor();
    executor
        .with_arena(|arena| {
            arena.mutate_root(|gc, engine| {
                // SAFETY: F1.GcHandleRooted — the engine is the live arena root for this mutation.
                let gc = unsafe { test_gc_handle(gc, engine) };
                let ctx = engine.ves_context(gc);
                let object = ObjectRef::new(gc, HeapStorage::Str(CLRString::from("monitor")));
                assert_eq!(object.sync_block_index(), None);

                let first = ctx.monitor_get_or_create_sync_block_for_object(object, gc);
                let index = object
                    .sync_block_index()
                    .expect("first lookup assigns an index");
                let second = ctx.monitor_get_or_create_sync_block_for_object(object, gc);
                assert!(Arc::ptr_eq(&first, &second));
                assert_eq!(object.sync_block_index(), Some(index));

                // An assigned index is never replaced: a later assignment attempt must not even
                // create a block.
                let assigned = object.get_or_assign_sync_block_index(gc, || {
                    panic!("an object with a sync block must not be assigned another")
                });
                assert_eq!(assigned, index);
                let (unrelated, _block) = ctx.shared.sync_blocks.create_sync_block();
                assert_ne!(unrelated, index);
                assert_eq!(object.sync_block_index(), Some(index));
            })
        })
        .unwrap();
}

#[cfg(feature = "multithreading")]
fn queue_collected_finalizer(executor: &mut Executor) {
    let mut res = Resolution::new(Module::new("mscorlib.dll"));
    res.assembly = Some(Assembly::new("mscorlib"));
    let mut object = TypeDefinition::new(None, "Object");
    object.namespace = Some("System".into());
    let ty_index = res.push_type_definition(object);
    let mut signature = parameterless_i32_signature();
    signature.instance = true;
    signature.return_type = ReturnType(vec![], None);
    let mut finalizer = Method::new(
        Accessibility::Public,
        signature,
        "Finalize",
        Some(make_test_method!(max_stack: 1, instructions: vec![Instruction::Return])),
    );
    finalizer.virtual_member = true;
    res.push_method(ty_index, finalizer);
    let res = executor.shared.loader.register_owned_assembly(res);
    let ty = TypeDescription::new(res, ty_index);
    executor
        .with_arena(|arena| {
            arena.mutate_root(|gc, engine| {
                // SAFETY: F1.GcHandleRooted — the engine is the live arena root for this mutation.
                let gc = unsafe { test_gc_handle(gc, engine) };
                let layout = dotnet_value::layout::FieldLayoutManager {
                    fields: Default::default(),
                    total_size: 0,
                    alignment: 1,
                    gc_desc: Default::default(),
                    has_ref_fields: false,
                };
                let object = ObjectRef::new(
                    gc,
                    HeapStorage::Obj(Box::new(
                        dotnet_value::object::Object::new_with_finalizer_flag(
                            ty,
                            GenericLookup::default(),
                            dotnet_value::storage::FieldStorage::new(Arc::new(layout), vec![]),
                            true,
                        ),
                    )),
                );
                engine.ves_context(gc).register_new_object(&object);
            })
        })
        .unwrap();
    collect(executor);
    assert!(executor.shared.has_pending_finalizers());
    executor
        .with_arena(|arena| {
            arena.mutate(|_, engine| {
                assert_eq!(
                    engine.stack.local.heap.pending_finalization.borrow().len(),
                    1
                );
            })
        })
        .unwrap();
}

#[cfg(feature = "multithreading")]
fn run_queued_finalizer(executor: &mut Executor) {
    executor
        .with_arena(|arena| {
            arena.mutate_root(|gc, engine| {
                // SAFETY: F1.GcHandleRooted — the engine is the live arena root for this mutation.
                let gc = unsafe { test_gc_handle(gc, engine) };
                assert_eq!(
                    engine.process_pending_finalizers(gc),
                    StepResult::FramePushed
                );
                assert_eq!(engine.step(gc), StepResult::Continue);
                assert!(engine.stack.parked_finalizer_caller.is_none());
            })
        })
        .unwrap();
    assert!(!executor.shared.has_pending_finalizers());
}

#[cfg(feature = "multithreading")]
#[test]
fn retired_owner_remains_registered_while_other_arena_has_queued_finalizer() {
    use dotnet_utils::gc::cross_arena::try_acquire_lease;
    use std::{
        sync::mpsc,
        time::{Duration, Instant},
    };
    let mut executor = executor();
    queue_collected_finalizer(&mut executor);
    let shared = executor.shared.clone();
    struct AbortAndJoin(Arc<SharedGlobalState>);
    impl Drop for AbortAndJoin {
        fn drop(&mut self) {
            self.0.abort_signal().request_abort();
            self.0.shutdown_workers();
        }
    }
    let cleanup = AbortAndJoin(shared.clone());
    let worker_shared = shared.clone();
    let (ready_tx, ready_rx) = mpsc::channel();
    let (probe_tx, probe_rx) = mpsc::channel();
    let worker = shared.workers.spawn_owner(move |running| {
        super::super::RETIRED_QUIESCENCE_TEST_HOOK.with(|hook| {
            *hook.borrow_mut() = Some(Box::new(move |quiescent| {
                let _ = probe_tx.send(quiescent);
            }));
        });
        let owner = Executor::new(worker_shared);
        ready_tx.send(owner.thread_id).unwrap();
        owner.serve_retired(running.complete(ExecutorResult::Exited(0)));
    });
    let id = ready_rx.recv_timeout(Duration::from_secs(30)).unwrap();
    let deadline = Instant::now() + Duration::from_secs(30);
    while !worker.is_finished() && Instant::now() < deadline {
        std::thread::yield_now();
    }
    let retired = worker.is_finished();
    let lease = try_acquire_lease(id).unwrap();
    shared.thread_manager.shutdown_retired();
    let quiescent = probe_rx.recv_timeout(Duration::from_secs(30));
    let still_registered = lease.is_valid();
    let phase = worker.phase();
    run_queued_finalizer(&mut executor);
    let deadline = Instant::now() + Duration::from_secs(30);
    while lease.is_valid() && Instant::now() < deadline {
        std::thread::yield_now();
    }
    let removed = !lease.is_valid();
    drop(lease);
    shared.shutdown_workers();
    assert_eq!(quiescent, Ok(false));
    assert!(retired && still_registered && removed);
    assert_eq!(phase, "retired");
    assert_eq!(worker.phase(), "drained");
    drop(cleanup);
}

#[cfg(feature = "multithreading")]
#[test]
fn finalizer_wait_rechecks_after_another_owner_finishes() {
    use crate::{
        stack::native_effects::{WAIT_TEST_HOOK, WaitTestPoint},
        test_utils::worker::Worker,
    };
    use dotnet_vm_ops::effects::{NativeDispatchMode, NativeIntrinsicHost};
    use std::{sync::mpsc, time::Duration};
    let mut executor = executor();
    queue_collected_finalizer(&mut executor);
    let info = executor
        .with_arena(|arena| {
            arena.mutate(|_, engine| {
                engine
                    .stack
                    .execution
                    .frame_stack
                    .current_frame()
                    .state
                    .info_handle
                    .clone()
            })
        })
        .unwrap();
    let method = condition_wait_method(&executor, false);
    let shared = executor.shared.clone();
    let (registered_tx, registered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let (result_tx, result_rx) = mpsc::channel();
    let handle = std::thread::spawn(move || {
        let mut waiter = Executor::new(shared);
        waiter
            .with_arena(|arena| {
                arena.mutate_root(|gc, engine| {
                    engine.stack.execution.frame_stack.push_frame(
                        BasePointer::default(),
                        info,
                        Default::default(),
                        Default::default(),
                    );
                    WAIT_TEST_HOOK.with(|hook| {
                        *hook.borrow_mut() = Some(Box::new(move |point| {
                            if matches!(point, WaitTestPoint::FinalizersRegistered) {
                                registered_tx.send(()).unwrap();
                                let _ = release_rx.recv_timeout(Duration::from_secs(30));
                            }
                        }))
                    });
                    // SAFETY: F1.GcHandleRooted — the engine is the live arena root for this mutation.
                    let gc = unsafe { test_gc_handle(gc, engine) };
                    let result = engine.ves_context(gc).start_native(
                        method,
                        NativeDispatchMode::InlineCall,
                        &Default::default(),
                        None,
                        native_test_fallback,
                        condition_finalizer_body,
                    );
                    WAIT_TEST_HOOK.with(|hook| drop(hook.borrow_mut().take()));
                    result_tx
                        .send((result, engine.stack.execution.call_state.native.is_empty()))
                        .unwrap();
                })
            })
            .unwrap();
    });
    let worker = Worker::new(handle, release_tx);
    registered_rx
        .recv_timeout(Duration::from_secs(30))
        .expect("wait was not registered");
    run_queued_finalizer(&mut executor);
    worker.resources().send(()).unwrap();
    let result = result_rx
        .recv_timeout(Duration::from_secs(30))
        .expect("waiter did not resume");
    worker.finish().unwrap();
    assert_eq!(result, (StepResult::Jump(1), true));
}

fn sleep_test_body(
    mut fx: dotnet_vm_ops::effects::VmEffects,
) -> dotnet_vm_ops::effects::NativeFuture {
    Box::pin(async move {
        fx.sleep_until(Some(
            std::time::Instant::now() + std::time::Duration::from_secs(30),
        ))
        .await
    })
}

fn start_sleep(
    executor: &mut Executor,
    body: fn(dotnet_vm_ops::effects::VmEffects) -> dotnet_vm_ops::effects::NativeFuture,
) -> StepResult {
    use dotnet_vm_ops::effects::{NativeDispatchMode, NativeIntrinsicHost};
    let method = condition_wait_method(executor, false);
    executor
        .with_arena(|arena| {
            arena.mutate_root(|gc, engine| {
                // SAFETY: F1.GcHandleRooted — the engine is the live arena root.
                let gc = unsafe { test_gc_handle(gc, engine) };
                let mut ctx = engine.ves_context(gc);
                ctx.push(StackValue::Int32(0));
                ctx.start_native(
                    method,
                    NativeDispatchMode::InlineCall,
                    &Default::default(),
                    None,
                    native_test_fallback,
                    body,
                )
            })
        })
        .unwrap()
}

#[test]
fn sleep_deadline_survives_collection_and_expires() {
    use crate::stack::native_effects::PendingWait;
    let mut executor = executor();
    assert_eq!(
        start_sleep(&mut executor, sleep_test_body),
        StepResult::Yield
    );
    let deadline = executor
        .with_arena(|arena| {
            arena.mutate(|_, engine| {
                match engine
                    .stack
                    .execution
                    .call_state
                    .native
                    .last()
                    .unwrap()
                    .wait
                    .as_ref()
                    .unwrap()
                {
                    PendingWait::Sleep(_, deadline) => *deadline,
                    _ => panic!("expected sleep"),
                }
            })
        })
        .unwrap();
    collect(&mut executor);
    assert_eq!(resume(&mut executor), StepResult::Yield);
    executor
        .with_arena(|arena| {
            arena.mutate_root(|_, engine| {
                match engine
                    .stack
                    .execution
                    .call_state
                    .native
                    .last_mut()
                    .unwrap()
                    .wait
                    .as_mut()
                    .unwrap()
                {
                    PendingWait::Sleep(_, current) => {
                        assert_eq!(*current, deadline);
                        *current = Some(std::time::Instant::now());
                    }
                    _ => panic!("expected sleep"),
                }
            })
        })
        .unwrap();
    assert!(matches!(executor.run(), ExecutorResult::Exited(0)));
}

#[cfg(feature = "multithreading")]
fn infinite_sleep_test_body(
    mut fx: dotnet_vm_ops::effects::VmEffects,
) -> dotnet_vm_ops::effects::NativeFuture {
    Box::pin(async move { fx.sleep_until(None).await })
}

#[cfg(feature = "multithreading")]
#[test]
fn sleep_park_participates_in_gc_and_abort() {
    use crate::test_utils::worker::Worker;
    use std::{sync::mpsc, thread, time::Duration};
    let mut executor = executor();
    assert_eq!(
        start_sleep(&mut executor, infinite_sleep_test_body),
        StepResult::Yield
    );
    let shared = executor.shared.clone();
    let abort = shared.abort_signal();
    let (start_tx, start_rx) = mpsc::channel();
    let worker = Worker::new(
        thread::spawn(move || {
            let mut collector = Executor::new(shared);
            start_rx.recv_timeout(Duration::from_secs(5)).unwrap();
            let result = collector.perform_full_gc().is_ok();
            abort.request_abort();
            result
        }),
        (),
    );
    crate::executor::WAIT_TEST_HOOK.with(|hook| {
        *hook.borrow_mut() = Some(Box::new(move || {
            start_tx.send(()).unwrap();
        }))
    });
    assert!(matches!(executor.run(), ExecutorResult::Error(_)));
    assert!(worker.finish().unwrap());
    executor
        .with_arena(|arena| {
            arena.mutate(|_, engine| assert!(engine.stack.execution.call_state.native.is_empty()))
        })
        .unwrap();
}

#[cfg(feature = "multithreading")]
fn pulse_wait_body(
    mut fx: dotnet_vm_ops::effects::VmEffects,
) -> dotnet_vm_ops::effects::NativeFuture {
    Box::pin(async move {
        let object = fx.vm().await?.pop_object()?;
        assert!(fx.monitor_wait(object, None).await?);
        Ok(())
    })
}

#[cfg(feature = "multithreading")]
fn start_pulse_wait(
    executor: &mut Executor,
    hook: impl FnOnce(Arc<crate::sync::SyncBlock>),
) -> (StepResult, Arc<crate::sync::SyncBlock>) {
    use crate::sync::SyncBlockOps;
    use dotnet_intrinsics_threading::MonitorHost;
    use dotnet_vm_ops::effects::{NativeDispatchMode, NativeIntrinsicHost};
    let method = condition_wait_method(executor, true);
    let thread = executor.thread_id;
    executor
        .with_arena(|arena| {
            arena.mutate_root(|gc, engine| {
                // SAFETY: the engine is the root of this arena mutation.
                let gc = unsafe { test_gc_handle(gc, engine) };
                let source = ObjectRef::new(gc, HeapStorage::Str(CLRString::from("wait")));
                let mut ctx = engine.ves_context(gc);
                let block = ctx.monitor_get_or_create_sync_block_for_object(source, gc);
                assert!(block.try_enter(thread));
                assert!(block.try_enter(thread));
                ctx.push(StackValue::Int32(0));
                ctx.push(StackValue::ObjectRef(source));
                hook(block.clone());
                let result = ctx.start_native(
                    method,
                    NativeDispatchMode::InlineCall,
                    &Default::default(),
                    None,
                    native_test_fallback,
                    pulse_wait_body,
                );
                (result, block)
            })
        })
        .unwrap()
}

#[cfg(feature = "multithreading")]
#[test]
fn monitor_pulse_schedules_preserve_wake_and_recursion() {
    use crate::stack::native_effects::{WAIT_TEST_HOOK, WaitTestPoint};
    use crate::sync::SyncBlockOps;
    let other = dotnet_utils::ArenaId::new(9999);
    for schedule in 0..3 {
        let mut executor = executor();
        let thread = executor.thread_id;
        let (result, block) = start_pulse_wait(&mut executor, |block| {
            if schedule == 2 {
                return;
            }
            let mut fired = false;
            WAIT_TEST_HOOK.with(|hook| {
                *hook.borrow_mut() = Some(Box::new(move |point| {
                    if !fired
                        && matches!(
                            (schedule, point),
                            (0, WaitTestPoint::Probed) | (1, WaitTestPoint::Registered)
                        )
                    {
                        fired = true;
                        assert!(block.try_enter(other));
                        assert!(block.pulse(other, false));
                        assert!(block.exit(other));
                    }
                }))
            });
        });
        WAIT_TEST_HOOK.with(|hook| drop(hook.borrow_mut().take()));
        if schedule == 2 {
            assert_eq!(result, StepResult::Yield);
            collect(&mut executor);
            assert!(block.try_enter(other));
            assert!(block.pulse(other, false));
            assert!(block.exit(other));
            assert_eq!(resume(&mut executor), StepResult::Continue);
        } else {
            assert_eq!(result, StepResult::Jump(1));
        }
        assert!(block.exit(thread));
        assert!(block.exit(thread));
        assert!(!block.exit(thread));
    }
}

#[cfg(feature = "multithreading")]
#[test]
fn monitor_timeout_and_pulse_are_serialized_and_cancel_removes_waiter() {
    use crate::sync::{MonitorWait, SyncBlockOps, SyncManagerOps};
    use std::time::{Duration, Instant};
    let (_, block) = crate::sync::SyncBlockManager::new().create_sync_block();
    let a = dotnet_utils::ArenaId::new(100);
    let b = dotnet_utils::ArenaId::new(101);
    let c = dotnet_utils::ArenaId::new(102);
    assert!(block.try_enter(a));
    assert!(block.pulse(a, false)); // No future waiter inherits this pulse.
    let mut expired = MonitorWait::begin(
        block.clone(),
        a,
        Some(Instant::now() - Duration::from_millis(1)),
    )
    .unwrap();
    assert!(block.try_enter(b));
    let mut live = MonitorWait::begin(block.clone(), b, None).unwrap();
    assert!(block.try_enter(c));
    assert!(block.pulse(c, false)); // The expired waiter is skipped.
    assert!(block.exit(c));
    assert_eq!(expired.poll(a), Some(false));
    assert!(block.exit(a));
    assert_eq!(live.poll(b), Some(true));
    assert!(block.exit(b));
    assert!(block.try_enter(a));
    let mut pulsed_before_deadline = MonitorWait::begin(
        block.clone(),
        a,
        Some(Instant::now() + Duration::from_secs(60)),
    )
    .unwrap();
    assert!(block.try_enter(b));
    assert!(block.pulse(b, false));
    assert!(block.exit(b));
    pulsed_before_deadline.expire_for_test();
    assert_eq!(pulsed_before_deadline.poll(a), Some(true));
    assert!(block.exit(a));
    assert!(block.try_enter(a));
    let cancelled = MonitorWait::begin(block.clone(), a, None).unwrap();
    drop(cancelled);
    assert!(block.try_enter(b));
    assert!(block.pulse(b, false));
    assert!(block.exit(b));
}

#[cfg(feature = "multithreading")]
#[test]
fn monitor_wait_parks_outside_mutation_during_collection() {
    use crate::{sync::SyncBlockOps, test_utils::worker::Worker};
    use std::{sync::mpsc, thread, time::Duration};
    let mut executor = executor();
    let thread = executor.thread_id;
    let (result, block) = start_pulse_wait(&mut executor, |_| {});
    assert_eq!(result, StepResult::Yield);
    let shared = executor.shared.clone();
    let pulse_block = block.clone();
    let (ready_tx, ready_rx) = mpsc::channel();
    let (parked_tx, parked_rx) = mpsc::channel();
    let worker = Worker::new(
        thread::spawn(move || {
            let mut collector = Executor::new(shared);
            ready_tx.send(()).unwrap();
            let parked = parked_rx.recv_timeout(Duration::from_secs(5)).is_ok();
            let collected = parked && collector.perform_full_gc().is_ok();
            let owner = collector.thread_id;
            assert!(pulse_block.try_enter(owner));
            assert!(pulse_block.pulse(owner, false));
            assert!(pulse_block.exit(owner));
            collected
        }),
        (),
    );
    ready_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    crate::sync::WAIT_TEST_HOOK.with(|hook| {
        *hook.borrow_mut() = Some(Box::new(move || {
            let _ = parked_tx.send(());
        }))
    });
    assert!(matches!(executor.run(), ExecutorResult::Exited(0)));
    crate::sync::WAIT_TEST_HOOK.with(|hook| drop(hook.borrow_mut().take()));
    assert!(
        worker.finish().unwrap(),
        "collection did not complete while waiting"
    );
    assert!(block.exit(thread));
    assert!(block.exit(thread));
}

#[cfg(feature = "multithreading")]
#[test]
fn legacy_monitor_wait_retries_and_restores_recursion() {
    use crate::sync::SyncBlockOps;
    use dotnet_intrinsics_threading::{MonitorHost, monitor::intrinsic_monitor_wait};
    use dotnet_vm_ops::TypedStackOps;
    let mut executor = executor();
    let method = condition_wait_method(&executor, true);
    let thread = executor.thread_id;
    let other = dotnet_utils::ArenaId::new(9999);
    executor
        .with_arena(|arena| {
            arena.mutate_root(|gc, engine| {
                // SAFETY: the engine is the root of this arena mutation.
                let gc = unsafe { test_gc_handle(gc, engine) };
                let source = ObjectRef::new(gc, HeapStorage::Str(CLRString::from("legacy wait")));
                let mut ctx = engine.ves_context(gc);
                let block = ctx.monitor_get_or_create_sync_block_for_object(source, gc);
                assert!(block.try_enter(thread));
                assert!(block.try_enter(thread));
                ctx.push(StackValue::ObjectRef(source));
                ctx.push(StackValue::Int32(1000));
                assert_eq!(
                    intrinsic_monitor_wait(&mut ctx, method.clone(), &Default::default()),
                    StepResult::Yield
                );
                assert!(block.try_enter(other));
                assert!(block.pulse(other, false));
                assert!(block.exit(other));
                assert_eq!(
                    intrinsic_monitor_wait(&mut ctx, method, &Default::default()),
                    StepResult::Continue
                );
                assert_eq!(ctx.pop_i32(), 1);
                assert!(block.exit(thread));
                assert!(block.exit(thread));
                assert!(!block.exit(thread));
            })
        })
        .unwrap();
}

#[cfg(not(feature = "multithreading"))]
#[test]
fn monitor_wait_infinite_in_single_thread_reports_deadlock() {
    use crate::sync::SyncBlockOps;
    use dotnet_intrinsics_threading::MonitorHost;
    use dotnet_vm_ops::effects::{NativeDispatchMode, NativeIntrinsicHost};
    fn wait_forever(
        mut fx: dotnet_vm_ops::effects::VmEffects,
    ) -> dotnet_vm_ops::effects::NativeFuture {
        Box::pin(async move {
            let object = fx.vm().await?.pop_object()?;
            fx.monitor_wait(object, None).await?;
            Ok(())
        })
    }
    let mut executor = executor();
    let method = condition_wait_method(&executor, true);
    let thread = executor.thread_id;
    executor
        .with_arena(|arena| {
            arena.mutate_root(|gc, engine| {
                // SAFETY: the engine is the root of this arena mutation.
                let gc = unsafe { test_gc_handle(gc, engine) };
                let source = ObjectRef::new(gc, HeapStorage::Str(CLRString::from("wait")));
                let mut ctx = engine.ves_context(gc);
                let block = ctx.monitor_get_or_create_sync_block_for_object(source, gc);
                assert!(block.try_enter(thread));
                ctx.push(StackValue::ObjectRef(source));
                let result = ctx.start_native(
                    method,
                    NativeDispatchMode::InlineCall,
                    &Default::default(),
                    None,
                    native_test_fallback,
                    wait_forever,
                );
                assert!(matches!(result, StepResult::Error(_)));
            })
        })
        .unwrap();
}

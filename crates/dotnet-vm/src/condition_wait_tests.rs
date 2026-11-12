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
                        Some(native_test_fallback),
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
                        Some(native_test_fallback),
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
                        Some(native_test_fallback),
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
                    StepResult::not_implemented(format!(
                        "async intrinsic {method:?} reached through non-inline dispatch"
                    ))
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
    finalizer_wait_executor(false, false);
}

#[test]
fn finalizer_wait_from_finalizer_returns_immediately() {
    finalizer_wait_executor(true, false);
}

#[test]
fn finalizer_wait_from_finalizer_triggered_cctor_returns_immediately() {
    finalizer_wait_executor(true, true);
}

fn finalizer_wait_executor(test_self_wait: bool, cctor: bool) {
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
    let initializer_type =
        res.push_type_definition(TypeDefinition::new(None, "FinalizerInitializer"));
    let mut signature = parameterless_i32_signature();
    signature.return_type = ReturnType(vec![], None);
    let initializer_index = res.push_method(
        initializer_type,
        Method::new(
            Accessibility::Private,
            signature,
            ".cctor",
            Some(make_test_method!(max_stack: 0, instructions: vec![Instruction::Return])),
        ),
    );
    let res = executor.shared.loader.register_owned_assembly(res);
    let initializer_type = TypeDescription::new(res.clone(), initializer_type);
    let index = initializer_type
        .definition()
        .methods
        .iter()
        .position(|m| std::ptr::eq(m, &res.definition()[initializer_index]))
        .unwrap();
    let initializer = MethodDescription::new(
        initializer_type,
        GenericLookup::default(),
        res.clone(),
        MethodMemberIndex::Method(index),
    );
    let initializer_info = executor
        .shared
        .caches
        .get_method_info(initializer, &Default::default(), executor.shared.clone())
        .unwrap();
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
                    StepResult::not_implemented(format!(
                        "async intrinsic {method:?} reached through non-inline dispatch"
                    ))
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
                    if cctor {
                        let ctx = engine.ves_context(gc);
                        let top = ctx.top_of_stack();
                        ctx.frame_stack.push_frame(
                            BasePointer {
                                arguments: top,
                                locals: top,
                                stack: top,
                            },
                            initializer_info,
                            Default::default(),
                            Default::default(),
                        );
                    }
                    let ip = engine.stack.execution.frame_stack.current_frame().state.ip;
                    for mode in [NativeDispatchMode::InlineCall, NativeDispatchMode::Other] {
                        assert_eq!(
                            finalizer_wait_dispatch(
                                &mut engine.ves_context(gc),
                                method.clone(),
                                mode
                            ),
                            if mode == NativeDispatchMode::InlineCall {
                                StepResult::Jump(ip + 1)
                            } else {
                                StepResult::not_implemented(format!(
                                    "async intrinsic {method:?} reached through non-inline dispatch"
                                ))
                            }
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
                    if cctor {
                        let frames = &mut engine.stack.execution.frame_stack;
                        let initializer = frames.pop().unwrap();
                        frames.recycle_frame(initializer);
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
                    Some(native_test_fallback),
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
                        Some(native_test_fallback),
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
                        Some(native_test_fallback),
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

fn static_descriptor_body(
    mut fx: dotnet_vm_ops::effects::VmEffects,
) -> dotnet_vm_ops::effects::NativeFuture {
    Box::pin(async move {
        let flag = fx.vm().await?.prepare_monitor(true, true)?.1.unwrap();
        fx.yield_now().await?;
        let mut vm = fx.vm().await?;
        vm.with(|region| {
            let StackValue::ManagedPtr(pointer) = region.fetch(&flag)? else {
                panic!("expected managed pointer")
            };
            assert!(matches!(
                pointer.origin(),
                dotnet_value::pointer::PointerOrigin::Static(_)
            ));
            assert!(pointer.observe_address().is_none());
            Ok::<_, dotnet_types::error::VmError>(())
        })??;
        // SAFETY: F10.RawMemoryAccessValid — this is an initialized static bool, accessed
        // exclusively by the test executor; the backend checks the complete range.
        assert!(!unsafe { vm.read_monitor_flag(&flag)? });
        // SAFETY: F10.RawMemoryAccessValid — the same exclusively accessed bool is writable.
        unsafe { vm.write_monitor_flag(&flag, true)? };
        // SAFETY: F10.RawMemoryAccessValid — the same bool is initialized and exclusively held.
        assert!(unsafe { vm.read_monitor_flag(&flag)? });
        Ok(())
    })
}

#[test]
fn static_descriptor_survives_collection_and_resolves_raw_access() {
    use crate::stack::ops::{RawMemoryOps, VmResolutionOps};
    use dotnet_value::pointer::ManagedPtr;
    use dotnet_vm_ops::effects::{NativeDispatchMode, NativeIntrinsicHost};
    let mut executor = executor();
    let method = timed_monitor_method(&executor);
    executor
        .with_arena(|arena| {
            arena.mutate_root(|gc, engine| {
                // SAFETY: F1.GcHandleRooted — the engine is the live arena root.
                let gc = unsafe { test_gc_handle(gc, engine) };
                let mut ctx = engine.ves_context(gc);
                let generics = GenericLookup::default();
                ctx.shared
                    .statics
                    .init(
                        method.parent.clone(),
                        &ctx.with_generics(&generics),
                        ctx.thread_id.get(),
                    )
                    .unwrap();
                let pointer = ManagedPtr::new_static(
                    None,
                    TypeDescription::NULL,
                    method.parent.clone(),
                    generics,
                    false,
                    dotnet_utils::ByteOffset::ZERO,
                );
                ctx.push(StackValue::ObjectRef(ObjectRef(None)));
                ctx.push(StackValue::Int32(0));
                ctx.push(StackValue::ManagedPtr(pointer.into()));
                assert_eq!(
                    ctx.start_native(
                        method.clone(),
                        NativeDispatchMode::InlineCall,
                        &Default::default(),
                        None,
                        None,
                        static_descriptor_body
                    ),
                    StepResult::Yield
                );
            })
        })
        .unwrap();
    collect(&mut executor);
    assert_eq!(resume(&mut executor), StepResult::Continue);
    executor
        .with_arena(|arena| {
            arena.mutate_root(|gc, engine| {
                // SAFETY: F1.GcHandleRooted — the engine is the live arena root.
                let gc = unsafe { test_gc_handle(gc, engine) };
                let ctx = engine.ves_context(gc);
                let pointer = ManagedPtr::new_static(
                    None,
                    TypeDescription::NULL,
                    method.parent.clone(),
                    Default::default(),
                    false,
                    dotnet_utils::ByteOffset::ZERO,
                );
                let mut byte = [0];
                // SAFETY: F10.RawMemoryAccessValid — this is the initialized static bool field and the
                // origin-aware access checks its range while the executor has exclusive access.
                unsafe {
                    ctx.read_bytes(pointer.origin().clone(), pointer.byte_offset(), &mut byte)
                        .unwrap();
                }
                assert_eq!(byte, [1]);
            })
        })
        .unwrap();
}

#[test]
fn monitor_static_byref_inline_and_out_of_bounds() {
    use crate::{stack::ops::VmResolutionOps, sync::SyncBlockOps};
    use dotnet_intrinsics_threading::MonitorHost;
    use dotnet_value::pointer::ManagedPtr;
    use dotnet_vm_ops::effects::NativeDispatchMode;
    for out_of_bounds in [false, true] {
        let mut executor = executor();
        let method = timed_monitor_method(&executor);
        let thread = executor.thread_id;
        executor.with_arena(|arena| arena.mutate_root(|gc, engine| {
            // SAFETY: F1.GcHandleRooted — the engine is the live arena root.
            let gc = unsafe { test_gc_handle(gc, engine) };
            let mut ctx = engine.ves_context(gc);
            let generics = GenericLookup::default();
            ctx.shared.statics.init(method.parent.clone(), &ctx.with_generics(&generics), ctx.thread_id.get()).unwrap();
            let length = ctx.shared.statics.get(method.parent.clone(), &generics).storage.with_data(|data| data.len());
            let offset = if out_of_bounds { length } else { 0 };
            let pointer = ManagedPtr::new_static(None, TypeDescription::NULL, method.parent.clone(), generics, false, dotnet_utils::ByteOffset::new(offset));
            let object = ObjectRef::new(gc, HeapStorage::Str(CLRString::from("monitor")));
            let block = ctx.monitor_get_or_create_sync_block_for_object(object, gc);
            ctx.push(StackValue::ObjectRef(object));
            ctx.push(StackValue::Int32(0));
            ctx.push(StackValue::ManagedPtr(pointer.into()));
            let result = crate::intrinsics::dispatch_method_intrinsic(
                crate::intrinsics::MethodIntrinsicId::System_Threading_Monitor_TryEnter_a8f16726fa8c35c7,
                &mut ctx, method.clone(), &Default::default(), NativeDispatchMode::InlineCall,
            );
            if out_of_bounds {
                assert!(matches!(result, StepResult::Error(_)));
                assert!(!block.exit(thread));
            } else {
                assert_eq!(result, StepResult::Jump(1));
                assert_eq!(ctx.shared.statics.get(method.parent.clone(), &Default::default()).storage.with_data(|data| data[0]), 1);
                assert!(block.exit(thread));
            }
            assert!(ctx.call_state.native.is_empty());
        })).unwrap();
    }
}

#[cfg(feature = "multithreading")]
#[test]
fn monitor_static_operand_is_addressless_during_initial_gc_stop() {
    use crate::{stack::ops::VmResolutionOps, sync::SyncBlockOps, threading::ThreadManagerOps};
    use dotnet_intrinsics_threading::MonitorHost;
    use dotnet_value::pointer::ManagedPtr;
    use dotnet_vm_ops::effects::NativeDispatchMode;
    let mut executor = executor();
    let method = timed_monitor_method(&executor);
    let thread = executor.thread_id;
    let shared = executor.shared.clone();
    let guard = shared.thread_manager.request_stop_the_world();
    let block = executor.with_arena(|arena| arena.mutate_root(|gc, engine| {
        // SAFETY: F1.GcHandleRooted — the engine is the live arena root.
        let gc = unsafe { test_gc_handle(gc, engine) };
        let mut ctx = engine.ves_context(gc);
        let generics = GenericLookup::default();
        ctx.shared.statics.init(method.parent.clone(), &ctx.with_generics(&generics), ctx.thread_id.get()).unwrap();
        let storage = ctx.shared.statics.get(method.parent.clone(), &generics);
        let address = storage.storage.with_data(|data| std::ptr::NonNull::new(data.as_ptr().cast_mut()).unwrap());
        let pointer = ManagedPtr::new_static(Some(address), TypeDescription::NULL, method.parent.clone(), generics, false, dotnet_utils::ByteOffset::ZERO);
        assert!(pointer.observe_address().is_some());
        let object = ObjectRef::new(gc, HeapStorage::Str(CLRString::from("monitor")));
        let block = ctx.monitor_get_or_create_sync_block_for_object(object, gc);
        ctx.push(StackValue::ObjectRef(object));
        ctx.push(StackValue::Int32(10_000));
        ctx.push(StackValue::ManagedPtr(pointer.into()));
        assert_eq!(crate::intrinsics::dispatch_method_intrinsic(
            crate::intrinsics::MethodIntrinsicId::System_Threading_Monitor_TryEnter_a8f16726fa8c35c7,
            &mut ctx, method.clone(), &Default::default(), NativeDispatchMode::InlineCall,
        ), StepResult::Yield);
        let activation = ctx.call_state.native.last().unwrap();
        let StackValue::ManagedPtr(pointer) = &activation.operands[2] else { panic!("expected static pointer") };
        assert!(matches!(pointer.origin(), dotnet_value::pointer::PointerOrigin::Static(_)));
        assert!(pointer.observe_address().is_none());
        block
    })).unwrap();
    drop(guard);
    collect(&mut executor);
    assert_eq!(resume(&mut executor), StepResult::Continue);
    assert_eq!(
        shared
            .statics
            .get(method.parent.clone(), &Default::default())
            .storage
            .with_data(|data| data[0]),
        1
    );
    assert!(block.exit(thread));
}

#[cfg(feature = "multithreading")]
#[test]
fn monitor_static_timed_wait_survives_collection_and_writes_result() {
    use crate::{
        stack::{
            native_effects::{MONITOR_FLAG_WRITES, PendingWait},
            ops::VmResolutionOps,
        },
        sync::SyncBlockOps,
    };
    use dotnet_intrinsics_threading::MonitorHost;
    use dotnet_value::pointer::ManagedPtr;
    use dotnet_vm_ops::effects::NativeDispatchMode;
    for expire in [false, true] {
        let mut executor = executor();
        let method = timed_monitor_method(&executor);
        let owner = dotnet_utils::ArenaId::new(9999);
        let thread = executor.thread_id;
        MONITOR_FLAG_WRITES
            .with(|writes| assert!(writes.borrow_mut().replace(Vec::new()).is_none()));
        let (block, deadline) = executor.with_arena(|arena| arena.mutate_root(|gc, engine| {
            // SAFETY: F1.GcHandleRooted — the engine is the live arena root.
            let gc = unsafe { test_gc_handle(gc, engine) };
            let mut ctx = engine.ves_context(gc);
            let generics = GenericLookup::default();
            ctx.shared.statics.init(method.parent.clone(), &ctx.with_generics(&generics), ctx.thread_id.get()).unwrap();
            let pointer = ManagedPtr::new_static(None, TypeDescription::NULL, method.parent.clone(), generics, false, dotnet_utils::ByteOffset::ZERO);
            let object = ObjectRef::new(gc, HeapStorage::Str(CLRString::from("monitor")));
            let block = ctx.monitor_get_or_create_sync_block_for_object(object, gc);
            assert!(block.try_enter(owner));
            ctx.push(StackValue::ObjectRef(object));
            ctx.push(StackValue::Int32(10_000));
            ctx.push(StackValue::ManagedPtr(pointer.into()));
            assert_eq!(crate::intrinsics::dispatch_method_intrinsic(
                crate::intrinsics::MethodIntrinsicId::System_Threading_Monitor_TryEnter_a8f16726fa8c35c7,
                &mut ctx, method.clone(), &Default::default(), NativeDispatchMode::InlineCall,
            ), StepResult::Yield);
            let Some(PendingWait::Monitor(wait)) = &ctx.call_state.native.last().unwrap().wait else { panic!("monitor wait missing") };
            (block, wait.deadline.unwrap())
        })).unwrap();
        MONITOR_FLAG_WRITES.with(|writes| assert!(writes.borrow().as_ref().unwrap().is_empty()));
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
                    if expire {
                        wait.deadline =
                            Some(std::time::Instant::now() - std::time::Duration::from_millis(1));
                    }
                })
            })
            .unwrap();
        if !expire {
            assert!(block.exit(owner));
        }
        assert_eq!(resume(&mut executor), StepResult::Continue);
        let writes = MONITOR_FLAG_WRITES.with(|writes| writes.borrow_mut().take().unwrap());
        assert_eq!(writes, vec![!expire]);
        let value = executor
            .shared
            .statics
            .get(method.parent.clone(), &Default::default())
            .storage
            .with_data(|data| data[0]);
        assert_eq!(value, u8::from(!expire));
        if expire {
            assert!(!block.exit(thread));
            assert!(block.exit(owner));
        } else {
            assert!(block.exit(thread));
        }
    }
}

#[cfg(feature = "multithreading")]
#[test]
fn monitor_unmanaged_timed_wait_survives_collection_and_writes_result() {
    use crate::{
        stack::native_effects::{MONITOR_FLAG_WRITES, PendingWait},
        sync::SyncBlockOps,
    };
    use dotnet_intrinsics_threading::MonitorHost;
    use dotnet_value::pointer::ManagedPtr;
    use dotnet_vm_ops::effects::NativeDispatchMode;
    for expire in [false, true] {
        let mut executor = executor();
        let method = timed_monitor_method(&executor);
        let owner = dotnet_utils::ArenaId::new(9999);
        let thread = executor.thread_id;
        let mut flag = Box::new(0u8);
        let address = std::ptr::NonNull::from(flag.as_mut());
        MONITOR_FLAG_WRITES
            .with(|writes| assert!(writes.borrow_mut().replace(Vec::new()).is_none()));
        let (block, deadline) = executor.with_arena(|arena| arena.mutate_root(|gc, engine| {
            // SAFETY: F1.GcHandleRooted — the engine is the live arena root.
            let gc = unsafe { test_gc_handle(gc, engine) };
            let mut ctx = engine.ves_context(gc);
            let pointer = ManagedPtr::new(Some(address), TypeDescription::NULL, None, false, None);
            let object = ObjectRef::new(gc, HeapStorage::Str(CLRString::from("monitor")));
            let block = ctx.monitor_get_or_create_sync_block_for_object(object, gc);
            assert!(block.try_enter(owner));
            ctx.push(StackValue::ObjectRef(object));
            ctx.push(StackValue::Int32(10_000));
            ctx.push(StackValue::ManagedPtr(pointer.into()));
            assert_eq!(crate::intrinsics::dispatch_method_intrinsic(
                crate::intrinsics::MethodIntrinsicId::System_Threading_Monitor_TryEnter_a8f16726fa8c35c7,
                &mut ctx, method.clone(), &Default::default(), NativeDispatchMode::InlineCall,
            ), StepResult::Yield);
            let Some(PendingWait::Monitor(wait)) = &ctx.call_state.native.last().unwrap().wait else { panic!("monitor wait missing") };
            (block, wait.deadline.unwrap())
        })).unwrap();
        MONITOR_FLAG_WRITES.with(|writes| assert!(writes.borrow().as_ref().unwrap().is_empty()));
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
                    if expire {
                        wait.deadline =
                            Some(std::time::Instant::now() - std::time::Duration::from_millis(1));
                    }
                })
            })
            .unwrap();
        if !expire {
            assert!(block.exit(owner));
        }
        assert_eq!(resume(&mut executor), StepResult::Continue);
        let writes = MONITOR_FLAG_WRITES.with(|writes| writes.borrow_mut().take().unwrap());
        assert_eq!(writes, vec![!expire]);
        assert_eq!(*flag, u8::from(!expire));
        if expire {
            assert!(!block.exit(thread));
            assert!(block.exit(owner));
        } else {
            assert!(block.exit(thread));
        }
    }
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
                        Some(native_test_fallback),
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
                    Some(native_test_fallback),
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
                // SAFETY: F1.GcHandleRooted — the engine is the root of this arena mutation.
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
                    Some(native_test_fallback),
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

#[test]
fn native_monitor_wait_timeout_restores_recursion() {
    use crate::sync::SyncBlockOps;
    use dotnet_intrinsics_threading::MonitorHost;
    use dotnet_vm_ops::effects::NativeDispatchMode;
    let mut executor = executor();
    let method = validation_method(&executor, true, false, true);
    let thread = executor.thread_id;
    executor
        .with_arena(|arena| {
            arena.mutate_root(|gc, engine| {
                // SAFETY: F1.GcHandleRooted — the engine is the live arena root for this mutation.
                let gc = unsafe { test_gc_handle(gc, engine) };
                let source = ObjectRef::new(gc, HeapStorage::Str(CLRString::from("wait timeout")));
                let mut ctx = engine.ves_context(gc);
                let block = ctx.monitor_get_or_create_sync_block_for_object(source, gc);
                assert!(block.try_enter(thread));
                assert!(block.try_enter(thread));
                ctx.push(StackValue::Int32(73));
                ctx.push(StackValue::ObjectRef(source));
                ctx.push(StackValue::Int32(0));
                let result = crate::intrinsics::dispatch_method_intrinsic(
                    crate::intrinsics::MethodIntrinsicId::System_Threading_Monitor_Wait_4a9ebf54fa5e8a7,
                    &mut ctx,
                    method,
                    &Default::default(),
                    NativeDispatchMode::InlineCall,
                );
                assert_eq!(result, StepResult::Jump(1));
                assert_eq!(
                    ctx.evaluation_stack.stack,
                    vec![StackValue::Int32(73), StackValue::Int32(0)]
                );
                assert!(ctx.call_state.native.is_empty());
                assert!(block.exit(thread));
                assert!(block.exit(thread));
                assert!(!block.exit(thread));
            })
        })
        .unwrap();
}

#[cfg(feature = "multithreading")]
#[test]
fn native_monitor_wait_pulse_restores_recursion_after_suspension() {
    use crate::sync::SyncBlockOps;
    use dotnet_intrinsics_threading::MonitorHost;
    use dotnet_vm_ops::effects::NativeDispatchMode;
    let mut executor = executor();
    let method = validation_method(&executor, true, false, true);
    let thread = executor.thread_id;
    let other = dotnet_utils::ArenaId::new(9999);
    let block = executor
        .with_arena(|arena| {
            arena.mutate_root(|gc, engine| {
                // SAFETY: F1.GcHandleRooted — the engine is the live arena root for this mutation.
                let gc = unsafe { test_gc_handle(gc, engine) };
                let source = ObjectRef::new(gc, HeapStorage::Str(CLRString::from("wait pulse")));
                let mut ctx = engine.ves_context(gc);
                let block = ctx.monitor_get_or_create_sync_block_for_object(source, gc);
                assert!(block.try_enter(thread));
                assert!(block.try_enter(thread));
                ctx.push(StackValue::Int32(73));
                ctx.push(StackValue::ObjectRef(source));
                ctx.push(StackValue::Int32(-1));
                let result = crate::intrinsics::dispatch_method_intrinsic(
                    crate::intrinsics::MethodIntrinsicId::System_Threading_Monitor_Wait_4a9ebf54fa5e8a7,
                    &mut ctx,
                    method,
                    &Default::default(),
                    NativeDispatchMode::InlineCall,
                );
                assert_eq!(result, StepResult::Yield);
                block
            })
        })
        .unwrap();
    collect(&mut executor);
    assert!(block.try_enter(other));
    assert!(block.pulse(other, false));
    assert!(block.exit(other));
    assert_eq!(resume(&mut executor), StepResult::Continue);
    executor
        .with_arena(|arena| {
            arena.mutate(|_, engine| {
                let execution = &engine.stack.execution;
                assert_eq!(
                    execution.evaluation_stack.stack,
                    vec![StackValue::Int32(73), StackValue::Int32(1)]
                );
                assert!(execution.call_state.native.is_empty());
                assert_eq!(execution.frame_stack.current_frame().state.ip, 1);
            })
        })
        .unwrap();
    assert!(block.exit(thread));
    assert!(block.exit(thread));
    assert!(!block.exit(thread));
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
                // SAFETY: F1.GcHandleRooted — the engine is the root of this arena mutation.
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
                    Some(native_test_fallback),
                    wait_forever,
                );
                assert!(matches!(result, StepResult::Error(_)));
            })
        })
        .unwrap();
}

fn validation_method(
    executor: &Executor,
    timed: bool,
    byref: bool,
    returns_bool: bool,
) -> MethodDescription {
    use dotnetdll::binary::signature::kinds::CallingConvention;
    let mut res = Resolution::new(Module::new("Validation.dll"));
    res.assembly = Some(Assembly::new("Validation"));
    let ty = res.push_type_definition(TypeDefinition::new(None, "Validation"));
    let mut parameters = vec![Parameter(
        vec![],
        ParameterType::Value(MethodType::Base(Box::new(BaseType::Object))),
    )];
    if timed {
        parameters.push(Parameter(
            vec![],
            ParameterType::Value(MethodType::Base(Box::new(BaseType::Int32))),
        ));
    }
    if byref {
        parameters.push(Parameter(
            vec![],
            ParameterType::Ref(MethodType::Base(Box::new(BaseType::Boolean))),
        ));
    }
    let signature = MethodSignature {
        instance: false,
        explicit_this: false,
        calling_convention: CallingConvention::Default,
        parameters,
        return_type: ReturnType(
            vec![],
            returns_bool.then_some(ParameterType::Value(MethodType::Base(Box::new(
                BaseType::Boolean,
            )))),
        ),
        varargs: None,
    };
    let index = res.push_method(
        ty,
        Method::new(Accessibility::Public, signature, "Validate", None),
    );
    let res = executor.shared.loader.register_owned_assembly(res);
    let ty = TypeDescription::new(res.clone(), ty);
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

#[test]
fn monitor_enter_aliases_validate_before_acquiring() {
    use crate::intrinsics::MethodIntrinsicId::*;
    use crate::sync::SyncBlockOps;
    use dotnet_intrinsics_threading::MonitorHost;
    use dotnet_value::pointer::ManagedPtr;
    use dotnet_vm_ops::effects::NativeDispatchMode;
    let aliases = [
        (
            System_Threading_Monitor_Enter_79ae3ba8ce743b48,
            false,
            false,
            false,
        ),
        (
            System_Threading_Monitor_Enter_cf1115d21714f8ee,
            false,
            true,
            false,
        ),
        (
            System_Threading_Monitor_ReliableEnter_21c46da698e3163c,
            false,
            true,
            false,
        ),
        (
            System_Threading_Monitor_TryEnter_a8f16726fa8c35c7,
            true,
            true,
            false,
        ),
        (
            System_Threading_Monitor_ReliableEnterTimeout_f0d8c1f7af155de7,
            true,
            true,
            false,
        ),
        (
            System_Threading_Monitor_TryEnter_FastPath_WithTimeout_41fdc46787c8a6d2,
            true,
            true,
            false,
        ),
        (
            System_Threading_Monitor_TryEnter_362323e1917a246a,
            true,
            false,
            true,
        ),
        (
            System_Threading_Monitor_TryEnter_FastPath_WithTimeout_cbf93f3b6e7975a8,
            true,
            false,
            true,
        ),
    ];
    for (id, timed, byref, returns_bool) in aliases {
        for (taken, null, timeout, expected, message) in [
            (
                true,
                true,
                -2,
                "System.ArgumentException",
                "The lockTaken argument must be initialized to false.",
            ),
            (
                false,
                true,
                -2,
                "System.ArgumentNullException",
                "Value cannot be null.",
            ),
            (
                false,
                false,
                -2,
                "System.ArgumentOutOfRangeException",
                "The timeout must be greater than or equal to -1.",
            ),
        ] {
            if (taken && !byref) || (!null && !timed) {
                continue;
            }
            let mut executor = validation_executor();
            let method = validation_method(&executor, timed, byref, returns_bool);
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
                            dotnet_vm_data::frame_homes::FrameHomes::allocate(
                                Arc::new(layout),
                                serial,
                            )
                            .unwrap();
                        let (address, _) = frames.home_block(serial).unwrap();
                        // SAFETY: F10.FrameHomeChecked — this is the allocated live one-byte home.
                        unsafe {
                            address.as_ptr().write(u8::from(taken));
                        }
                        let pointer = ManagedPtr::new_frame_home(
                            address,
                            TypeDescription::NULL,
                            serial,
                            dotnet_utils::ByteOffset::new(0),
                            false,
                        );
                        let object =
                            ObjectRef::new(gc, HeapStorage::Str(CLRString::from("monitor")));
                        let mut ctx = engine.ves_context(gc);
                        ctx.frame_stack
                            .current_frame_mut()
                            .state
                            .info_handle
                            .exceptions = vec![Arc::new(boundary_section(
                            dotnet_vm_data::exceptions::HandlerKind::Filter { clause_offset: 1 },
                            0..1,
                            1..2,
                        ))];
                        let block = ctx.monitor_get_or_create_sync_block_for_object(object, gc);
                        assert!(block.try_enter(thread));
                        ctx.push(StackValue::ObjectRef(if null {
                            ObjectRef(None)
                        } else {
                            object
                        }));
                        if timed {
                            ctx.push(StackValue::Int32(timeout));
                        }
                        if byref {
                            ctx.push(StackValue::ManagedPtr(pointer.into()));
                        }
                        let result = crate::intrinsics::dispatch_method_intrinsic(
                            id,
                            &mut ctx,
                            method,
                            &Default::default(),
                            NativeDispatchMode::InlineCall,
                        );
                        assert_validation_exception(&mut ctx, result, expected, message, None);
                        // SAFETY: F10.FrameHomeChecked — the frame still owns this one-byte home.
                        assert_eq!(unsafe { address.as_ptr().read() }, u8::from(taken));
                        assert!(block.exit(thread));
                        assert!(!block.exit(thread));
                    })
                })
                .unwrap();
        }
    }
}

#[test]
fn monitor_wait_validates_before_releasing_or_registering() {
    use crate::sync::SyncBlockOps;
    use dotnet_intrinsics_threading::MonitorHost;
    use dotnet_vm_ops::effects::NativeDispatchMode;
    for (null, timeout, owned, expected, message) in [
        (
            true,
            -2,
            false,
            "System.ArgumentNullException",
            "Value cannot be null.",
        ),
        (
            false,
            -2,
            false,
            "System.ArgumentOutOfRangeException",
            "The timeout must be greater than or equal to -1.",
        ),
        (
            false,
            -2,
            true,
            "System.ArgumentOutOfRangeException",
            "The timeout must be greater than or equal to -1.",
        ),
        (
            false,
            0,
            false,
            "System.Threading.SynchronizationLockException",
            "Object not locked by current thread",
        ),
    ] {
        let mut executor = validation_executor();
        let method = validation_method(&executor, true, false, true);
        let thread = executor.thread_id;
        executor.with_arena(|arena| arena.mutate_root(|gc, engine| {
            // SAFETY: F1.GcHandleRooted — the engine is the live arena root for this mutation.
            let gc = unsafe { test_gc_handle(gc, engine) };
            let object = ObjectRef::new(gc, HeapStorage::Str(CLRString::from("wait")));
            let mut ctx = engine.ves_context(gc);
            ctx.frame_stack.current_frame_mut().state.info_handle.exceptions = vec![Arc::new(boundary_section(dotnet_vm_data::exceptions::HandlerKind::Filter { clause_offset: 1 }, 0..1, 1..2))];
            let block = ctx.monitor_get_or_create_sync_block_for_object(object, gc);
            if owned { assert!(block.try_enter(thread)); }
            ctx.push(StackValue::ObjectRef(if null { ObjectRef(None) } else { object }));
            ctx.push(StackValue::Int32(timeout));
            let result = crate::intrinsics::dispatch_method_intrinsic(crate::intrinsics::MethodIntrinsicId::System_Threading_Monitor_Wait_4a9ebf54fa5e8a7, &mut ctx, method, &Default::default(), NativeDispatchMode::InlineCall);
            assert_validation_exception(&mut ctx, result, expected, message, None);
            assert_eq!(block.is_owned(thread), owned);
            if owned { assert!(block.exit(thread)); }
        })).unwrap();
    }
}

#[test]
fn sleep_invalid_timeout_throws_without_waiting() {
    use dotnet_vm_ops::effects::NativeDispatchMode;
    let mut executor = validation_executor();
    let mut signature = parameterless_i32_signature();
    signature.parameters.push(Parameter(
        vec![],
        ParameterType::Value(MethodType::Base(Box::new(BaseType::Int32))),
    ));
    signature.return_type = ReturnType(vec![], None);
    let mut res = Resolution::new(Module::new("SleepValidation.dll"));
    res.assembly = Some(Assembly::new("SleepValidation"));
    let ty = res.push_type_definition(TypeDefinition::new(None, "SleepValidation"));
    let index = res.push_method(
        ty,
        Method::new(Accessibility::Public, signature, "Sleep", None),
    );
    let res = executor.shared.loader.register_owned_assembly(res);
    let ty = TypeDescription::new(res.clone(), ty);
    let index = ty
        .definition()
        .methods
        .iter()
        .position(|m| std::ptr::eq(m, &res.definition()[index]))
        .unwrap();
    let method = MethodDescription::new(
        ty,
        GenericLookup::default(),
        res,
        MethodMemberIndex::Method(index),
    );
    executor.with_arena(|arena| arena.mutate_root(|gc, engine| {
        // SAFETY: F1.GcHandleRooted — the engine is the live arena root for this mutation.
        let gc = unsafe { test_gc_handle(gc, engine) };
        let mut ctx = engine.ves_context(gc);
        ctx.frame_stack.current_frame_mut().state.info_handle.exceptions = vec![Arc::new(boundary_section(dotnet_vm_data::exceptions::HandlerKind::Filter { clause_offset: 1 }, 0..1, 1..2))];
        ctx.push(StackValue::Int32(-2));
        let result = crate::intrinsics::dispatch_method_intrinsic(crate::intrinsics::MethodIntrinsicId::System_Threading_Thread_Sleep_34c7990db73fe738, &mut ctx, method, &Default::default(), NativeDispatchMode::InlineCall);
        assert_validation_exception(&mut ctx, result, "System.ArgumentOutOfRangeException", "The timeout must be greater than or equal to -1.", None);
    })).unwrap();
}

#[test]
fn non_inline_dispatch_preserves_optional_fallback_contract() {
    for name in ["Yield", "SpinWait", "Sleep"] {
        let mut executor = executor();
        let mut res = Resolution::new(Module::new("HostThread.dll"));
        res.assembly = Some(Assembly::new("HostThread"));
        let mut ty = TypeDefinition::new(None, "Thread");
        ty.namespace = Some("System.Threading".into());
        ty.flags.before_field_init = true;
        let ty_index = res.push_type_definition(ty);
        let mut signature = parameterless_i32_signature();
        signature.return_type = ReturnType(
            vec![],
            (name == "Yield").then_some(ParameterType::Value(MethodType::Base(Box::new(
                BaseType::Boolean,
            )))),
        );
        if name != "Yield" {
            signature.parameters.push(Parameter(
                vec![],
                ParameterType::Value(MethodType::Base(Box::new(BaseType::Int32))),
            ));
        }
        res.push_method(
            ty_index,
            Method::new(Accessibility::Public, signature, name, None),
        );
        let res = executor.shared.loader.register_owned_assembly(res);
        let method = MethodDescription::new(
            TypeDescription::new(res.clone(), ty_index),
            GenericLookup::default(),
            res.clone(),
            MethodMemberIndex::Method(0),
        );
        let expected = StepResult::not_implemented(format!(
            "async intrinsic {method:?} reached through non-inline dispatch"
        ));
        executor
            .with_arena(|arena| {
                arena.mutate_root(|gc, engine| {
                    // SAFETY: F1.GcHandleRooted — the engine is the live arena root for this mutation.
                    let gc = unsafe { test_gc_handle(gc, engine) };
                    // A sentinel below the argument window must survive either host API.
                    engine.ves_context(gc).push(StackValue::Int32(73));
                    if name != "Yield" {
                        engine.ves_context(gc).push(StackValue::Int32(0));
                    }
                    let before = engine.stack.execution.evaluation_stack.stack.clone();
                    let lookup = GenericLookup::default();
                    let result = engine.ves_context(gc).dispatch_method(method, lookup);
                    assert_eq!(result, expected, "{name}");
                    assert_eq!(engine.stack.execution.evaluation_stack.stack, before);
                    assert!(engine.stack.execution.call_state.native.is_empty());
                })
            })
            .unwrap();
    }
}

#[test]
fn absent_fallback_other_preserves_operands_and_inline_selection_is_invariant_error() {
    use dotnet_vm_ops::effects::{
        NativeDispatchMode, NativeIntrinsicHost, NativeOperandView, NativeSelection,
    };
    fn fallback_selector<'gc>(
        _: &crate::stack::VesContext<'_, 'gc>,
        _: NativeOperandView<'_, 'gc>,
        _: &MethodDescription,
        _: &GenericLookup,
    ) -> NativeSelection {
        NativeSelection::Fallback
    }
    for mode in [NativeDispatchMode::Other, NativeDispatchMode::InlineCall] {
        let mut executor = executor();
        let method = condition_wait_method(&executor, false);
        let expected = if mode == NativeDispatchMode::Other {
            StepResult::not_implemented(format!(
                "async intrinsic {method:?} reached through non-inline dispatch"
            ))
        } else {
            StepResult::internal_error(
                "native selector requested a fallback but none is registered",
            )
        };
        executor
            .with_arena(|arena| {
                arena.mutate_root(|gc, engine| {
                    // SAFETY: F1.GcHandleRooted — the engine is the live arena root for this mutation.
                    let gc = unsafe { test_gc_handle(gc, engine) };
                    let mut ctx = engine.ves_context(gc);
                    ctx.push(StackValue::Int32(17));
                    let before = ctx.evaluation_stack.stack.clone();
                    assert_eq!(
                        ctx.start_native(
                            method,
                            mode,
                            &GenericLookup::default(),
                            Some(fallback_selector),
                            None,
                            condition_finalizer_body
                        ),
                        expected
                    );
                    assert_eq!(ctx.evaluation_stack.stack, before);
                    assert!(ctx.call_state.native.is_empty());
                })
            })
            .unwrap();
    }
}

#[test]
fn transient_monitor_byref_is_rejected_before_commitment() {
    use crate::stack::VesContext;
    use dotnet_intrinsics_threading::monitor::reliable_enter_select;
    use dotnet_value::{
        layout::{FieldLayoutManager, GcDesc},
        object::Object,
        pointer::ManagedPtr,
        storage::FieldStorage,
    };
    use dotnet_vm_ops::effects::{NativeDispatchMode, NativeOperandView, NativeSelection};
    let mut executor = validation_executor();
    let method = validation_method(&executor, false, true, false);
    executor.with_arena(|arena| arena.mutate_root(|gc, engine| {
        // SAFETY: F1.GcHandleRooted — the engine is the live arena root.
        let gc = unsafe { test_gc_handle(gc, engine) };
        let mut ctx = engine.ves_context(gc);
        ctx.frame_stack.current_frame_mut().state.info_handle.exceptions = vec![Arc::new(boundary_section(dotnet_vm_data::exceptions::HandlerKind::Filter { clause_offset: 1 }, 0..1, 1..2))];
        let layout = Arc::new(FieldLayoutManager { fields: Default::default(), total_size: 1, alignment: 1, gc_desc: GcDesc::default(), has_ref_fields: false });
        let object = Object::new(TypeDescription::NULL, GenericLookup::default(), FieldStorage::new(layout, vec![0]));
        let address = object.with_data(|data| std::ptr::NonNull::new(data.as_ptr().cast_mut()));
        let pointer = ManagedPtr::new_transient(address, TypeDescription::NULL, object, dotnet_utils::ByteOffset::ZERO);
        ctx.push(StackValue::ObjectRef(ObjectRef(None)));
        ctx.push(StackValue::ManagedPtr(pointer.into()));
        let before = ctx.evaluation_stack.stack.clone();
        assert!(matches!(reliable_enter_select::<VesContext<'_, '_>>(&ctx, NativeOperandView::new(&ctx), &method, &Default::default()), NativeSelection::InvalidProgram));
        assert_eq!(ctx.evaluation_stack.stack, before);
        let result = crate::intrinsics::dispatch_method_intrinsic(
            crate::intrinsics::MethodIntrinsicId::System_Threading_Monitor_ReliableEnter_21c46da698e3163c,
            &mut ctx, method, &Default::default(), NativeDispatchMode::InlineCall,
        );
        assert_validation_exception(&mut ctx, result, "System.InvalidProgramException", "Common Language Runtime detected an invalid program.", None);
    })).unwrap();
}

#[test]
fn threading_selectors_reject_malformed_operand_shapes() {
    use crate::stack::VesContext;
    use dotnet_intrinsics_threading::{
        monitor::{reliable_enter_select, wait_select},
        thread::spin_select,
    };
    use dotnet_vm_ops::effects::{NativeOperandView, NativeSelection};
    let mut executor = executor();
    let method = validation_method(&executor, true, false, true);
    executor
        .with_arena(|arena| {
            arena.mutate_root(|gc, engine| {
                // SAFETY: F1.GcHandleRooted — the engine is the live arena root for this mutation.
                let gc = unsafe { test_gc_handle(gc, engine) };
                let mut ctx = engine.ves_context(gc);
                ctx.push(StackValue::ObjectRef(ObjectRef(None)));
                let before = ctx.evaluation_stack.stack.clone();
                for selector in [
                    wait_select::<VesContext<'_, '_>>,
                    spin_select::<VesContext<'_, '_>>,
                    reliable_enter_select::<VesContext<'_, '_>>,
                ] {
                    assert!(matches!(
                        selector(
                            &ctx,
                            NativeOperandView::new(&ctx),
                            &method,
                            &Default::default()
                        ),
                        NativeSelection::Reject(_)
                    ));
                    assert_eq!(ctx.evaluation_stack.stack, before);
                }
            })
        })
        .unwrap();
}

#[test]
fn fallback_free_monitor_other_preserves_operands() {
    use crate::intrinsics::MethodIntrinsicId::*;
    use dotnet_vm_ops::effects::NativeDispatchMode;
    for (id, timed) in [
        (System_Threading_Monitor_Enter_79ae3ba8ce743b48, false),
        (System_Threading_Monitor_TryEnter_362323e1917a246a, true),
        (System_Threading_Monitor_Wait_4a9ebf54fa5e8a7, true),
        (
            System_Threading_Monitor_TryEnter_FastPath_WithTimeout_cbf93f3b6e7975a8,
            true,
        ),
    ] {
        let mut executor = executor();
        let method = validation_method(&executor, timed, false, timed);
        let expected = StepResult::not_implemented(format!(
            "async intrinsic {method:?} reached through non-inline dispatch"
        ));
        executor
            .with_arena(|arena| {
                arena.mutate_root(|gc, engine| {
                    // SAFETY: F1.GcHandleRooted — the engine is the live arena root for this mutation.
                    let gc = unsafe { test_gc_handle(gc, engine) };
                    let mut ctx = engine.ves_context(gc);
                    ctx.push(StackValue::Int32(73));
                    ctx.push(StackValue::ObjectRef(ObjectRef(None)));
                    if timed {
                        ctx.push(StackValue::Int32(-2));
                    }
                    let before = ctx.evaluation_stack.stack.clone();
                    assert_eq!(
                        crate::intrinsics::dispatch_method_intrinsic(
                            id,
                            &mut ctx,
                            method,
                            &Default::default(),
                            NativeDispatchMode::Other
                        ),
                        expected
                    );
                    assert_eq!(ctx.evaluation_stack.stack, before);
                    assert!(ctx.call_state.native.is_empty());
                })
            })
            .unwrap();
    }
}

#[test]
fn inline_unmanaged_monitor_succeeds_and_non_inline_preserves_operands() {
    use crate::sync::SyncBlockOps;
    use dotnet_intrinsics_threading::MonitorHost;
    use dotnet_value::pointer::ManagedPtr;
    use dotnetdll::binary::signature::kinds::CallingConvention;
    for unified in [false, true] {
        let mut executor = executor();
        let thread = executor.thread_id;
        let mut res = Resolution::new(Module::new("HostMonitor.dll"));
        res.assembly = Some(Assembly::new("HostMonitor"));
        let mut ty = TypeDefinition::new(None, "Monitor");
        ty.namespace = Some("System.Threading".into());
        ty.flags.before_field_init = true;
        let ty_index = res.push_type_definition(ty);
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
        res.push_method(
            ty_index,
            Method::new(Accessibility::Public, signature, "Enter", None),
        );
        let res = executor.shared.loader.register_owned_assembly(res);
        let method = MethodDescription::new(
            TypeDescription::new(res.clone(), ty_index),
            GenericLookup::default(),
            res.clone(),
            MethodMemberIndex::Method(0),
        );
        let mut flag = Box::new(0u8);
        let address = std::ptr::NonNull::from(flag.as_mut());
        executor
            .with_arena(|arena| {
                arena.mutate_root(|gc, engine| {
                    // SAFETY: F1.GcHandleRooted — the engine is the live arena root for this mutation.
                    let gc = unsafe { test_gc_handle(gc, engine) };
                    let object =
                        ObjectRef::new(gc, HeapStorage::Str(CLRString::from("host monitor")));
                    let mut ctx = engine.ves_context(gc);
                    let block = ctx.monitor_get_or_create_sync_block_for_object(object, gc);
                    ctx.push(StackValue::Int32(73));
                    ctx.push(StackValue::ObjectRef(object));
                    let ptr =
                        ManagedPtr::new(Some(address), TypeDescription::NULL, None, false, None);
                    ctx.push(StackValue::ManagedPtr(ptr.into()));
                    let lookup = GenericLookup::default();
                    let expected = StepResult::not_implemented(format!(
                        "async intrinsic {method:?} reached through non-inline dispatch"
                    ));
                    let before = ctx.evaluation_stack.stack.clone();
                    let result = if unified {
                        ctx.dispatch_method_inline(method, lookup)
                    } else {
                        ctx.dispatch_method(method, lookup)
                    };
                    if unified {
                        assert_eq!(result, StepResult::Jump(1));
                        assert_eq!(ctx.evaluation_stack.stack, vec![StackValue::Int32(73)]);
                        assert!(block.exit(thread));
                    } else {
                        assert_eq!(result, expected);
                        assert_eq!(ctx.evaluation_stack.stack, before);
                    }
                    assert!(ctx.call_state.native.is_empty());
                })
            })
            .unwrap();
        assert_eq!(*flag, u8::from(unified));
    }
}

#[cfg(feature = "multithreading")]
#[test]
fn pool_worker_parks_participates_in_gc_reuses_executor_and_shuts_down() {
    use crate::stack::native_effects::{WAIT_TEST_HOOK, WaitTestPoint};
    use std::{sync::mpsc, time::Duration};

    let mut executor = executor();
    let mut corlib = Resolution::new(Module::new("mscorlib.dll"));
    corlib.assembly = Some(Assembly::new("mscorlib"));
    for name in ["Object", "Enum", "ValueType"] {
        let mut ty = TypeDefinition::new(None, name);
        ty.namespace = Some("System".into());
        corlib.push_type_definition(ty);
    }
    executor.shared.loader.register_owned_assembly(corlib);
    let (mut res, _, type_idx) = make_test_assembly!("PoolWorker.dll", "PoolWorker", "Worker");
    res[type_idx].extends = None;
    let mut setter_sig = crate::test_utils::parameterless_i32_signature();
    setter_sig.instance = true;
    setter_sig.parameters.push(Parameter(
        vec![],
        ParameterType::Value(MethodType::Base(Box::new(BaseType::Boolean))),
    ));
    setter_sig.return_type = ReturnType(vec![], None);
    let setter_idx = res.push_method(
        type_idx,
        Method::new(
            Accessibility::Public,
            setter_sig,
            "SetPoolThread",
            Some(make_test_method!(max_stack: 1, instructions: vec![Instruction::Return])),
        ),
    );
    let dispatch_idx = res.push_method(type_idx, Method::new(Accessibility::Public, crate::test_utils::parameterless_i32_signature(), "Dispatch", Some(make_test_method!(max_stack: 1, instructions: vec![Instruction::LoadConstantInt32(1), Instruction::Return]))));
    let res = executor.shared.loader.register_owned_assembly(res);
    let ty = TypeDescription::new(res.clone(), type_idx);
    let describe = |index| {
        let i = ty
            .definition()
            .methods
            .iter()
            .position(|m| std::ptr::eq(m, &res.definition()[index]))
            .unwrap();
        MethodDescription::new(
            ty.clone(),
            GenericLookup::default(),
            res.clone(),
            MethodMemberIndex::Method(i),
        )
    };
    let setter = describe(setter_idx);
    let dispatch = describe(dispatch_idx);
    let shared = executor.shared.clone();
    assert!(shared.pool.request());
    let worker_shared = shared.clone();
    let (parked_tx, parked_rx) = mpsc::channel();
    let request_when_registered = Arc::new(dotnet_utils::sync::AtomicBool::new(false));
    let request_flag = request_when_registered.clone();
    let hook_shared = shared.clone();
    let completion = shared.workers.spawn_pool_owner(move |running| {
        let mut announced = false;
        let mut requested = false;
        WAIT_TEST_HOOK.with(|hook| {
            *hook.borrow_mut() = Some(Box::new(move |point| {
                if matches!(point, WaitTestPoint::Registered) {
                    if request_flag.swap(false, dotnet_utils::sync::Ordering::SeqCst) {
                        assert!(!hook_shared.pool.request());
                        requested = true;
                        let _ = parked_tx.send(true);
                    } else if requested {
                        requested = false;
                        let _ = parked_tx.send(false);
                    } else if !announced {
                        announced = true;
                        let _ = parked_tx.send(false);
                    }
                }
            }))
        });
        Executor::run_pool_worker(worker_shared, dispatch, setter, running);
    });
    assert!(
        !parked_rx
            .recv_timeout(Duration::from_secs(10))
            .expect("worker should park after dispatch")
    );
    collect(&mut executor);
    request_when_registered.store(true, dotnet_utils::sync::Ordering::SeqCst);
    assert!(
        parked_rx
            .recv_timeout(Duration::from_secs(10))
            .expect("worker should register again")
    );
    assert!(
        !parked_rx
            .recv_timeout(Duration::from_secs(10))
            .expect("worker should dispatch and park again")
    );
    assert_eq!(shared.pool.pending_requests_for_test(), 0);
    assert_eq!(shared.pool.worker_count(), 1);
    shared.abort_signal().request_abort();
    executor.wait_for_foreground_workers();
    drop(executor);
    shared.shutdown_workers();
    assert!(completion.is_finished());
}

#[test]
fn pool_tuning_out_parameters_write_checked_frame_homes() {
    use dotnet_value::pointer::ManagedPtr;
    let mut executor = executor();
    let method = condition_wait_method(&executor, false);
    executor
        .with_arena(|arena| {
            arena.mutate_root(|gc, engine| {
                // SAFETY: F1.GcHandleRooted — the engine remains the live arena root for this mutation.
                let gc = unsafe { test_gc_handle(gc, engine) };
                let frames = &mut engine.stack.execution.frame_stack;
                let serial = frames.issue_home_serial();
                let layout = dotnet_value::layout::FieldLayoutManager {
                    fields: Default::default(),
                    total_size: 8,
                    alignment: 4,
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
                let pointer = |offset| {
                    ManagedPtr::new_frame_home(
                        address,
                        TypeDescription::NULL,
                        serial,
                        dotnet_utils::ByteOffset::new(offset),
                        false,
                    )
                };
                let mut ctx = engine.ves_context(gc);
                ctx.push(StackValue::ManagedPtr(pointer(0).into()));
                ctx.push(StackValue::ManagedPtr(pointer(4).into()));
                assert_eq!(
                    dotnet_intrinsics_threading::pool::get_max(
                        &mut ctx,
                        method,
                        &GenericLookup::default()
                    ),
                    StepResult::Continue
                );
                // SAFETY: F10.FrameHomeChecked — the live frame owns the eight-byte block.
                let worker = unsafe { address.as_ptr().cast::<i32>().read_unaligned() };
                let io_address = address.as_ptr().wrapping_add(4).cast::<i32>();
                // SAFETY: F10.FrameHomeChecked — the second four-byte read fits the same block.
                let io = unsafe { io_address.read_unaligned() };
                assert_eq!((worker, io), (32767, 1000));
            })
        })
        .unwrap();
}

#[test]
fn pool_placeholder_is_retained_and_shared_across_collection() {
    let mut executor = executor();
    let original = executor
        .with_arena(|arena| {
            arena.mutate_root(|gc, engine| {
                // SAFETY: F1.GcHandleRooted — the engine remains the live arena root during mutation.
                let gc = unsafe { test_gc_handle(gc, engine) };
                let candidate =
                    ObjectRef::new(gc, HeapStorage::Str(CLRString::from("pool placeholder")));
                engine.ves_context(gc).register_new_object(&candidate);
                engine
                    .stack
                    .shared
                    .statics
                    .pool_placeholder(candidate)
                    .as_ptr()
                    .unwrap()
            })
        })
        .unwrap();
    collect(&mut executor);
    executor
        .with_arena(|arena| {
            arena.mutate_root(|gc, engine| {
                // SAFETY: F1.GcHandleRooted — the engine remains the live arena root during mutation.
                let gc = unsafe { test_gc_handle(gc, engine) };
                let candidate = ObjectRef::new(gc, HeapStorage::Str(CLRString::from("other")));
                engine.ves_context(gc).register_new_object(&candidate);
                let placeholder = engine.stack.shared.statics.pool_placeholder(candidate);
                assert_eq!(placeholder.as_ptr(), Some(original));
                assert_ne!(placeholder.as_ptr(), candidate.as_ptr());
            })
        })
        .unwrap();
}

fn event_wait_body(
    mut fx: dotnet_vm_ops::effects::VmEffects,
) -> dotnet_vm_ops::effects::NativeFuture {
    Box::pin(async move {
        assert!(fx.event_wait(1, None).await?);
        Ok(())
    })
}

#[test]
fn event_wait_signaled_before_registration() {
    let mut executor = executor();
    let id = executor.shared.events.create(false, false).unwrap();
    let event = executor.shared.events.get(id).unwrap();
    event.set();
    assert_eq!(
        start_sleep(&mut executor, event_wait_body),
        StepResult::Jump(1)
    );
    assert_eq!(event.waiter_count(), 0);
}

#[cfg(feature = "multithreading")]
#[test]
fn event_wait_registration_rechecks_set() {
    use crate::stack::native_effects::{WAIT_TEST_HOOK, WaitTestPoint};
    for after_registration in [false, true] {
        let mut executor = executor();
        let id = executor.shared.events.create(false, false).unwrap();
        let event = executor.shared.events.get(id).unwrap();
        let signal = event.clone();
        WAIT_TEST_HOOK.with(|hook| {
            let mut fired = false;
            *hook.borrow_mut() = Some(Box::new(move |point| {
                if !fired
                    && matches!(
                        (after_registration, point),
                        (false, WaitTestPoint::Probed) | (true, WaitTestPoint::Registered)
                    )
                {
                    fired = true;
                    signal.set();
                }
            }));
        });
        assert_eq!(
            start_sleep(&mut executor, event_wait_body),
            StepResult::Jump(1)
        );
        WAIT_TEST_HOOK.with(|hook| drop(hook.borrow_mut().take()));
        assert_eq!(event.waiter_count(), 0);
    }
}

#[cfg(feature = "multithreading")]
fn rooted_event_wait_body(
    mut fx: dotnet_vm_ops::effects::VmEffects,
) -> dotnet_vm_ops::effects::NativeFuture {
    Box::pin(async move {
        let root = fx.vm().await?.pop_string()?;
        assert!(fx.event_wait(1, None).await?);
        let mut chars = Vec::new();
        fx.vm().await?.string(&root).copy_chunk(0, 7, &mut chars)?;
        assert_eq!(chars, "witness".encode_utf16().collect::<Vec<_>>());
        Ok(())
    })
}

#[cfg(feature = "multithreading")]
fn start_rooted_event_wait(executor: &mut Executor) -> StepResult {
    use dotnet_vm_ops::effects::{NativeDispatchMode, NativeIntrinsicHost};
    let method = condition_wait_method(executor, true);
    executor
        .with_arena(|arena| {
            arena.mutate_root(|gc, engine| {
                // SAFETY: F1.GcHandleRooted — the engine is the live root of this mutation.
                let gc = unsafe { test_gc_handle(gc, engine) };
                let source = ObjectRef::new(gc, HeapStorage::Str(CLRString::from("witness")));
                let mut ctx = engine.ves_context(gc);
                ctx.push(StackValue::Int32(0));
                ctx.push(StackValue::ObjectRef(source));
                ctx.start_native(
                    method,
                    NativeDispatchMode::InlineCall,
                    &Default::default(),
                    None,
                    Some(native_test_fallback),
                    rooted_event_wait_body,
                )
            })
        })
        .unwrap()
}

#[cfg(feature = "multithreading")]
#[test]
fn event_wait_parked_gc_and_signal() {
    use crate::test_utils::worker::Worker;
    use std::{sync::mpsc, thread, time::Duration};
    let mut executor = executor();
    let id = executor.shared.events.create(false, false).unwrap();
    let event = executor.shared.events.get(id).unwrap();
    assert_eq!(start_rooted_event_wait(&mut executor), StepResult::Yield);
    let shared = executor.shared.clone();
    let signal = event.clone();
    let (ready_tx, ready_rx) = mpsc::channel();
    let (start_tx, start_rx) = mpsc::channel();
    let worker = Worker::new(
        thread::spawn(move || {
            let mut collector = Executor::new(shared);
            ready_tx.send(()).unwrap();
            start_rx.recv_timeout(Duration::from_secs(5)).unwrap();
            let collected = collector.perform_full_gc().is_ok();
            signal.set();
            collected
        }),
        (),
    );
    ready_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    crate::executor::WAIT_TEST_HOOK.with(|hook| {
        *hook.borrow_mut() = Some(Box::new(move || {
            start_tx.send(()).unwrap();
        }))
    });
    assert!(matches!(executor.run(), ExecutorResult::Exited(0)));
    assert!(worker.finish().unwrap());
    assert_eq!(event.waiter_count(), 0);
}

#[cfg(feature = "multithreading")]
#[test]
fn event_wait_spurious_wakes_and_abort_remove_registration() {
    let mut executor = executor();
    let id = executor.shared.events.create(false, false).unwrap();
    let event = executor.shared.events.get(id).unwrap();
    assert_eq!(
        start_sleep(&mut executor, event_wait_body),
        StepResult::Yield
    );
    event.notify_spurious();
    event.notify_spurious();
    assert_eq!(resume(&mut executor), StepResult::Yield);
    let abort = executor.shared.abort_signal();
    crate::executor::WAIT_TEST_HOOK
        .with(|hook| *hook.borrow_mut() = Some(Box::new(move || abort.request_abort())));
    assert!(matches!(executor.run(), ExecutorResult::Error(_)));
    assert_eq!(event.waiter_count(), 0);
    event.set();
    assert_eq!(event.begin(None).poll(), Some(true));
}

#[cfg(not(feature = "multithreading"))]
fn timed_event_wait_body(
    mut fx: dotnet_vm_ops::effects::VmEffects,
) -> dotnet_vm_ops::effects::NativeFuture {
    Box::pin(async move {
        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(20);
        assert!(!fx.event_wait(1, Some(deadline)).await?);
        assert!(std::time::Instant::now() >= deadline);
        Ok(())
    })
}

#[cfg(not(feature = "multithreading"))]
#[test]
fn event_wait_single_thread_timeout_and_deadlock() {
    let mut timed_executor = executor();
    let id = timed_executor.shared.events.create(false, false).unwrap();
    let event = timed_executor.shared.events.get(id).unwrap();
    assert_eq!(
        start_sleep(&mut timed_executor, timed_event_wait_body),
        StepResult::Yield
    );
    assert!(matches!(timed_executor.run(), ExecutorResult::Exited(0)));
    assert_eq!(event.waiter_count(), 0);
    let mut executor = executor();
    let id = executor.shared.events.create(false, false).unwrap();
    let event = executor.shared.events.get(id).unwrap();
    assert!(matches!(
        start_sleep(&mut executor, event_wait_body),
        StepResult::Error(_)
    ));
    assert_eq!(event.waiter_count(), 0);
}

#[cfg(feature = "multithreading")]
#[test]
fn event_wait_parked_spurious_wakes_recheck_without_grant() {
    use crate::test_utils::worker::Worker;
    use std::{sync::mpsc, thread, time::Duration};
    let mut executor = executor();
    let id = executor.shared.events.create(false, false).unwrap();
    let event = executor.shared.events.get(id).unwrap();
    assert_eq!(
        start_sleep(&mut executor, event_wait_body),
        StepResult::Yield
    );
    let signal = event.clone();
    let (parked_tx, parked_rx) = mpsc::channel();
    let worker = Worker::new(
        thread::spawn(move || {
            let first = parked_rx.recv_timeout(Duration::from_secs(5)).is_ok();
            if first {
                signal.notify_spurious();
                signal.notify_spurious();
            }
            let second = parked_rx.recv_timeout(Duration::from_secs(5)).is_ok();
            signal.set();
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
    assert!(worker.finish().unwrap());
    assert_eq!(event.waiter_count(), 0);
}

#[cfg(feature = "multithreading")]
fn timed_event_expected_signal(
    mut fx: dotnet_vm_ops::effects::VmEffects,
) -> dotnet_vm_ops::effects::NativeFuture {
    Box::pin(async move {
        assert!(
            fx.event_wait(
                1,
                Some(std::time::Instant::now() + std::time::Duration::from_secs(60))
            )
            .await?
        );
        Ok(())
    })
}
#[cfg(feature = "multithreading")]
fn timed_event_expected_timeout(
    mut fx: dotnet_vm_ops::effects::VmEffects,
) -> dotnet_vm_ops::effects::NativeFuture {
    Box::pin(async move {
        assert!(
            !fx.event_wait(
                1,
                Some(std::time::Instant::now() + std::time::Duration::from_secs(60))
            )
            .await?
        );
        Ok(())
    })
}
#[cfg(feature = "multithreading")]
#[test]
fn event_wait_timeout_and_set_both_orders() {
    use crate::stack::native_effects::{WAIT_TEST_HOOK, WaitTestPoint};
    for set_first in [true, false] {
        let mut executor = executor();
        let id = executor.shared.events.create(false, false).unwrap();
        let event = executor.shared.events.get(id).unwrap();
        let signal = event.clone();
        WAIT_TEST_HOOK.with(|hook| {
            let mut fired = false;
            *hook.borrow_mut() = Some(Box::new(move |point| {
                if !fired && matches!(point, WaitTestPoint::Probed) {
                    fired = true;
                    if set_first {
                        signal.set();
                    }
                    signal.expire_waiters_for_test();
                    if !set_first {
                        signal.set();
                    }
                }
            }));
        });
        let body = if set_first {
            timed_event_expected_signal
        } else {
            timed_event_expected_timeout
        };
        assert_eq!(start_sleep(&mut executor, body), StepResult::Jump(1));
        WAIT_TEST_HOOK.with(|hook| drop(hook.borrow_mut().take()));
        assert_eq!(
            event.begin(Some(std::time::Instant::now())).poll(),
            Some(!set_first)
        );
    }
}

fn unknown_event_wait_body(
    mut fx: dotnet_vm_ops::effects::VmEffects,
) -> dotnet_vm_ops::effects::NativeFuture {
    Box::pin(async move {
        let _ = fx.event_wait(999, Some(std::time::Instant::now())).await?;
        Ok(())
    })
}
#[test]
fn event_wait_rejects_unknown_handle() {
    let mut executor = executor();
    assert!(matches!(
        start_sleep(&mut executor, unknown_event_wait_body),
        StepResult::Error(_)
    ));
}

#[cfg(feature = "multithreading")]
#[test]
fn event_wait_deadline_survives_collection() {
    use crate::stack::native_effects::PendingWait;
    let mut executor = executor();
    let id = executor.shared.events.create(false, false).unwrap();
    let event = executor.shared.events.get(id).unwrap();
    assert_eq!(
        start_sleep(&mut executor, timed_event_expected_signal),
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
                    PendingWait::Event(wait) => wait.wait.deadline,
                    _ => panic!("expected event wait"),
                }
            })
        })
        .unwrap();
    assert!(deadline.is_some());
    collect(&mut executor);
    assert_eq!(resume(&mut executor), StepResult::Yield);
    executor
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
                    PendingWait::Event(wait) => assert_eq!(wait.wait.deadline, deadline),
                    _ => panic!("expected event wait"),
                }
            })
        })
        .unwrap();
    event.set();
    assert_eq!(resume(&mut executor), StepResult::Continue);
}

#[cfg(feature = "multithreading")]
#[test]
fn join_timeout_survives_collection() {
    use dotnet_vm_ops::effects::NativeDispatchMode;
    use std::time::Duration;
    let mut executor = executor();
    let (mut res, _, owner) = make_test_assembly!("JoinTest.dll", "JoinTest", "Thread");
    let mut signature = parameterless_i32_signature();
    signature.instance = true;
    signature.parameters.push(Parameter(
        vec![],
        ParameterType::Value(MethodType::Base(Box::new(BaseType::Int32))),
    ));
    res.push_method(
        owner,
        Method::new(Accessibility::Public, signature, "Join", None),
    );
    let res = executor.shared.loader.register_owned_assembly(res);
    let method = MethodDescription::new(
        TypeDescription::new(res.clone(), owner),
        GenericLookup::default(),
        res,
        MethodMemberIndex::Method(0),
    );
    let (release, held) = std::sync::mpsc::channel();
    let completion = executor.shared.workers.spawn_owner(move |running| {
        let _ = held.recv_timeout(Duration::from_secs(5));
        running.complete(ExecutorResult::Exited(0)).retire();
    });
    executor
        .with_arena(|arena| {
            arena.mutate_root(|gc, engine| {
                // SAFETY: F1.GcHandleRooted — the engine is the live root for this mutation.
                let gc = unsafe { test_gc_handle(gc, engine) };
                let thread = ObjectRef::new(gc, HeapStorage::Str(CLRString::new(vec![])));
                engine.stack.shared.workers.register_managed_thread(
                    thread.as_ptr().unwrap(),
                    crate::threading::workers::ManagedThreadMetadata {
                        id: 1,
                        delegate: None,
                        parameterized: false,
                        started: true,
                        background: false,
                        completion_index: Some(completion.index()),
                        retained: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true)),
                    },
                );
                let mut ctx = engine.ves_context(gc);
                ctx.push(StackValue::ObjectRef(thread));
                ctx.push(StackValue::Int32(50));
                assert_eq!(crate::intrinsics::dispatch_method_intrinsic(
            crate::intrinsics::MethodIntrinsicId::System_Threading_Thread_Join_29405bd0fb2c031a,
            &mut ctx, method, &Default::default(), NativeDispatchMode::InlineCall,
        ), StepResult::Yield);
            })
        })
        .unwrap();
    collect(&mut executor);
    std::thread::sleep(Duration::from_millis(80));
    assert_eq!(resume(&mut executor), StepResult::Continue);
    executor
        .with_arena(|arena| {
            arena.mutate_root(|gc, engine| {
                // SAFETY: F1.GcHandleRooted — the engine is the live root for this mutation.
                let gc = unsafe { test_gc_handle(gc, engine) };
                assert_eq!(engine.ves_context(gc).peek_stack_at(0).as_i32(), 0);
            })
        })
        .unwrap();
    release.send(()).unwrap();
    drop(executor);
}

#[test]
fn static_storage_identity_ignores_generic_argument_resolution() {
    use crate::stack::ops::VmResolutionOps;
    use dotnet_types::{generics::ConcreteType, resolution::ResolutionS};
    let mut executor = executor();
    let method = timed_monitor_method(&executor);
    // Stand-ins for the corlib-stamped resolution (arguments rebuilt from a `RuntimeType`) and
    // the referencing assembly's resolution (arguments from a field operand).
    let mut handle_res = Resolution::new(Module::new("HandleSide.dll"));
    handle_res.assembly = Some(Assembly::new("HandleSide"));
    let corlib = executor.shared.loader.register_owned_assembly(handle_res);
    let other = method.parent.resolution.clone();
    assert!(corlib != other);
    let int_in = |res: &ResolutionS| ConcreteType::new(res.clone(), BaseType::Int32);
    let vector_in =
        |res: &ResolutionS| ConcreteType::new(res.clone(), BaseType::Vector(vec![], int_in(res)));
    executor
        .with_arena(|arena| {
            arena.mutate_root(|gc, engine| {
                // SAFETY: F1.GcHandleRooted — the engine is the live arena root.
                let gc = unsafe { test_gc_handle(gc, engine) };
                let ctx = engine.ves_context(gc);
                let lookup = |f: &dyn Fn(&ResolutionS) -> ConcreteType, res: &ResolutionS| {
                    GenericLookup::new(vec![f(res)])
                };
                for shape in [&int_in as &dyn Fn(&ResolutionS) -> ConcreteType, &vector_in] {
                    let via_handle = lookup(shape, &corlib);
                    let via_field = lookup(shape, &other);
                    assert!(
                        via_handle != via_field,
                        "raw lookups must differ by resolution"
                    );
                    let statics = ctx.shared.statics.clone();
                    statics
                        .init(
                            method.parent.clone(),
                            &ctx.with_generics(&via_handle),
                            ctx.thread_id.get(),
                        )
                        .unwrap();
                    // The second spelling reaches the same entry, both before and after `init`.
                    let first = statics.get(method.parent.clone(), &via_handle);
                    let second = statics.get(method.parent.clone(), &via_field);
                    assert!(Arc::ptr_eq(&first, &second));
                    statics
                        .init(
                            method.parent.clone(),
                            &ctx.with_generics(&via_field),
                            ctx.thread_id.get(),
                        )
                        .unwrap();
                    assert!(Arc::ptr_eq(
                        &first,
                        &statics.get(method.parent.clone(), &via_field)
                    ));
                    // Thread-static storage canonicalizes identically.
                    let t1 = statics.get_for_thread(method.parent.clone(), &via_handle, 7);
                    let t2 = statics.get_for_thread(method.parent.clone(), &via_field, 7);
                    assert!(Arc::ptr_eq(&t1, &t2));
                }
                // Different arguments still get different storage.
                let ints = GenericLookup::new(vec![int_in(&other)]);
                let longs =
                    GenericLookup::new(vec![ConcreteType::new(other.clone(), BaseType::Int64)]);
                let statics = ctx.shared.statics.clone();
                statics
                    .init(
                        method.parent.clone(),
                        &ctx.with_generics(&longs),
                        ctx.thread_id.get(),
                    )
                    .unwrap();
                assert!(!Arc::ptr_eq(
                    &statics.get(method.parent.clone(), &ints),
                    &statics.get(method.parent.clone(), &longs)
                ));
            })
        })
        .unwrap();
}

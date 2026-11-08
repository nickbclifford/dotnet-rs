use crate::{MonitorLockResult, ThreadingIntrinsicHost};
use dotnet_macros::dotnet_intrinsic;
use dotnet_types::{generics::GenericLookup, members::MethodDescription};
use dotnet_utils::ArenaId;
use dotnet_value::{StackValue, object::ObjectRef, pointer::PointerOrigin};
use dotnet_vm_data::StepResult;
use dotnet_vm_ops::NULL_REF_MSG;
use dotnet_vm_ops::effects::{
    NativeInvocation, NativeOperandView, NativeResult, NativeSelection, VmEffects,
};
use std::time::{Duration, Instant};

/// System.Threading.Monitor::Exit(object) - Releases the lock on an object.
#[dotnet_intrinsic("static void System.Threading.Monitor::Exit(object)")]
pub fn intrinsic_monitor_exit<'gc, T: ThreadingIntrinsicHost<'gc>>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let _gc = ctx.gc_with_token(&ctx.no_active_borrows_token());
    let obj_ref = ctx.pop_obj();

    if obj_ref.0.is_some() {
        // Get the current thread ID from the call stack
        let thread_id = ctx.thread_id();
        assert_ne!(
            thread_id,
            ArenaId::INVALID,
            "Monitor.Exit called from unregistered thread"
        );

        // Get the sync block if it exists
        if let Some(sync_block) = ctx.monitor_get_sync_block_for_object(obj_ref) {
            if !ctx.monitor_exit(&sync_block, thread_id) {
                return ctx.throw_by_name_with_message(
                    "System.Threading.SynchronizationLockException",
                    "Object not locked by current thread",
                );
            }
        } else {
            return ctx.throw_by_name_with_message(
                "System.Threading.SynchronizationLockException",
                "Object not locked",
            );
        }
    } else {
        return ctx
            .throw_by_name_with_message("System.ArgumentNullException", "Value cannot be null.");
    }

    StepResult::Continue
}

/// System.Threading.Monitor::Enter(object)
pub fn intrinsic_monitor_enter_obj<'gc, T: ThreadingIntrinsicHost<'gc>>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let gc = ctx.gc_with_token(&ctx.no_active_borrows_token());
    let obj_ref = ctx.peek_stack_at(0).as_object_ref();

    if obj_ref.0.is_some() {
        let thread_id = ctx.thread_id();
        assert_ne!(
            thread_id,
            ArenaId::INVALID,
            "Monitor.Enter called from unregistered thread"
        );

        let sync_block = ctx.monitor_get_or_create_sync_block_for_object(obj_ref, gc);

        match ctx.monitor_enter_safe(&sync_block, thread_id) {
            MonitorLockResult::Success => {
                let _ = ctx.pop();
                StepResult::Continue
            }
            MonitorLockResult::Yield => StepResult::Yield,
            MonitorLockResult::Timeout => unreachable!("Infinite timeout cannot time out"),
        }
    } else {
        let _ = ctx.pop();
        ctx.throw_by_name_with_message("System.NullReferenceException", NULL_REF_MSG)
    }
}

/// System.Threading.Monitor::ReliableEnter(object, ref bool)
pub fn intrinsic_monitor_reliable_enter<'gc, T: ThreadingIntrinsicHost<'gc>>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let gc = ctx.gc_with_token(&ctx.no_active_borrows_token());
    let success_ptr = ctx.peek_stack_at(0).as_managed_ptr();
    let obj_ref = ctx.peek_stack_at(1).as_object_ref();

    if obj_ref.0.is_some() {
        let thread_id = ctx.thread_id();
        assert_ne!(
            thread_id,
            ArenaId::INVALID,
            "Monitor.ReliableEnter called from unregistered thread"
        );

        let sync_block = ctx.monitor_get_or_create_sync_block_for_object(obj_ref, gc);

        let lock_res = ctx.monitor_enter_safe(&sync_block, thread_id);

        match lock_res {
            MonitorLockResult::Success => {}
            MonitorLockResult::Yield => return StepResult::Yield,
            MonitorLockResult::Timeout => unreachable!(),
        }

        // SAFETY: F10.RawMemoryAccessValid — the out parameter denotes a writable bool;
        // managed origins check the complete byte range and unmanaged callers supply a valid address.
        if let Err(error) = unsafe {
            ctx.write_bytes(
                success_ptr.origin().clone(),
                success_ptr.byte_offset(),
                &[1u8],
            )
        } {
            return StepResult::Error(error.into());
        }

        // Pop success_ptr, then obj_ref after all work that may yield or trigger GC.
        ctx.drop_top(2);
    } else {
        ctx.drop_top(2);
        return ctx.throw_by_name_with_message("System.NullReferenceException", NULL_REF_MSG);
    }

    StepResult::Continue
}

/// System.Threading.Monitor::TryEnter_FastPath(object)
#[dotnet_intrinsic("static bool System.Threading.Monitor::TryEnter_FastPath(object)")]
pub fn intrinsic_monitor_try_enter_fast_path<'gc, T: ThreadingIntrinsicHost<'gc>>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let gc = ctx.gc_with_token(&ctx.no_active_borrows_token());
    let obj_ref = ctx.pop_obj();

    if obj_ref.0.is_some() {
        let thread_id = ctx.thread_id();
        assert_ne!(
            thread_id,
            ArenaId::INVALID,
            "Monitor.TryEnter_FastPath called from unregistered thread"
        );

        let sync_block = ctx.monitor_get_or_create_sync_block_for_object(obj_ref, gc);
        let success = ctx.monitor_try_enter(&sync_block, thread_id);
        ctx.push_i32(if success { 1 } else { 0 });
    } else {
        return ctx.throw_by_name_with_message("System.NullReferenceException", NULL_REF_MSG);
    }

    StepResult::Continue
}

/// System.Threading.Monitor::TryEnter(object, int, ref bool)
pub fn intrinsic_monitor_try_enter_timeout_ref<'gc, T: ThreadingIntrinsicHost<'gc>>(
    ctx: &mut T,
    method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let gc = ctx.gc_with_token(&ctx.no_active_borrows_token());
    let success_ptr = ctx.peek_stack_at(0).as_managed_ptr();
    let timeout_ms = ctx.peek_stack_at(1).as_i32();
    let obj_ref = ctx.peek_stack_at(2).as_object_ref();

    if obj_ref.0.is_some() {
        let thread_id = ctx.thread_id();
        assert_ne!(
            thread_id,
            ArenaId::INVALID,
            "Monitor.TryEnter called from unregistered thread"
        );

        let sync_block = ctx.monitor_get_or_create_sync_block_for_object(obj_ref, gc);

        let deadline = ctx.monitor_retry_deadline(&method, timeout_ms);
        let lock_res = ctx.monitor_enter_with_timeout_safe(&sync_block, thread_id, deadline);

        if lock_res != MonitorLockResult::Yield {
            ctx.monitor_clear_retry();
        }

        let success = match lock_res {
            MonitorLockResult::Success => true,
            MonitorLockResult::Timeout => false,
            MonitorLockResult::Yield => return StepResult::Yield,
        };

        // SAFETY: F10.RawMemoryAccessValid — the out parameter denotes a writable bool;
        // managed origins check the complete byte range and unmanaged callers supply a valid address.
        if let Err(error) = unsafe {
            ctx.write_bytes(
                success_ptr.origin().clone(),
                success_ptr.byte_offset(),
                &[if success { 1u8 } else { 0u8 }],
            )
        } {
            return StepResult::Error(error.into());
        }

        // Pop success_ptr, timeout_ms, then obj_ref after all work that may yield or trigger GC.
        ctx.drop_top(3);
    } else {
        ctx.monitor_clear_retry();
        ctx.drop_top(3);
        return ctx.throw_by_name_with_message("System.NullReferenceException", NULL_REF_MSG);
    }

    StepResult::Continue
}

/// System.Threading.Monitor::TryEnter(object, int)
pub fn intrinsic_monitor_try_enter_timeout<'gc, T: ThreadingIntrinsicHost<'gc>>(
    ctx: &mut T,
    method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let gc = ctx.gc_with_token(&ctx.no_active_borrows_token());
    let timeout_ms = ctx.peek_stack_at(0).as_i32();
    let obj_ref = ctx.peek_stack_at(1).as_object_ref();

    if obj_ref.0.is_some() {
        let thread_id = ctx.thread_id();
        assert_ne!(
            thread_id,
            ArenaId::INVALID,
            "Monitor.TryEnter called from unregistered thread"
        );

        let sync_block = ctx.monitor_get_or_create_sync_block_for_object(obj_ref, gc);

        let deadline = ctx.monitor_retry_deadline(&method, timeout_ms);
        let lock_res = ctx.monitor_enter_with_timeout_safe(&sync_block, thread_id, deadline);

        if lock_res != MonitorLockResult::Yield {
            ctx.monitor_clear_retry();
        }

        let success = match lock_res {
            MonitorLockResult::Success => true,
            MonitorLockResult::Timeout => false,
            MonitorLockResult::Yield => return StepResult::Yield,
        };

        // Pop timeout_ms, then obj_ref after all work that may yield or trigger GC.
        ctx.drop_top(2);
        ctx.push_i32(if success { 1 } else { 0 });
    } else {
        ctx.monitor_clear_retry();
        ctx.drop_top(2);
        return ctx.throw_by_name_with_message("System.NullReferenceException", NULL_REF_MSG);
    }

    StepResult::Continue
}

#[cfg(feature = "multithreading")]
fn cross_arena_origin(origin: &PointerOrigin<'_>) -> bool {
    matches!(origin, PointerOrigin::CrossArenaObjectRef(_))
}

#[cfg(not(feature = "multithreading"))]
fn cross_arena_origin(_: &PointerOrigin<'_>) -> bool {
    false
}

fn select_monitor<'gc, T: ThreadingIntrinsicHost<'gc>>(
    operands: NativeOperandView<'_, 'gc>,
    method: &MethodDescription,
    timed: bool,
    byref: bool,
    fallback: dotnet_vm_ops::effects::NativeSyncHandler<T>,
) -> NativeSelection<T> {
    let count = 1 + usize::from(timed) + usize::from(byref);
    if !matches!(
        operands.peek_at(count - 1),
        StackValue::ObjectRef(ObjectRef(Some(_)))
    ) {
        return NativeSelection::Synchronous(fallback);
    }
    if byref {
        let StackValue::ManagedPtr(ptr) = operands.peek_at(0) else {
            return NativeSelection::Synchronous(fallback);
        };
        let supported = matches!(
            ptr.origin(),
            PointerOrigin::Heap(ObjectRef(Some(_))) | PointerOrigin::FrameHome(_)
        ) || cross_arena_origin(ptr.origin());
        if !supported {
            return NativeSelection::Synchronous(fallback);
        }
    }
    let timeout = if timed {
        let StackValue::Int32(timeout) = operands.peek_at(usize::from(byref)) else {
            return NativeSelection::Synchronous(fallback);
        };
        timeout
    } else {
        -1
    };
    // Capture the signed timeout before operand commitment so its deadline starts before
    // the first access grant can wait for a stop.
    NativeSelection::Async(NativeInvocation {
        start: timeout as u32 as usize,
        length: 0,
        parameter_count: method.signature().parameters.len(),
    })
}

pub fn enter_select<'gc, T: ThreadingIntrinsicHost<'gc>>(
    _: &T,
    operands: NativeOperandView<'_, 'gc>,
    method: &MethodDescription,
    _: &GenericLookup,
) -> NativeSelection<T> {
    select_monitor(
        operands,
        method,
        false,
        false,
        intrinsic_monitor_enter_obj::<T>,
    )
}
pub fn reliable_enter_select<'gc, T: ThreadingIntrinsicHost<'gc>>(
    _: &T,
    operands: NativeOperandView<'_, 'gc>,
    method: &MethodDescription,
    _: &GenericLookup,
) -> NativeSelection<T> {
    select_monitor(
        operands,
        method,
        false,
        true,
        intrinsic_monitor_reliable_enter::<T>,
    )
}
pub fn try_enter_ref_select<'gc, T: ThreadingIntrinsicHost<'gc>>(
    _: &T,
    operands: NativeOperandView<'_, 'gc>,
    method: &MethodDescription,
    _: &GenericLookup,
) -> NativeSelection<T> {
    select_monitor(
        operands,
        method,
        true,
        true,
        intrinsic_monitor_try_enter_timeout_ref::<T>,
    )
}
pub fn try_enter_select<'gc, T: ThreadingIntrinsicHost<'gc>>(
    _: &T,
    operands: NativeOperandView<'_, 'gc>,
    method: &MethodDescription,
    _: &GenericLookup,
) -> NativeSelection<T> {
    select_monitor(
        operands,
        method,
        true,
        false,
        intrinsic_monitor_try_enter_timeout::<T>,
    )
}

async fn enter_effect(
    mut fx: VmEffects,
    timed: bool,
    byref: bool,
    returns_bool: bool,
) -> NativeResult {
    let timeout = fx.invocation().start as u32 as i32;
    let deadline = if timed && timeout >= 0 {
        Some(Instant::now() + Duration::from_millis(timeout as u64))
    } else {
        None
    };
    let (object, flag, _) = fx.vm().await?.prepare_monitor(timed, byref)?;
    let acquired = fx.monitor_enter(object, deadline).await?;
    if let Some(flag) = flag {
        let mut vm = fx.vm().await?;
        // SAFETY: F10.RawMemoryAccessValid — the admitted managed out argument is a writable bool;
        // the backend resolves its owner and validates the complete byte range.
        unsafe { vm.write_monitor_flag(&flag, acquired)? };
    }
    if returns_bool {
        fx.vm().await?.push_bool(acquired)?;
    }
    Ok(())
}

#[dotnet_intrinsic("static void System.Threading.Monitor::Enter(object)", select = enter_select, fallback = intrinsic_monitor_enter_obj)]
pub async fn monitor_enter(fx: VmEffects) -> NativeResult {
    enter_effect(fx, false, false, false).await
}
#[dotnet_intrinsic("static void System.Threading.Monitor::ReliableEnter(object, bool&)", select = reliable_enter_select, fallback = intrinsic_monitor_reliable_enter)]
#[dotnet_intrinsic("static void System.Threading.Monitor::Enter(object, bool&)", select = reliable_enter_select, fallback = intrinsic_monitor_reliable_enter)]
pub async fn monitor_reliable_enter(fx: VmEffects) -> NativeResult {
    enter_effect(fx, false, true, false).await
}
#[dotnet_intrinsic("static void System.Threading.Monitor::TryEnter(object, int, bool&)", select = try_enter_ref_select, fallback = intrinsic_monitor_try_enter_timeout_ref)]
pub async fn monitor_try_enter_ref(fx: VmEffects) -> NativeResult {
    enter_effect(fx, true, true, false).await
}
#[dotnet_intrinsic("static bool System.Threading.Monitor::TryEnter(object, int)", select = try_enter_select, fallback = intrinsic_monitor_try_enter_timeout)]
pub async fn monitor_try_enter(fx: VmEffects) -> NativeResult {
    enter_effect(fx, true, false, true).await
}

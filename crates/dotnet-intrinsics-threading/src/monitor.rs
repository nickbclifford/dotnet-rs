use crate::{MonitorLockResult, ThreadingIntrinsicHost};
use dotnet_macros::dotnet_intrinsic;
use dotnet_types::{generics::GenericLookup, members::MethodDescription};
use dotnet_utils::ArenaId;
use dotnet_value::{
    StackValue,
    object::ObjectRef,
    pointer::{ManagedPtr, PointerOrigin},
};
use dotnet_vm_data::StepResult;
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
    let obj_ref = ctx.peek_stack_at(0).as_object_ref();
    if let Some(error) = monitor_argument_error(false, obj_ref.0.is_some(), None) {
        ctx.drop_top(1);
        return throw_monitor_argument_error(ctx, error);
    }

    let gc = ctx.gc_with_token(&ctx.no_active_borrows_token());
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
}

/// System.Threading.Monitor::ReliableEnter(object, ref bool)
pub fn intrinsic_monitor_reliable_enter<'gc, T: ThreadingIntrinsicHost<'gc>>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let success_ptr = ctx.peek_stack_at(0).as_managed_ptr();
    let obj_ref = ctx.peek_stack_at(1).as_object_ref();
    let lock_taken = match read_monitor_lock_taken(ctx, &success_ptr) {
        Ok(value) => value,
        Err(error) => return StepResult::Error(error),
    };
    if let Some(error) = monitor_argument_error(lock_taken, obj_ref.0.is_some(), None) {
        ctx.drop_top(2);
        return throw_monitor_argument_error(ctx, error);
    }

    let gc = ctx.gc_with_token(&ctx.no_active_borrows_token());
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

    // SAFETY: F10.RawMemoryAccessValid — the out parameter is a `bool&` by signature;
    // managed origins are bounds-checked against the live owner. Legacy dispatch preserves the
    // VM's trust in guest-supplied unmanaged addresses.
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
        return ctx
            .throw_by_name_with_message("System.ArgumentNullException", "Value cannot be null.");
    }

    StepResult::Continue
}

/// System.Threading.Monitor::TryEnter(object, int, ref bool)
pub fn intrinsic_monitor_try_enter_timeout_ref<'gc, T: ThreadingIntrinsicHost<'gc>>(
    ctx: &mut T,
    method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let success_ptr = ctx.peek_stack_at(0).as_managed_ptr();
    let timeout_ms = ctx.peek_stack_at(1).as_i32();
    let obj_ref = ctx.peek_stack_at(2).as_object_ref();
    let lock_taken = match read_monitor_lock_taken(ctx, &success_ptr) {
        Ok(value) => value,
        Err(error) => return StepResult::Error(error),
    };
    if let Some(error) = monitor_argument_error(lock_taken, obj_ref.0.is_some(), Some(timeout_ms)) {
        ctx.monitor_clear_retry();
        ctx.drop_top(3);
        return throw_monitor_argument_error(ctx, error);
    }

    let gc = ctx.gc_with_token(&ctx.no_active_borrows_token());
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

    // SAFETY: F10.RawMemoryAccessValid — the out parameter is a `bool&` by signature;
    // managed origins are bounds-checked against the live owner. Legacy dispatch preserves the
    // VM's trust in guest-supplied unmanaged addresses.
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
    StepResult::Continue
}

/// System.Threading.Monitor::TryEnter(object, int)
pub fn intrinsic_monitor_try_enter_timeout<'gc, T: ThreadingIntrinsicHost<'gc>>(
    ctx: &mut T,
    method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let timeout_ms = ctx.peek_stack_at(0).as_i32();
    let obj_ref = ctx.peek_stack_at(1).as_object_ref();
    if let Some(error) = monitor_argument_error(false, obj_ref.0.is_some(), Some(timeout_ms)) {
        ctx.monitor_clear_retry();
        ctx.drop_top(2);
        return throw_monitor_argument_error(ctx, error);
    }

    let gc = ctx.gc_with_token(&ctx.no_active_borrows_token());
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MonitorArgumentError {
    LockTaken,
    NullObject,
    Timeout,
}

fn monitor_argument_error(
    lock_taken: bool,
    object_is_non_null: bool,
    timeout: Option<i32>,
) -> Option<MonitorArgumentError> {
    if lock_taken {
        Some(MonitorArgumentError::LockTaken)
    } else if !object_is_non_null {
        Some(MonitorArgumentError::NullObject)
    } else if timeout.is_some_and(|timeout| timeout < -1) {
        Some(MonitorArgumentError::Timeout)
    } else {
        None
    }
}

fn throw_monitor_argument_error<'gc, T: ThreadingIntrinsicHost<'gc>>(
    ctx: &mut T,
    error: MonitorArgumentError,
) -> StepResult {
    match error {
        MonitorArgumentError::LockTaken => ctx.throw_by_name_with_message(
            "System.ArgumentException",
            "The lockTaken argument must be initialized to false.",
        ),
        MonitorArgumentError::NullObject => {
            ctx.throw_by_name_with_message("System.ArgumentNullException", "Value cannot be null.")
        }
        MonitorArgumentError::Timeout => ctx.throw_by_name_with_message(
            "System.ArgumentOutOfRangeException",
            "The timeout must be greater than or equal to -1.",
        ),
    }
}

fn read_monitor_lock_taken<'gc, T: ThreadingIntrinsicHost<'gc>>(
    ctx: &T,
    pointer: &ManagedPtr<'gc>,
) -> Result<bool, dotnet_types::error::VmError> {
    let mut value = [0u8; 1];
    // SAFETY: F10.RawMemoryAccessValid — `pointer` is the `bool&` operand by the resolved method
    // signature; managed origins are bounds-checked by `read_bytes`, while legacy dispatch retains
    // the VM's trust contract for guest-supplied unmanaged references.
    unsafe { ctx.read_bytes(pointer.origin().clone(), pointer.byte_offset(), &mut value) }
        .map_err(dotnet_types::error::VmError::from)?;
    Ok(value[0] != 0)
}

fn select_monitor<'gc, T: ThreadingIntrinsicHost<'gc>>(
    ctx: &T,
    operands: NativeOperandView<'_, 'gc>,
    method: &MethodDescription,
    timed: bool,
    byref: bool,
    fallback: dotnet_vm_ops::effects::NativeSyncHandler<T>,
) -> NativeSelection<T> {
    let count = 1 + usize::from(timed) + usize::from(byref);
    let lock_taken = if byref {
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
        match read_monitor_lock_taken(ctx, &ptr) {
            Ok(value) => value,
            Err(_) => return NativeSelection::Synchronous(fallback),
        }
    } else {
        false
    };

    let object_is_non_null = matches!(
        operands.peek_at(count - 1),
        StackValue::ObjectRef(ObjectRef(Some(_)))
    );
    let timeout = if timed {
        let StackValue::Int32(timeout) = operands.peek_at(usize::from(byref)) else {
            return NativeSelection::Synchronous(fallback);
        };
        Some(timeout)
    } else {
        None
    };
    if monitor_argument_error(lock_taken, object_is_non_null, timeout).is_some() {
        return NativeSelection::Synchronous(fallback);
    }

    // Capture the signed timeout before operand commitment so its deadline starts before
    // the first access grant can wait for a stop.
    NativeSelection::Async(NativeInvocation {
        start: timeout.unwrap_or(-1) as u32 as usize,
        length: 0,
        parameter_count: method.signature().parameters.len(),
    })
}

pub fn enter_select<'gc, T: ThreadingIntrinsicHost<'gc>>(
    ctx: &T,
    operands: NativeOperandView<'_, 'gc>,
    method: &MethodDescription,
    _: &GenericLookup,
) -> NativeSelection<T> {
    select_monitor(
        ctx,
        operands,
        method,
        false,
        false,
        intrinsic_monitor_enter_obj::<T>,
    )
}
pub fn reliable_enter_select<'gc, T: ThreadingIntrinsicHost<'gc>>(
    ctx: &T,
    operands: NativeOperandView<'_, 'gc>,
    method: &MethodDescription,
    _: &GenericLookup,
) -> NativeSelection<T> {
    select_monitor(
        ctx,
        operands,
        method,
        false,
        true,
        intrinsic_monitor_reliable_enter::<T>,
    )
}
pub fn try_enter_ref_select<'gc, T: ThreadingIntrinsicHost<'gc>>(
    ctx: &T,
    operands: NativeOperandView<'_, 'gc>,
    method: &MethodDescription,
    _: &GenericLookup,
) -> NativeSelection<T> {
    select_monitor(
        ctx,
        operands,
        method,
        true,
        true,
        intrinsic_monitor_try_enter_timeout_ref::<T>,
    )
}
pub fn try_enter_select<'gc, T: ThreadingIntrinsicHost<'gc>>(
    ctx: &T,
    operands: NativeOperandView<'_, 'gc>,
    method: &MethodDescription,
    _: &GenericLookup,
) -> NativeSelection<T> {
    select_monitor(
        ctx,
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
        // SAFETY: F10.RawMemoryAccessValid — admission restricts the origin to managed storage, so
        // the backend resolves its owner and bounds-checks the complete byte range. That the byte is
        // a bool and not a reference slot rests on the `bool&` signature and is not verified here.
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
// These private CoreLib helpers are also called by public TryEnter overloads; route them through
// the same argument gate so helper dispatch cannot bypass validation.
#[dotnet_intrinsic("static void System.Threading.Monitor::TryEnter(object, int, bool&)", select = try_enter_ref_select, fallback = intrinsic_monitor_try_enter_timeout_ref)]
#[dotnet_intrinsic("static void System.Threading.Monitor::ReliableEnterTimeout(object, int, bool&)", select = try_enter_ref_select, fallback = intrinsic_monitor_try_enter_timeout_ref)]
#[dotnet_intrinsic("static void System.Threading.Monitor::TryEnter_FastPath_WithTimeout(object, int, bool&)", select = try_enter_ref_select, fallback = intrinsic_monitor_try_enter_timeout_ref)]
pub async fn monitor_try_enter_ref(fx: VmEffects) -> NativeResult {
    enter_effect(fx, true, true, false).await
}
#[dotnet_intrinsic("static bool System.Threading.Monitor::TryEnter(object, int)", select = try_enter_select, fallback = intrinsic_monitor_try_enter_timeout)]
#[dotnet_intrinsic("static bool System.Threading.Monitor::TryEnter_FastPath_WithTimeout(object, int)", select = try_enter_select, fallback = intrinsic_monitor_try_enter_timeout)]
pub async fn monitor_try_enter(fx: VmEffects) -> NativeResult {
    enter_effect(fx, true, false, true).await
}

pub fn intrinsic_monitor_wait<'gc, T: ThreadingIntrinsicHost<'gc>>(
    ctx: &mut T,
    method: MethodDescription,
    _: &GenericLookup,
) -> StepResult {
    let timeout = ctx.peek_stack_at(0).as_i32();
    let object = ctx.peek_stack_at(1).as_object_ref();
    if let Some(error) = monitor_argument_error(false, object.0.is_some(), Some(timeout)) {
        ctx.drop_top(2);
        return throw_monitor_argument_error(ctx, error);
    }
    let Some(block) = ctx.monitor_get_sync_block_for_object(object) else {
        ctx.drop_top(2);
        return ctx.throw_by_name_with_message(
            "System.Threading.SynchronizationLockException",
            "Object not locked by current thread",
        );
    };
    ctx.monitor_wait_legacy(block, &method, timeout)
}

pub fn wait_select<'gc, T: ThreadingIntrinsicHost<'gc>>(
    ctx: &T,
    operands: NativeOperandView<'_, 'gc>,
    method: &MethodDescription,
    _: &GenericLookup,
) -> NativeSelection<T> {
    if let (StackValue::ObjectRef(object @ ObjectRef(Some(_))), StackValue::Int32(timeout)) =
        (operands.peek_at(1), operands.peek_at(0))
        && timeout >= -1
        && ctx
            .monitor_get_sync_block_for_object(object)
            .is_some_and(|block| ctx.monitor_is_owned(&block, ctx.thread_id()))
    {
        return NativeSelection::Async(NativeInvocation {
            start: timeout as u32 as usize,
            length: 0,
            parameter_count: method.signature().parameters.len(),
        });
    }
    NativeSelection::Synchronous(intrinsic_monitor_wait::<T>)
}

#[dotnet_intrinsic("static bool System.Threading.Monitor::Wait(object, int)", select = wait_select, fallback = intrinsic_monitor_wait)]
pub async fn monitor_wait(mut fx: VmEffects) -> NativeResult {
    let timeout = fx.invocation().start as u32 as i32;
    let deadline = (timeout >= 0).then(|| Instant::now() + Duration::from_millis(timeout as u64));
    let (object, _, _) = fx.vm().await?.prepare_monitor(true, false)?;
    let pulsed = fx.monitor_wait(object, deadline).await?;
    fx.vm().await?.push_bool(pulsed)?;
    Ok(())
}

fn pulse<'gc, T: ThreadingIntrinsicHost<'gc>>(ctx: &mut T, all: bool) -> StepResult {
    let object = ctx.pop_obj();
    if object.0.is_none() {
        return ctx
            .throw_by_name_with_message("System.ArgumentNullException", "Value cannot be null.");
    }
    if !ctx
        .monitor_get_sync_block_for_object(object)
        .is_some_and(|block| ctx.monitor_pulse(&block, ctx.thread_id(), all))
    {
        return ctx.throw_by_name_with_message(
            "System.Threading.SynchronizationLockException",
            "Object not locked by current thread",
        );
    }
    StepResult::Continue
}

#[dotnet_intrinsic("static void System.Threading.Monitor::Pulse(object)")]
pub fn intrinsic_monitor_pulse<'gc, T: ThreadingIntrinsicHost<'gc>>(
    ctx: &mut T,
    _: MethodDescription,
    _: &GenericLookup,
) -> StepResult {
    pulse(ctx, false)
}

#[dotnet_intrinsic("static void System.Threading.Monitor::PulseAll(object)")]
pub fn intrinsic_monitor_pulse_all<'gc, T: ThreadingIntrinsicHost<'gc>>(
    ctx: &mut T,
    _: MethodDescription,
    _: &GenericLookup,
) -> StepResult {
    pulse(ctx, true)
}

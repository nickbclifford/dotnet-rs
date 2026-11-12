//! VM monitor-backed implementation of CoreLib's queue-assignment lock.
//!
//! CoreLib's `ThreadPoolWorkQueue` owns a `LowLevelLock`. Its default constructor and
//! uncontended fast path use the native `LowLevelMonitor` even though the work queue is
//! managed. Keeping the lock on the VM's existing monitor path avoids that native dependency.

use crate::ThreadingIntrinsicHost;
use dotnet_macros::dotnet_intrinsic;
use dotnet_types::{generics::GenericLookup, members::MethodDescription};
use dotnet_utils::ArenaId;
use dotnet_vm_data::StepResult;

#[dotnet_intrinsic("void System.Threading.LowLevelMonitor::Initialize()")]
pub fn low_level_monitor_initialize<'gc, T: ThreadingIntrinsicHost<'gc>>(
    ctx: &mut T,
    _: MethodDescription,
    _: &GenericLookup,
) -> StepResult {
    // LowLevelLock's VM-backed acquire/release methods do not use the native monitor field.
    ctx.drop_top(1);
    StepResult::Continue
}

#[dotnet_intrinsic("bool System.Threading.LowLevelLock::TryAcquire()")]
pub fn low_level_lock_try_acquire<'gc, T: ThreadingIntrinsicHost<'gc>>(
    ctx: &mut T,
    _: MethodDescription,
    _: &GenericLookup,
) -> StepResult {
    let receiver = ctx.peek_stack_at(0).as_object_ref();
    assert!(receiver.0.is_some(), "LowLevelLock receiver is null");
    let thread_id = ctx.thread_id();
    assert_ne!(
        thread_id,
        ArenaId::INVALID,
        "LowLevelLock used by an unregistered thread"
    );
    let gc = ctx.gc_with_token(&ctx.no_active_borrows_token());
    let sync_block = ctx.monitor_get_or_create_sync_block_for_object(receiver, gc);
    let acquired = ctx.monitor_try_enter(&sync_block, thread_id);
    ctx.drop_top(1);
    ctx.push_i32(i32::from(acquired));
    StepResult::Continue
}

#[dotnet_intrinsic("void System.Threading.LowLevelLock::Acquire()")]
pub async fn low_level_lock_acquire(
    fx: dotnet_vm_ops::effects::VmEffects,
) -> dotnet_vm_ops::effects::NativeResult {
    crate::monitor::enter_effect(fx, false, false, false).await
}

#[dotnet_intrinsic("void System.Threading.LowLevelLock::Release()")]
pub fn low_level_lock_release<'gc, T: ThreadingIntrinsicHost<'gc>>(
    ctx: &mut T,
    _: MethodDescription,
    _: &GenericLookup,
) -> StepResult {
    let receiver = ctx.pop_obj();
    assert!(receiver.0.is_some(), "LowLevelLock receiver is null");
    let thread_id = ctx.thread_id();
    assert_ne!(
        thread_id,
        ArenaId::INVALID,
        "LowLevelLock used by an unregistered thread"
    );
    let Some(sync_block) = ctx.monitor_get_sync_block_for_object(receiver) else {
        unreachable!("LowLevelLock released without acquisition");
    };
    assert!(
        ctx.monitor_exit(&sync_block, thread_id),
        "LowLevelLock released by a non-owner"
    );
    StepResult::Continue
}

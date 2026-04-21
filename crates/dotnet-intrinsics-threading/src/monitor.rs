use crate::{MonitorLockResult, ThreadingIntrinsicHost};
use dotnet_macros::dotnet_intrinsic;
use dotnet_types::{generics::GenericLookup, members::MethodDescription};
use dotnet_utils::{ArenaId, StackSlotIndex};
use dotnet_value::{ManagedPtr, StackValue, pointer::PointerOrigin};
use dotnet_vm_data::StepResult;
use std::{
    cell::Cell,
    time::{Duration, Instant},
};

thread_local! {
    static CURRENT_DEADLINE: Cell<Option<Instant>> = const { Cell::new(None) };
}

fn get_deadline(timeout_ms: i32) -> Instant {
    CURRENT_DEADLINE.with(|c| {
        if let Some(d) = c.get() {
            d
        } else {
            let timeout = if timeout_ms < 0 {
                // For infinite timeout, use a very long duration (approx 100 years)
                Duration::from_secs(100 * 365 * 24 * 3600)
            } else {
                Duration::from_millis(timeout_ms as u64)
            };
            let d = Instant::now() + timeout;
            c.set(Some(d));
            d
        }
    })
}

fn clear_deadline() {
    CURRENT_DEADLINE.with(|c| c.set(None));
}

const NULL_REF_MSG: &str = "Object reference not set to an instance of an object.";

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
                panic!("SynchronizationLockException: Object not locked by current thread");
            }
        } else {
            panic!("SynchronizationLockException: Object not locked");
        }
    } else {
        // Monitor.Exit(null) is a no-op or throws ArgumentNullException in .NET?
        // Actually it throws ArgumentNullException.
        panic!("ArgumentNullException: Monitor.Exit(null)");
    }

    StepResult::Continue
}

fn find_success_flag_index(success_ptr: &ManagedPtr) -> Option<usize> {
    if let PointerOrigin::Stack(idx) = &success_ptr.origin() {
        return Some(idx.0);
    }
    None
}

/// System.Threading.Monitor::Enter(object)
#[dotnet_intrinsic("static void System.Threading.Monitor::Enter(object)")]
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
#[dotnet_intrinsic("static void System.Threading.Monitor::ReliableEnter(object, bool&)")]
#[dotnet_intrinsic("static void System.Threading.Monitor::Enter(object, bool&)")]
pub fn intrinsic_monitor_reliable_enter<'gc, T: ThreadingIntrinsicHost<'gc>>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let gc = ctx.gc_with_token(&ctx.no_active_borrows_token());
    let success_ptr = ctx.peek_stack_at(0).as_managed_ptr();
    let obj_ref = ctx.peek_stack_at(1).as_object_ref();

    let success_flag_index = find_success_flag_index(&success_ptr);

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

        if let Some(index) = success_flag_index {
            ctx.threading_set_stack_slot(StackSlotIndex(index), StackValue::Int32(1));
        } else {
            unsafe {
                ctx.write_bytes(
                    success_ptr.origin().clone(),
                    success_ptr.byte_offset(),
                    &[1u8],
                )
                .expect("Failed to write success flag");
            }
        };

        // Pop arguments now that we're done with things that might trigger GC or reallocation
        let _ = ctx.pop(); // success_ptr
        let _ = ctx.pop(); // obj_ref
    } else {
        let _ = ctx.pop();
        let _ = ctx.pop();
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
#[dotnet_intrinsic("static void System.Threading.Monitor::TryEnter(object, int, bool&)")]
pub fn intrinsic_monitor_try_enter_timeout_ref<'gc, T: ThreadingIntrinsicHost<'gc>>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let gc = ctx.gc_with_token(&ctx.no_active_borrows_token());
    let success_ptr = ctx.peek_stack_at(0).as_managed_ptr();
    let timeout_ms = ctx.peek_stack_at(1).as_i32();
    let obj_ref = ctx.peek_stack_at(2).as_object_ref();

    let success_flag_index = find_success_flag_index(&success_ptr);

    if obj_ref.0.is_some() {
        let thread_id = ctx.thread_id();
        assert_ne!(
            thread_id,
            ArenaId::INVALID,
            "Monitor.TryEnter called from unregistered thread"
        );

        let sync_block = ctx.monitor_get_or_create_sync_block_for_object(obj_ref, gc);

        let deadline = get_deadline(timeout_ms);
        let lock_res = ctx.monitor_enter_with_timeout_safe(&sync_block, thread_id, deadline);

        if lock_res != MonitorLockResult::Yield {
            clear_deadline();
        }

        let success = match lock_res {
            MonitorLockResult::Success => true,
            MonitorLockResult::Timeout => false,
            MonitorLockResult::Yield => return StepResult::Yield,
        };

        if let Some(index) = success_flag_index {
            ctx.threading_set_stack_slot(
                StackSlotIndex(index),
                StackValue::Int32(if success { 1 } else { 0 }),
            );
        } else {
            unsafe {
                ctx.write_bytes(
                    success_ptr.origin().clone(),
                    success_ptr.byte_offset(),
                    &[if success { 1u8 } else { 0u8 }],
                )
                .expect("Failed to write success flag");
            }
        };

        // Pop arguments now that we're done
        let _ = ctx.pop(); // success_ptr
        let _ = ctx.pop(); // timeout_ms
        let _ = ctx.pop(); // obj_ref
    } else {
        let _ = ctx.pop();
        let _ = ctx.pop();
        let _ = ctx.pop();
        return ctx.throw_by_name_with_message("System.NullReferenceException", NULL_REF_MSG);
    }

    StepResult::Continue
}

/// System.Threading.Monitor::TryEnter(object, int)
#[dotnet_intrinsic("static bool System.Threading.Monitor::TryEnter(object, int)")]
pub fn intrinsic_monitor_try_enter_timeout<'gc, T: ThreadingIntrinsicHost<'gc>>(
    ctx: &mut T,
    _method: MethodDescription,
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

        let deadline = get_deadline(timeout_ms);
        let lock_res = ctx.monitor_enter_with_timeout_safe(&sync_block, thread_id, deadline);

        if lock_res != MonitorLockResult::Yield {
            clear_deadline();
        }

        let success = match lock_res {
            MonitorLockResult::Success => true,
            MonitorLockResult::Timeout => false,
            MonitorLockResult::Yield => return StepResult::Yield,
        };

        let _ = ctx.pop(); // timeout_ms
        let _ = ctx.pop(); // obj_ref
        ctx.push_i32(if success { 1 } else { 0 });
    } else {
        let _ = ctx.pop();
        let _ = ctx.pop();
        return ctx.throw_by_name_with_message("System.NullReferenceException", NULL_REF_MSG);
    }

    StepResult::Continue
}

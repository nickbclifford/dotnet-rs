use crate::{
    StackSlotIndex, StepResult,
    stack::ops::{
        ExceptionOps, LoaderOps, MemoryOps, RawMemoryOps, ReflectionOps, ResolutionOps, StackOps,
        ThreadOps,
    },
    sync::{Arc, LockResult, SyncBlockOps, SyncManagerOps},
};
use dotnet_macros::dotnet_intrinsic;
use dotnet_types::{generics::GenericLookup, members::MethodDescription};
use dotnet_utils::gc::GCHandle;
use dotnet_value::{ManagedPtr, StackValue, object::ObjectRef, pointer::PointerOrigin};
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
pub fn intrinsic_monitor_exit<
    'gc,
    T: StackOps<'gc>
        + ThreadOps
        + MemoryOps<'gc>
        + RawMemoryOps<'gc>
        + ExceptionOps<'gc>
        + LoaderOps
        + ResolutionOps<'gc>
        + ReflectionOps<'gc>,
>(
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
            dotnet_utils::ArenaId::INVALID,
            "Monitor.Exit called from unregistered thread"
        );

        // Get the sync block if it exists
        if let Some(index) = obj_ref.as_object(|o| o.sync_block_index) {
            let sync_block = ctx
                .shared()
                .sync_blocks
                .get_sync_block(index)
                .expect("Sync block missing");
            if !sync_block.exit(thread_id) {
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

fn get_or_create_sync_block<'gc, T: SyncManagerOps>(
    manager: &T,
    obj_ref: ObjectRef<'gc>,
    gc: GCHandle<'gc>,
) -> Arc<T::Block> {
    let (_index, result) = manager.get_or_create_sync_block(
        || obj_ref.as_object(|o| o.sync_block_index),
        |new_index| {
            obj_ref.as_object_mut(gc, |o| {
                o.sync_block_index = Some(new_index);
            });
        },
    );
    result
}

fn find_success_flag_index<'gc, T: RawMemoryOps<'gc>>(
    _ctx: &T,
    success_ptr: &ManagedPtr,
) -> Option<usize> {
    if let PointerOrigin::Stack(idx) = &success_ptr.origin() {
        return Some(idx.0);
    }
    None
}

/// System.Threading.Monitor::Enter(object)
#[dotnet_intrinsic("static void System.Threading.Monitor::Enter(object)")]
pub fn intrinsic_monitor_enter_obj<
    'gc,
    T: StackOps<'gc>
        + ThreadOps
        + MemoryOps<'gc>
        + RawMemoryOps<'gc>
        + ExceptionOps<'gc>
        + LoaderOps
        + ResolutionOps<'gc>
        + ReflectionOps<'gc>,
>(
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
            dotnet_utils::ArenaId::INVALID,
            "Monitor.Enter called from unregistered thread"
        );

        let sync_block = get_or_create_sync_block(&ctx.shared().sync_blocks, obj_ref, gc);

        let gc_coordinator = &ctx.shared().gc_coordinator;

        match sync_block.enter_safe(
            thread_id,
            &ctx.shared().metrics,
            ctx.shared().thread_manager.as_ref(),
            gc_coordinator,
        ) {
            LockResult::Success => {
                let _ = ctx.pop();
                StepResult::Continue
            }
            LockResult::Yield => StepResult::Yield,
            LockResult::Timeout => unreachable!("Infinite timeout cannot time out"),
        }
    } else {
        let _ = ctx.pop();
        ctx.throw_by_name_with_message("System.NullReferenceException", NULL_REF_MSG)
    }
}

/// System.Threading.Monitor::ReliableEnter(object, ref bool)
#[dotnet_intrinsic("static void System.Threading.Monitor::ReliableEnter(object, bool&)")]
#[dotnet_intrinsic("static void System.Threading.Monitor::Enter(object, bool&)")]
pub fn intrinsic_monitor_reliable_enter<
    'gc,
    T: StackOps<'gc>
        + ThreadOps
        + MemoryOps<'gc>
        + RawMemoryOps<'gc>
        + ExceptionOps<'gc>
        + LoaderOps
        + ResolutionOps<'gc>
        + ReflectionOps<'gc>,
>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let gc = ctx.gc_with_token(&ctx.no_active_borrows_token());
    let success_ptr = ctx.peek_stack_at(0).as_managed_ptr();
    let obj_ref = ctx.peek_stack_at(1).as_object_ref();

    let success_flag_index = find_success_flag_index(ctx, &success_ptr);

    if obj_ref.0.is_some() {
        let thread_id = ctx.thread_id();
        assert_ne!(
            thread_id,
            dotnet_utils::ArenaId::INVALID,
            "Monitor.ReliableEnter called from unregistered thread"
        );

        let sync_block = get_or_create_sync_block(&ctx.shared().sync_blocks, obj_ref, gc);

        let gc_coordinator = &ctx.shared().gc_coordinator;

        let lock_res = sync_block.enter_safe(
            thread_id,
            &ctx.shared().metrics,
            ctx.shared().thread_manager.as_ref(),
            gc_coordinator,
        );

        match lock_res {
            LockResult::Success => {}
            LockResult::Yield => return StepResult::Yield,
            LockResult::Timeout => unreachable!(),
        }

        if let Some(index) = success_flag_index {
            ctx.set_slot(StackSlotIndex(index), StackValue::Int32(1));
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
pub fn intrinsic_monitor_try_enter_fast_path<
    'gc,
    T: StackOps<'gc>
        + ThreadOps
        + MemoryOps<'gc>
        + RawMemoryOps<'gc>
        + ExceptionOps<'gc>
        + LoaderOps
        + ResolutionOps<'gc>
        + ReflectionOps<'gc>,
>(
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
            dotnet_utils::ArenaId::INVALID,
            "Monitor.TryEnter_FastPath called from unregistered thread"
        );

        let sync_block = get_or_create_sync_block(&ctx.shared().sync_blocks, obj_ref, gc);
        let success = sync_block.try_enter(thread_id);
        ctx.push_i32(if success { 1 } else { 0 });
    } else {
        return ctx.throw_by_name_with_message("System.NullReferenceException", NULL_REF_MSG);
    }

    StepResult::Continue
}

/// System.Threading.Monitor::TryEnter(object, int, ref bool)
#[dotnet_intrinsic("static void System.Threading.Monitor::TryEnter(object, int, bool&)")]
pub fn intrinsic_monitor_try_enter_timeout_ref<
    'gc,
    T: StackOps<'gc>
        + ThreadOps
        + MemoryOps<'gc>
        + RawMemoryOps<'gc>
        + ExceptionOps<'gc>
        + LoaderOps
        + ResolutionOps<'gc>
        + ReflectionOps<'gc>,
>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let gc = ctx.gc_with_token(&ctx.no_active_borrows_token());
    let success_ptr = ctx.peek_stack_at(0).as_managed_ptr();
    let timeout_ms = ctx.peek_stack_at(1).as_i32();
    let obj_ref = ctx.peek_stack_at(2).as_object_ref();

    let success_flag_index = find_success_flag_index(ctx, &success_ptr);

    if obj_ref.0.is_some() {
        let thread_id = ctx.thread_id();
        assert_ne!(
            thread_id,
            dotnet_utils::ArenaId::INVALID,
            "Monitor.TryEnter called from unregistered thread"
        );

        let sync_block = get_or_create_sync_block(&ctx.shared().sync_blocks, obj_ref, gc);

        let deadline = get_deadline(timeout_ms);
        let lock_res = sync_block.enter_with_timeout_safe(
            thread_id,
            deadline,
            &ctx.shared().metrics,
            ctx.shared().thread_manager.as_ref(),
            &ctx.shared().gc_coordinator,
        );

        if lock_res != LockResult::Yield {
            clear_deadline();
        }

        let success = match lock_res {
            LockResult::Success => true,
            LockResult::Timeout => false,
            LockResult::Yield => return StepResult::Yield,
        };

        if let Some(index) = success_flag_index {
            ctx.set_slot(
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
pub fn intrinsic_monitor_try_enter_timeout<
    'gc,
    T: StackOps<'gc>
        + ThreadOps
        + MemoryOps<'gc>
        + RawMemoryOps<'gc>
        + ExceptionOps<'gc>
        + LoaderOps
        + ResolutionOps<'gc>
        + ReflectionOps<'gc>,
>(
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
            dotnet_utils::ArenaId::INVALID,
            "Monitor.TryEnter called from unregistered thread"
        );

        let sync_block = get_or_create_sync_block(&ctx.shared().sync_blocks, obj_ref, gc);

        let deadline = get_deadline(timeout_ms);
        let lock_res = sync_block.enter_with_timeout_safe(
            thread_id,
            deadline,
            &ctx.shared().metrics,
            ctx.shared().thread_manager.as_ref(),
            &ctx.shared().gc_coordinator,
        );

        if lock_res != LockResult::Yield {
            clear_deadline();
        }

        let success = match lock_res {
            LockResult::Success => true,
            LockResult::Timeout => false,
            LockResult::Yield => return StepResult::Yield,
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

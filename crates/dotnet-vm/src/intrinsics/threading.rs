use crate::{
    pop_args,
    utils::gc::GCHandle,
    value::object::ObjectRef,
    vm::{
        sync::{Arc, AtomicI32, Ordering, SyncBlockOps, SyncManagerOps},
        CallStack, StepResult,
    },
    vm_pop, vm_push,
};
use dotnet_types::{generics::GenericLookup, members::MethodDescription};
use std::{ptr, sync::atomic, thread};

/// System.Threading.Monitor::Exit(object) - Releases the lock on an object.
pub fn intrinsic_monitor_exit<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    pop_args!(stack, gc, [ObjectRef(obj_ref)]);

    if obj_ref.0.is_some() {
        // Get the current thread ID from the call stack
        let thread_id = stack.thread_id.get();
        assert_ne!(thread_id, 0, "Monitor.Exit called from unregistered thread");

        // Get the sync block if it exists
        if let Some(index) = obj_ref.as_object(|o| o.sync_block_index) {
            let sync_block = stack
                .shared
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

    StepResult::InstructionStepped
}

/// System.Threading.Interlocked::CompareExchange(ref int, int, int) -
/// Atomically compares two values for equality and, if they are equal,
/// replaces one of the values.
pub fn intrinsic_interlocked_compare_exchange<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    pop_args!(
        stack,
        gc,
        [Int32(comparand), Int32(value), ManagedPtr(target_ptr)]
    );

    // Ensure we are pointing to an Int32
    // TODO: support other types (long, IntPtr, object, etc.) via overloads
    let target = target_ptr.value.as_ptr() as *mut i32;

    let prev = match unsafe { AtomicI32::from_ptr(target) }.compare_exchange(
        comparand,
        value,
        Ordering::SeqCst,
        Ordering::SeqCst,
    ) {
        Ok(prev) | Err(prev) => prev,
    };

    vm_push!(stack, gc, Int32(prev));
    StepResult::InstructionStepped
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

/// System.Threading.Monitor::ReliableEnter(object, ref bool)
pub fn intrinsic_monitor_reliable_enter<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let success_flag = vm_pop!(stack, gc).as_ptr();
    pop_args!(stack, gc, [ObjectRef(obj_ref)]);

    if obj_ref.0.is_some() {
        let thread_id = stack.thread_id.get();
        assert_ne!(
            thread_id, 0,
            "Monitor.ReliableEnter called from unregistered thread"
        );

        let sync_block = get_or_create_sync_block(&stack.shared.sync_blocks, obj_ref, gc);

        while !stack.shared.sync_blocks.try_enter_block(
            sync_block.clone(),
            thread_id,
            &stack.shared.metrics,
        ) {
            stack.check_gc_safe_point();
            thread::yield_now();
        }

        unsafe {
            *success_flag = 1u8;
        }
    } else {
        return stack.throw_by_name(gc, "System.NullReferenceException");
    }

    StepResult::InstructionStepped
}

/// System.Threading.Monitor::TryEnter_FastPath(object)
pub fn intrinsic_monitor_try_enter_fast_path<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    pop_args!(stack, gc, [ObjectRef(obj_ref)]);

    if obj_ref.0.is_some() {
        let thread_id = stack.thread_id.get();
        assert_ne!(
            thread_id, 0,
            "Monitor.TryEnter_FastPath called from unregistered thread"
        );

        let sync_block = get_or_create_sync_block(&stack.shared.sync_blocks, obj_ref, gc);
        let success = sync_block.try_enter(thread_id);
        vm_push!(stack, gc, Int32(if success { 1 } else { 0 }));
    } else {
        return stack.throw_by_name(gc, "System.NullReferenceException");
    }

    StepResult::InstructionStepped
}

/// System.Threading.Monitor::TryEnter(object, int, ref bool)
pub fn intrinsic_monitor_try_enter_timeout_ref<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let success_flag = vm_pop!(stack, gc).as_ptr();
    pop_args!(stack, gc, [Int32(timeout_ms), ObjectRef(obj_ref)]);

    if obj_ref.0.is_some() {
        let thread_id = stack.thread_id.get();
        assert_ne!(
            thread_id, 0,
            "Monitor.TryEnter called from unregistered thread"
        );

        let sync_block = get_or_create_sync_block(&stack.shared.sync_blocks, obj_ref, gc);

        #[cfg(feature = "multithreaded-gc")]
        let success = sync_block.enter_with_timeout_safe(
            thread_id,
            timeout_ms as u64,
            &stack.shared.metrics,
            stack.shared.thread_manager.as_ref(),
            &stack.shared.gc_coordinator,
        );
        #[cfg(not(feature = "multithreaded-gc"))]
        let success =
            sync_block.enter_with_timeout(thread_id, timeout_ms as u64, &stack.shared.metrics);

        unsafe {
            *success_flag = if success { 1u8 } else { 0u8 };
        }
    } else {
        return stack.throw_by_name(gc, "System.NullReferenceException");
    }

    StepResult::InstructionStepped
}

/// System.Threading.Monitor::TryEnter(object, int)
pub fn intrinsic_monitor_try_enter_timeout<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    pop_args!(stack, gc, [Int32(timeout_ms), ObjectRef(obj_ref)]);

    if obj_ref.0.is_some() {
        let thread_id = stack.thread_id.get();
        assert_ne!(
            thread_id, 0,
            "Monitor.TryEnter called from unregistered thread"
        );

        let sync_block = get_or_create_sync_block(&stack.shared.sync_blocks, obj_ref, gc);

        #[cfg(feature = "multithreaded-gc")]
        let success = sync_block.enter_with_timeout_safe(
            thread_id,
            timeout_ms as u64,
            &stack.shared.metrics,
            stack.shared.thread_manager.as_ref(),
            &stack.shared.gc_coordinator,
        );
        #[cfg(not(feature = "multithreaded-gc"))]
        let success =
            sync_block.enter_with_timeout(thread_id, timeout_ms as u64, &stack.shared.metrics);

        vm_push!(stack, gc, Int32(if success { 1 } else { 0 }));
    } else {
        return stack.throw_by_name(gc, "System.NullReferenceException");
    }

    StepResult::InstructionStepped
}

/// System.Threading.Volatile::Read<T>(ref T location)
pub fn intrinsic_volatile_read<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let ptr = vm_pop!(stack, gc).as_ptr() as *const ObjectRef<'gc>;

    let value = unsafe { ptr::read_volatile(ptr) };
    // Ensure acquire semantics to match .NET memory model
    atomic::fence(Ordering::Acquire);

    vm_push!(stack, gc, ObjectRef(value));
    StepResult::InstructionStepped
}

/// System.Threading.Volatile::Write(ref bool location, bool value)
pub fn intrinsic_volatile_write_bool<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    pop_args!(stack, gc, [Int32(value)]);
    let as_bool = value as u8;
    let src = vm_pop!(stack, gc).as_ptr();

    // Ensure release semantics to match .NET memory model
    atomic::fence(Ordering::Release);
    unsafe { ptr::write_volatile(src, as_bool) };
    StepResult::InstructionStepped
}

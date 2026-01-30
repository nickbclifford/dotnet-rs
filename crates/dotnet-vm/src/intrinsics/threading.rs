use crate::{
    pop_args,
    sync::{Arc, AtomicI32, Ordering, SyncBlockOps, SyncManagerOps},
    vm_pop, vm_push, CallStack, StepResult,
};
use dotnet_macros::dotnet_intrinsic;
use dotnet_types::{generics::GenericLookup, members::MethodDescription};
use dotnet_utils::gc::GCHandle;
use dotnet_value::{object::ObjectRef, StackValue};
use dotnetdll::prelude::{BaseType, MethodType, Parameter, ParameterType};
use gc_arena::Gc;
use std::{ptr, sync::atomic, thread};

/// System.Threading.Monitor::Exit(object) - Releases the lock on an object.
#[dotnet_intrinsic("static void System.Threading.Monitor::Exit(object)")]
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

/// System.Threading.Interlocked::CompareExchange(ref T, T, T)
/// Atomically compares two values for equality and, if they are equal,
/// replaces one of the values.
#[dotnet_intrinsic("static int System.Threading.Interlocked::CompareExchange(int&, int, int)")]
#[dotnet_intrinsic("static long System.Threading.Interlocked::CompareExchange(long&, long, long)")]
#[dotnet_intrinsic(
    "static IntPtr System.Threading.Interlocked::CompareExchange(IntPtr&, IntPtr, IntPtr)"
)]
#[dotnet_intrinsic(
    "static object System.Threading.Interlocked::CompareExchange(object&, object, object)"
)]
#[dotnet_intrinsic("static T System.Threading.Interlocked::CompareExchange<T>(T&, T, T)")]
pub fn intrinsic_interlocked_compare_exchange<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    method: MethodDescription,
    generics: &GenericLookup,
) -> StepResult {
    let params = &method.method.signature.parameters;
    // CompareExchange(ref T, T, T) -> T
    // params[0] is 'ref T'.
    let Parameter(_, first_param_type) = &params[0];

    let target_type = if let ParameterType::Ref(inner) = first_param_type {
        generics.make_concrete(method.resolution(), inner.clone())
    } else {
        panic!(
            "intrinsic_interlocked_compare_exchange: First parameter must be Ref, found {:?}",
            first_param_type
        );
    };

    match target_type.get() {
        BaseType::Int32 => {
            pop_args!(
                stack,
                gc,
                [ManagedPtr(target_ptr), Int32(value), Int32(comparand)]
            );

            let target = target_ptr
                .value
                .expect("Target pointer should not be null")
                .as_ptr() as *mut i32;

            let prev = match unsafe { AtomicI32::from_ptr(target) }.compare_exchange(
                comparand,
                value,
                Ordering::SeqCst,
                Ordering::SeqCst,
            ) {
                Ok(prev) | Err(prev) => prev,
            };

            vm_push!(stack, gc, Int32(prev));
        }
        BaseType::IntPtr | BaseType::UIntPtr => {
            pop_args!(
                stack,
                gc,
                [
                    ManagedPtr(target_ptr),
                    NativeInt(value),
                    NativeInt(comparand)
                ]
            );

            let target = target_ptr
                .value
                .expect("Target pointer should not be null")
                .as_ptr() as *mut usize;

            let prev = match unsafe { atomic::AtomicUsize::from_ptr(target) }.compare_exchange(
                comparand as usize,
                value as usize,
                Ordering::SeqCst,
                Ordering::SeqCst,
            ) {
                Ok(prev) | Err(prev) => prev,
            };

            vm_push!(stack, gc, NativeInt(prev as isize));
        }
        _ => {
            // Assume ObjectRef (pointer sized) for all other types for now.
            pop_args!(
                stack,
                gc,
                [
                    ManagedPtr(target_ptr),
                    ObjectRef(value),
                    ObjectRef(comparand)
                ]
            );

            let target = target_ptr
                .value
                .expect("Target pointer should not be null")
                .as_ptr() as *mut usize;

            let val_raw = match value.0 {
                Some(ptr) => Gc::as_ptr(ptr) as usize,
                None => 0,
            };
            let comp_raw = match comparand.0 {
                Some(ptr) => Gc::as_ptr(ptr) as usize,
                None => 0,
            };

            let prev_raw = match unsafe { atomic::AtomicUsize::from_ptr(target) }.compare_exchange(
                comp_raw,
                val_raw,
                Ordering::SeqCst,
                Ordering::SeqCst,
            ) {
                Ok(prev) | Err(prev) => prev,
            };

            let prev = if prev_raw == 0 {
                ObjectRef(None)
            } else {
                // SAFETY: We just read this from an AtomicUsize where we stored valid Gc payload pointers.
                // The object is kept alive because we are in an intrinsic call and the stack roots it (or the static field roots it).
                ObjectRef(Some(unsafe { Gc::from_ptr(prev_raw as *const _) }))
            };
            vm_push!(stack, gc, ObjectRef(prev));
        }
    }

    StepResult::InstructionStepped
}

#[dotnet_intrinsic("static int System.Threading.Interlocked::Exchange(int&, int)")]
#[dotnet_intrinsic("static long System.Threading.Interlocked::Exchange(long&, long)")]
#[dotnet_intrinsic("static IntPtr System.Threading.Interlocked::Exchange(IntPtr&, IntPtr)")]
#[dotnet_intrinsic("static object System.Threading.Interlocked::Exchange(object&, object)")]
#[dotnet_intrinsic("static T System.Threading.Interlocked::Exchange<T>(T&, T)")]
pub fn intrinsic_interlocked_exchange<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    method: MethodDescription,
    generics: &GenericLookup,
) -> StepResult {
    let params = &method.method.signature.parameters;
    // Exchange(ref T, T) -> T
    // params[0] is 'ref T'.
    let Parameter(_, first_param_type) = &params[0];

    let target_type = if let ParameterType::Ref(inner) = first_param_type {
        generics.make_concrete(method.resolution(), inner.clone())
    } else {
        panic!(
            "intrinsic_interlocked_exchange: First parameter must be Ref, found {:?}",
            first_param_type
        );
    };

    match target_type.get() {
        BaseType::Int32 => {
            pop_args!(stack, gc, [ManagedPtr(target_ptr), Int32(value)]);

            let target = target_ptr
                .value
                .expect("Target pointer should not be null")
                .as_ptr() as *mut i32;

            let prev = unsafe { AtomicI32::from_ptr(target) }.swap(value, Ordering::SeqCst);

            vm_push!(stack, gc, Int32(prev));
        }
        BaseType::Int64 => {
            pop_args!(stack, gc, [ManagedPtr(target_ptr), Int64(value)]);

            let target = target_ptr
                .value
                .expect("Target pointer should not be null")
                .as_ptr() as *mut i64;

            let prev = unsafe { atomic::AtomicI64::from_ptr(target) }.swap(value, Ordering::SeqCst);

            vm_push!(stack, gc, Int64(prev));
        }
        BaseType::IntPtr | BaseType::UIntPtr => {
            pop_args!(stack, gc, [ManagedPtr(target_ptr), NativeInt(value)]);

            let target = target_ptr
                .value
                .expect("Target pointer should not be null")
                .as_ptr() as *mut usize;

            let prev = unsafe { atomic::AtomicUsize::from_ptr(target) }
                .swap(value as usize, Ordering::SeqCst);

            vm_push!(stack, gc, NativeInt(prev as isize));
        }
        _ => {
            // Assume ObjectRef (pointer sized) for all other types for now.
            // We use manual popping to handle both ObjectRef and NativeInt (which might be used for null or pointers).
            let value = vm_pop!(stack, gc);
            let target_ptr_val = vm_pop!(stack, gc);

            // DEBUG PRINT
            eprintln!(
                "Interlocked.Exchange (Object): value={:?}, target_ptr={:?}",
                value, target_ptr_val
            );

            let target_ptr = match target_ptr_val {
                StackValue::ManagedPtr(p) => p,
                _ => panic!(
                    "intrinsic_interlocked_exchange: Expected ManagedPtr, got {:?}",
                    target_ptr_val
                ),
            };

            let target = target_ptr
                .value
                .expect("Target pointer should not be null")
                .as_ptr() as *mut usize;

            let val_raw = match value {
                StackValue::ObjectRef(ObjectRef(Some(ptr))) => Gc::as_ptr(ptr) as usize,
                StackValue::ObjectRef(ObjectRef(None)) => 0,
                StackValue::NativeInt(i) => i as usize,
                _ => panic!(
                    "intrinsic_interlocked_exchange: Expected ObjectRef or NativeInt, got {:?}",
                    value
                ),
            };

            let prev_raw =
                unsafe { atomic::AtomicUsize::from_ptr(target) }.swap(val_raw, Ordering::SeqCst);

            eprintln!("Interlocked.Exchange (Object): prev_raw={:#x}", prev_raw);

            let prev = if prev_raw == 0 {
                ObjectRef(None)
            } else {
                // SAFETY: We just read this from an AtomicUsize where we stored valid Gc payload pointers.
                // The object is kept alive because we are in an intrinsic call and the stack roots it.
                ObjectRef(Some(unsafe { Gc::from_ptr(prev_raw as *const _) }))
            };
            vm_push!(stack, gc, ObjectRef(prev));
        }
    }

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
#[dotnet_intrinsic("static void System.Threading.Monitor::ReliableEnter(object, bool&)")]
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
#[dotnet_intrinsic("static bool System.Threading.Monitor::TryEnter_FastPath(object)")]
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
#[dotnet_intrinsic("static void System.Threading.Monitor::TryEnter(object, int, bool&)")]
pub fn intrinsic_monitor_try_enter_timeout_ref<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let success_flag = vm_pop!(stack, gc).as_ptr();
    pop_args!(stack, gc, [ObjectRef(obj_ref), Int32(timeout_ms)]);

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
#[dotnet_intrinsic("static bool System.Threading.Monitor::TryEnter(object, int)")]
pub fn intrinsic_monitor_try_enter_timeout<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    pop_args!(stack, gc, [ObjectRef(obj_ref), Int32(timeout_ms)]);

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
#[dotnet_intrinsic("static T System.Threading.Volatile::Read<T>(T&)")]
#[dotnet_intrinsic("static bool System.Threading.Volatile::Read(bool&)")]
#[dotnet_intrinsic("static sbyte System.Threading.Volatile::Read(sbyte&)")]
#[dotnet_intrinsic("static byte System.Threading.Volatile::Read(byte&)")]
#[dotnet_intrinsic("static short System.Threading.Volatile::Read(short&)")]
#[dotnet_intrinsic("static ushort System.Threading.Volatile::Read(ushort&)")]
#[dotnet_intrinsic("static int System.Threading.Volatile::Read(int&)")]
#[dotnet_intrinsic("static uint System.Threading.Volatile::Read(uint&)")]
#[dotnet_intrinsic("static long System.Threading.Volatile::Read(long&)")]
#[dotnet_intrinsic("static ulong System.Threading.Volatile::Read(ulong&)")]
#[dotnet_intrinsic("static IntPtr System.Threading.Volatile::Read(IntPtr&)")]
#[dotnet_intrinsic("static UIntPtr System.Threading.Volatile::Read(UIntPtr&)")]
#[dotnet_intrinsic("static float System.Threading.Volatile::Read(float&)")]
#[dotnet_intrinsic("static double System.Threading.Volatile::Read(double&)")]
pub fn intrinsic_volatile_read<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let ptr = vm_pop!(stack, gc).as_ptr() as *const usize;

    let raw = unsafe { ptr::read_volatile(ptr) };
    // Ensure acquire semantics to match .NET memory model
    atomic::fence(Ordering::Acquire);

    let value = if raw == 0 {
        ObjectRef(None)
    } else {
        ObjectRef(Some(unsafe { Gc::from_ptr(raw as *const _) }))
    };

    vm_push!(stack, gc, ObjectRef(value));
    StepResult::InstructionStepped
}

/// System.Threading.Volatile::Write(ref T location, T value)
#[dotnet_intrinsic("static void System.Threading.Volatile::Write<T>(T&, T)")]
#[dotnet_intrinsic("static void System.Threading.Volatile::Write(bool&, bool)")]
#[dotnet_intrinsic("static void System.Threading.Volatile::Write(sbyte&, sbyte)")]
#[dotnet_intrinsic("static void System.Threading.Volatile::Write(byte&, byte)")]
#[dotnet_intrinsic("static void System.Threading.Volatile::Write(short&, short)")]
#[dotnet_intrinsic("static void System.Threading.Volatile::Write(ushort&, ushort)")]
#[dotnet_intrinsic("static void System.Threading.Volatile::Write(int&, int)")]
#[dotnet_intrinsic("static void System.Threading.Volatile::Write(uint&, uint)")]
#[dotnet_intrinsic("static void System.Threading.Volatile::Write(long&, long)")]
#[dotnet_intrinsic("static void System.Threading.Volatile::Write(ulong&, ulong)")]
#[dotnet_intrinsic("static void System.Threading.Volatile::Write(IntPtr&, IntPtr)")]
#[dotnet_intrinsic("static void System.Threading.Volatile::Write(UIntPtr&, UIntPtr)")]
#[dotnet_intrinsic("static void System.Threading.Volatile::Write(float&, float)")]
#[dotnet_intrinsic("static void System.Threading.Volatile::Write(double&, double)")]
pub fn intrinsic_volatile_write<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    method: MethodDescription,
    generics: &GenericLookup,
) -> StepResult {
    let value = vm_pop!(stack, gc);
    let ptr_val = vm_pop!(stack, gc);
    let ptr = ptr_val.as_ptr();

    // Ensure release semantics to match .NET memory model
    atomic::fence(Ordering::Release);

    let val_param = &method.method.signature.parameters[1].1;

    // Determine if it's a byte-sized type (bool, byte, sbyte)
    let is_byte = if let ParameterType::Value(MethodType::Base(b)) = val_param {
        matches!(**b, BaseType::Boolean | BaseType::Int8 | BaseType::UInt8)
    } else if let ParameterType::Value(MethodType::MethodGeneric(0)) = val_param {
        // Check generic type
        if let Some(t) = generics.method_generics.first() {
            matches!(
                t.get(),
                BaseType::Boolean | BaseType::Int8 | BaseType::UInt8
            )
        } else {
            false
        }
    } else {
        false
    };

    match value {
        StackValue::Int32(i) => {
            if is_byte {
                unsafe { ptr::write_volatile(ptr, i as u8) };
            } else {
                unsafe { ptr::write_volatile(ptr as *mut i32, i) };
            }
        }
        StackValue::ObjectRef(o) => {
            let raw = match o.0 {
                Some(ptr) => Gc::as_ptr(ptr) as usize,
                None => 0,
            };
            unsafe { ptr::write_volatile(ptr as *mut usize, raw) };
        }
        StackValue::NativeInt(i) => {
            unsafe { ptr::write_volatile(ptr as *mut isize, i) };
        }
        StackValue::Int64(i) => {
            unsafe { ptr::write_volatile(ptr as *mut i64, i) };
        }
        _ => panic!("Volatile.Write not implemented for {:?}", value),
    }

    StepResult::InstructionStepped
}

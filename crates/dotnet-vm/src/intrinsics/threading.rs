use crate::{
    StepResult,
    stack::ops::VesOps,
    sync::{Arc, Ordering, SyncBlockOps, SyncManagerOps},
};
use dotnet_macros::dotnet_intrinsic;
use dotnet_types::{generics::GenericLookup, members::MethodDescription};
use dotnet_utils::{
    atomic::{AtomicAccess, StandardAtomicAccess},
    gc::GCHandle,
};
use dotnet_value::{ManagedPtr, StackValue, object::ObjectRef};
use dotnetdll::prelude::{BaseType, Parameter, ParameterType};
use gc_arena::Gc;
use std::thread;

/// System.Threading.Monitor::Exit(object) - Releases the lock on an object.
#[dotnet_intrinsic("static void System.Threading.Monitor::Exit(object)")]
pub fn intrinsic_monitor_exit<'gc, 'm: 'gc>(
    ctx: &mut dyn VesOps<'gc, 'm>,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let _gc = ctx.gc();
    let obj_ref = ctx.pop_obj();

    if obj_ref.0.is_some() {
        // Get the current thread ID from the call stack
        let thread_id = ctx.thread_id() as u64;
        assert_ne!(thread_id, 0, "Monitor.Exit called from unregistered thread");

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
    ctx: &mut dyn VesOps<'gc, 'm>,
    method: MethodDescription,
    generics: &GenericLookup,
) -> StepResult {
    let _gc = ctx.gc();
    let params = &method.method.signature.parameters;
    // CompareExchange(ref T, T, T) -> T
    // params[0] is 'ref T'.
    let Parameter(_, first_param_type) = &params[0];

    let target_type = if let ParameterType::Ref(inner) = first_param_type {
        vm_try!(generics.make_concrete(method.resolution(), inner.clone()))
    } else {
        panic!(
            "intrinsic_interlocked_compare_exchange: First parameter must be Ref, found {:?}",
            first_param_type
        );
    };

    match target_type.get() {
        BaseType::Int32 => {
            let comparand = ctx.pop_i32();
            let value = ctx.pop_i32();
            let target_ptr = ctx.pop_managed_ptr();

            let target = target_ptr
                .pointer()
                .expect("Target pointer should not be null")
                .as_ptr();

            let prev = match unsafe {
                StandardAtomicAccess::compare_exchange_atomic(
                    target,
                    4,
                    comparand as u64,
                    value as u64,
                    Ordering::SeqCst,
                    Ordering::SeqCst,
                )
            } {
                Ok(prev) | Err(prev) => prev as i32,
            };

            ctx.push_i32(prev);
        }
        BaseType::Int64 => {
            let comparand = ctx.pop_i64();
            let value = ctx.pop_i64();
            let target_ptr = ctx.pop_managed_ptr();

            let target = target_ptr
                .pointer()
                .expect("Target pointer should not be null")
                .as_ptr();

            let prev = match unsafe {
                StandardAtomicAccess::compare_exchange_atomic(
                    target,
                    8,
                    comparand as u64,
                    value as u64,
                    Ordering::SeqCst,
                    Ordering::SeqCst,
                )
            } {
                Ok(prev) | Err(prev) => prev as i64,
            };

            ctx.push_i64(prev);
        }
        BaseType::IntPtr | BaseType::UIntPtr => {
            let comparand = ctx.pop_isize();
            let value = ctx.pop_isize();
            let target_ptr = ctx.pop_managed_ptr();

            let target = target_ptr
                .pointer()
                .expect("Target pointer should not be null")
                .as_ptr();

            let size = size_of::<usize>();
            let prev = match unsafe {
                StandardAtomicAccess::compare_exchange_atomic(
                    target,
                    size,
                    comparand as u64,
                    value as u64,
                    Ordering::SeqCst,
                    Ordering::SeqCst,
                )
            } {
                Ok(prev) | Err(prev) => prev as isize,
            };

            ctx.push_isize(prev);
        }
        _ => {
            // Assume ObjectRef (pointer sized) for all other types for now.
            let comparand = ctx.pop_obj();
            let value = ctx.pop_obj();
            let target_ptr = ctx.pop_managed_ptr();

            let target = target_ptr
                .pointer()
                .expect("Target pointer should not be null")
                .as_ptr();

            let val_raw = match value.0 {
                Some(ptr) => Gc::as_ptr(ptr) as usize,
                None => 0,
            };
            let comp_raw = match comparand.0 {
                Some(ptr) => Gc::as_ptr(ptr) as usize,
                None => 0,
            };

            let size = size_of::<usize>();
            let prev_raw = match unsafe {
                StandardAtomicAccess::compare_exchange_atomic(
                    target,
                    size,
                    comp_raw as u64,
                    val_raw as u64,
                    Ordering::SeqCst,
                    Ordering::SeqCst,
                )
            } {
                Ok(prev) | Err(prev) => prev as usize,
            };

            let prev = if prev_raw == 0 {
                ObjectRef(None)
            } else {
                // SAFETY: We just read this from an atomic access where we stored valid Gc payload pointers.
                // The object is kept alive because we are in an intrinsic call and the stack roots it (or the static field roots it).
                ObjectRef(Some(unsafe { Gc::from_ptr(prev_raw as *const _) }))
            };
            ctx.push_obj(prev);
        }
    }

    StepResult::Continue
}

#[dotnet_intrinsic("static int System.Threading.Interlocked::Exchange(int&, int)")]
#[dotnet_intrinsic("static long System.Threading.Interlocked::Exchange(long&, long)")]
#[dotnet_intrinsic("static IntPtr System.Threading.Interlocked::Exchange(IntPtr&, IntPtr)")]
#[dotnet_intrinsic("static object System.Threading.Interlocked::Exchange(object&, object)")]
#[dotnet_intrinsic("static T System.Threading.Interlocked::Exchange<T>(T&, T)")]
pub fn intrinsic_interlocked_exchange<'gc, 'm: 'gc>(
    ctx: &mut dyn VesOps<'gc, 'm>,
    method: MethodDescription,
    generics: &GenericLookup,
) -> StepResult {
    let gc = ctx.gc();
    let params = &method.method.signature.parameters;
    // Exchange(ref T, T) -> T
    // params[0] is 'ref T'.
    let Parameter(_, first_param_type) = &params[0];

    let target_type = if let ParameterType::Ref(inner) = first_param_type {
        vm_try!(generics.make_concrete(method.resolution(), inner.clone()))
    } else {
        panic!(
            "intrinsic_interlocked_exchange: First parameter must be Ref, found {:?}",
            first_param_type
        );
    };

    match target_type.get() {
        BaseType::Int32 => {
            let value = ctx.pop_i32();
            let target_ptr = ctx.pop_managed_ptr();

            let target = target_ptr
                .pointer()
                .expect("Target pointer should not be null")
                .as_ptr();

            let prev = unsafe {
                StandardAtomicAccess::exchange_atomic(target, 4, value as u64, Ordering::SeqCst)
            } as i32;

            ctx.push_i32(prev);
        }
        BaseType::Int64 => {
            let value = ctx.pop_i64();
            let target_ptr = ctx.pop_managed_ptr();

            let target = target_ptr
                .pointer()
                .expect("Target pointer should not be null")
                .as_ptr();

            let prev = unsafe {
                StandardAtomicAccess::exchange_atomic(target, 8, value as u64, Ordering::SeqCst)
            } as i64;

            ctx.push_i64(prev);
        }
        BaseType::IntPtr | BaseType::UIntPtr => {
            let value = ctx.pop_isize();
            let target_ptr = ctx.pop_managed_ptr();

            let target = target_ptr
                .pointer()
                .expect("Target pointer should not be null")
                .as_ptr();

            let size = size_of::<usize>();
            let prev = unsafe {
                StandardAtomicAccess::exchange_atomic(target, size, value as u64, Ordering::SeqCst)
            } as isize;

            ctx.push_isize(prev);
        }
        _ => {
            // Assume ObjectRef (pointer sized) for all other types for now.
            // We use manual popping to handle both ObjectRef and NativeInt (which might be used for null or pointers).
            let value = ctx.pop();
            let target_ptr = ctx.pop_managed_ptr();

            let target = target_ptr
                .pointer()
                .expect("Target pointer should not be null")
                .as_ptr();

            let val_raw = match value {
                StackValue::ObjectRef(ObjectRef(Some(ptr))) => Gc::as_ptr(ptr) as usize,
                StackValue::ObjectRef(ObjectRef(None)) => 0,
                StackValue::NativeInt(i) => i as usize,
                _ => panic!(
                    "intrinsic_interlocked_exchange: Expected ObjectRef or NativeInt, got {:?}",
                    value
                ),
            };

            let size = size_of::<usize>();
            let prev_raw = unsafe {
                StandardAtomicAccess::exchange_atomic(
                    target,
                    size,
                    val_raw as u64,
                    Ordering::SeqCst,
                )
            } as usize;

            let prev = unsafe { ObjectRef::read_branded(&prev_raw.to_ne_bytes(), gc) };
            ctx.push_obj(prev);
        }
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

fn find_success_flag_index(ctx: &dyn VesOps, success_ptr: ManagedPtr) -> Option<usize> {
    let mut success_flag_index = None;
    if let (None, Some(p)) = (success_ptr.owner, success_ptr.pointer()) {
        let raw_ptr = p.as_ptr();
        for i in 0..ctx.top_of_stack() {
            if ctx.get_slot_address(i).as_ptr() == raw_ptr {
                success_flag_index = Some(i);
                break;
            }
        }
    }
    success_flag_index
}

/// System.Threading.Monitor::Enter(object)
#[dotnet_intrinsic("static void System.Threading.Monitor::Enter(object)")]
pub fn intrinsic_monitor_enter_obj<'gc, 'm: 'gc>(
    ctx: &mut dyn VesOps<'gc, 'm>,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let gc = ctx.gc();
    let obj_ref = ctx.pop_obj();

    if obj_ref.0.is_some() {
        let thread_id = ctx.thread_id() as u64;
        assert_ne!(
            thread_id, 0,
            "Monitor.Enter called from unregistered thread"
        );

        let sync_block = get_or_create_sync_block(&ctx.shared().sync_blocks, obj_ref, gc);

        while !ctx.shared().sync_blocks.try_enter_block(
            sync_block.clone(),
            thread_id,
            &ctx.shared().metrics,
        ) {
            ctx.check_gc_safe_point();
            thread::yield_now();
        }
    } else {
        return ctx.throw_by_name("System.NullReferenceException");
    }

    StepResult::Continue
}

/// System.Threading.Monitor::ReliableEnter(object, ref bool)
#[dotnet_intrinsic("static void System.Threading.Monitor::ReliableEnter(object, bool&)")]
#[dotnet_intrinsic("static void System.Threading.Monitor::Enter(object, bool&)")]
pub fn intrinsic_monitor_reliable_enter<'gc, 'm: 'gc>(
    ctx: &mut dyn VesOps<'gc, 'm>,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let gc = ctx.gc();
    let success_ptr = ctx.peek_stack_at(0).as_managed_ptr();
    let obj_ref = ctx.peek_stack_at(1).as_object_ref();

    let success_flag_index = find_success_flag_index(ctx, success_ptr);

    if obj_ref.0.is_some() {
        let thread_id = ctx.thread_id() as u64;
        assert_ne!(
            thread_id, 0,
            "Monitor.ReliableEnter called from unregistered thread"
        );

        let sync_block = get_or_create_sync_block(&ctx.shared().sync_blocks, obj_ref, gc);

        while !ctx.shared().sync_blocks.try_enter_block(
            sync_block.clone(),
            thread_id,
            &ctx.shared().metrics,
        ) {
            ctx.check_gc_safe_point();
            thread::yield_now();
        }

        let ptr = if let Some(index) = success_flag_index {
            ctx.get_slot_address(index)
        } else {
            success_ptr.pointer().expect("null success flag")
        };
        unsafe {
            *ptr.as_ptr() = 1u8;
        }

        // Pop arguments now that we're done with things that might trigger GC or reallocation
        let _ = ctx.pop(); // success_ptr
        let _ = ctx.pop(); // obj_ref
    } else {
        let _ = ctx.pop();
        let _ = ctx.pop();
        return ctx.throw_by_name("System.NullReferenceException");
    }

    StepResult::Continue
}

/// System.Threading.Monitor::TryEnter_FastPath(object)
#[dotnet_intrinsic("static bool System.Threading.Monitor::TryEnter_FastPath(object)")]
pub fn intrinsic_monitor_try_enter_fast_path<'gc, 'm: 'gc>(
    ctx: &mut dyn VesOps<'gc, 'm>,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let gc = ctx.gc();
    let obj_ref = ctx.pop_obj();

    if obj_ref.0.is_some() {
        let thread_id = ctx.thread_id() as u64;
        assert_ne!(
            thread_id, 0,
            "Monitor.TryEnter_FastPath called from unregistered thread"
        );

        let sync_block = get_or_create_sync_block(&ctx.shared().sync_blocks, obj_ref, gc);
        let success = sync_block.try_enter(thread_id);
        ctx.push_i32(if success { 1 } else { 0 });
    } else {
        return ctx.throw_by_name("System.NullReferenceException");
    }

    StepResult::Continue
}

/// System.Threading.Monitor::TryEnter(object, int, ref bool)
#[dotnet_intrinsic("static void System.Threading.Monitor::TryEnter(object, int, bool&)")]
pub fn intrinsic_monitor_try_enter_timeout_ref<'gc, 'm: 'gc>(
    ctx: &mut dyn VesOps<'gc, 'm>,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let gc = ctx.gc();
    let success_ptr = ctx.peek_stack_at(0).as_managed_ptr();
    let timeout_ms = ctx.peek_stack_at(1).as_i32();
    let obj_ref = ctx.peek_stack_at(2).as_object_ref();

    let success_flag_index = find_success_flag_index(ctx, success_ptr);

    if obj_ref.0.is_some() {
        let thread_id = ctx.thread_id() as u64;
        assert_ne!(
            thread_id, 0,
            "Monitor.TryEnter called from unregistered thread"
        );

        let sync_block = get_or_create_sync_block(&ctx.shared().sync_blocks, obj_ref, gc);

        #[cfg(feature = "multithreaded-gc")]
        let success = sync_block.enter_with_timeout_safe(
            thread_id,
            timeout_ms as u64,
            &ctx.shared().metrics,
            ctx.shared().thread_manager.as_ref(),
            &ctx.shared().gc_coordinator,
        );
        #[cfg(not(feature = "multithreaded-gc"))]
        let success =
            sync_block.enter_with_timeout(thread_id, timeout_ms as u64, &ctx.shared().metrics);

        let ptr = if let Some(index) = success_flag_index {
            ctx.get_slot_address(index)
        } else {
            success_ptr.pointer().expect("null success flag")
        };
        unsafe {
            *ptr.as_ptr() = if success { 1u8 } else { 0u8 };
        }

        // Pop arguments now that we're done
        let _ = ctx.pop(); // success_ptr
        let _ = ctx.pop(); // timeout_ms
        let _ = ctx.pop(); // obj_ref
    } else {
        let _ = ctx.pop();
        let _ = ctx.pop();
        let _ = ctx.pop();
        return ctx.throw_by_name("System.NullReferenceException");
    }

    StepResult::Continue
}

/// System.Threading.Monitor::TryEnter(object, int)
#[dotnet_intrinsic("static bool System.Threading.Monitor::TryEnter(object, int)")]
pub fn intrinsic_monitor_try_enter_timeout<'gc, 'm: 'gc>(
    ctx: &mut dyn VesOps<'gc, 'm>,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let gc = ctx.gc();
    let timeout_ms = ctx.pop_i32();
    let obj_ref = ctx.pop_obj();

    if obj_ref.0.is_some() {
        let thread_id = ctx.thread_id() as u64;
        assert_ne!(
            thread_id, 0,
            "Monitor.TryEnter called from unregistered thread"
        );

        let sync_block = get_or_create_sync_block(&ctx.shared().sync_blocks, obj_ref, gc);

        #[cfg(feature = "multithreaded-gc")]
        let success = sync_block.enter_with_timeout_safe(
            thread_id,
            timeout_ms as u64,
            &ctx.shared().metrics,
            ctx.shared().thread_manager.as_ref(),
            &ctx.shared().gc_coordinator,
        );
        #[cfg(not(feature = "multithreaded-gc"))]
        let success =
            sync_block.enter_with_timeout(thread_id, timeout_ms as u64, &ctx.shared().metrics);

        ctx.push_i32(if success { 1 } else { 0 });
    } else {
        return ctx.throw_by_name("System.NullReferenceException");
    }

    StepResult::Continue
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
    ctx: &mut dyn VesOps<'gc, 'm>,
    method: MethodDescription,
    generics: &GenericLookup,
) -> StepResult {
    let _gc = ctx.gc();
    let params = &method.method.signature.parameters;
    let Parameter(_, first_param_type) = &params[0];

    let target_type = if let ParameterType::Ref(inner) = first_param_type {
        vm_try!(generics.make_concrete(method.resolution(), inner.clone()))
    } else {
        panic!(
            "intrinsic_volatile_read: First parameter must be Ref, found {:?}",
            first_param_type
        );
    };

    match target_type.get() {
        BaseType::Boolean | BaseType::Int8 | BaseType::UInt8 => {
            let target_ptr = ctx.pop_managed_ptr();
            let target = target_ptr
                .pointer()
                .expect("Target pointer should not be null")
                .as_ptr();

            let val = unsafe { StandardAtomicAccess::load_atomic(target, 1, Ordering::Acquire) };
            ctx.push_i32(val as i32);
        }
        BaseType::Int16 | BaseType::UInt16 => {
            let target_ptr = ctx.pop_managed_ptr();
            let target = target_ptr
                .pointer()
                .expect("Target pointer should not be null")
                .as_ptr();

            let val = unsafe { StandardAtomicAccess::load_atomic(target, 2, Ordering::Acquire) };
            ctx.push_i32(val as i32);
        }
        BaseType::Int32 | BaseType::UInt32 | BaseType::Float32 => {
            let target_ptr = ctx.pop_managed_ptr();
            let target = target_ptr
                .pointer()
                .expect("Target pointer should not be null")
                .as_ptr();

            let val = unsafe { StandardAtomicAccess::load_atomic(target, 4, Ordering::Acquire) };
            if matches!(target_type.get(), BaseType::Float32) {
                ctx.push_f64(f32::from_bits(val as u32) as f64);
            } else {
                ctx.push_i32(val as i32);
            }
        }
        BaseType::Int64 | BaseType::UInt64 | BaseType::Float64 => {
            let target_ptr = ctx.pop_managed_ptr();
            let target = target_ptr
                .pointer()
                .expect("Target pointer should not be null")
                .as_ptr();

            let val = unsafe { StandardAtomicAccess::load_atomic(target, 8, Ordering::Acquire) };
            if matches!(target_type.get(), BaseType::Float64) {
                ctx.push_f64(f64::from_bits(val));
            } else {
                ctx.push_i64(val as i64);
            }
        }
        BaseType::IntPtr | BaseType::UIntPtr => {
            let target_ptr = ctx.pop_managed_ptr();
            let target = target_ptr
                .pointer()
                .expect("Target pointer should not be null")
                .as_ptr();

            let size = size_of::<usize>();
            let val = unsafe { StandardAtomicAccess::load_atomic(target, size, Ordering::Acquire) };
            ctx.push_isize(val as isize);
        }
        _ => {
            // Assume ObjectRef
            let target_ptr = ctx.pop_managed_ptr();
            let target = target_ptr
                .pointer()
                .expect("Target pointer should not be null")
                .as_ptr();

            let val = unsafe {
                StandardAtomicAccess::load_atomic(target, ObjectRef::SIZE, Ordering::Acquire)
            };
            let obj = if val == 0 {
                ObjectRef(None)
            } else {
                ObjectRef(Some(unsafe { Gc::from_ptr(val as usize as *const _) }))
            };
            ctx.push_obj(obj);
        }
    }

    StepResult::Continue
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
    ctx: &mut dyn VesOps<'gc, 'm>,
    method: MethodDescription,
    generics: &GenericLookup,
) -> StepResult {
    let _gc = ctx.gc();
    let value = ctx.pop();
    let target_ptr = ctx.pop_managed_ptr();
    let target = target_ptr
        .pointer()
        .expect("Target pointer should not be null")
        .as_ptr();

    let params = &method.method.signature.parameters;
    let Parameter(_, target_ref_type) = &params[0];

    let target_type = if let ParameterType::Ref(inner) = target_ref_type {
        vm_try!(generics.make_concrete(method.resolution(), inner.clone()))
    } else {
        panic!(
            "intrinsic_volatile_write: First parameter must be Ref, found {:?}",
            target_ref_type
        );
    };

    match target_type.get() {
        BaseType::Boolean | BaseType::Int8 | BaseType::UInt8 => {
            let val = match value {
                StackValue::Int32(i) => i as u64,
                _ => panic!("Expected Int32 for byte-sized Volatile.Write"),
            };
            unsafe { StandardAtomicAccess::store_atomic(target, 1, val, Ordering::Release) };
        }
        BaseType::Int16 | BaseType::UInt16 => {
            let val = match value {
                StackValue::Int32(i) => i as u64,
                _ => panic!("Expected Int32 for 16-bit Volatile.Write"),
            };
            unsafe { StandardAtomicAccess::store_atomic(target, 2, val, Ordering::Release) };
        }
        BaseType::Int32 | BaseType::UInt32 | BaseType::Float32 => {
            let val = match value {
                StackValue::Int32(i) => i as u32 as u64,
                StackValue::NativeFloat(f) => (f as f32).to_bits() as u64,
                _ => panic!("Expected Int32 or Float for 32-bit Volatile.Write"),
            };
            unsafe { StandardAtomicAccess::store_atomic(target, 4, val, Ordering::Release) };
        }
        BaseType::Int64 | BaseType::UInt64 | BaseType::Float64 => {
            let val = match value {
                StackValue::Int64(i) => i as u64,
                StackValue::NativeFloat(f) => f.to_bits(),
                _ => panic!("Expected Int64 or Float for 64-bit Volatile.Write"),
            };
            unsafe { StandardAtomicAccess::store_atomic(target, 8, val, Ordering::Release) };
        }
        BaseType::IntPtr | BaseType::UIntPtr => {
            let val = match value {
                StackValue::NativeInt(i) => i as u64,
                _ => panic!("Expected NativeInt for Volatile.Write"),
            };
            let size = size_of::<usize>();
            unsafe { StandardAtomicAccess::store_atomic(target, size, val, Ordering::Release) };
        }
        _ => {
            // Assume ObjectRef
            let val_raw = match value {
                StackValue::ObjectRef(ObjectRef(Some(ptr))) => Gc::as_ptr(ptr) as usize as u64,
                StackValue::ObjectRef(ObjectRef(None)) => 0,
                StackValue::NativeInt(i) => i as u64,
                _ => panic!("Expected ObjectRef or NativeInt for Volatile.Write"),
            };
            let size = size_of::<usize>();
            unsafe { StandardAtomicAccess::store_atomic(target, size, val_raw, Ordering::Release) };
        }
    }

    StepResult::Continue
}

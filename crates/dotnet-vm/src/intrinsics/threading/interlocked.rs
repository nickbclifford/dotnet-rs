use crate::{
    StepResult,
    stack::ops::{
        ExceptionOps, LoaderOps, MemoryOps, RawMemoryOps, ReflectionOps, ResolutionOps, StackOps,
        ThreadOps,
    },
    sync::Ordering,
};
use dotnet_macros::dotnet_intrinsic;
use dotnet_types::{generics::GenericLookup, members::MethodDescription};
use dotnet_value::{StackValue, object::ObjectRef};
use dotnetdll::prelude::{BaseType, Parameter, ParameterType};
use gc_arena::Gc;

#[allow(dead_code)]
const NULL_REF_MSG: &str = "Object reference not set to an instance of an object.";

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
pub fn intrinsic_interlocked_compare_exchange<
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
    method: MethodDescription,
    generics: &GenericLookup,
) -> StepResult {
    let _gc = ctx.gc_with_token(&dotnet_utils::NoActiveBorrows::new());
    let params = &method.method().signature.parameters;
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

            let prev = match unsafe {
                ctx.compare_exchange_atomic(
                    target_ptr.origin().clone(),
                    target_ptr.byte_offset(),
                    comparand as u64,
                    value as u64,
                    4,
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

            let prev = match unsafe {
                ctx.compare_exchange_atomic(
                    target_ptr.origin().clone(),
                    target_ptr.byte_offset(),
                    comparand as u64,
                    value as u64,
                    8,
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

            let size = ObjectRef::SIZE;
            let prev = match unsafe {
                ctx.compare_exchange_atomic(
                    target_ptr.origin().clone(),
                    target_ptr.byte_offset(),
                    comparand as u64,
                    value as u64,
                    size,
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

            let val_raw = match value.0 {
                Some(ptr) => Gc::as_ptr(ptr) as usize,
                None => 0,
            };
            let comp_raw = match comparand.0 {
                Some(ptr) => Gc::as_ptr(ptr) as usize,
                None => 0,
            };

            let size = ObjectRef::SIZE;
            let prev_raw = match unsafe {
                ctx.compare_exchange_atomic(
                    target_ptr.origin().clone(),
                    target_ptr.byte_offset(),
                    comp_raw as u64,
                    val_raw as u64,
                    size,
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
pub fn intrinsic_interlocked_exchange<
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
    method: MethodDescription,
    generics: &GenericLookup,
) -> StepResult {
    let gc = ctx.gc_with_token(&dotnet_utils::NoActiveBorrows::new());
    let params = &method.method().signature.parameters;
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

            let prev = unsafe {
                ctx.exchange_atomic(
                    target_ptr.origin().clone(),
                    target_ptr.byte_offset(),
                    value as u64,
                    4,
                    Ordering::SeqCst,
                )
                .expect("Interlocked.Exchange failed")
            } as i32;

            ctx.push_i32(prev);
        }
        BaseType::Int64 => {
            let value = ctx.pop_i64();
            let target_ptr = ctx.pop_managed_ptr();

            let prev = unsafe {
                ctx.exchange_atomic(
                    target_ptr.origin().clone(),
                    target_ptr.byte_offset(),
                    value as u64,
                    8,
                    Ordering::SeqCst,
                )
                .expect("Interlocked.Exchange failed")
            } as i64;

            ctx.push_i64(prev);
        }
        BaseType::IntPtr | BaseType::UIntPtr => {
            let value = ctx.pop_isize();
            let target_ptr = ctx.pop_managed_ptr();

            let size = ObjectRef::SIZE;
            let prev = unsafe {
                ctx.exchange_atomic(
                    target_ptr.origin().clone(),
                    target_ptr.byte_offset(),
                    value as u64,
                    size,
                    Ordering::SeqCst,
                )
                .expect("Interlocked.Exchange failed")
            } as isize;

            ctx.push_isize(prev);
        }
        _ => {
            // Assume ObjectRef (pointer sized) for all other types for now.
            // We use manual popping to handle both ObjectRef and NativeInt (which might be used for null or pointers).
            let value = ctx.pop();
            let target_ptr = ctx.pop_managed_ptr();

            let val_raw = match value {
                StackValue::ObjectRef(ObjectRef(Some(ptr))) => Gc::as_ptr(ptr) as usize,
                StackValue::ObjectRef(ObjectRef(None)) => 0,
                StackValue::NativeInt(i) => i as usize,
                _ => panic!(
                    "intrinsic_interlocked_exchange: Expected ObjectRef or NativeInt, got {:?}",
                    value
                ),
            };

            let size = ObjectRef::SIZE;
            let prev_raw = unsafe {
                ctx.exchange_atomic(
                    target_ptr.origin().clone(),
                    target_ptr.byte_offset(),
                    val_raw as u64,
                    size,
                    Ordering::SeqCst,
                )
                .expect("Interlocked.Exchange failed")
            } as usize;

            let prev = unsafe { ObjectRef::read_branded(&prev_raw.to_ne_bytes(), &gc) };
            ctx.push_obj(prev);
        }
    }

    StepResult::Continue
}

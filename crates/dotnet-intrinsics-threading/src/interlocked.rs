use crate::{
    ThreadingIntrinsicHost,
    atomic_dispatch::{
        InterlockedAtomicTypeDispatch, interlocked_atomic_dispatch, resolve_atomic_ref_target_type,
    },
};
use dotnet_macros::dotnet_intrinsic;
use dotnet_types::{
    error::{CompareExchangeError, VmError},
    generics::GenericLookup,
    members::MethodDescription,
};
use dotnet_utils::sync::Ordering;
use dotnet_value::{StackValue, object::ObjectRef};
use dotnet_vm_ops::StepResult;

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
pub fn intrinsic_interlocked_compare_exchange<'gc, T: ThreadingIntrinsicHost<'gc>>(
    ctx: &mut T,
    method: MethodDescription,
    generics: &GenericLookup,
) -> StepResult {
    let _gc = ctx.gc_with_token(&ctx.no_active_borrows_token());
    // CompareExchange(ref T, T, T) -> T
    // First parameter is always `ref T`.
    let target_type = dotnet_vm_ops::vm_try!(resolve_atomic_ref_target_type(
        ctx,
        &method,
        generics,
        "intrinsic_interlocked_compare_exchange",
    ));

    match interlocked_atomic_dispatch(target_type.get()) {
        InterlockedAtomicTypeDispatch::Byte => {
            let is_signed = matches!(target_type.get(), dotnetdll::prelude::BaseType::Int8);
            let comparand = ctx.pop_i32() as u8;
            let value = ctx.pop_i32() as u8;
            let target_ptr = ctx.pop_managed_ptr();

            // SAFETY: `target_ptr` is the managed `ref T` argument and size matches byte-width CAS.
            let prev = match unsafe {
                ctx.threading_compare_exchange_atomic(
                    target_ptr.origin().clone(),
                    target_ptr.byte_offset(),
                    comparand as u64,
                    value as u64,
                    1,
                    Ordering::SeqCst,
                    Ordering::SeqCst,
                )
            } {
                Ok(prev) | Err(CompareExchangeError::Mismatch(prev)) => prev as u8,
                Err(CompareExchangeError::Bounds(e)) => {
                    return StepResult::Error(VmError::from(e));
                }
            };

            if is_signed {
                ctx.push_i32((prev as i8) as i32);
            } else {
                ctx.push_i32(prev as i32);
            }
        }
        InterlockedAtomicTypeDispatch::Int16 => {
            let is_signed = matches!(target_type.get(), dotnetdll::prelude::BaseType::Int16);
            let comparand = ctx.pop_i32() as u16;
            let value = ctx.pop_i32() as u16;
            let target_ptr = ctx.pop_managed_ptr();

            // SAFETY: `target_ptr` is the managed `ref T` argument and size matches 16-bit CAS.
            let prev = match unsafe {
                ctx.threading_compare_exchange_atomic(
                    target_ptr.origin().clone(),
                    target_ptr.byte_offset(),
                    comparand as u64,
                    value as u64,
                    2,
                    Ordering::SeqCst,
                    Ordering::SeqCst,
                )
            } {
                Ok(prev) | Err(CompareExchangeError::Mismatch(prev)) => prev as u16,
                Err(CompareExchangeError::Bounds(e)) => {
                    return StepResult::Error(VmError::from(e));
                }
            };

            if is_signed {
                ctx.push_i32((prev as i16) as i32);
            } else {
                ctx.push_i32(prev as i32);
            }
        }
        InterlockedAtomicTypeDispatch::Int32 => {
            let comparand = ctx.pop_i32();
            let value = ctx.pop_i32();
            let target_ptr = ctx.pop_managed_ptr();

            // SAFETY: `target_ptr` is the managed `ref T` argument for this intrinsic,
            // and the size/orderings match the selected primitive operation.
            let prev = match unsafe {
                ctx.threading_compare_exchange_atomic(
                    target_ptr.origin().clone(),
                    target_ptr.byte_offset(),
                    comparand as u64,
                    value as u64,
                    4,
                    Ordering::SeqCst,
                    Ordering::SeqCst,
                )
            } {
                Ok(prev) | Err(CompareExchangeError::Mismatch(prev)) => prev as i32,
                Err(CompareExchangeError::Bounds(e)) => {
                    return StepResult::Error(VmError::from(e));
                }
            };

            ctx.push_i32(prev);
        }
        InterlockedAtomicTypeDispatch::Int64 => {
            let comparand = ctx.pop_i64();
            let value = ctx.pop_i64();
            let target_ptr = ctx.pop_managed_ptr();

            // SAFETY: `target_ptr` is the managed `ref T` argument for this intrinsic,
            // and the size/orderings match the selected primitive operation.
            let prev = match unsafe {
                ctx.threading_compare_exchange_atomic(
                    target_ptr.origin().clone(),
                    target_ptr.byte_offset(),
                    comparand as u64,
                    value as u64,
                    8,
                    Ordering::SeqCst,
                    Ordering::SeqCst,
                )
            } {
                Ok(prev) | Err(CompareExchangeError::Mismatch(prev)) => prev as i64,
                Err(CompareExchangeError::Bounds(e)) => {
                    return StepResult::Error(VmError::from(e));
                }
            };

            ctx.push_i64(prev);
        }
        InterlockedAtomicTypeDispatch::PointerSized => {
            let comparand = ctx.pop_isize();
            let value = ctx.pop_isize();
            let target_ptr = ctx.pop_managed_ptr();

            let size = ObjectRef::SIZE;
            // SAFETY: `target_ptr` is the managed `ref T` argument for this intrinsic,
            // and the size/orderings match pointer-width CAS.
            let prev = match unsafe {
                ctx.threading_compare_exchange_atomic(
                    target_ptr.origin().clone(),
                    target_ptr.byte_offset(),
                    comparand as u64,
                    value as u64,
                    size,
                    Ordering::SeqCst,
                    Ordering::SeqCst,
                )
            } {
                Ok(prev) | Err(CompareExchangeError::Mismatch(prev)) => prev as isize,
                Err(CompareExchangeError::Bounds(e)) => {
                    return StepResult::Error(VmError::from(e));
                }
            };

            ctx.push_isize(prev);
        }
        InterlockedAtomicTypeDispatch::ObjectRef => {
            // Assume ObjectRef (pointer sized) for all other types for now.
            let comparand = ctx.pop_obj();
            let value = ctx.pop_obj();
            let target_ptr = ctx.pop_managed_ptr();

            // Encode using ObjectRef::write so the tagged representation (Tag-5 +
            // ArenaId in upper 16 bits) matches what is stored in memory by the
            // normal field-write path.  Using Gc::as_ptr directly would produce an
            // untagged pointer that never matches the stored tagged bytes.
            let mut val_buf = [0u8; ObjectRef::SIZE];
            value.write(&mut val_buf);
            let val_raw = usize::from_ne_bytes(val_buf);

            let mut comp_buf = [0u8; ObjectRef::SIZE];
            comparand.write(&mut comp_buf);
            let comp_raw = usize::from_ne_bytes(comp_buf);

            let gc = ctx.gc_with_token(&ctx.no_active_borrows_token());
            let size = ObjectRef::SIZE;
            // SAFETY: `target_ptr` is the managed `ref T` argument and `comp_raw`/`val_raw`
            // use the same tagged object representation as regular field writes.
            let prev_raw = match unsafe {
                ctx.threading_compare_exchange_atomic(
                    target_ptr.origin().clone(),
                    target_ptr.byte_offset(),
                    comp_raw as u64,
                    val_raw as u64,
                    size,
                    Ordering::SeqCst,
                    Ordering::SeqCst,
                )
            } {
                Ok(prev) | Err(CompareExchangeError::Mismatch(prev)) => prev as usize,
                Err(CompareExchangeError::Bounds(e)) => {
                    return StepResult::Error(VmError::from(e));
                }
            };

            // Decode via read_branded so the tag bits are stripped correctly and
            // the GC lifetime is properly branded.  Gc::from_ptr(prev_raw) would
            // use the tagged value as a raw address, producing an invalid pointer.
            // SAFETY: `prev_raw` came from VM-managed object slot bytes and `gc`
            // brands the returned reference to the current arena lifetime.
            let prev = unsafe { ObjectRef::read_branded(&prev_raw.to_ne_bytes(), &gc) };
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
pub fn intrinsic_interlocked_exchange<'gc, T: ThreadingIntrinsicHost<'gc>>(
    ctx: &mut T,
    method: MethodDescription,
    generics: &GenericLookup,
) -> StepResult {
    let gc = ctx.gc_with_token(&ctx.no_active_borrows_token());
    // Exchange(ref T, T) -> T
    // First parameter is always `ref T`.
    let target_type = dotnet_vm_ops::vm_try!(resolve_atomic_ref_target_type(
        ctx,
        &method,
        generics,
        "intrinsic_interlocked_exchange",
    ));

    match interlocked_atomic_dispatch(target_type.get()) {
        InterlockedAtomicTypeDispatch::Byte => {
            let is_signed = matches!(target_type.get(), dotnetdll::prelude::BaseType::Int8);
            let value = ctx.pop_i32() as u8;
            let target_ptr = ctx.pop_managed_ptr();

            // SAFETY: `target_ptr` is the managed `ref T` argument and size matches byte-width exchange.
            let prev = unsafe {
                ctx.threading_exchange_atomic(
                    target_ptr.origin().clone(),
                    target_ptr.byte_offset(),
                    value as u64,
                    1,
                    Ordering::SeqCst,
                )
                .expect("Interlocked.Exchange failed")
            } as u8;

            if is_signed {
                ctx.push_i32((prev as i8) as i32);
            } else {
                ctx.push_i32(prev as i32);
            }
        }
        InterlockedAtomicTypeDispatch::Int16 => {
            let is_signed = matches!(target_type.get(), dotnetdll::prelude::BaseType::Int16);
            let value = ctx.pop_i32() as u16;
            let target_ptr = ctx.pop_managed_ptr();

            // SAFETY: `target_ptr` is the managed `ref T` argument and size matches 16-bit exchange.
            let prev = unsafe {
                ctx.threading_exchange_atomic(
                    target_ptr.origin().clone(),
                    target_ptr.byte_offset(),
                    value as u64,
                    2,
                    Ordering::SeqCst,
                )
                .expect("Interlocked.Exchange failed")
            } as u16;

            if is_signed {
                ctx.push_i32((prev as i16) as i32);
            } else {
                ctx.push_i32(prev as i32);
            }
        }
        InterlockedAtomicTypeDispatch::Int32 => {
            let value = ctx.pop_i32();
            let target_ptr = ctx.pop_managed_ptr();

            // SAFETY: `target_ptr` is the managed `ref T` argument and size matches `i32`.
            let prev = unsafe {
                ctx.threading_exchange_atomic(
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
        InterlockedAtomicTypeDispatch::Int64 => {
            let value = ctx.pop_i64();
            let target_ptr = ctx.pop_managed_ptr();

            // SAFETY: `target_ptr` is the managed `ref T` argument and size matches `i64`.
            let prev = unsafe {
                ctx.threading_exchange_atomic(
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
        InterlockedAtomicTypeDispatch::PointerSized => {
            let value = ctx.pop_isize();
            let target_ptr = ctx.pop_managed_ptr();

            let size = ObjectRef::SIZE;
            // SAFETY: `target_ptr` is the managed `ref T` argument and size matches pointer width.
            let prev = unsafe {
                ctx.threading_exchange_atomic(
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
        InterlockedAtomicTypeDispatch::ObjectRef => {
            // Assume ObjectRef (pointer sized) for all other types for now.
            // We use manual popping to handle both ObjectRef and NativeInt (which might be used for null or pointers).
            let value = ctx.pop();
            let target_ptr = ctx.pop_managed_ptr();

            // Encode using ObjectRef::write so the tagged representation matches
            // what is stored in memory by the normal field-write path.
            let val_raw = match value {
                StackValue::ObjectRef(ref obj_ref) => {
                    let mut buf = [0u8; ObjectRef::SIZE];
                    obj_ref.write(&mut buf);
                    usize::from_ne_bytes(buf)
                }
                StackValue::NativeInt(i) => i as usize,
                _ => panic!(
                    "intrinsic_interlocked_exchange: Expected ObjectRef or NativeInt, got {:?}",
                    value
                ),
            };

            let size = ObjectRef::SIZE;
            // SAFETY: `target_ptr` is the managed `ref T` argument and `val_raw`
            // uses the VM tagged object representation.
            let prev_raw = unsafe {
                ctx.threading_exchange_atomic(
                    target_ptr.origin().clone(),
                    target_ptr.byte_offset(),
                    val_raw as u64,
                    size,
                    Ordering::SeqCst,
                )
                .expect("Interlocked.Exchange failed")
            } as usize;

            // SAFETY: `prev_raw` came from VM-managed object slot bytes and `gc`
            // brands the returned reference to the current arena lifetime.
            let prev = unsafe { ObjectRef::read_branded(&prev_raw.to_ne_bytes(), &gc) };
            ctx.push_obj(prev);
        }
    }

    StepResult::Continue
}

#[dotnet_intrinsic("static int System.Threading.Interlocked::ExchangeAdd(int&, int)")]
#[dotnet_intrinsic("static long System.Threading.Interlocked::ExchangeAdd(long&, long)")]
#[dotnet_intrinsic("static int System.Threading.Interlocked::Add(int&, int)")]
#[dotnet_intrinsic("static long System.Threading.Interlocked::Add(long&, long)")]
pub fn intrinsic_interlocked_exchange_add<'gc, T: ThreadingIntrinsicHost<'gc>>(
    ctx: &mut T,
    method: MethodDescription,
    generics: &GenericLookup,
) -> StepResult {
    let _gc = ctx.gc_with_token(&ctx.no_active_borrows_token());
    let target_type = dotnet_vm_ops::vm_try!(resolve_atomic_ref_target_type(
        ctx,
        &method,
        generics,
        "intrinsic_interlocked_exchange_add",
    ));

    match interlocked_atomic_dispatch(target_type.get()) {
        InterlockedAtomicTypeDispatch::Byte | InterlockedAtomicTypeDispatch::Int16 => {
            return StepResult::not_implemented(
                "Interlocked.ExchangeAdd/Add is only supported for int and long.",
            );
        }
        InterlockedAtomicTypeDispatch::Int32 => {
            let value = ctx.pop_i32();
            let target_ptr = ctx.pop_managed_ptr();

            // SAFETY: `target_ptr` is the managed `ref T` argument and size matches `i32`.
            let prev = unsafe {
                ctx.threading_exchange_add_atomic(
                    target_ptr.origin().clone(),
                    target_ptr.byte_offset(),
                    value as u64,
                    4,
                    Ordering::SeqCst,
                )
                .expect("Interlocked.ExchangeAdd failed")
            } as i32;

            if method.method().name.contains("Add") && !method.method().name.contains("ExchangeAdd")
            {
                ctx.push_i32(prev + value);
            } else {
                ctx.push_i32(prev);
            }
        }
        InterlockedAtomicTypeDispatch::Int64 => {
            let value = ctx.pop_i64();
            let target_ptr = ctx.pop_managed_ptr();

            // SAFETY: `target_ptr` is the managed `ref T` argument and size matches `i64`.
            let prev = unsafe {
                ctx.threading_exchange_add_atomic(
                    target_ptr.origin().clone(),
                    target_ptr.byte_offset(),
                    value as u64,
                    8,
                    Ordering::SeqCst,
                )
                .expect("Interlocked.ExchangeAdd failed")
            } as i64;

            if method.method().name.contains("Add") && !method.method().name.contains("ExchangeAdd")
            {
                ctx.push_i64(prev + value);
            } else {
                ctx.push_i64(prev);
            }
        }
        InterlockedAtomicTypeDispatch::PointerSized | InterlockedAtomicTypeDispatch::ObjectRef => {
            panic!(
                "intrinsic_interlocked_exchange_add: Unsupported type {:?}",
                target_type
            );
        }
    }

    StepResult::Continue
}

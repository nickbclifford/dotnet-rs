use crate::{
    ThreadingIntrinsicHost,
    atomic_dispatch::{
        InterlockedAtomicTypeDispatch, interlocked_atomic_dispatch, resolve_atomic_ref_target_type,
    },
};
use dotnet_macros::dotnet_intrinsic;
use dotnet_types::{
    error::{CompareExchangeError, ExecutionError, MemoryAccessError, VmError},
    generics::GenericLookup,
    members::MethodDescription,
};
use dotnet_utils::{atomic::PointerAtomicOp, sync::Ordering};
use dotnet_value::{StackValue, object::ObjectRef};
use dotnet_vm_data::StepResult;
use dotnet_vm_ops::ops::RawMemoryOps;

const DATA_MISALIGNED_MESSAGE: &str = "Object accessed by unaligned reference.";

fn handle_interlocked_memory_access_error<'gc, T: ThreadingIntrinsicHost<'gc>>(
    ctx: &mut T,
    error: MemoryAccessError,
) -> StepResult {
    match error {
        MemoryAccessError::UnalignedAccess(_) => ctx
            .throw_by_name_with_message("System.DataMisalignedException", DATA_MISALIGNED_MESSAGE),
        error => StepResult::Error(VmError::from(error)),
    }
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

            // SAFETY: F10.RawMemoryAccessValid, F4.WidthAligned — `target_ptr` is the managed `ref T` argument and size matches byte-width CAS.
            // The atomic subsystem uses the selected width for an aligned atomic operation or a synchronized fallback.
            let prev = match unsafe {
                RawMemoryOps::compare_exchange_atomic(
                    ctx,
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
                Err(CompareExchangeError::Bounds(error)) => {
                    return handle_interlocked_memory_access_error(ctx, error);
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

            // SAFETY: F10.RawMemoryAccessValid, F4.WidthAligned — `target_ptr` is the managed `ref T` argument and size matches 16-bit CAS.
            // The atomic subsystem uses the selected width for an aligned atomic operation or a synchronized fallback.
            let prev = match unsafe {
                RawMemoryOps::compare_exchange_atomic(
                    ctx,
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
                Err(CompareExchangeError::Bounds(error)) => {
                    return handle_interlocked_memory_access_error(ctx, error);
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

            // SAFETY: F10.RawMemoryAccessValid, F4.WidthAligned — `target_ptr` is the managed `ref T` argument for this intrinsic,
            // and the size/orderings match the selected primitive operation.
            // The atomic subsystem uses the selected width for an aligned atomic operation or a synchronized fallback.
            let prev = match unsafe {
                RawMemoryOps::compare_exchange_atomic(
                    ctx,
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
                Err(CompareExchangeError::Bounds(error)) => {
                    return handle_interlocked_memory_access_error(ctx, error);
                }
            };

            ctx.push_i32(prev);
        }
        InterlockedAtomicTypeDispatch::Int64 => {
            let comparand = ctx.pop_i64();
            let value = ctx.pop_i64();
            let target_ptr = ctx.pop_managed_ptr();

            // SAFETY: F10.RawMemoryAccessValid, F4.WidthAligned — `target_ptr` is the managed `ref T` argument for this intrinsic,
            // and the size/orderings match the selected primitive operation.
            // The atomic subsystem uses the selected width for an aligned atomic operation or a synchronized fallback.
            let prev = match unsafe {
                RawMemoryOps::compare_exchange_atomic(
                    ctx,
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
                Err(CompareExchangeError::Bounds(error)) => {
                    return handle_interlocked_memory_access_error(ctx, error);
                }
            };

            ctx.push_i64(prev);
        }
        InterlockedAtomicTypeDispatch::PointerSized => {
            let comparand = ctx.pop_isize();
            let value = ctx.pop_isize();
            let target_ptr = ctx.pop_managed_ptr();

            let size = ObjectRef::SIZE;
            // SAFETY: F10.RawMemoryAccessValid, F4.WidthAligned — `target_ptr` is the managed `ref T` argument for this intrinsic,
            // and the size/orderings match pointer-width CAS.
            // The atomic subsystem uses the selected width for an aligned atomic operation or a synchronized fallback.
            let prev = match unsafe {
                RawMemoryOps::compare_exchange_atomic(
                    ctx,
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
                Err(CompareExchangeError::Bounds(error)) => {
                    return handle_interlocked_memory_access_error(ctx, error);
                }
            };

            ctx.push_isize(prev);
        }
        InterlockedAtomicTypeDispatch::ObjectRef => {
            // Assume ObjectRef (pointer sized) for all other types for now.
            let comparand = ctx.pop_obj();
            let value = ctx.pop_obj();
            let target_ptr = ctx.pop_managed_ptr();

            let val_raw = value.to_storage_ptr();
            let comp_raw = comparand.to_storage_ptr();
            // SAFETY: F10.RawMemoryAccessValid, F4.WidthAligned — The target is a live object slot and both inputs are encoded live references.
            let prev_raw = match unsafe {
                RawMemoryOps::object_atomic(
                    ctx,
                    target_ptr.origin().clone(),
                    target_ptr.byte_offset(),
                    PointerAtomicOp::CompareExchange(
                        comp_raw,
                        val_raw,
                        Ordering::SeqCst,
                        Ordering::SeqCst,
                    ),
                )
            } {
                Ok(prev) => prev,
                Err(error) => {
                    return handle_interlocked_memory_access_error(ctx, error);
                }
            };
            // SAFETY: F1.GcHandleRooted — The previous slot value remains rooted in the active GC scope.
            let prev = unsafe { ObjectRef::from_storage_ptr(prev_raw) };
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
    let _gc = ctx.gc_with_token(&ctx.no_active_borrows_token());
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

            // SAFETY: F10.RawMemoryAccessValid, F4.WidthAligned — `target_ptr` is the managed `ref T` argument and size matches byte-width exchange.
            // The atomic subsystem uses the selected width for an aligned atomic operation or a synchronized fallback.
            let prev = match unsafe {
                RawMemoryOps::exchange_atomic(
                    ctx,
                    target_ptr.origin().clone(),
                    target_ptr.byte_offset(),
                    value as u64,
                    1,
                    Ordering::SeqCst,
                )
            } {
                Ok(prev) => prev as u8,
                Err(error) => return handle_interlocked_memory_access_error(ctx, error),
            };

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

            // SAFETY: F10.RawMemoryAccessValid, F4.WidthAligned — `target_ptr` is the managed `ref T` argument and size matches 16-bit exchange.
            // The atomic subsystem uses the selected width for an aligned atomic operation or a synchronized fallback.
            let prev = match unsafe {
                RawMemoryOps::exchange_atomic(
                    ctx,
                    target_ptr.origin().clone(),
                    target_ptr.byte_offset(),
                    value as u64,
                    2,
                    Ordering::SeqCst,
                )
            } {
                Ok(prev) => prev as u16,
                Err(error) => return handle_interlocked_memory_access_error(ctx, error),
            };

            if is_signed {
                ctx.push_i32((prev as i16) as i32);
            } else {
                ctx.push_i32(prev as i32);
            }
        }
        InterlockedAtomicTypeDispatch::Int32 => {
            let value = ctx.pop_i32();
            let target_ptr = ctx.pop_managed_ptr();

            // SAFETY: F10.RawMemoryAccessValid, F4.WidthAligned — `target_ptr` is the managed `ref T` argument and size matches `i32`.
            // The atomic subsystem uses the selected width for an aligned atomic operation or a synchronized fallback.
            let prev = match unsafe {
                RawMemoryOps::exchange_atomic(
                    ctx,
                    target_ptr.origin().clone(),
                    target_ptr.byte_offset(),
                    value as u64,
                    4,
                    Ordering::SeqCst,
                )
            } {
                Ok(prev) => prev as i32,
                Err(error) => return handle_interlocked_memory_access_error(ctx, error),
            };

            ctx.push_i32(prev);
        }
        InterlockedAtomicTypeDispatch::Int64 => {
            let value = ctx.pop_i64();
            let target_ptr = ctx.pop_managed_ptr();

            // SAFETY: F10.RawMemoryAccessValid, F4.WidthAligned — `target_ptr` is the managed `ref T` argument and size matches `i64`.
            // The atomic subsystem uses the selected width for an aligned atomic operation or a synchronized fallback.
            let prev = match unsafe {
                RawMemoryOps::exchange_atomic(
                    ctx,
                    target_ptr.origin().clone(),
                    target_ptr.byte_offset(),
                    value as u64,
                    8,
                    Ordering::SeqCst,
                )
            } {
                Ok(prev) => prev as i64,
                Err(error) => return handle_interlocked_memory_access_error(ctx, error),
            };

            ctx.push_i64(prev);
        }
        InterlockedAtomicTypeDispatch::PointerSized => {
            let value = ctx.pop_isize();
            let target_ptr = ctx.pop_managed_ptr();

            let size = ObjectRef::SIZE;
            // SAFETY: F10.RawMemoryAccessValid, F4.WidthAligned — `target_ptr` is the managed `ref T` argument and size matches pointer width.
            // The atomic subsystem uses the selected width for an aligned atomic operation or a synchronized fallback.
            let prev = match unsafe {
                RawMemoryOps::exchange_atomic(
                    ctx,
                    target_ptr.origin().clone(),
                    target_ptr.byte_offset(),
                    value as u64,
                    size,
                    Ordering::SeqCst,
                )
            } {
                Ok(prev) => prev as isize,
                Err(error) => return handle_interlocked_memory_access_error(ctx, error),
            };

            ctx.push_isize(prev);
        }
        InterlockedAtomicTypeDispatch::ObjectRef => {
            // Assume ObjectRef (pointer sized) for all other types for now.
            // We use manual popping to handle both ObjectRef and NativeInt (which might be used for null or pointers).
            let value = ctx.pop();
            let target_ptr = ctx.pop_managed_ptr();

            let val_raw = match value {
                StackValue::ObjectRef(ref obj_ref) => obj_ref.to_storage_ptr(),
                // Native integers are an explicit exposed-provenance boundary.
                StackValue::NativeInt(0) => std::ptr::null_mut(),
                StackValue::NativeInt(i) => std::ptr::with_exposed_provenance_mut(i as usize),
                actual => {
                    return StepResult::Error(VmError::Execution(ExecutionError::TypeMismatch {
                        expected: "ObjectRef or NativeInt",
                        actual: format!("{actual:?}").into(),
                    }));
                }
            };

            // SAFETY: F10.RawMemoryAccessValid, F4.WidthAligned — `target_ptr` is a live object slot and the value uses its representation.
            let prev_raw = match unsafe {
                RawMemoryOps::object_atomic(
                    ctx,
                    target_ptr.origin().clone(),
                    target_ptr.byte_offset(),
                    PointerAtomicOp::Exchange(val_raw, Ordering::SeqCst),
                )
            } {
                Ok(prev) => prev,
                Err(error) => return handle_interlocked_memory_access_error(ctx, error),
            };

            // SAFETY: F1.GcHandleRooted — The previous slot value remains rooted in the active GC scope.
            let prev = unsafe { ObjectRef::from_storage_ptr(prev_raw) };
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

            // SAFETY: F10.RawMemoryAccessValid, F4.WidthAligned — `target_ptr` is the managed `ref T` argument and size matches `i32`.
            // The atomic subsystem uses the selected width for an aligned atomic operation or a synchronized fallback.
            let prev = match unsafe {
                RawMemoryOps::exchange_add_atomic(
                    ctx,
                    target_ptr.origin().clone(),
                    target_ptr.byte_offset(),
                    value as u64,
                    4,
                    Ordering::SeqCst,
                )
            } {
                Ok(prev) => prev as i32,
                Err(error) => return handle_interlocked_memory_access_error(ctx, error),
            };

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

            // SAFETY: F10.RawMemoryAccessValid, F4.WidthAligned — `target_ptr` is the managed `ref T` argument and size matches `i64`.
            // The atomic subsystem uses the selected width for an aligned atomic operation or a synchronized fallback.
            let prev = match unsafe {
                RawMemoryOps::exchange_add_atomic(
                    ctx,
                    target_ptr.origin().clone(),
                    target_ptr.byte_offset(),
                    value as u64,
                    8,
                    Ordering::SeqCst,
                )
            } {
                Ok(prev) => prev as i64,
                Err(error) => return handle_interlocked_memory_access_error(ctx, error),
            };

            if method.method().name.contains("Add") && !method.method().name.contains("ExchangeAdd")
            {
                ctx.push_i64(prev + value);
            } else {
                ctx.push_i64(prev);
            }
        }
        InterlockedAtomicTypeDispatch::PointerSized | InterlockedAtomicTypeDispatch::ObjectRef => {
            return StepResult::Error(VmError::Execution(ExecutionError::TypeMismatch {
                expected: "Int32 or Int64",
                actual: format!("{:?}", target_type.get()).into(),
            }));
        }
    }

    StepResult::Continue
}

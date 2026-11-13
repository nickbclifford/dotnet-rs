use crate::mem_helpers::{copy_one_chunk, fill_one_chunk};
use crate::{NULL_REF_MSG, UnsafeIntrinsicHost, ptr_info};
use dotnet_macros::dotnet_intrinsic;
use dotnet_types::{error::ExecutionError, generics::GenericLookup, members::MethodDescription};
use dotnet_utils::ByteOffset;
use dotnet_value::{
    StackValue,
    layout::{HasLayout, LayoutManager, Scalar},
    object::HeapStorage,
    pointer::RetainedPtr,
};
use dotnet_vm_data::StepResult;
use dotnet_vm_ops::effects::{
    BULK_CHUNK_SIZE, NativeInvocation, NativeOperandView, NativeResult, NativeSelection, VmEffects,
};
use dotnet_vm_ops::ops::{EvalStackOps, ExceptionOps, LoaderOps, TypedStackOps};

#[inline]
fn stack_value_to_byte(value: &StackValue<'_>) -> Option<u8> {
    match value {
        StackValue::Int32(v) => Some(*v as u8),
        StackValue::Int64(v) => Some(*v as u8),
        StackValue::NativeInt(v) => Some(*v as u8),
        _ => None,
    }
}

/// Retains a non-null pointer-like fill destination, including the arena lease
/// of a cross-arena managed pointer, for the whole synchronous fill.
fn retained_fill_destination<'gc>(
    value: &StackValue<'gc>,
) -> Result<Option<RetainedPtr<'gc>>, ExecutionError> {
    match value {
        StackValue::NativeInt(_) | StackValue::UnmanagedPtr(_) | StackValue::ManagedPtr(_) => {
            let retained = RetainedPtr::try_from_stack_value(value.clone())?;
            Ok((!retained.is_null()).then_some(retained))
        }
        _ => Ok(None),
    }
}

/// Selects the async body only for byte counts above one chunk; smaller operations cannot
/// reach a safepoint, so they keep the single-poll synchronous handler.
fn exceeds_chunk(bytes: Option<usize>) -> bool {
    bytes.is_none_or(|bytes| bytes > BULK_CHUNK_SIZE)
}

fn scalar_len(value: StackValue<'_>) -> Option<isize> {
    match value {
        StackValue::Int32(v) => Some(v as isize),
        StackValue::Int64(v) => Some(v as isize),
        StackValue::NativeInt(v) => Some(v),
        _ => None,
    }
}

/// Element size in bytes of `Memmove<T>`'s `T`, or 0 for the byte-count overloads.
pub fn memmove_select<'gc, T: UnsafeIntrinsicHost<'gc>>(
    ctx: &T,
    operands: NativeOperandView<'_, 'gc>,
    _method: &MethodDescription,
    generics: &GenericLookup,
) -> NativeSelection {
    let Some(len) = scalar_len(operands.peek_at(0)) else {
        return NativeSelection::Fallback;
    };
    let element = if generics.method_generics.is_empty() {
        0
    } else {
        let Ok(target) = generics.method_arg(0) else {
            return NativeSelection::Fallback;
        };
        let Ok(layout) = ctx.unsafe_type_layout(target.clone()) else {
            return NativeSelection::Fallback;
        };
        layout.size().as_usize()
    };
    let bytes = if element == 0 {
        Some(len as usize)
    } else {
        element.checked_mul(len as usize)
    };
    if exceeds_chunk(bytes) {
        NativeSelection::Async(NativeInvocation {
            start: element,
            length: len as usize,
            parameter_count: 3,
        })
    } else {
        NativeSelection::Fallback
    }
}

/// Copies in bounded chunks. Pointers are rooted descriptors resolved afresh for every chunk,
/// so a collection between chunks neither invalidates them nor restarts the move.
#[dotnet_intrinsic("static void System.Buffer::Memmove(byte*, byte*, ulong)", select = memmove_select, fallback = intrinsic_buffer_memmove)]
#[dotnet_intrinsic("static void System.Buffer::Memmove<T>(T&, T&, ulong)", select = memmove_select, fallback = intrinsic_buffer_memmove)]
#[dotnet_intrinsic("static void System.Buffer::Memmove<T>(T&, T&, nuint)", select = memmove_select, fallback = intrinsic_buffer_memmove)]
#[dotnet_intrinsic("static void System.SpanHelpers::Memmove(ref byte, ref byte, nuint)", select = memmove_select, fallback = intrinsic_buffer_memmove)]
pub async fn memmove_async(mut fx: VmEffects) -> NativeResult {
    let invocation = fx.invocation();
    let (source, destination, total) = {
        let mut vm = fx.vm().await?;
        vm.discard_scalars(1)?;
        let source = vm.pop_pointer()?;
        let destination = vm.pop_pointer()?;
        let total = if invocation.start == 0 {
            invocation.length
        } else {
            match invocation.start.checked_mul(invocation.length) {
                Some(bytes) => bytes,
                None => {
                    return Err(vm.exception(
                        "System.OverflowException",
                        "Arithmetic operation resulted in an overflow.",
                    )?);
                }
            }
        };
        (source, destination, total)
    };
    let mut done = 0usize;
    while done < total {
        {
            let mut vm = fx.vm().await?;
            // SAFETY: F10.RawMemoryAccessValid — managed ranges are resolved and checked for the
            // full `total` bytes on every chunk; unmanaged ranges follow the guest contract.
            done += unsafe { vm.bulk_copy_chunk(&source, &destination, total, done)? };
        }
        if done < total {
            fx.yield_now().await?;
        }
    }
    Ok(())
}

#[allow(unused_variables)]
pub fn intrinsic_buffer_memmove<'gc, T: UnsafeIntrinsicHost<'gc>>(
    ctx: &mut T,
    _method: MethodDescription,
    generics: &GenericLookup,
) -> StepResult {
    let len = ctx.pop_isize();
    let src = dotnet_vm_ops::vm_try!(RetainedPtr::try_from_stack_value(ctx.pop()));
    let dst = dotnet_vm_ops::vm_try!(RetainedPtr::try_from_stack_value(ctx.pop()));

    let total_count = if generics.method_generics.is_empty() {
        len as usize
    } else {
        let target = dotnet_vm_ops::vm_try!(generics.method_arg(0));
        let layout = dotnet_vm_ops::vm_try!(ctx.unsafe_type_layout(target.clone()));
        let Some(bytes) = layout.size().as_usize().checked_mul(len as usize) else {
            return ctx.throw_by_name_with_message(
                "System.OverflowException",
                "Arithmetic operation resulted in an overflow.",
            );
        };
        bytes
    };

    let source = dotnet_vm_ops::vm_try!(ctx.retained_range(&src, total_count));
    let destination = dotnet_vm_ops::vm_try!(ctx.retained_range(&dst, total_count));
    // SAFETY: F10.RawMemoryAccessValid — retained_range checked the full managed range; retained operands stay alive through the call. Unmanaged validity and synchronization follow the guest intrinsic contract.
    unsafe { copy_one_chunk(source, destination, total_count) }
}

pub fn clear_select<'gc, T: UnsafeIntrinsicHost<'gc>>(
    _ctx: &T,
    operands: NativeOperandView<'_, 'gc>,
    _method: &MethodDescription,
    _generics: &GenericLookup,
) -> NativeSelection {
    match scalar_len(operands.peek_at(0)) {
        Some(len) if exceeds_chunk(Some(len.max(0) as usize)) => {
            NativeSelection::Async(NativeInvocation {
                start: 0,
                length: len.max(0) as usize,
                parameter_count: 2,
            })
        }
        _ => NativeSelection::Fallback,
    }
}

#[dotnet_intrinsic("static void System.SpanHelpers::ClearWithoutReferences(ref byte, nuint)", select = clear_select, fallback = intrinsic_span_helpers_clear_without_references)]
#[dotnet_intrinsic("static void System.SpanHelpers::ClearWithoutReferences(byte*, nuint)", select = clear_select, fallback = intrinsic_span_helpers_clear_without_references)]
pub async fn clear_async(fx: VmEffects) -> NativeResult {
    fill_async_with(fx, 0).await
}

async fn fill_async_with(mut fx: VmEffects, value: u8) -> NativeResult {
    let invocation = fx.invocation();
    let total = invocation.length;
    let destination = {
        let mut vm = fx.vm().await?;
        // The selector captured the length (and fill value); drop them, then the pointer.
        vm.discard_scalars(invocation.parameter_count - 1)?;
        vm.pop_pointer()?
    };
    let mut done = 0usize;
    while done < total {
        {
            let mut vm = fx.vm().await?;
            // SAFETY: F10.RawMemoryAccessValid — the range is resolved and checked for the full
            // `total` bytes on every chunk; unmanaged ranges follow the guest contract.
            done += unsafe { vm.bulk_fill_chunk(&destination, total, done, value)? };
        }
        if done < total {
            fx.yield_now().await?;
        }
    }
    Ok(())
}

#[allow(unused_variables)]
pub fn intrinsic_span_helpers_clear_without_references<'gc, T: UnsafeIntrinsicHost<'gc>>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let len = ctx.pop_isize();
    let dst = dotnet_vm_ops::vm_try!(RetainedPtr::try_from_stack_value(ctx.pop()));
    let total_count = len.max(0) as usize;

    let destination = dotnet_vm_ops::vm_try!(ctx.retained_range(&dst, total_count));
    // SAFETY: F10.RawMemoryAccessValid — retained_range checked the full destination range.
    unsafe { fill_one_chunk(destination, total_count, 0) }
}

/// Byte-compatible fills over one chunk stay synchronous. Larger ones capture the fill byte and
/// length for the async body. Wider element types keep the synchronous element loop (see the
/// plan's "Kept" table), as do null destinations and wrong operand shapes (they throw there).
pub fn fill_select<'gc, T: UnsafeIntrinsicHost<'gc>>(
    ctx: &T,
    operands: NativeOperandView<'_, 'gc>,
    _method: &MethodDescription,
    generics: &GenericLookup,
) -> NativeSelection {
    let (Some(byte), Some(len)) = (
        stack_value_to_byte(&operands.peek_at(0)),
        scalar_len(operands.peek_at(1)),
    ) else {
        return NativeSelection::Fallback;
    };
    let len = len.max(0) as usize;
    let destination = operands.peek_at(2);
    let Ok(target) = generics.cloned_method_arg(0) else {
        return NativeSelection::Fallback;
    };
    let Ok(layout) = ctx.unsafe_type_layout(target) else {
        return NativeSelection::Fallback;
    };
    let byte_compatible = layout.size().as_usize() == 1
        && matches!(
            layout.as_ref(),
            LayoutManager::Scalar(Scalar::Int8 | Scalar::UInt8)
        );
    let pointer_like = matches!(
        destination,
        StackValue::NativeInt(_) | StackValue::UnmanagedPtr(_) | StackValue::ManagedPtr(_)
    ) && !destination.is_null();
    if byte_compatible && pointer_like && exceeds_chunk(Some(len)) {
        NativeSelection::Async(NativeInvocation {
            start: byte as usize,
            length: len,
            parameter_count: 3,
        })
    } else {
        NativeSelection::Fallback
    }
}

#[dotnet_intrinsic("static void System.SpanHelpers::Fill<T>(T&, nuint, T)", select = fill_select, fallback = intrinsic_span_helpers_fill)]
pub async fn fill_async(fx: VmEffects) -> NativeResult {
    let byte = fx.invocation().start as u8;
    fill_async_with(fx, byte).await
}

#[allow(unused_variables)]
pub fn intrinsic_span_helpers_fill<'gc, T: UnsafeIntrinsicHost<'gc>>(
    ctx: &mut T,
    _method: MethodDescription,
    generics: &GenericLookup,
) -> StepResult {
    let value = ctx.pop();
    let len = ctx.pop_isize().max(0) as usize;
    let destination = ctx.pop();
    if destination.is_null() {
        return ctx.throw_by_name_with_message("System.NullReferenceException", NULL_REF_MSG);
    }

    let (origin, base_offset) = match ptr_info(ctx, &destination) {
        Ok(v) => v,
        Err(e) => return e,
    };

    let target = dotnet_vm_ops::vm_try!(generics.cloned_method_arg(0));
    let layout = dotnet_vm_ops::vm_try!(ctx.unsafe_type_layout(target));
    let elem_size = layout.size().as_usize();

    // Fast path for byte-compatible fills.
    if elem_size == 1
        && matches!(
            layout.as_ref(),
            LayoutManager::Scalar(Scalar::Int8 | Scalar::UInt8)
        )
        && let Some(byte_value) = stack_value_to_byte(&value)
        && let Some(dst) = dotnet_vm_ops::vm_try!(retained_fill_destination(&destination))
    {
        let address = dotnet_vm_ops::vm_try!(ctx.retained_range(&dst, len));
        // SAFETY: F10.RawMemoryAccessValid — retained_range checked the full managed range; `dst` keeps its owner alive through the call. Unmanaged validity and synchronization follow the guest intrinsic contract.
        let result = unsafe { fill_one_chunk(address, len, byte_value) };
        drop(dst);
        return result;
    }

    for i in 0..len {
        let offset = base_offset + i * elem_size;
        if let Err(e) =
            // SAFETY: F10.RawMemoryAccessValid — `ptr_info` returned the managed origin and offset for `destination`; the
            // layout was resolved for the generic element type before the write.
            unsafe { ctx.write_unaligned(origin.clone(), offset, value.clone(), &layout) }
        {
            return StepResult::not_implemented(format!("SpanHelpers.Fill failed: {e}"));
        }
        // No collection poll: a yield retries the call, which would restart the fill.
    }

    StepResult::Continue
}

#[dotnet_intrinsic(
    "static T& System.Runtime.InteropServices.MemoryMarshal::GetArrayDataReference<T>(T[])"
)]
#[allow(unused_variables)]
pub fn intrinsic_memory_marshal_get_array_data_reference<
    'gc,
    T: EvalStackOps<'gc> + TypedStackOps<'gc> + LoaderOps + ExceptionOps<'gc>,
>(
    ctx: &mut T,
    _method: MethodDescription,
    generics: &GenericLookup,
) -> StepResult {
    let obj = ctx.pop_obj();
    let Some(array_handle) = obj.0 else {
        return ctx.throw_by_name_with_message("System.NullReferenceException", NULL_REF_MSG);
    };

    let (data_ptr, fallback_element_type) = match &array_handle.borrow().storage {
        HeapStorage::Vec(v) => (v.get().as_ptr() as *mut u8, v.element.clone()),
        _ => {
            return ctx.throw_by_name_with_message(
                "System.ArgumentException",
                "The argument must be an array.",
            );
        }
    };

    let element_concrete = generics
        .method_generics
        .first()
        .cloned()
        .unwrap_or(fallback_element_type);
    let element_type = dotnet_vm_ops::vm_try!(ctx.loader().find_concrete_type(element_concrete));
    ctx.push_ptr(
        data_ptr,
        element_type,
        false,
        Some(obj),
        Some(ByteOffset::new(0)),
    );
    StepResult::Continue
}

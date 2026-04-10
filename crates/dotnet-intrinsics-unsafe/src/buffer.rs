use crate::{NULL_REF_MSG, UnsafeIntrinsicHost, ptr_info, vm_try};
use dotnet_macros::dotnet_intrinsic;
use dotnet_types::{generics::GenericLookup, members::MethodDescription};
use dotnet_utils::ByteOffset;
use dotnet_value::{layout::HasLayout, object::HeapStorage};
use dotnet_vm_ops::{
    StepResult,
    ops::{EvalStackOps, ExceptionOps, LoaderOps, TypedStackOps},
};
use std::ptr;

#[dotnet_intrinsic("static void System.Buffer::Memmove(byte*, byte*, ulong)")]
#[dotnet_intrinsic("static void System.Buffer::Memmove<T>(T&, T&, ulong)")]
#[dotnet_intrinsic("static void System.Buffer::Memmove<T>(T&, T&, nuint)")]
#[dotnet_intrinsic("static void System.SpanHelpers::Memmove(ref byte, ref byte, nuint)")]
#[allow(unused_variables)]
pub fn intrinsic_buffer_memmove<'gc, T: UnsafeIntrinsicHost<'gc>>(
    ctx: &mut T,
    _method: MethodDescription,
    generics: &GenericLookup,
) -> StepResult {
    let len = ctx.pop_isize();
    let src = ctx.pop_ptr();
    let dst = ctx.pop_ptr();

    let total_count = if generics.method_generics.is_empty() {
        len as usize
    } else {
        let target = &generics.method_generics[0];
        let layout = vm_try!(ctx.unsafe_type_layout(target.clone()));
        (layout.size() * (len as usize)).as_usize()
    };

    // Perform the move in chunks to allow GC safe points if necessary.
    // Since our GC is currently non-moving, raw pointers remain valid across safe points.
    const MEMMOVE_CHUNK_SIZE: usize = 1024 * 1024; // 1MB chunks
    let mut offset = 0;
    while offset < total_count {
        let current_chunk = std::cmp::min(total_count - offset, MEMMOVE_CHUNK_SIZE);
        unsafe {
            ptr::copy(src.add(offset), dst.add(offset), current_chunk);
        }
        offset += current_chunk;
        if offset < total_count && ctx.check_gc_safe_point() {
            return StepResult::Yield;
        }
    }

    StepResult::Continue
}

#[dotnet_intrinsic("static void System.SpanHelpers::ClearWithoutReferences(ref byte, nuint)")]
#[dotnet_intrinsic("static void System.SpanHelpers::ClearWithoutReferences(byte*, nuint)")]
#[allow(unused_variables)]
pub fn intrinsic_span_helpers_clear_without_references<'gc, T: UnsafeIntrinsicHost<'gc>>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let len = ctx.pop_isize();
    let dst = ctx.pop_ptr();
    let total_count = len.max(0) as usize;

    // Keep long clears interruptible for the scheduler/GC.
    const CLEAR_CHUNK_SIZE: usize = 1024 * 1024; // 1MB chunks
    let mut offset = 0;
    while offset < total_count {
        let current_chunk = std::cmp::min(total_count - offset, CLEAR_CHUNK_SIZE);
        unsafe {
            ptr::write_bytes(dst.add(offset), 0, current_chunk);
        }
        offset += current_chunk;
        if offset < total_count && ctx.check_gc_safe_point() {
            return StepResult::Yield;
        }
    }

    StepResult::Continue
}

#[dotnet_intrinsic("static void System.SpanHelpers::Fill<T>(T&, nuint, T)")]
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

    let target = generics.method_generics[0].clone();
    let layout = vm_try!(ctx.unsafe_type_layout(target));
    let elem_size = layout.size().as_usize();

    for i in 0..len {
        let offset = ByteOffset(base_offset.as_usize() + i * elem_size);
        if let Err(e) =
            unsafe { ctx.write_unaligned(origin.clone(), offset, value.clone(), &layout) }
        {
            return StepResult::not_implemented(format!("SpanHelpers.Fill failed: {e}"));
        }
        if i + 1 < len && ctx.check_gc_safe_point() {
            return StepResult::Yield;
        }
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
    let element_type = vm_try!(ctx.loader().find_concrete_type(element_concrete));
    ctx.push_ptr(
        data_ptr,
        element_type,
        false,
        Some(obj),
        Some(ByteOffset(0)),
    );
    StepResult::Continue
}

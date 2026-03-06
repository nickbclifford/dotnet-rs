use crate::{
    StepResult,
    layout::type_layout,
    stack::ops::{
        EvalStackOps, ExceptionOps, LoaderOps, RawMemoryOps, ResolutionOps, TypedStackOps,
    },
};
use dotnet_macros::dotnet_intrinsic;
use dotnet_types::{generics::GenericLookup, members::MethodDescription};
use dotnet_utils::ByteOffset;
use dotnet_value::{layout::HasLayout, object::HeapStorage};
use std::ptr;

const NULL_REF_MSG: &str = "Object reference not set to an instance of an object.";

#[dotnet_intrinsic("static void System.Buffer::Memmove(byte*, byte*, ulong)")]
#[dotnet_intrinsic("static void System.Buffer::Memmove<T>(T&, T&, ulong)")]
#[dotnet_intrinsic("static void System.Buffer::Memmove<T>(T&, T&, nuint)")]
#[allow(unused_variables)]
pub fn intrinsic_buffer_memmove<
    'gc,
    T: EvalStackOps<'gc> + TypedStackOps<'gc> + ResolutionOps<'gc> + RawMemoryOps<'gc>,
>(
    ctx: &mut T,
    method: MethodDescription,
    generics: &GenericLookup,
) -> StepResult {
    let len = ctx.pop_isize();
    let src = ctx.pop_ptr();
    let dst = ctx.pop_ptr();

    let res_ctx = ctx.with_generics(generics);
    let total_count = if generics.method_generics.is_empty() {
        len as usize
    } else {
        let target = &generics.method_generics[0];
        let layout = vm_try!(type_layout(target.clone(), &res_ctx));
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

#[dotnet_intrinsic(
    "static T& System.Runtime.InteropServices.MemoryMarshal::GetArrayDataReference<T>(T[])"
)]
#[allow(unused_variables)]
pub fn intrinsic_memory_marshal_get_array_data_reference<
    'gc,
    T: EvalStackOps<'gc> + TypedStackOps<'gc> + LoaderOps + ExceptionOps<'gc>,
>(
    ctx: &mut T,
    method: MethodDescription,
    generics: &GenericLookup,
) -> StepResult {
    let obj = ctx.pop_obj();
    let Some(array_handle) = obj.0 else {
        return ctx.throw_by_name_with_message("System.NullReferenceException", NULL_REF_MSG);
    };

    let data_ptr = match &array_handle.borrow().storage {
        HeapStorage::Vec(v) => v.get().as_ptr() as *mut u8,
        _ => {
            return ctx.throw_by_name_with_message(
                "System.ArgumentException",
                "The argument must be an array.",
            );
        }
    };

    let element_type = vm_try!(
        ctx.loader()
            .find_concrete_type(generics.method_generics[0].clone())
    );
    ctx.push_ptr(
        data_ptr,
        element_type,
        false,
        Some(obj),
        Some(ByteOffset(0)),
    );
    StepResult::Continue
}

use dotnet_macros::dotnet_intrinsic;
use dotnet_types::{generics::GenericLookup, members::MethodDescription};
use dotnet_value::{
    StackValue,
    layout::HasLayout,
    object::{HeapStorage, ObjectRef, Vector},
};
use dotnet_vm_ops::{
    NULL_REF_MSG, StepResult,
    ops::{EvalStackOps, ExceptionOps, MemoryOps, TypedStackOps},
};
const INDEX_OUT_OF_RANGE_MSG: &str = "Index was outside the bounds of the array.";
const NOT_ARRAY_MSG: &str = "Object must be of type Array.";
const INDEX_ARG_TYPE_MSG: &str = "Index must be Int32, Int64, or Int32[].";
const INDEX_ARRAY_TYPE_MSG: &str = "Indices must be provided as an Int32 array.";

fn extract_first_index_from_indices_array<'gc, T: ExceptionOps<'gc>>(
    ctx: &mut T,
    indices: &Vector<'gc>,
) -> Result<usize, StepResult> {
    if indices.layout.length == 0 {
        return Err(ctx.throw_by_name_with_message(
            "System.ArgumentException",
            "The number of indices provided does not match the number of dimensions of the array.",
        ));
    }

    // For now only support 1D access via the first index in the array.
    if indices.get().len() < 4 {
        return Err(
            ctx.throw_by_name_with_message("System.ArgumentException", INDEX_ARRAY_TYPE_MSG)
        );
    }

    let Ok(bytes): Result<[u8; 4], _> = indices.get()[0..4].try_into() else {
        return Err(
            ctx.throw_by_name_with_message("System.ArgumentException", INDEX_ARRAY_TYPE_MSG)
        );
    };

    Ok(i32::from_ne_bytes(bytes) as usize)
}

#[dotnet_intrinsic("int System.Array::get_Length()")]
pub fn intrinsic_array_get_length<'gc, T: TypedStackOps<'gc> + ExceptionOps<'gc>>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let obj = ctx.pop_obj();
    let ObjectRef(Some(handle)) = obj else {
        return ctx.throw_by_name_with_message("System.NullReferenceException", NULL_REF_MSG);
    };

    let heap = handle.borrow();
    let len = match &heap.storage {
        HeapStorage::Vec(v) => v.layout.length,
        _ => return ctx.throw_by_name_with_message("System.ArgumentException", NOT_ARRAY_MSG),
    };

    ctx.push_i32(len as i32);
    StepResult::Continue
}

#[dotnet_intrinsic("int System.Array::get_Rank()")]
pub fn intrinsic_array_get_rank<'gc, T: TypedStackOps<'gc> + ExceptionOps<'gc>>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let obj = ctx.pop_obj();
    let ObjectRef(Some(handle)) = obj else {
        return ctx.throw_by_name_with_message("System.NullReferenceException", NULL_REF_MSG);
    };

    let heap = handle.borrow();
    let rank = match &heap.storage {
        HeapStorage::Vec(_) => 1,
        _ => return ctx.throw_by_name_with_message("System.ArgumentException", NOT_ARRAY_MSG),
    };

    ctx.push_i32(rank);
    StepResult::Continue
}

#[dotnet_intrinsic("int System.Array::GetLowerBound(int)")]
pub fn intrinsic_array_get_lower_bound<'gc, T: TypedStackOps<'gc> + ExceptionOps<'gc>>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let dimension = ctx.pop_i32();
    let obj = ctx.pop_obj();
    let ObjectRef(Some(handle)) = obj else {
        return ctx.throw_by_name_with_message("System.NullReferenceException", NULL_REF_MSG);
    };

    let heap = handle.borrow();
    match &heap.storage {
        HeapStorage::Vec(_) => {
            if dimension != 0 {
                return ctx.throw_by_name_with_message(
                    "System.IndexOutOfRangeException",
                    INDEX_OUT_OF_RANGE_MSG,
                );
            }
            ctx.push_i32(0);
            StepResult::Continue
        }
        _ => ctx.throw_by_name_with_message("System.ArgumentException", NOT_ARRAY_MSG),
    }
}

#[dotnet_intrinsic("int System.Array::GetUpperBound(int)")]
pub fn intrinsic_array_get_upper_bound<'gc, T: TypedStackOps<'gc> + ExceptionOps<'gc>>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let dimension = ctx.pop_i32();
    let obj = ctx.pop_obj();
    let ObjectRef(Some(handle)) = obj else {
        return ctx.throw_by_name_with_message("System.NullReferenceException", NULL_REF_MSG);
    };

    let heap = handle.borrow();
    match &heap.storage {
        HeapStorage::Vec(v) => {
            if dimension != 0 {
                return ctx.throw_by_name_with_message(
                    "System.IndexOutOfRangeException",
                    INDEX_OUT_OF_RANGE_MSG,
                );
            }
            ctx.push_i32(v.layout.length as i32 - 1);
            StepResult::Continue
        }
        _ => ctx.throw_by_name_with_message("System.ArgumentException", NOT_ARRAY_MSG),
    }
}

#[dotnet_intrinsic("object System.Array::GetValue(int)")]
#[dotnet_intrinsic("object System.Array::GetValue(long)")]
#[dotnet_intrinsic("object System.Array::GetValue(int[])")]
pub fn intrinsic_array_get_value<'gc, T: EvalStackOps<'gc> + ExceptionOps<'gc> + MemoryOps<'gc>>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    // Handles GetValue(int), GetValue(long), and GetValue(params int[]) overloads.
    let arg = ctx.pop();
    let this_val = ctx.pop();
    let StackValue::ObjectRef(ObjectRef(Some(handle))) = this_val else {
        return ctx.throw_by_name_with_message("System.NullReferenceException", NULL_REF_MSG);
    };

    let index = match arg {
        StackValue::Int32(i) => i as usize,
        StackValue::Int64(i) => i as usize,
        StackValue::ObjectRef(ObjectRef(Some(indices_handle))) => {
            let heap = indices_handle.borrow();
            let HeapStorage::Vec(v) = &heap.storage else {
                return ctx
                    .throw_by_name_with_message("System.ArgumentException", INDEX_ARRAY_TYPE_MSG);
            };
            match extract_first_index_from_indices_array(ctx, v) {
                Ok(index) => index,
                Err(step) => return step,
            }
        }
        _ => return ctx.throw_by_name_with_message("System.ArgumentException", INDEX_ARG_TYPE_MSG),
    };

    let heap = handle.borrow();
    let HeapStorage::Vec(v) = &heap.storage else {
        return ctx.throw_by_name_with_message("System.ArgumentException", NOT_ARRAY_MSG);
    };

    if index >= v.layout.length {
        return ctx
            .throw_by_name_with_message("System.IndexOutOfRangeException", INDEX_OUT_OF_RANGE_MSG);
    }

    let elem_size = v.layout.element_layout.size();
    let start = (elem_size * index).as_usize();
    let end = (elem_size * (index + 1)).as_usize();
    let val =
        dotnet_vm_ops::vm_try!(ctx.read_cts_value(&v.element, &v.get()[start..end])).into_stack();

    ctx.push(val);
    StepResult::Continue
}

#[dotnet_intrinsic("void System.Array::SetValue(object, int)")]
#[dotnet_intrinsic("void System.Array::SetValue(object, long)")]
#[dotnet_intrinsic("void System.Array::SetValue(object, int[])")]
pub fn intrinsic_array_set_value<'gc, T: EvalStackOps<'gc> + ExceptionOps<'gc> + MemoryOps<'gc>>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let index_arg = ctx.pop();
    let value = ctx.pop();
    let this_val = ctx.pop();

    let StackValue::ObjectRef(ObjectRef(Some(handle))) = this_val else {
        return ctx.throw_by_name_with_message("System.NullReferenceException", NULL_REF_MSG);
    };

    let index = match index_arg {
        StackValue::Int32(i) => i as usize,
        StackValue::Int64(i) => i as usize,
        StackValue::ObjectRef(ObjectRef(Some(indices_handle))) => {
            let heap = indices_handle.borrow();
            let HeapStorage::Vec(v) = &heap.storage else {
                return ctx
                    .throw_by_name_with_message("System.ArgumentException", INDEX_ARRAY_TYPE_MSG);
            };
            match extract_first_index_from_indices_array(ctx, v) {
                Ok(index) => index,
                Err(step) => return step,
            }
        }
        _ => return ctx.throw_by_name_with_message("System.ArgumentException", INDEX_ARG_TYPE_MSG),
    };

    {
        let heap = handle.borrow();
        let HeapStorage::Vec(v) = &heap.storage else {
            return ctx.throw_by_name_with_message("System.ArgumentException", NOT_ARRAY_MSG);
        };

        if index >= v.layout.length {
            drop(heap);
            return ctx.throw_by_name_with_message(
                "System.IndexOutOfRangeException",
                INDEX_OUT_OF_RANGE_MSG,
            );
        }
    }
    let mut heap = handle.borrow_mut(&ctx.gc_with_token(&ctx.no_active_borrows_token()));
    let HeapStorage::Vec(v) = &mut heap.storage else {
        return ctx.throw_by_name_with_message("System.ArgumentException", NOT_ARRAY_MSG);
    };

    let elem_size = v.layout.element_layout.size();
    let start = (elem_size * index).as_usize();
    let end = (elem_size * (index + 1)).as_usize();
    dotnet_vm_ops::vm_try!(ctx.new_cts_value(&v.element, value))
        .write(&mut v.get_mut()[start..end]);

    StepResult::Continue
}

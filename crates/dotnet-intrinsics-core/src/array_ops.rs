use dotnet_macros::dotnet_intrinsic;
use dotnet_types::{generics::GenericLookup, members::MethodDescription};
use dotnet_value::{
    StackValue,
    layout::HasLayout,
    object::{HeapStorage, ObjectRef},
};
use dotnet_vm_ops::{
    StepResult,
    ops::{EvalStackOps, ExceptionOps, MemoryOps, TypedStackOps},
};

const NULL_REF_MSG: &str = "Object reference not set to an instance of an object.";
const INDEX_OUT_OF_RANGE_MSG: &str = "Index was outside the bounds of the array.";
const NOT_ARRAY_MSG: &str = "Object must be of type Array.";
const INDEX_ARG_TYPE_MSG: &str = "Index must be Int32, Int64, or Int32[].";
const INDEX_ARRAY_TYPE_MSG: &str = "Indices must be provided as an Int32 array.";

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

#[dotnet_intrinsic("object System.Array::GetValue(int)")]
#[dotnet_intrinsic("object System.Array::GetValue(long)")]
#[dotnet_intrinsic("object System.Array::GetValue(int[])")]
pub fn intrinsic_array_get_value<'gc, T: EvalStackOps<'gc> + ExceptionOps<'gc> + MemoryOps<'gc>>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    // This handles GetValue(int index) or GetValue(params int[] indices) depending on which one is called
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
            if v.layout.length == 0 {
                return ctx.throw_by_name_with_message(
                    "System.ArgumentException",
                    "The number of indices provided does not match the number of dimensions of the array.",
                );
            }
            // For now only support 1D access via the first index in the array
            if v.get().len() < 4 {
                return ctx
                    .throw_by_name_with_message("System.ArgumentException", INDEX_ARRAY_TYPE_MSG);
            }
            let Ok(bytes): Result<[u8; 4], _> = v.get()[0..4].try_into() else {
                return ctx
                    .throw_by_name_with_message("System.ArgumentException", INDEX_ARRAY_TYPE_MSG);
            };
            i32::from_ne_bytes(bytes) as usize
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
    let val = crate::vm_try!(ctx.read_cts_value(&v.element, &v.get()[start..end])).into_stack();

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
            if v.layout.length == 0 {
                return ctx.throw_by_name_with_message(
                    "System.ArgumentException",
                    "The number of indices provided does not match the number of dimensions of the array.",
                );
            }
            if v.get().len() < 4 {
                return ctx
                    .throw_by_name_with_message("System.ArgumentException", INDEX_ARRAY_TYPE_MSG);
            }
            let Ok(bytes): Result<[u8; 4], _> = v.get()[0..4].try_into() else {
                return ctx
                    .throw_by_name_with_message("System.ArgumentException", INDEX_ARRAY_TYPE_MSG);
            };
            i32::from_ne_bytes(bytes) as usize
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
    crate::vm_try!(ctx.new_cts_value(&v.element, value)).write(&mut v.get_mut()[start..end]);

    StepResult::Continue
}

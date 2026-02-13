use crate::{StepResult, resolution::ValueResolution, stack::ops::VesOps};
use dotnet_macros::dotnet_intrinsic;
use dotnet_types::{generics::GenericLookup, members::MethodDescription};
use dotnet_value::{
    StackValue,
    layout::HasLayout,
    object::{HeapStorage, ObjectRef},
};

#[dotnet_intrinsic("int System.Array::get_Length()")]
#[dotnet_intrinsic("int DotnetRs.Array::get_Length()")]
pub fn intrinsic_array_get_length<'gc, 'm: 'gc>(
    ctx: &mut dyn VesOps<'gc, 'm>,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let obj = ctx.pop_obj();
    let ObjectRef(Some(handle)) = obj else {
        return ctx.throw_by_name("System.NullReferenceException");
    };

    let heap = handle.borrow();
    let len = match &heap.storage {
        HeapStorage::Vec(v) => v.layout.length,
        _ => panic!("Not an array"),
    };

    ctx.push_i32(len as i32);
    StepResult::Continue
}

#[dotnet_intrinsic("int System.Array::get_Rank()")]
#[dotnet_intrinsic("int DotnetRs.Array::get_Rank()")]
pub fn intrinsic_array_get_rank<'gc, 'm: 'gc>(
    ctx: &mut dyn VesOps<'gc, 'm>,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let obj = ctx.pop_obj();
    let ObjectRef(Some(handle)) = obj else {
        return ctx.throw_by_name("System.NullReferenceException");
    };

    let heap = handle.borrow();
    let rank = match &heap.storage {
        HeapStorage::Vec(_) => 1,
        _ => panic!("Not an array"),
    };

    ctx.push_i32(rank);
    StepResult::Continue
}

#[dotnet_intrinsic("object System.Array::GetValue(int)")]
#[dotnet_intrinsic("object System.Array::GetValue(long)")]
#[dotnet_intrinsic("object System.Array::GetValue(int[])")]
#[dotnet_intrinsic("object DotnetRs.Array::GetValue(int)")]
#[dotnet_intrinsic("object DotnetRs.Array::GetValue(long)")]
#[dotnet_intrinsic("object DotnetRs.Array::GetValue(int[])")]
pub fn intrinsic_array_get_value<'gc, 'm: 'gc>(
    ctx: &mut dyn VesOps<'gc, 'm>,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    // This handles GetValue(int index) or GetValue(params int[] indices) depending on which one is called
    let arg = ctx.pop();
    let this_val = ctx.pop();
    let StackValue::ObjectRef(ObjectRef(Some(handle))) = this_val else {
        return ctx.throw_by_name("System.NullReferenceException");
    };

    let index = match arg {
        StackValue::Int32(i) => i as usize,
        StackValue::Int64(i) => i as usize,
        StackValue::ObjectRef(ObjectRef(Some(indices_handle))) => {
            let heap = indices_handle.borrow();
            let HeapStorage::Vec(v) = &heap.storage else {
                panic!("Expected int array for indices");
            };
            if v.layout.length == 0 {
                return ctx.throw_by_name("System.ArgumentException");
            }
            // For now only support 1D access via the first index in the array
            let bytes = &v.get()[0..4];
            i32::from_ne_bytes(bytes.try_into().unwrap()) as usize
        }
        _ => panic!("Invalid index type for GetValue: {:?}", arg),
    };

    let heap = handle.borrow();
    let HeapStorage::Vec(v) = &heap.storage else {
        panic!("Not an array");
    };

    if index >= v.layout.length {
        return ctx.throw_by_name("System.IndexOutOfRangeException");
    }

    let elem_size = v.layout.element_layout.size();
    let start = (elem_size * index).as_usize();
    let end = (elem_size * (index + 1)).as_usize();
    let val = vm_try!(ctx.read_cts_value(&v.element, &v.get()[start..end])).into_stack(ctx.gc());

    ctx.push(val);
    StepResult::Continue
}

#[dotnet_intrinsic("void System.Array::SetValue(object, int)")]
#[dotnet_intrinsic("void System.Array::SetValue(object, long)")]
#[dotnet_intrinsic("void System.Array::SetValue(object, int[])")]
#[dotnet_intrinsic("void DotnetRs.Array::SetValue(object, int)")]
#[dotnet_intrinsic("void DotnetRs.Array::SetValue(object, long)")]
#[dotnet_intrinsic("void DotnetRs.Array::SetValue(object, int[])")]
pub fn intrinsic_array_set_value<'gc, 'm: 'gc>(
    ctx: &mut dyn VesOps<'gc, 'm>,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let index_arg = ctx.pop();
    let value = ctx.pop();
    let this_val = ctx.pop();

    let StackValue::ObjectRef(ObjectRef(Some(handle))) = this_val else {
        return ctx.throw_by_name("System.NullReferenceException");
    };

    let index = match index_arg {
        StackValue::Int32(i) => i as usize,
        StackValue::Int64(i) => i as usize,
        StackValue::ObjectRef(ObjectRef(Some(indices_handle))) => {
            let heap = indices_handle.borrow();
            let HeapStorage::Vec(v) = &heap.storage else {
                panic!("Expected int array for indices");
            };
            if v.layout.length == 0 {
                return ctx.throw_by_name("System.ArgumentException");
            }
            let bytes = &v.get()[0..4];
            i32::from_ne_bytes(bytes.try_into().unwrap()) as usize
        }
        _ => panic!("Invalid index type for SetValue: {:?}", index_arg),
    };

    {
        let heap = handle.borrow();
        let HeapStorage::Vec(v) = &heap.storage else {
            panic!("Not an array");
        };

        if index >= v.layout.length {
            drop(heap);
            return ctx.throw_by_name("System.IndexOutOfRangeException");
        }
    }
    let mut heap = handle.borrow_mut(&ctx.gc());
    let HeapStorage::Vec(v) = &mut heap.storage else {
        panic!("Not an array");
    };

    let elem_size = v.layout.element_layout.size();
    let start = (elem_size * index).as_usize();
    let end = (elem_size * (index + 1)).as_usize();
    let res_ctx = ctx.current_context();
    vm_try!(res_ctx.new_cts_value(&v.element, value)).write(&mut v.get_mut()[start..end]);

    StepResult::Continue
}

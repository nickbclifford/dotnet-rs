use crate::{pop_args, resolution::ValueResolution, vm_push, CallStack, StepResult};
use dotnet_macros::dotnet_intrinsic;
use dotnet_types::{generics::GenericLookup, members::MethodDescription};
use dotnet_utils::gc::GCHandle;
use dotnet_value::{
    layout::HasLayout,
    object::{HeapStorage, ObjectRef},
    StackValue,
};

#[dotnet_intrinsic("int System.Array::get_Length()")]
#[dotnet_intrinsic("int DotnetRs.Array::get_Length()")]
pub fn intrinsic_array_get_length<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    pop_args!(stack, gc, [ObjectRef(obj)]);
    let ObjectRef(Some(handle)) = obj else {
        return stack.throw_by_name(gc, "System.NullReferenceException");
    };

    let heap = handle.borrow();
    let len = match &heap.storage {
        HeapStorage::Vec(v) => v.layout.length,
        _ => panic!("Not an array"),
    };

    vm_push!(stack, gc, Int32(len as i32));
    StepResult::InstructionStepped
}

#[dotnet_intrinsic("int System.Array::get_Rank()")]
#[dotnet_intrinsic("int DotnetRs.Array::get_Rank()")]
pub fn intrinsic_array_get_rank<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    pop_args!(stack, gc, [ObjectRef(obj)]);
    let ObjectRef(Some(handle)) = obj else {
        return stack.throw_by_name(gc, "System.NullReferenceException");
    };

    let heap = handle.borrow();
    let rank = match &heap.storage {
        HeapStorage::Vec(_) => 1,
        _ => panic!("Not an array"),
    };

    vm_push!(stack, gc, Int32(rank));
    StepResult::InstructionStepped
}

#[dotnet_intrinsic("object System.Array::GetValue(int)")]
#[dotnet_intrinsic("object System.Array::GetValue(long)")]
#[dotnet_intrinsic("object System.Array::GetValue(int[])")]
#[dotnet_intrinsic("object DotnetRs.Array::GetValue(int)")]
#[dotnet_intrinsic("object DotnetRs.Array::GetValue(long)")]
#[dotnet_intrinsic("object DotnetRs.Array::GetValue(int[])")]
pub fn intrinsic_array_get_value<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    // This handles GetValue(int index) or GetValue(params int[] indices) depending on which one is called
    let arg = stack.pop(gc);
    let this_val = stack.pop(gc);
    let StackValue::ObjectRef(ObjectRef(Some(handle))) = this_val else {
        return stack.throw_by_name(gc, "System.NullReferenceException");
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
                return stack.throw_by_name(gc, "System.ArgumentException");
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
        return stack.throw_by_name(gc, "System.IndexOutOfRangeException");
    }

    let elem_size = v.layout.element_layout.size();
    let start = index * elem_size;
    let end = start + elem_size;
    let ctx = stack.current_context();
    let val = ctx
        .read_cts_value(&v.element, &v.get()[start..end], gc)
        .into_stack(gc);

    vm_push!(stack, gc, val);
    StepResult::InstructionStepped
}

#[dotnet_intrinsic("void System.Array::SetValue(object, int)")]
#[dotnet_intrinsic("void System.Array::SetValue(object, long)")]
#[dotnet_intrinsic("void System.Array::SetValue(object, int[])")]
#[dotnet_intrinsic("void DotnetRs.Array::SetValue(object, int)")]
#[dotnet_intrinsic("void DotnetRs.Array::SetValue(object, long)")]
#[dotnet_intrinsic("void DotnetRs.Array::SetValue(object, int[])")]
pub fn intrinsic_array_set_value<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let index_arg = stack.pop(gc);
    let value = stack.pop(gc);
    let this_val = stack.pop(gc);

    let StackValue::ObjectRef(ObjectRef(Some(handle))) = this_val else {
        return stack.throw_by_name(gc, "System.NullReferenceException");
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
                return stack.throw_by_name(gc, "System.ArgumentException");
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
            return stack.throw_by_name(gc, "System.IndexOutOfRangeException");
        }
    }
    let mut heap = handle.borrow_mut(gc);
    let HeapStorage::Vec(v) = &mut heap.storage else {
        panic!("Not an array");
    };

    let elem_size = v.layout.element_layout.size();
    let start = index * elem_size;
    let end = start + elem_size;
    let ctx = stack.current_context();
    ctx.new_cts_value(&v.element, value)
        .write(&mut v.get_mut()[start..end]);

    StepResult::InstructionStepped
}

use crate::{StepResult, layout::type_layout, stack::ops::VesOps};
use dotnet_macros::dotnet_instruction;
use dotnet_value::{
    StackValue,
    layout::{HasLayout, LayoutManager, Scalar},
    object::{HeapStorage, ObjectRef},
    pointer::{ManagedPtr, PointerOrigin},
};
use dotnetdll::prelude::*;
use std::ptr::NonNull;

#[dotnet_instruction(LoadElement { param0 })]
pub fn ldelem<'gc, 'm: 'gc, T: VesOps<'gc, 'm> + ?Sized>(
    ctx: &mut T,
    param0: &MethodType,
) -> StepResult {
    let index = match ctx.pop() {
        StackValue::Int32(i) => i as usize,
        StackValue::NativeInt(i) => i as usize,
        _ => return ctx.throw_by_name("System.InvalidProgramException"),
    };
    let val = ctx.pop();
    let StackValue::ObjectRef(obj) = val else {
        return ctx.throw_by_name("System.InvalidProgramException");
    };

    if obj.0.is_none() {
        return ctx.throw_by_name("System.NullReferenceException");
    }

    let res_ctx = ctx.current_context();
    let load_type = vm_try!(res_ctx.make_concrete(param0));
    let layout = vm_try!(type_layout(load_type.clone(), &res_ctx));
    let target_type = vm_try!(ctx.loader().find_concrete_type(load_type));
    let offset = dotnet_utils::ByteOffset(layout.size().as_usize() * index);

    // SAFETY: read_unaligned handles GC-safe reading from the heap with bounds checking.
    let value = match unsafe {
        ctx.read_unaligned(PointerOrigin::Heap(obj), offset, &layout, Some(target_type))
    } {
        Ok(v) => v,
        Err(_) => return ctx.throw_by_name("System.IndexOutOfRangeException"),
    };
    ctx.push(value);
    StepResult::Continue
}

#[dotnet_instruction(LoadElementPrimitive { param0 })]
pub fn ldelem_primitive<'gc, 'm: 'gc, T: VesOps<'gc, 'm> + ?Sized>(
    ctx: &mut T,
    param0: LoadType,
) -> StepResult {
    let index = match ctx.pop() {
        StackValue::Int32(i) => i as usize,
        StackValue::NativeInt(i) => i as usize,
        _ => return ctx.throw_by_name("System.InvalidProgramException"),
    };
    let array = ctx.pop();

    let StackValue::ObjectRef(obj) = array else {
        return ctx.throw_by_name("System.InvalidProgramException");
    };

    if obj.0.is_none() {
        return ctx.throw_by_name("System.NullReferenceException");
    }

    let layout = match param0 {
        LoadType::Int8 => Scalar::Int8,
        LoadType::UInt8 => Scalar::UInt8,
        LoadType::Int16 => Scalar::Int16,
        LoadType::UInt16 => Scalar::UInt16,
        LoadType::Int32 | LoadType::UInt32 => Scalar::Int32,
        LoadType::Int64 => Scalar::Int64,
        LoadType::Float32 => Scalar::Float32,
        LoadType::Float64 => Scalar::Float64,
        LoadType::IntPtr => Scalar::NativeInt,
        LoadType::Object => Scalar::ObjectRef,
    };
    let layout = LayoutManager::Scalar(layout);
    let offset = dotnet_utils::ByteOffset(layout.size().as_usize() * index);

    // SAFETY: read_unaligned handles GC-safe reading from the heap with bounds checking.
    let value = match unsafe { ctx.read_unaligned(PointerOrigin::Heap(obj), offset, &layout, None) }
    {
        Ok(v) => v,
        Err(_) => return ctx.throw_by_name("System.IndexOutOfRangeException"),
    };
    ctx.push(value);
    StepResult::Continue
}

#[dotnet_instruction(LoadElementAddress { param0 })]
pub fn ldelema<'gc, 'm: 'gc, T: VesOps<'gc, 'm> + ?Sized>(
    ctx: &mut T,

    param0: &MethodType,
) -> StepResult {
    ldelema_internal(ctx, param0, false)
}

#[dotnet_instruction(LoadElementAddressReadonly(param0))]
pub fn ldelema_readonly<'gc, 'm: 'gc, T: VesOps<'gc, 'm> + ?Sized>(
    ctx: &mut T,
    param0: &MethodType,
) -> StepResult {
    ldelema_internal(ctx, param0, true)
}

fn ldelema_internal<'gc, 'm: 'gc, T: VesOps<'gc, 'm> + ?Sized>(
    ctx: &mut T,
    param0: &MethodType,
    readonly: bool,
) -> StepResult {
    let index = match ctx.pop() {
        StackValue::Int32(i) => i as usize,
        StackValue::NativeInt(i) => i as usize,
        _ => return ctx.throw_by_name("System.InvalidProgramException"),
    };
    let array = ctx.pop();
    if array.is_null() {
        return ctx.throw_by_name("System.NullReferenceException");
    }
    let StackValue::ObjectRef(obj) = array else {
        return ctx.throw_by_name("System.InvalidProgramException");
    };

    let res_ctx = ctx.current_context();
    let concrete_t = vm_try!(res_ctx.make_concrete(param0));
    let element_layout = vm_try!(type_layout(concrete_t.clone(), &res_ctx));

    let element_offset = (element_layout.size() * index).as_usize();
    let result = obj.as_vector(|v| {
        if index >= v.layout.length {
            return Err(());
        }
        let ptr = unsafe { v.raw_data_ptr().add(element_offset) };
        Ok(ptr)
    });

    let ptr = match result {
        Ok(p) => p,
        Err(_) => return ctx.throw_by_name("System.IndexOutOfRangeException"),
    };

    let target_type: dotnet_types::TypeDescription = ctx
        .loader()
        .find_concrete_type(concrete_t)
        .expect("Array element type must exist for ldelema");
    ctx.push(StackValue::ManagedPtr(ManagedPtr::new(
        NonNull::new(ptr),
        target_type,
        Some(obj),
        readonly,
        Some(dotnet_value::ByteOffset(element_offset)),
    )));

    StepResult::Continue
}

#[dotnet_instruction(StoreElement { param0 })]
pub fn stelem<'gc, 'm: 'gc, T: VesOps<'gc, 'm> + ?Sized>(
    ctx: &mut T,
    param0: &MethodType,
) -> StepResult {
    let value = ctx.pop();
    let index = match ctx.pop() {
        StackValue::Int32(i) => i as usize,
        StackValue::NativeInt(i) => i as usize,
        _ => return ctx.throw_by_name("System.InvalidProgramException"),
    };
    let array = ctx.pop();
    if array.is_null() {
        return ctx.throw_by_name("System.NullReferenceException");
    }
    let StackValue::ObjectRef(obj) = array else {
        return ctx.throw_by_name("System.InvalidProgramException");
    };

    let res_ctx = ctx.current_context();
    let store_type = vm_try!(res_ctx.make_concrete(param0));
    let layout = vm_try!(type_layout(store_type, &res_ctx));
    let offset = dotnet_utils::ByteOffset(layout.size().as_usize() * index);

    // SAFETY: write_unaligned handles GC-safe writing to the heap with bounds checking and write barriers.
    match unsafe { ctx.write_unaligned(PointerOrigin::Heap(obj), offset, value, &layout) } {
        Ok(_) => StepResult::Continue,
        Err(_) => ctx.throw_by_name("System.IndexOutOfRangeException"),
    }
}

#[dotnet_instruction(StoreElementPrimitive { param0 })]
pub fn stelem_primitive<'gc, 'm: 'gc, T: VesOps<'gc, 'm> + ?Sized>(
    ctx: &mut T,
    param0: StoreType,
) -> StepResult {
    let value = ctx.pop();
    let index = match ctx.pop() {
        StackValue::Int32(i) => i as usize,
        StackValue::NativeInt(i) => i as usize,
        _ => return ctx.throw_by_name("System.InvalidProgramException"),
    };
    let array = ctx.pop();
    if array.is_null() {
        return ctx.throw_by_name("System.NullReferenceException");
    }
    let StackValue::ObjectRef(obj) = array else {
        return ctx.throw_by_name("System.InvalidProgramException");
    };

    let layout = match param0 {
        StoreType::Int8 => Scalar::Int8,
        StoreType::Int16 => Scalar::Int16,
        StoreType::Int32 => Scalar::Int32,
        StoreType::Int64 => Scalar::Int64,
        StoreType::Float32 => Scalar::Float32,
        StoreType::Float64 => Scalar::Float64,
        StoreType::IntPtr => Scalar::NativeInt,
        StoreType::Object => Scalar::ObjectRef,
    };
    let layout = LayoutManager::Scalar(layout);
    let offset = dotnet_utils::ByteOffset(layout.size().as_usize() * index);

    // SAFETY: write_unaligned handles GC-safe writing to the heap with bounds checking and write barriers.
    match unsafe { ctx.write_unaligned(PointerOrigin::Heap(obj), offset, value, &layout) } {
        Ok(_) => StepResult::Continue,
        Err(_) => ctx.throw_by_name("System.IndexOutOfRangeException"),
    }
}

#[dotnet_instruction(NewArray(param0))]
pub fn newarr<'gc, 'm: 'gc, T: VesOps<'gc, 'm> + ?Sized>(
    ctx: &mut T,
    param0: &MethodType,
) -> StepResult {
    // Check for GC safe point before large allocations
    // Threshold: arrays with > 1024 elements
    const LARGE_ARRAY_THRESHOLD: usize = 1024;

    let length = match ctx.pop() {
        StackValue::Int32(i) => {
            if i < 0 {
                return ctx.throw_by_name("System.OverflowException");
            }
            i as usize
        }
        StackValue::NativeInt(i) => {
            if i < 0 {
                return ctx.throw_by_name("System.OverflowException");
            }
            i as usize
        }
        _ => return ctx.throw_by_name("System.InvalidProgramException"),
    };

    if length > LARGE_ARRAY_THRESHOLD {
        // ctx.check_gc_safe_point();
    }

    let res_ctx = ctx.current_context();
    let elem_type = vm_try!(res_ctx.normalize_type(vm_try!(res_ctx.make_concrete(param0))));

    let v = vm_try!(ctx.new_vector(elem_type, length));
    let o = ObjectRef::new(ctx.gc(), HeapStorage::Vec(v));
    ctx.register_new_object(&o);
    ctx.push(StackValue::ObjectRef(o));
    StepResult::Continue
}

#[dotnet_instruction(LoadLength)]
pub fn ldlen<'gc, 'm: 'gc, T: VesOps<'gc, 'm> + ?Sized>(ctx: &mut T) -> StepResult {
    let array = ctx.pop();
    if array.is_null() {
        return ctx.throw_by_name("System.NullReferenceException");
    }
    let StackValue::ObjectRef(obj) = array else {
        return ctx.throw_by_name("System.InvalidProgramException");
    };

    let h = obj.0.expect("ObjectRef cannot be null after is_null check");

    let inner = h.borrow();
    let len = match &inner.storage {
        HeapStorage::Vec(v) => v.layout.length as isize,
        HeapStorage::Str(s) => s.len() as isize,
        HeapStorage::Obj(_) => {
            return ctx.throw_by_name("System.InvalidCastException");
        }
        HeapStorage::Boxed(_) => {
            return ctx.throw_by_name("System.InvalidCastException");
        }
    };
    ctx.push(StackValue::NativeInt(len));
    StepResult::Continue
}

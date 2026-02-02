use crate::{layout::type_layout, resolution::ValueResolution, CallStack, StepResult};
use dotnet_macros::dotnet_instruction;
use dotnet_utils::gc::GCHandle;
use dotnet_value::{
    layout::HasLayout,
    object::{HeapStorage, ObjectRef},
    pointer::ManagedPtr,
    StackValue,
};
use dotnetdll::prelude::*;
use std::ptr::NonNull;

#[dotnet_instruction(LoadElement)]
pub fn ldelem<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    param0: &MethodType,
) -> StepResult {
    let index = match stack.pop(gc) {
        StackValue::Int32(i) => i as usize,
        StackValue::NativeInt(i) => i as usize,
        rest => panic!("invalid index for ldelem: {:?}", rest),
    };
    let val = stack.pop(gc);
    let StackValue::ObjectRef(obj) = val else {
        panic!("ldelem: expected object on stack, got {:?}", val);
    };

    if obj.0.is_none() {
        return stack.throw_by_name(gc, "System.NullReferenceException");
    }

    let ctx = stack.current_context();
    let load_type = ctx.make_concrete(param0);
    let value = obj.as_vector(|array| {
        if index >= array.layout.length {
            return Err(());
        }
        let elem_size = array.layout.element_layout.size();
        let target = &array.get()[(elem_size * index)..(elem_size * (index + 1))];
        Ok(ctx.read_cts_value(&load_type, target, gc).into_stack(gc))
    });
    match value {
        Ok(v) => stack.push(gc, v),
        Err(_) => return stack.throw_by_name(gc, "System.IndexOutOfRangeException"),
    }
    StepResult::Continue
}

#[dotnet_instruction(LoadElementPrimitive)]
pub fn ldelem_primitive<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    param0: LoadType,
) -> StepResult {
    let index = match stack.pop(gc) {
        StackValue::Int32(i) => i as usize,
        StackValue::NativeInt(i) => i as usize,
        rest => panic!(
            "invalid index for ldelem (expected int32 or native int, received {:?})",
            rest
        ),
    };
    let array = stack.pop(gc);

    let StackValue::ObjectRef(obj) = array else {
        panic!("ldelem: expected object on stack, got {:?}", array);
    };

    if obj.0.is_none() {
        return stack.throw_by_name(gc, "System.NullReferenceException");
    }

    let elem_size: usize = match param0 {
        LoadType::Int8 | LoadType::UInt8 => 1,
        LoadType::Int16 | LoadType::UInt16 => 2,
        LoadType::Int32 | LoadType::UInt32 => 4,
        LoadType::Int64 => 8,
        LoadType::Float32 => 4,
        LoadType::Float64 => 8,
        LoadType::IntPtr | LoadType::Object => ObjectRef::SIZE,
    };

    let value = obj.as_vector(|array| {
        if index >= array.layout.length {
            return Err(());
        }
        let target = &array.get()[(elem_size * index)..(elem_size * (index + 1))];

        macro_rules! from_bytes {
            ($t:ty) => {
                <$t>::from_ne_bytes(target.try_into().expect("source data was too small"))
            };
        }

        Ok(match param0 {
            LoadType::Int8 => StackValue::Int32(from_bytes!(i8) as i32),
            LoadType::UInt8 => StackValue::Int32(from_bytes!(u8) as i32),
            LoadType::Int16 => StackValue::Int32(from_bytes!(i16) as i32),
            LoadType::UInt16 => StackValue::Int32(from_bytes!(u16) as i32),
            LoadType::Int32 => StackValue::Int32(from_bytes!(i32)),
            LoadType::UInt32 => StackValue::Int32(from_bytes!(u32) as i32),
            LoadType::Int64 => StackValue::Int64(from_bytes!(i64)),
            LoadType::Float32 => StackValue::NativeFloat(from_bytes!(f32) as f64),
            LoadType::Float64 => StackValue::NativeFloat(from_bytes!(f64)),
            LoadType::IntPtr => StackValue::NativeInt(from_bytes!(isize)),
            LoadType::Object => {
                StackValue::ObjectRef(unsafe { ObjectRef::read_branded(target, gc) })
            }
        })
    });
    match value {
        Ok(v) => stack.push(gc, v),
        Err(_) => return stack.throw_by_name(gc, "System.IndexOutOfRangeException"),
    }
    StepResult::Continue
}

#[dotnet_instruction(LoadElementAddress)]
pub fn ldelema<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    param0: &MethodType,
) -> StepResult {
    ldelema_internal(gc, stack, param0, false)
}

#[dotnet_instruction(LoadElementAddressReadonly)]
pub fn ldelema_readonly<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    param0: &MethodType,
) -> StepResult {
    ldelema_internal(gc, stack, param0, true)
}

fn ldelema_internal<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    param0: &MethodType,
    readonly: bool,
) -> StepResult {
    let index = match stack.pop(gc) {
        StackValue::Int32(i) => i as usize,
        StackValue::NativeInt(i) => i as usize,
        rest => panic!("invalid index for ldelema: {:?}", rest),
    };
    let array = stack.pop(gc);
    let StackValue::ObjectRef(obj) = array else {
        panic!("ldelema: expected object on stack, got {:?}", array);
    };

    if obj.0.is_none() {
        return stack.throw_by_name(gc, "System.NullReferenceException");
    }

    let ctx = stack.current_context();
    let concrete_t = ctx.make_concrete(param0);
    let element_layout = type_layout(concrete_t.clone(), &ctx);

    let value = obj.as_vector(|v| {
        if index >= v.layout.length {
            return Err(());
        }
        let ptr = unsafe { v.get().as_ptr().add(index * element_layout.size()) as *mut u8 };
        Ok(ptr)
    });

    let ptr = match value {
        Ok(p) => p,
        Err(_) => return stack.throw_by_name(gc, "System.IndexOutOfRangeException"),
    };

    let target_type = stack.loader().find_concrete_type(concrete_t);
    stack.push(
        gc,
        StackValue::ManagedPtr(ManagedPtr::new(
            NonNull::new(ptr),
            target_type,
            Some(obj),
            readonly,
        )),
    );

    StepResult::Continue
}

#[dotnet_instruction(StoreElement)]
pub fn stelem<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    param0: &MethodType,
) -> StepResult {
    let value = stack.pop(gc);
    let index = match stack.pop(gc) {
        StackValue::Int32(i) => i as usize,
        StackValue::NativeInt(i) => i as usize,
        rest => panic!("invalid index for stelem: {:?}", rest),
    };
    let array = stack.pop(gc);
    let StackValue::ObjectRef(obj) = array else {
        panic!("stelem: expected object on stack, got {:?}", array);
    };

    if obj.0.is_none() {
        return stack.throw_by_name(gc, "System.NullReferenceException");
    }

    let ctx = stack.current_context();
    let store_type = ctx.make_concrete(param0);
    let result = obj.as_vector_mut(gc, |array| {
        if index >= array.layout.length {
            return Err(());
        }
        let elem_size = array.layout.element_layout.size();
        let target = &mut array.get_mut()[(elem_size * index)..(elem_size * (index + 1))];
        ctx.new_cts_value(&store_type, value).write(target);
        Ok(())
    });
    match result {
        Ok(_) => StepResult::Continue,
        Err(_) => stack.throw_by_name(gc, "System.IndexOutOfRangeException"),
    }
}

#[dotnet_instruction(StoreElementPrimitive)]
pub fn stelem_primitive<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    param0: StoreType,
) -> StepResult {
    let value = stack.pop(gc);
    let index = match stack.pop(gc) {
        StackValue::Int32(i) => i as usize,
        StackValue::NativeInt(i) => i as usize,
        rest => panic!("invalid index for stelem: {:?}", rest),
    };
    let array = stack.pop(gc);
    let StackValue::ObjectRef(obj) = array else {
        panic!("stelem: expected object on stack, got {:?}", array);
    };

    if obj.0.is_none() {
        return stack.throw_by_name(gc, "System.NullReferenceException");
    }

    let elem_size: usize = match param0 {
        StoreType::Int8 => 1,
        StoreType::Int16 => 2,
        StoreType::Int32 => 4,
        StoreType::Int64 => 8,
        StoreType::Float32 => 4,
        StoreType::Float64 => 8,
        StoreType::IntPtr | StoreType::Object => ObjectRef::SIZE,
    };

    let result = obj.as_vector_mut(gc, |array| {
        if index >= array.layout.length {
            return Err(());
        }
        let target = &mut array.get_mut()[(elem_size * index)..(elem_size * (index + 1))];

        macro_rules! to_bytes {
            ($t:ty, $v:expr) => {{
                let val = $v as $t;
                target.copy_from_slice(&val.to_ne_bytes());
            }};
        }

        match param0 {
            StoreType::Int8 => to_bytes!(
                i8,
                match value {
                    StackValue::Int32(i) => i,
                    _ => panic!("Invalid value for i8 store"),
                }
            ),
            StoreType::Int16 => to_bytes!(
                i16,
                match value {
                    StackValue::Int32(i) => i,
                    _ => panic!("Invalid value for i16 store"),
                }
            ),
            StoreType::Int32 => to_bytes!(
                i32,
                match value {
                    StackValue::Int32(i) => i,
                    _ => panic!("Invalid value for i32 store"),
                }
            ),
            StoreType::Int64 => to_bytes!(
                i64,
                match value {
                    StackValue::Int64(i) => i,
                    _ => panic!("Invalid value for i64 store"),
                }
            ),
            StoreType::Float32 => to_bytes!(
                f32,
                match value {
                    StackValue::NativeFloat(f) => f,
                    _ => panic!("Invalid value for f32 store"),
                }
            ),
            StoreType::Float64 => to_bytes!(
                f64,
                match value {
                    StackValue::NativeFloat(f) => f,
                    _ => panic!("Invalid value for f64 store"),
                }
            ),
            StoreType::IntPtr => to_bytes!(
                usize,
                match value {
                    StackValue::NativeInt(i) => i as usize,
                    _ => panic!("Invalid value for IntPtr store"),
                }
            ),
            StoreType::Object => {
                let StackValue::ObjectRef(r) = value else {
                    panic!("Invalid value for object store")
                };
                r.write(target);
            }
        }
        Ok(())
    });
    match result {
        Ok(_) => StepResult::Continue,
        Err(_) => stack.throw_by_name(gc, "System.IndexOutOfRangeException"),
    }
}

#[dotnet_instruction(NewArray)]
pub fn newarr<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    param0: &MethodType,
) -> StepResult {
    // Check for GC safe point before large allocations
    // Threshold: arrays with > 1024 elements
    const LARGE_ARRAY_THRESHOLD: usize = 1024;

    let length = match stack.pop(gc) {
        StackValue::Int32(i) => {
            if i < 0 {
                return stack.throw_by_name(gc, "System.OverflowException");
            }
            i as usize
        }
        StackValue::NativeInt(i) => {
            if i < 0 {
                return stack.throw_by_name(gc, "System.OverflowException");
            }
            i as usize
        }
        rest => panic!("invalid length for newarr: {:?}", rest),
    };

    if length > LARGE_ARRAY_THRESHOLD {
        stack.check_gc_safe_point();
    }

    let ctx = stack.current_context();
    let elem_type = ctx.normalize_type(ctx.make_concrete(param0));

    let v = ctx.new_vector(elem_type, length);
    let o = ObjectRef::new(gc, HeapStorage::Vec(v));
    stack.register_new_object(&o);
    stack.push(gc, StackValue::ObjectRef(o));
    StepResult::Continue
}

#[dotnet_instruction(LoadLength)]
pub fn ldlen<'gc, 'm: 'gc>(gc: GCHandle<'gc>, stack: &mut CallStack<'gc, 'm>) -> StepResult {
    let array = stack.pop(gc);
    let StackValue::ObjectRef(obj) = array else {
        panic!("ldlen: expected object on stack, got {:?}", array);
    };

    let Some(h) = obj.0 else {
        return stack.throw_by_name(gc, "System.NullReferenceException");
    };

    let inner = h.borrow();
    let len = match &inner.storage {
        HeapStorage::Vec(v) => v.layout.length as isize,
        HeapStorage::Str(s) => s.len() as isize,
        HeapStorage::Obj(o) => {
            panic!("ldlen called on Obj: {:?}", o.description.type_name());
        }
        HeapStorage::Boxed(b) => {
            panic!("ldlen called on Boxed value (expected Vec or Str): {:?}", b);
        }
    };
    stack.push(gc, StackValue::NativeInt(len));
    StepResult::Continue
}

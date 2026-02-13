use crate::{StepResult, layout::type_layout, stack::ops::VesOps};
use dotnet_macros::dotnet_instruction;
use dotnet_value::{
    StackValue,
    layout::HasLayout,
    object::{HeapStorage, ObjectRef},
    pointer::ManagedPtr,
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
        rest => panic!("invalid index for ldelem: {:?}", rest),
    };
    let val = ctx.pop();
    let StackValue::ObjectRef(obj) = val else {
        panic!("ldelem: expected object on stack, got {:?}", val);
    };

    if obj.0.is_none() {
        return ctx.throw_by_name("System.NullReferenceException");
    }

    let res_ctx = ctx.current_context();
    let load_type = vm_try!(res_ctx.make_concrete(param0));
    let value = obj.as_vector(|array| {
        if index >= array.layout.length {
            return Err(());
        }
        let elem_size = array.layout.element_layout.size();
        let target = &array.get()[(elem_size.as_usize() * index)..(elem_size.as_usize() * (index + 1))];
        ctx.read_cts_value(&load_type, target)
            .map(|v| v.into_stack(ctx.gc()))
            .map_err(|_| ())
    });
    match value {
        Ok(v) => ctx.push(v),
        Err(_) => return ctx.throw_by_name("System.IndexOutOfRangeException"),
    }
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
        rest => panic!(
            "invalid index for ldelem (expected int32 or native int, received {:?})",
            rest
        ),
    };
    let array = ctx.pop();

    let StackValue::ObjectRef(obj) = array else {
        panic!("ldelem: expected object on stack, got {:?}", array);
    };

    if obj.0.is_none() {
        return ctx.throw_by_name("System.NullReferenceException");
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
                StackValue::ObjectRef(unsafe { ObjectRef::read_branded(target, &ctx.gc()) })
            }
        })
    });
    match value {
        Ok(v) => ctx.push(v),
        Err(_) => return ctx.throw_by_name("System.IndexOutOfRangeException"),
    }
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
        rest => panic!("invalid index for ldelema: {:?}", rest),
    };
    let array = ctx.pop();
    let StackValue::ObjectRef(obj) = array else {
        panic!("ldelema: expected object on stack, got {:?}", array);
    };

    if obj.0.is_none() {
        return ctx.throw_by_name("System.NullReferenceException");
    }

    let res_ctx = ctx.current_context();
    let concrete_t = vm_try!(res_ctx.make_concrete(param0));
    let element_layout = vm_try!(type_layout(concrete_t.clone(), &res_ctx));

    let value = obj.as_vector(|v| {
        if index >= v.layout.length {
            return Err(());
        }
        let ptr = unsafe { v.get().as_ptr().add((element_layout.size() * index).as_usize()) as *mut u8 };
        Ok(ptr)
    });

    let ptr = match value {
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
        rest => panic!("invalid index for stelem: {:?}", rest),
    };
    let array = ctx.pop();
    let StackValue::ObjectRef(obj) = array else {
        panic!("stelem: expected object on stack, got {:?}", array);
    };

    if obj.0.is_none() {
        return ctx.throw_by_name("System.NullReferenceException");
    }

    let res_ctx = ctx.current_context();
    let store_type = vm_try!(res_ctx.make_concrete(param0));
    let result = obj.as_vector_mut(ctx.gc(), |array| {
        if index >= array.layout.length {
            return Err(());
        }
        let elem_size = array.layout.element_layout.size();
        let target = &mut array.get_mut()[(elem_size.as_usize() * index)..(elem_size.as_usize() * (index + 1))];
        ctx.new_cts_value(&store_type, value)
            .map(|v| v.write(target))
            .map_err(|_| ())
    });
    match result {
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
        rest => panic!("invalid index for stelem: {:?}", rest),
    };
    let array = ctx.pop();
    let StackValue::ObjectRef(obj) = array else {
        panic!("stelem: expected object on stack, got {:?}", array);
    };

    if obj.0.is_none() {
        return ctx.throw_by_name("System.NullReferenceException");
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

    let result = obj.as_vector_mut(ctx.gc(), |array| {
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
        rest => panic!("invalid length for newarr: {:?}", rest),
    };

    if length > LARGE_ARRAY_THRESHOLD {
        ctx.check_gc_safe_point();
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
    let StackValue::ObjectRef(obj) = array else {
        panic!("ldlen: expected object on stack, got {:?}", array);
    };

    let Some(h) = obj.0 else {
        return ctx.throw_by_name("System.NullReferenceException");
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
    ctx.push(StackValue::NativeInt(len));
    StepResult::Continue
}

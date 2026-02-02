use crate::instructions::type_layout;
use crate::instructions::LayoutFactory;
use crate::sync::Ordering as AtomicOrdering;
use crate::{
    instructions::is_intrinsic_cached, instructions::is_intrinsic_field_cached, intrinsics::intrinsic_call, intrinsics::intrinsic_field,
    resolution::{TypeResolutionExt, ValueResolution}, CallStack,
    StepResult,
};
use dotnet_assemblies::decompose_type_source;
use dotnet_macros::dotnet_instruction;
use dotnet_types::members::MethodDescription;
use dotnet_utils::gc::GCHandle;
use dotnet_utils::is_ptr_aligned_to_field;
use dotnet_value::{
    layout::HasLayout,
    object::{HeapStorage, ObjectRef},
    pointer::{ManagedPtr, UnmanagedPtr},
    StackValue,
};
use dotnetdll::prelude::*;
use std::ptr::{self, NonNull};

fn get_ptr<'gc, 'm: 'gc>(stack: &mut CallStack<'gc, 'm>, gc: GCHandle<'gc>, val: StackValue<'gc>) -> Result<(*mut u8, Option<ObjectRef<'gc>>), StepResult> {
    match val {
        StackValue::ObjectRef(o @ ObjectRef(Some(h))) => {
            let inner = h.borrow();
            let ptr = match &inner.storage {
                HeapStorage::Obj(o) => o.instance_storage.get().as_ptr() as *mut u8,
                HeapStorage::Vec(v) => v.get().as_ptr() as *mut u8,
                HeapStorage::Str(s) => s.as_ptr() as *mut u8,
                HeapStorage::Boxed(v) => match v {
                     dotnet_value::object::ValueType::Struct(s) => s.instance_storage.get().as_ptr() as *mut u8,
                     _ => ptr::null_mut(),
                }
            };
            Ok((ptr, Some(o)))
        }
        StackValue::ValueType(o) => {
            let ptr = o.instance_storage.get().as_ptr() as *mut u8;
            Ok((ptr, None))
        }
        StackValue::ManagedPtr(m) => Ok((m.pointer().map(|p| p.as_ptr()).unwrap_or(ptr::null_mut()), m.owner)),
        StackValue::UnmanagedPtr(UnmanagedPtr(p)) => Ok((p.as_ptr(), None)),
        StackValue::ObjectRef(ObjectRef(None)) => Err(stack.throw_by_name(gc, "System.NullReferenceException")),
        _ => panic!("Invalid parent for field/element access: {:?}", val),
    }
}

#[dotnet_instruction(NewObject)]
pub fn new_object<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    ctor: &UserMethod,
) -> StepResult {
    let (mut method, lookup) = stack.find_generic_method(&MethodSource::User(*ctor));

    if method.method.name == "CtorArraySentinel" {
        if let UserMethod::Reference(r) = *ctor {
            let resolution = stack.current_frame().source_resolution;
            let method_ref = &resolution[r];

            if let MethodReferenceParent::Type(t) = &method_ref.parent {
                let concrete = {
                    let ctx = stack.current_context();
                    // Use generics from context to resolve the MethodType
                    ctx.generics.make_concrete(resolution, t.clone())
                };

                if let BaseType::Array(element, shape) = concrete.get() {
                    let rank = shape.rank;
                    let mut dims: Vec<usize> = (0..rank)
                        .map(|_| {
                            let v = stack.pop(gc);
                            match v {
                                StackValue::Int32(i) => i as usize,
                                StackValue::NativeInt(i) => i as usize,
                                _ => panic!("Invalid dimension {:?}", v),
                            }
                        })
                        .collect();
                    dims.reverse();

                    let total_len: usize = dims.iter().product();

                    let elem_type_concrete = element.clone();

                    let ctx = stack.current_context();
                    let elem_type = ctx.normalize_type(elem_type_concrete.clone());

                    let layout = LayoutFactory::create_array_layout(
                        elem_type.clone(),
                        total_len,
                        &ctx,
                    );
                    let total_size_bytes = layout.element_layout.size() * total_len;

                    let vec_obj =
                        dotnet_value::object::Vector::new(elem_type, layout, vec![0; total_size_bytes], dims);
                    let o = ObjectRef::new(gc, HeapStorage::Vec(vec_obj));
                    stack.register_new_object(&o);
                    stack.push(gc, StackValue::ObjectRef(o));
                    return StepResult::InstructionStepped;
                }
            }
        }
    }

    let parent = method.parent;
    if let (None, Some(ts)) = (&method.method.body, &parent.definition().extends) {
        let (ut, _) = decompose_type_source(ts);
        let type_name = ut.type_name(parent.resolution.definition());
        // delegate types are only allowed to have these base types
        if matches!(
            type_name.as_ref(),
            "System.Delegate" | "System.MulticastDelegate"
        ) {
            let base = stack.loader().corlib_type(&type_name);
            method = MethodDescription {
                parent: base,
                method_resolution: base.resolution,
                method: base
                    .definition()
                    .methods
                    .iter()
                    .find(|m| m.name == ".ctor")
                    .unwrap(),
            };
        }
    }

    let method_name = &*method.method.name;
    let parent_name = method.parent.definition().type_name();
    if parent_name == "System.IntPtr"
        && method_name == ".ctor"
        && method.method.signature.parameters.len() == 1
    {
        let val = stack.pop(gc);
        let native_val = match val {
            StackValue::Int32(i) => i as isize,
            StackValue::Int64(i) => i as isize,
            StackValue::NativeInt(i) => i,
            _ => panic!("Invalid argument for IntPtr constructor: {:?}", val),
        };
        stack.push(gc, StackValue::NativeInt(native_val));
        StepResult::InstructionStepped
    } else {
        if is_intrinsic_cached(stack, method) {
            return intrinsic_call(gc, stack, method, &lookup);
        }

        if stack.initialize_static_storage(gc, parent, lookup.clone()) {
            return StepResult::InstructionStepped;
        }

        let new_ctx = stack
            .current_context()
            .for_type_with_generics(parent, &lookup);
        let instance = new_ctx.new_object(parent);

        stack.constructor_frame(
            gc,
            instance,
            crate::MethodInfo::new(method, &lookup, stack.shared.clone()),
            lookup,
        );
        StepResult::InstructionStepped
    }
}

#[dotnet_instruction(LoadField)]
pub fn ldfld<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    param0: &FieldSource,
    volatile: bool,
) -> StepResult {
    let (field, lookup) = stack.locate_field(*param0);

    // Special fields check (intrinsic fields)
    if is_intrinsic_field_cached(stack, field) {
        return intrinsic_field(
            gc,
            stack,
            field,
            stack.current_context().generics.type_generics.clone(),
            false,
        );
    }

    let parent = stack.pop(gc);

    let ctx = stack
        .current_context()
        .for_type_with_generics(field.parent, &lookup);
    let name = &field.field.name;
    let t = ctx.get_field_type(field);

    let ordering = if volatile {
        AtomicOrdering::SeqCst
    } else {
        AtomicOrdering::Acquire
    };

    let read_data = |d: &[u8]| -> dotnet_value::object::CTSValue<'gc> { ctx.read_cts_value(&t, d, gc) };

    if let StackValue::ObjectRef(ObjectRef(Some(h))) = &parent {
        if field.parent.type_name() == "System.Runtime.CompilerServices.RawArrayData" {
            let data = h.borrow();
            if let HeapStorage::Vec(ref vector) = data.storage {
                let intercepted = if name == "Length" {
                    Some(dotnet_value::object::CTSValue::Value(dotnet_value::object::ValueType::UInt32(
                        vector.layout.length as u32,
                    )))
                } else if name == "Data" {
                    let b = if vector.layout.length > 0 {
                        vector.get()[0]
                    } else {
                        0
                    };
                    Some(dotnet_value::object::CTSValue::Value(dotnet_value::object::ValueType::UInt8(b)))
                } else {
                    None
                };

                if let Some(val) = intercepted {
                    stack.push(gc, val.into_stack(gc));
                    return StepResult::InstructionStepped;
                }
            }
        }
    }

    let parent_data = if let StackValue::ObjectRef(ObjectRef(Some(h))) = &parent {
        Some(h.borrow())
    } else {
        None
    };

    let ptr = match &parent {
        StackValue::ObjectRef(ObjectRef(Some(_))) => {
            let inner = parent_data.as_ref().unwrap();
            match &inner.storage {
                HeapStorage::Obj(o) => o.instance_storage.get().as_ptr() as *mut u8,
                HeapStorage::Vec(v) => v.get().as_ptr() as *mut u8,
                HeapStorage::Str(s) => s.as_ptr() as *mut u8,
                HeapStorage::Boxed(v) => match v {
                     dotnet_value::object::ValueType::Struct(s) => s.instance_storage.get().as_ptr() as *mut u8,
                     _ => ptr::null_mut(),
                }
            }
        }
        StackValue::ValueType(v) => v.instance_storage.get().as_ptr() as *mut u8,
        StackValue::ManagedPtr(m) => m.pointer().map(|p| p.as_ptr()).unwrap_or(ptr::null_mut()),
        StackValue::UnmanagedPtr(UnmanagedPtr(p)) => p.as_ptr(),
        StackValue::NativeInt(i) => {
            if *i == 0 {
                return stack.throw_by_name(gc, "System.NullReferenceException");
            }
            *i as *mut u8
        }
        StackValue::ObjectRef(ObjectRef(None)) => {
            return stack.throw_by_name(gc, "System.NullReferenceException");
        }
        v => panic!("Invalid parent for ldfld: {:?}", v),
    };

    debug_assert!(!ptr.is_null(), "Attempted to read field from null pointer");
    let layout = LayoutFactory::instance_field_layout_cached(
        field.parent,
        &ctx,
        Some(&stack.shared.metrics),
    );
    let field_layout = layout.get_field(field.parent, name.as_ref()).unwrap();

    let size = field_layout.layout.size();
    let field_ptr = unsafe { ptr.add(field_layout.position) };

    let value = if size <= size_of::<usize>() && is_ptr_aligned_to_field(field_ptr, size) {
        match size {
            1 => {
                let val = unsafe { (*(field_ptr as *const crate::sync::AtomicU8)).load(ordering) };
                read_data(&val.to_ne_bytes())
            }
            2 => {
                let val = unsafe { (*(field_ptr as *const crate::sync::AtomicU16)).load(ordering) };
                read_data(&val.to_ne_bytes())
            }
            4 => {
                let val = unsafe { (*(field_ptr as *const crate::sync::AtomicU32)).load(ordering) };
                read_data(&val.to_ne_bytes())
            }
            8 => {
                let val = unsafe { (*(field_ptr as *const crate::sync::AtomicU64)).load(ordering) };
                read_data(&val.to_ne_bytes())
            }
            _ => {
                let mut buf = vec![0u8; size];
                unsafe { ptr::copy_nonoverlapping(field_ptr, buf.as_mut_ptr(), size) };
                read_data(&buf)
            }
        }
    } else {
        let mut buf = vec![0u8; size];
        unsafe { ptr::copy_nonoverlapping(field_ptr, buf.as_mut_ptr(), size) };
        read_data(&buf)
    };

    stack.push(gc, value.into_stack(gc));
    StepResult::InstructionStepped
}

#[dotnet_instruction(StoreField)]
pub fn stfld<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    param0: &FieldSource,
    volatile: bool,
) -> StepResult {
    let (field, lookup) = stack.locate_field(*param0);
    
    let value = stack.pop(gc);
    let parent = stack.pop(gc);

    let ctx = stack
        .current_context()
        .for_type_with_generics(field.parent, &lookup);
    let name = &field.field.name;
    let t = ctx.get_field_type(field);

    let ordering = if volatile {
        AtomicOrdering::SeqCst
    } else {
        AtomicOrdering::Release
    };

    let parent_data = if let StackValue::ObjectRef(ObjectRef(Some(h))) = &parent {
        Some(h.borrow_mut(gc))
    } else {
        None
    };

    let ptr = match &parent {
        StackValue::ObjectRef(ObjectRef(Some(_))) => {
            let inner = parent_data.as_ref().unwrap();
            match &inner.storage {
                HeapStorage::Obj(o) => o.instance_storage.get().as_ptr() as *mut u8,
                HeapStorage::Vec(v) => v.get().as_ptr() as *mut u8,
                HeapStorage::Str(s) => s.as_ptr() as *mut u8,
                HeapStorage::Boxed(v) => match v {
                     dotnet_value::object::ValueType::Struct(s) => s.instance_storage.get().as_ptr() as *mut u8,
                     _ => ptr::null_mut(),
                }
            }
        }
        StackValue::ValueType(v) => v.instance_storage.get().as_ptr() as *mut u8,
        StackValue::ManagedPtr(m) => m.pointer().map(|p| p.as_ptr()).unwrap_or(ptr::null_mut()),
        StackValue::UnmanagedPtr(UnmanagedPtr(p)) => p.as_ptr(),
        StackValue::NativeInt(i) => {
            if *i == 0 {
                return stack.throw_by_name(gc, "System.NullReferenceException");
            }
            *i as *mut u8
        }
        StackValue::ObjectRef(ObjectRef(None)) => {
            return stack.throw_by_name(gc, "System.NullReferenceException");
        }
        v => panic!("Invalid parent for stfld: {:?}", v),
    };

    debug_assert!(!ptr.is_null(), "Attempted to write field to null pointer");
    let layout = LayoutFactory::instance_field_layout_cached(
        field.parent,
        &ctx,
        Some(&stack.shared.metrics),
    );
    let field_layout = layout.get_field(field.parent, name.as_ref()).unwrap();

    let size = field_layout.layout.size();
    let field_ptr = unsafe { ptr.add(field_layout.position) };

    let mut val_bytes = vec![0u8; size];
    ctx.new_cts_value(&t, value).write(&mut val_bytes);

    if size <= std::mem::size_of::<usize>() && is_ptr_aligned_to_field(field_ptr, size) {
        match size {
            1 => unsafe {
                (*(field_ptr as *const crate::sync::AtomicU8))
                    .store(u8::from_ne_bytes(val_bytes.try_into().unwrap()), ordering)
            },
            2 => unsafe {
                (*(field_ptr as *const crate::sync::AtomicU16))
                    .store(u16::from_ne_bytes(val_bytes.try_into().unwrap()), ordering)
            },
            4 => unsafe {
                (*(field_ptr as *const crate::sync::AtomicU32))
                    .store(u32::from_ne_bytes(val_bytes.try_into().unwrap()), ordering)
            },
            8 => unsafe {
                (*(field_ptr as *const crate::sync::AtomicU64))
                    .store(u64::from_ne_bytes(val_bytes.try_into().unwrap()), ordering)
            },
            _ => unsafe {
                ptr::copy_nonoverlapping(val_bytes.as_ptr(), field_ptr, size);
            },
        }
    } else {
        unsafe {
            ptr::copy_nonoverlapping(val_bytes.as_ptr(), field_ptr, size);
        }
    }

    StepResult::InstructionStepped
}


#[dotnet_instruction(LoadStaticField)]
pub fn ldsfld<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    param0: &FieldSource,
    volatile: bool,
) -> StepResult {
    let (field, lookup) = stack.locate_field(*param0);

    // Special fields check (intrinsic fields)
    if is_intrinsic_field_cached(stack, field) {
        return intrinsic_field(
            gc,
            stack,
            field,
            stack.current_context().generics.type_generics.clone(),
            false,
        );
    }

    if stack.initialize_static_storage(gc, field.parent, lookup.clone()) {
        return StepResult::InstructionStepped;
    }

    let ctx = stack
        .current_context()
        .for_type_with_generics(field.parent, &lookup);
    let name = &field.field.name;

    let ordering = if volatile {
        AtomicOrdering::SeqCst
    } else {
        AtomicOrdering::Acquire
    };

    // Thread-safe path: use GlobalState
    let storage = stack.statics().get(field.parent, &lookup);
    let t = ctx.make_concrete(&field.field.return_type);
    let val_bytes = storage
        .storage
        .get_field_atomic(field.parent, name, ordering);
    let value = ctx.read_cts_value(&t, &val_bytes, gc);

    stack.push(gc, value.into_stack(gc));
    StepResult::InstructionStepped
}

#[dotnet_instruction(StoreStaticField)]
pub fn stsfld<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    param0: &FieldSource,
    volatile: bool,
) -> StepResult {
    let (field, lookup) = stack.locate_field(*param0);
    let name = &field.field.name;

    if stack.initialize_static_storage(gc, field.parent, lookup.clone()) {
        return StepResult::InstructionStepped;
    }

    let value = stack.pop(gc);
    let ctx = stack
        .current_context()
        .for_type_with_generics(field.parent, &lookup);

    let ordering = if volatile {
        AtomicOrdering::SeqCst
    } else {
        AtomicOrdering::Release
    };

    // Thread-safe path: use GlobalState
    let storage = stack.statics().get(field.parent, &lookup);
    let t = ctx.make_concrete(&field.field.return_type);

    let layout = storage.layout();
    let field_layout = layout.get_field(field.parent, name.as_ref()).unwrap();
    let mut val_bytes = vec![0u8; field_layout.layout.size()];
    ctx.new_cts_value(&t, value).write(&mut val_bytes);
    storage
        .storage
        .set_field_atomic(field.parent, name, &val_bytes, ordering);

    StepResult::InstructionStepped
}

#[dotnet_instruction(LoadFieldAddress)]
pub fn ldflda<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    param0: &FieldSource,
) -> StepResult {
    let (field, lookup) = stack.locate_field(*param0);

    // Special fields check (intrinsic fields)
    if is_intrinsic_field_cached(stack, field) {
        return intrinsic_field(
            gc,
            stack,
            field,
            stack.current_context().generics.type_generics.clone(),
            true,
        );
    }

    let parent = stack.pop(gc);

    if let StackValue::ObjectRef(ObjectRef(Some(h))) = &parent {
        if field.parent.type_name() == "System.Runtime.CompilerServices.RawData" {
            let data = h.borrow();
            let ptr = match &data.storage {
                HeapStorage::Obj(o) => o.instance_storage.get().as_ptr() as *mut u8,
                HeapStorage::Vec(v) => v.get().as_ptr() as *mut u8,
                HeapStorage::Boxed(b) => match b {
                    dotnet_value::object::ValueType::Struct(s) => {
                        s.instance_storage.get().as_ptr() as *mut u8
                    }
                    _ => ptr::null_mut(),
                },
                HeapStorage::Str(_) => ptr::null_mut(),
            };

            if !ptr.is_null() {
                let target_type = stack.current_context().get_field_desc(field);
                drop(data);
                stack.push(
                    gc,
                    StackValue::ManagedPtr(ManagedPtr::new(
                        NonNull::new(ptr),
                        target_type,
                        Some(ObjectRef(Some(*h))),
                        false,
                    )),
                );
                return StepResult::InstructionStepped;
            }
        }

        if field.parent.type_name() == "System.Runtime.CompilerServices.RawArrayData" {
            let data = h.borrow();
            if let HeapStorage::Vec(ref vector) = data.storage {
                let ptr = if field.field.name == "Data" {
                    vector.get().as_ptr() as *mut u8
                } else if field.field.name == "Length" {
                    (&vector.layout.length as *const usize) as *mut u8
                } else {
                    std::ptr::null_mut()
                };

                if !ptr.is_null() {
                    let target_type = stack.current_context().get_field_desc(field);
                    drop(data);
                    stack.push(
                        gc,
                        StackValue::ManagedPtr(ManagedPtr::new(
                            NonNull::new(ptr),
                            target_type,
                            Some(ObjectRef(Some(*h))),
                            false,
                        )),
                    );
                    return StepResult::InstructionStepped;
                }
            }
        }
    }

    let (ptr, owner) = match get_ptr(stack, gc, parent) {
        Ok(res) => res,
        Err(e) => return e,
    };

    let ctx = stack
        .current_context()
        .for_type_with_generics(field.parent, &lookup);
    let name = &field.field.name;

    let layout = LayoutFactory::instance_field_layout_cached(
        field.parent,
        &ctx,
        Some(&stack.shared.metrics),
    );
    let field_layout = layout.get_field(field.parent, name.as_ref()).unwrap();

    let ptr = if field.parent.type_name() == "System.String" && name == "_firstChar" {
         unsafe { ptr.sub(field_layout.position) }
    } else {
         ptr
    };

    let field_ptr = unsafe { ptr.add(field_layout.position) };
    let t = ctx.get_field_type(field);
    let target_type = stack.loader().find_concrete_type(t);

    stack.push(
        gc,
        StackValue::ManagedPtr(ManagedPtr::new(
            NonNull::new(field_ptr),
            target_type,
            owner,
            false,
        )),
    );

    StepResult::InstructionStepped
}

#[dotnet_instruction(LoadStaticFieldAddress)]
pub fn ldsflda<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    param0: &FieldSource,
) -> StepResult {
    let (field, lookup) = stack.locate_field(*param0);

    if stack.initialize_static_storage(gc, field.parent, lookup.clone()) {
        return StepResult::InstructionStepped;
    }

    let ctx = stack
        .current_context()
        .for_type_with_generics(field.parent, &lookup);
    let name = &field.field.name;

    let storage = stack.statics().get(field.parent, &lookup);
    let ptr = storage.storage.get().as_ptr() as *mut u8;

    let layout = storage.layout();
    let field_layout = layout.get_field(field.parent, name.as_ref()).unwrap();
    let field_ptr = unsafe { ptr.add(field_layout.position) };

    let t = ctx.get_field_type(field);
    let target_type = stack.loader().find_concrete_type(t);

    stack.push(
        gc,
        StackValue::ManagedPtr(ManagedPtr::new(
            NonNull::new(field_ptr),
            target_type,
            None,
            false,
        )),
    );

    StepResult::InstructionStepped
}


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
    StepResult::InstructionStepped
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

    let _ctx = stack.current_context();
    let value = obj.as_vector(|array| {
        if index >= array.layout.length {
            return Err(());
        }
        let target = &array.get()[(elem_size * index)..(elem_size * (index + 1))];
        
        macro_rules! from_bytes {
            ($t:ty) => {
                <$t>::from_ne_bytes(
                    target.try_into().expect("source data was too small"),
                )
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
    StepResult::InstructionStepped
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

    StepResult::InstructionStepped
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
        Ok(_) => StepResult::InstructionStepped,
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
            StoreType::Int8 => to_bytes!(i8, match value { StackValue::Int32(i) => i, _ => panic!("Invalid value for i8 store") }),
            StoreType::Int16 => to_bytes!(i16, match value { StackValue::Int32(i) => i, _ => panic!("Invalid value for i16 store") }),
            StoreType::Int32 => to_bytes!(i32, match value { StackValue::Int32(i) => i, _ => panic!("Invalid value for i32 store") }),
            StoreType::Int64 => to_bytes!(i64, match value { StackValue::Int64(i) => i, _ => panic!("Invalid value for i64 store") }),
            StoreType::Float32 => to_bytes!(f32, match value { StackValue::NativeFloat(f) => f, _ => panic!("Invalid value for f32 store") }),
            StoreType::Float64 => to_bytes!(f64, match value { StackValue::NativeFloat(f) => f, _ => panic!("Invalid value for f64 store") }),
            StoreType::IntPtr => to_bytes!(usize, match value { StackValue::NativeInt(i) => i as usize, _ => panic!("Invalid value for IntPtr store") }),
            StoreType::Object => {
                let StackValue::ObjectRef(r) = value else { panic!("Invalid value for object store") };
                r.write(target);
            }
        }
        Ok(())
    });
    match result {
        Ok(_) => StepResult::InstructionStepped,
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
    StepResult::InstructionStepped
}

#[dotnet_instruction(LoadLength)]
pub fn ldlen<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
) -> StepResult {
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
    StepResult::InstructionStepped
}


#[dotnet_instruction(CastClass)]
pub fn castclass<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    param0: &MethodType,
) -> StepResult {
    let target_obj_val = stack.pop(gc);
    let StackValue::ObjectRef(target_obj) = target_obj_val else {
        panic!("castclass: expected object on stack, got {:?}", target_obj_val);
    };

    if let ObjectRef(Some(o)) = target_obj {
        let ctx = stack.current_context();
        let obj_type = ctx.get_heap_description(o);
        let target_ct = ctx.make_concrete(param0);

        if ctx.is_a(obj_type.into(), target_ct) {
            stack.push(gc, StackValue::ObjectRef(target_obj));
        } else {
            return stack.throw_by_name(gc, "System.InvalidCastException");
        }
    } else {
        // castclass returns null for null (III.4.3)
        stack.push(gc, StackValue::ObjectRef(ObjectRef(None)));
    }
    StepResult::InstructionStepped
}

#[dotnet_instruction(IsInstance)]
pub fn isinst<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    param0: &MethodType,
) -> StepResult {
    let target_obj_val = stack.pop(gc);
    let StackValue::ObjectRef(target_obj) = target_obj_val else {
        panic!("isinst: expected object on stack, got {:?}", target_obj_val);
    };

    if let ObjectRef(Some(o)) = target_obj {
        let ctx = stack.current_context();
        let obj_type = ctx.get_heap_description(o);
        let target_ct = ctx.make_concrete(param0);

        if ctx.is_a(obj_type.into(), target_ct) {
            stack.push(gc, StackValue::ObjectRef(target_obj));
        } else {
            stack.push(gc, StackValue::ObjectRef(ObjectRef(None)));
        }
    } else {
        // isinst returns null for null inputs (III.4.6)
        stack.push(gc, StackValue::ObjectRef(ObjectRef(None)));
    }
    StepResult::InstructionStepped
}

#[dotnet_instruction(BoxValue)]
pub fn box_value<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    param0: &MethodType,
) -> StepResult {
    let t = stack.current_context().make_concrete(param0);
    let value = stack.pop(gc);

    if let StackValue::ObjectRef(_) = value {
        // boxing is a noop for all reference types
        stack.push(gc, value);
    } else {
        let obj = ObjectRef::new(
            gc,
            HeapStorage::Boxed(stack.current_context().new_value_type(&t, value)),
        );
        stack.register_new_object(&obj);
        stack.push(gc, StackValue::ObjectRef(obj));
    }
    StepResult::InstructionStepped
}

#[dotnet_instruction(UnboxIntoValue)]
pub fn unbox_any<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    param0: &MethodType,
) -> StepResult {
    let val = stack.pop(gc);
    let ctx = stack.current_context();
    let target_ct = ctx.make_concrete(param0);

    let is_vt = match target_ct.get() {
        BaseType::Type { .. } => {
            let td = stack.loader().find_concrete_type(target_ct.clone());
            td.is_value_type(&ctx)
        }
        BaseType::Vector(_, _)
        | BaseType::Array(_, _)
        | BaseType::Object
        | BaseType::String => false,
        _ => true, // Primitives, IntPtr, etc are value types
    };

    if is_vt {
        // If it's a value type, unbox it.
        let StackValue::ObjectRef(obj) = val else {
            panic!("unbox.any: expected object on stack, got {:?}", val);
        };
        if obj.0.is_none() {
            // unbox.any on null value type throws NullReferenceException (III.4.33)
            return stack.throw_by_name(gc, "System.NullReferenceException");
        }

        let result = obj.as_heap_storage(|storage| {
            match storage {
                HeapStorage::Boxed(v) => dotnet_value::object::CTSValue::Value(v.clone()).into_stack(gc),
                HeapStorage::Obj(o) => {
                    // Boxed struct is just an Object of that struct type.
                    StackValue::ValueType(o.clone())
                }
                _ => panic!("unbox.any: expected boxed value, got {:?}", storage),
            }
        });
        stack.push(gc, result);
    } else {
        // Reference type: identical to castclass.
        let StackValue::ObjectRef(target_obj) = val else {
            panic!("unbox.any: expected object on stack, got {:?}", val);
        };
        if let ObjectRef(Some(o)) = target_obj {
            let obj_type = ctx.get_heap_description(o);
            if ctx.is_a(obj_type.into(), target_ct) {
                stack.push(gc, StackValue::ObjectRef(target_obj));
            } else {
                return stack.throw_by_name(gc, "System.InvalidCastException");
            }
        } else {
            stack.push(gc, StackValue::ObjectRef(ObjectRef(None)));
        }
    }
    StepResult::InstructionStepped
}

#[dotnet_instruction(UnboxIntoAddress)]
pub fn unbox<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    param0: &MethodType,
) -> StepResult {
    let value = stack.pop(gc);
    let ctx = stack.current_context();
    let target_ct = ctx.make_concrete(param0);

    let StackValue::ObjectRef(obj) = value else {
        panic!("unbox on non-object: {:?}", value);
    };
    let Some(h) = obj.0 else {
        return stack.throw_by_name(gc, "System.NullReferenceException");
    };

    let inner = h.borrow();
    let ptr = match &inner.storage {
        HeapStorage::Boxed(dotnet_value::object::ValueType::Struct(s)) => {
            s.instance_storage.get().as_ptr() as *mut u8
        }
        _ => panic!("unbox on non-boxed struct"),
    };

    let target_type = stack.loader().find_concrete_type(target_ct);
    stack.push(
        gc,
        StackValue::ManagedPtr(ManagedPtr::new(
            NonNull::new(ptr),
            target_type,
            Some(obj),
            false,
        )),
    );

    StepResult::InstructionStepped
}

#[dotnet_instruction(LoadObject)]
pub fn ldobj<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    param0: &MethodType,
) -> StepResult {
    let addr = stack.pop(gc);
    let source_ptr = addr.as_ptr();

    if source_ptr.is_null() {
        return stack.throw_by_name(gc, "System.NullReferenceException");
    }

    let ctx = stack.current_context();
    let load_type = ctx.make_concrete(param0);
    let layout = type_layout(load_type.clone(), &ctx);

    let mut source_vec = vec![0u8; layout.size()];
    unsafe { ptr::copy_nonoverlapping(source_ptr, source_vec.as_mut_ptr(), layout.size()) };
    let value = ctx.read_cts_value(&load_type, &source_vec, gc).into_stack(gc);

    stack.push(gc, value);
    StepResult::InstructionStepped
}

#[dotnet_instruction(StoreObject)]
pub fn stobj<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    param0: &MethodType,
) -> StepResult {
    let value = stack.pop(gc);
    let addr = stack.pop(gc);

    let ctx = stack.current_context();
    let concrete_t = ctx.make_concrete(param0);

    let dest_ptr = match &addr {
        StackValue::NativeInt(i) => {
            if *i == 0 {
                return stack.throw_by_name(gc, "System.NullReferenceException");
            }
            *i as *mut u8
        }
        StackValue::UnmanagedPtr(UnmanagedPtr(p)) => p.as_ptr(),
        StackValue::ManagedPtr(m) => match m.pointer() {
            Some(ptr) => ptr.as_ptr(),
            None => return stack.throw_by_name(gc, "System.NullReferenceException"),
        },
        _ => panic!("stobj: expected pointer on stack, got {:?}", addr),
    };

    let layout = type_layout(concrete_t.clone(), &ctx);
    let mut bytes = vec![0u8; layout.size()];
    ctx.new_cts_value(&concrete_t, value)
        .write(&mut bytes);

    unsafe {
        ptr::copy_nonoverlapping(bytes.as_ptr(), dest_ptr, bytes.len());
    }
    StepResult::InstructionStepped
}

#[dotnet_instruction(InitializeForObject)]
pub fn initobj<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    param0: &MethodType,
) -> StepResult {
    let addr = stack.pop(gc);
    let target = match addr {
        StackValue::NativeInt(i) => {
            if i == 0 {
                return stack.throw_by_name(gc, "System.NullReferenceException");
            }
            i as *mut u8
        }
        StackValue::UnmanagedPtr(UnmanagedPtr(p)) => p.as_ptr(),
        StackValue::ManagedPtr(m) => match m.pointer() {
            Some(ptr) => ptr.as_ptr(),
            None => return stack.throw_by_name(gc, "System.NullReferenceException"),
        },
        _ => panic!("initobj: expected pointer on stack, got {:?}", addr),
    };

    let ctx = stack.current_context();
    let ct = ctx.make_concrete(param0);
    let layout = type_layout(ct.clone(), &ctx);

    unsafe { ptr::write_bytes(target, 0, layout.size()) };
    StepResult::InstructionStepped
}

#[dotnet_instruction(Sizeof)]
pub fn sizeof<'gc, 'm: 'gc>(
    _gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    param0: &MethodType,
) -> StepResult {
    let ctx = stack.current_context();
    let target = ctx.make_concrete(param0);
    let layout = type_layout(target, &ctx);
    stack.push(_gc, StackValue::Int32(layout.size() as i32));
    StepResult::InstructionStepped
}

#[dotnet_instruction(LoadString)]
pub fn ldstr<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    chars: Vec<u16>,
) -> StepResult {
    use dotnet_value::{object::{HeapStorage, ObjectRef}, string::CLRString};
    
    let s = CLRString::new(chars);
    let storage = HeapStorage::Str(s);
    let obj_ref = ObjectRef::new(gc, storage);
    stack.push(gc, StackValue::ObjectRef(obj_ref));
    StepResult::InstructionStepped
}

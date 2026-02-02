use crate::{
    instructions::{is_intrinsic_field_cached, LayoutFactory},
    intrinsics::intrinsic_field,
    resolution::ValueResolution,
    sync::Ordering as AtomicOrdering,
    CallStack, StepResult,
};
use dotnet_macros::dotnet_instruction;
use dotnet_utils::{gc::GCHandle, is_ptr_aligned_to_field};
use dotnet_value::{
    layout::HasLayout,
    object::{HeapStorage, ObjectRef},
    pointer::{ManagedPtr, UnmanagedPtr},
    StackValue,
};
use dotnetdll::prelude::*;
use std::ptr::{self, NonNull};

use super::get_ptr;

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

    let read_data =
        |d: &[u8]| -> dotnet_value::object::CTSValue<'gc> { ctx.read_cts_value(&t, d, gc) };

    if let StackValue::ObjectRef(ObjectRef(Some(h))) = &parent {
        if field.parent.type_name() == "System.Runtime.CompilerServices.RawArrayData" {
            let data = h.borrow();
            if let HeapStorage::Vec(ref vector) = data.storage {
                let intercepted = if name == "Length" {
                    Some(dotnet_value::object::CTSValue::Value(
                        dotnet_value::object::ValueType::UInt32(vector.layout.length as u32),
                    ))
                } else if name == "Data" {
                    let b = if vector.layout.length > 0 {
                        vector.get()[0]
                    } else {
                        0
                    };
                    Some(dotnet_value::object::CTSValue::Value(
                        dotnet_value::object::ValueType::UInt8(b),
                    ))
                } else {
                    None
                };

                if let Some(val) = intercepted {
                    stack.push(gc, val.into_stack(gc));
                    return StepResult::Continue;
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
                    dotnet_value::object::ValueType::Struct(s) => {
                        s.instance_storage.get().as_ptr() as *mut u8
                    }
                    _ => ptr::null_mut(),
                },
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

    let value = if size <= std::mem::size_of::<usize>() && is_ptr_aligned_to_field(field_ptr, size)
    {
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
    StepResult::Continue
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
                    dotnet_value::object::ValueType::Struct(s) => {
                        s.instance_storage.get().as_ptr() as *mut u8
                    }
                    _ => ptr::null_mut(),
                },
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

    StepResult::Continue
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
        return StepResult::FramePushed;
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
    StepResult::Continue
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
        return StepResult::FramePushed;
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

    StepResult::Continue
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
                return StepResult::Continue;
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
                    return StepResult::Continue;
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

    StepResult::Continue
}

#[dotnet_instruction(LoadStaticFieldAddress)]
pub fn ldsflda<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    param0: &FieldSource,
) -> StepResult {
    let (field, lookup) = stack.locate_field(*param0);

    if stack.initialize_static_storage(gc, field.parent, lookup.clone()) {
        return StepResult::FramePushed;
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

    StepResult::Continue
}

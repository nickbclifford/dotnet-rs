use crate::{
    StepResult, instructions::objects::get_ptr_context, layout::LayoutFactory, memory::Atomic,
    resolution::ValueResolution, stack::ops::VesOps, sync::Ordering as AtomicOrdering,
};
use dotnet_macros::dotnet_instruction;
use dotnet_value::{
    StackValue,
    layout::HasLayout,
    object::{HeapStorage, ObjectRef},
    pointer::{ManagedPtr, UnmanagedPtr},
};
use dotnetdll::prelude::*;
use std::ptr::{self, NonNull};

#[dotnet_instruction(LoadField { param0, volatile })]
pub fn ldfld<'gc, 'm: 'gc>(
    ctx: &mut dyn VesOps<'gc, 'm>,
    param0: &FieldSource,
    volatile: bool,
) -> StepResult {
    let (field, lookup) = vm_try!(ctx.locate_field(*param0));

    // Special fields check (intrinsic fields)
    if ctx.is_intrinsic_field_cached(field) {
        let type_generics = ctx.current_context().generics.type_generics.clone();
        return crate::intrinsics::intrinsic_field(ctx, field, type_generics, false);
    }

    let parent = ctx.pop();

    let res_ctx = ctx
        .current_context()
        .for_type_with_generics(field.parent, &lookup);
    let name = &field.field.name;
    let t = vm_try!(res_ctx.get_field_type(field));

    let ordering = if volatile {
        AtomicOrdering::SeqCst
    } else {
        AtomicOrdering::Acquire
    };

    if let StackValue::ObjectRef(ObjectRef(Some(h))) = &parent
        && field.parent.type_name() == "System.Runtime.CompilerServices.RawArrayData"
    {
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
                ctx.push(val.into_stack(ctx.gc()));
                return StepResult::Continue;
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
        StackValue::ManagedPtr(_) => {
            let (ptr, _, _, _) = get_ptr_context(ctx, &parent);
            ptr
        }
        StackValue::UnmanagedPtr(UnmanagedPtr(p)) => p.as_ptr(),
        StackValue::NativeInt(i) => {
            if *i == 0 {
                return ctx.throw_by_name("System.NullReferenceException");
            }
            *i as *mut u8
        }
        StackValue::ObjectRef(ObjectRef(None)) => {
            return ctx.throw_by_name("System.NullReferenceException");
        }
        v => panic!("Invalid parent for ldfld: {:?}", v),
    };

    debug_assert!(!ptr.is_null(), "Attempted to read field from null pointer");
    let layout = vm_try!(LayoutFactory::instance_field_layout_cached(
        field.parent,
        &res_ctx,
        Some(&ctx.shared().metrics),
    ));
    let field_layout = layout.get_field(field.parent, name.as_ref()).unwrap();

    let size = field_layout.layout.size();
    let field_ptr = unsafe { ptr.add(field_layout.position) };

    let val_bytes = unsafe { Atomic::load_field(field_ptr, size, ordering) };
    let value = vm_try!(res_ctx.read_cts_value(&t, &val_bytes, ctx.gc()));

    ctx.push(value.into_stack(ctx.gc()));
    StepResult::Continue
}

#[dotnet_instruction(StoreField { param0, volatile })]
pub fn stfld<'gc, 'm: 'gc>(
    ctx: &mut dyn VesOps<'gc, 'm>,
    param0: &FieldSource,
    volatile: bool,
) -> StepResult {
    let (field, lookup) = vm_try!(ctx.locate_field(*param0));

    let value = ctx.pop();
    let parent = ctx.pop();

    let res_ctx = ctx
        .current_context()
        .for_type_with_generics(field.parent, &lookup);
    let name = &field.field.name;
    let t = vm_try!(res_ctx.get_field_type(field));

    let ordering = if volatile {
        AtomicOrdering::SeqCst
    } else {
        AtomicOrdering::Release
    };

    let parent_data = if let StackValue::ObjectRef(ObjectRef(Some(h))) = &parent {
        Some(h.borrow_mut(&ctx.gc()))
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
        StackValue::ManagedPtr(_) => {
            let (ptr, _, _, _) = get_ptr_context(ctx, &parent);
            ptr
        }
        StackValue::UnmanagedPtr(UnmanagedPtr(p)) => p.as_ptr(),
        StackValue::NativeInt(i) => {
            if *i == 0 {
                return ctx.throw_by_name("System.NullReferenceException");
            }
            *i as *mut u8
        }
        StackValue::ObjectRef(ObjectRef(None)) => {
            return ctx.throw_by_name("System.NullReferenceException");
        }
        v => panic!("Invalid parent for stfld: {:?}", v),
    };

    debug_assert!(!ptr.is_null(), "Attempted to write field to null pointer");
    let layout = vm_try!(LayoutFactory::instance_field_layout_cached(
        field.parent,
        &res_ctx,
        Some(&ctx.shared().metrics),
    ));
    let field_layout = layout.get_field(field.parent, name.as_ref()).unwrap();

    let size = field_layout.layout.size();
    let field_ptr = unsafe { ptr.add(field_layout.position) };

    let mut val_bytes = vec![0u8; size];
    vm_try!(ctx.new_cts_value(&t, value)).write(&mut val_bytes);

    unsafe { Atomic::store_field(field_ptr, &val_bytes, ordering) };

    StepResult::Continue
}

#[dotnet_instruction(LoadStaticField { param0, volatile })]
pub fn ldsfld<'gc, 'm: 'gc>(
    ctx: &mut dyn VesOps<'gc, 'm>,
    param0: &FieldSource,
    volatile: bool,
) -> StepResult {
    let (field, lookup): (_, dotnet_types::generics::GenericLookup) =
        vm_try!(ctx.locate_field(*param0));

    // Special fields check (intrinsic fields)
    if ctx.is_intrinsic_field_cached(field) {
        let type_generics = ctx.current_context().generics.type_generics.clone();
        return crate::intrinsics::intrinsic_field(ctx, field, type_generics, false);
    }

    let res = ctx.initialize_static_storage(field.parent, lookup.clone());
    if res != StepResult::Continue {
        return res;
    }

    let res_ctx = ctx
        .current_context()
        .for_type_with_generics(field.parent, &lookup);
    let name = &field.field.name;

    let ordering = if volatile {
        AtomicOrdering::SeqCst
    } else {
        AtomicOrdering::Acquire
    };

    // Thread-safe path: use GlobalState
    let storage = ctx.statics().get(field.parent, &lookup);
    let t = vm_try!(res_ctx.make_concrete(&field.field.return_type));
    let val_bytes = storage
        .storage
        .get_field_atomic(field.parent, name, ordering);
    let value = vm_try!(res_ctx.read_cts_value(&t, &val_bytes, ctx.gc()));

    ctx.push(value.into_stack(ctx.gc()));
    StepResult::Continue
}

#[dotnet_instruction(StoreStaticField { param0, volatile })]
pub fn stsfld<'gc, 'm: 'gc>(
    ctx: &mut dyn VesOps<'gc, 'm>,
    param0: &FieldSource,
    volatile: bool,
) -> StepResult {
    let (field, lookup): (_, dotnet_types::generics::GenericLookup) =
        vm_try!(ctx.locate_field(*param0));
    let name = &field.field.name;

    let res = ctx.initialize_static_storage(field.parent, lookup.clone());
    if res != StepResult::Continue {
        return res;
    }

    let value = ctx.pop();
    let res_ctx = ctx
        .current_context()
        .for_type_with_generics(field.parent, &lookup);

    let ordering = if volatile {
        AtomicOrdering::SeqCst
    } else {
        AtomicOrdering::Release
    };

    // Thread-safe path: use GlobalState
    let storage = ctx.statics().get(field.parent, &lookup);
    let t = vm_try!(res_ctx.make_concrete(&field.field.return_type));

    let layout = storage.layout();
    let field_layout = layout.get_field(field.parent, name.as_ref()).unwrap();
    let mut val_bytes = vec![0u8; field_layout.layout.size()];
    vm_try!(ctx.new_cts_value(&t, value)).write(&mut val_bytes);
    storage
        .storage
        .set_field_atomic(field.parent, name, &val_bytes, ordering);

    StepResult::Continue
}

#[dotnet_instruction(LoadFieldAddress(param0))]
pub fn ldflda<'gc, 'm: 'gc>(ctx: &mut dyn VesOps<'gc, 'm>, param0: &FieldSource) -> StepResult {
    let (field, lookup) = vm_try!(ctx.locate_field(*param0));

    // Special fields check (intrinsic fields)
    if ctx.is_intrinsic_field_cached(field) {
        let type_generics = ctx.current_context().generics.type_generics.clone();
        return crate::intrinsics::intrinsic_field(ctx, field, type_generics, true);
    }

    let parent = ctx.pop();

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
                let target_type = vm_try!(ctx.current_context().get_field_desc(field));
                drop(data);
                ctx.push(StackValue::ManagedPtr(ManagedPtr::new(
                    NonNull::new(ptr),
                    target_type,
                    Some(ObjectRef(Some(*h))),
                    false,
                )));
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
                    ptr::null_mut()
                };

                if !ptr.is_null() {
                    let target_type = vm_try!(ctx.current_context().get_field_desc(field));
                    drop(data);
                    ctx.push(StackValue::ManagedPtr(ManagedPtr::new(
                        NonNull::new(ptr),
                        target_type,
                        Some(ObjectRef(Some(*h))),
                        false,
                    )));
                    return StepResult::Continue;
                }
            }
        }
    }

    if let StackValue::ObjectRef(ObjectRef(None)) = &parent {
        return ctx.throw_by_name("System.NullReferenceException");
    }
    let (ptr, owner, stack_slot_origin, base_offset) = get_ptr_context(ctx, &parent);

    let res_ctx = ctx
        .current_context()
        .for_type_with_generics(field.parent, &lookup);
    let name = &field.field.name;

    let layout = vm_try!(LayoutFactory::instance_field_layout_cached(
        field.parent,
        &res_ctx,
        Some(&ctx.shared().metrics),
    ));
    let field_layout = layout.get_field(field.parent, name.as_ref()).unwrap();

    let ptr = if field.parent.type_name() == "System.String" && name == "_firstChar" {
        unsafe { ptr.sub(field_layout.position) }
    } else {
        ptr
    };

    let field_ptr = unsafe { ptr.add(field_layout.position) };
    let t = vm_try!(res_ctx.get_field_type(field));
    let target_type = vm_try!(ctx.loader().find_concrete_type(t));

    if let Some((idx, _)) = stack_slot_origin {
        let new_offset = base_offset + field_layout.position;
        ctx.push(StackValue::managed_stack_ptr(
            idx,
            new_offset,
            field_ptr,
            target_type,
            false,
        ));
    } else {
        ctx.push(StackValue::ManagedPtr(ManagedPtr::new(
            NonNull::new(field_ptr),
            target_type,
            owner,
            false,
        )));
    }

    StepResult::Continue
}

#[dotnet_instruction(LoadStaticFieldAddress(param0))]
pub fn ldsflda<'gc, 'm: 'gc>(ctx: &mut dyn VesOps<'gc, 'm>, param0: &FieldSource) -> StepResult {
    let (field, lookup): (_, dotnet_types::generics::GenericLookup) =
        vm_try!(ctx.locate_field(*param0));

    let res = ctx.initialize_static_storage(field.parent, lookup.clone());
    if res != StepResult::Continue {
        return res;
    }

    let res_ctx = ctx
        .current_context()
        .for_type_with_generics(field.parent, &lookup);
    let name = &field.field.name;

    let storage = ctx.statics().get(field.parent, &lookup);
    let ptr = storage.storage.get().as_ptr() as *mut u8;

    let layout = storage.layout();
    let field_layout = layout.get_field(field.parent, name.as_ref()).unwrap();
    let field_ptr = unsafe { ptr.add(field_layout.position) };

    let t = vm_try!(res_ctx.get_field_type(field));
    let target_type = vm_try!(ctx.loader().find_concrete_type(t));

    ctx.push(StackValue::ManagedPtr(ManagedPtr::new(
        NonNull::new(field_ptr),
        target_type,
        None,
        false,
    )));

    StepResult::Continue
}

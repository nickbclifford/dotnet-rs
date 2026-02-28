use crate::{
    StepResult, instructions::objects::get_ptr_context, layout::LayoutFactory,
    resolution::ValueResolution, stack::ops::VesOps, sync::Ordering as AtomicOrdering,
};

const NULL_REF_MSG: &str = "Object reference not set to an instance of an object.";
const ACCESS_VIOLATION_MSG: &str = "Attempted to read or write protected memory.";
use dotnet_macros::dotnet_instruction;
use dotnet_value::{
    StackValue,
    layout::{HasLayout, LayoutManager, Scalar},
    object::{HeapStorage, ObjectRef},
    pointer::ManagedPtr,
};
use dotnetdll::prelude::*;
use std::ptr::{self, NonNull};

#[dotnet_instruction(LoadField { param0, volatile })]
pub fn ldfld<'gc, 'm: 'gc, T: VesOps<'gc, 'm>>(
    ctx: &mut T,
    param0: &FieldSource,
    volatile: bool,
) -> StepResult {
    let (field, lookup) = vm_try!(ctx.locate_field(*param0));

    // Special fields check (intrinsic fields)
    if ctx.is_intrinsic_field_cached(field) {
        let type_generics = ctx.current_context().generics.type_generics.clone();
        return ctx.execute_intrinsic_field(field, type_generics, false);
    }

    let parent = ctx.pop();

    if parent.is_null() {
        return ctx.throw_by_name_with_message("System.NullReferenceException", NULL_REF_MSG);
    }
    let (origin, base_offset) = match crate::instructions::objects::get_ptr_info(ctx, &parent) {
        Ok(v) => v,
        Err(e) => return e,
    };

    let res_ctx = ctx
        .current_context()
        .for_type_with_generics(field.parent, &lookup);
    let name = &field.field.name;
    let t = vm_try!(res_ctx.get_field_type(field));

    let _ordering = if volatile {
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
                ctx.push(val.into_stack());
                return StepResult::Continue;
            }
        }
    }

    let layout = vm_try!(LayoutFactory::instance_field_layout_cached(
        field.parent,
        &res_ctx,
        Some(&ctx.shared().metrics),
    ));
    let field_layout = layout.get_field(field.parent, name.as_ref()).unwrap();

    let offset = base_offset + field_layout.position;
    let target_type = vm_try!(ctx.loader().find_concrete_type(t));

    // SAFETY: read_unaligned handles GC-safe reading from the heap if an owner is provided.
    // It also performs bounds checking.
    let value = match unsafe {
        ctx.read_unaligned(origin, offset, &field_layout.layout, Some(target_type))
    } {
        Ok(v) => v,
        Err(_) => {
            if offset.0 == 0 {
                return ctx
                    .throw_by_name_with_message("System.NullReferenceException", NULL_REF_MSG);
            }
            return ctx.throw_by_name_with_message(
                "System.AccessViolationException",
                ACCESS_VIOLATION_MSG,
            );
        }
    };

    ctx.push(value);
    StepResult::Continue
}

#[dotnet_instruction(StoreField { param0, volatile })]
pub fn stfld<'gc, 'm: 'gc, T: VesOps<'gc, 'm>>(
    ctx: &mut T,
    param0: &FieldSource,
    volatile: bool,
) -> StepResult {
    let (field, lookup) = vm_try!(ctx.locate_field(*param0));

    let value = ctx.pop();
    let parent = ctx.pop();

    if parent.is_null() {
        return ctx.throw_by_name_with_message("System.NullReferenceException", NULL_REF_MSG);
    }
    let (origin, base_offset) = match crate::instructions::objects::get_ptr_info(ctx, &parent) {
        Ok(v) => v,
        Err(e) => return e,
    };

    let res_ctx = ctx
        .current_context()
        .for_type_with_generics(field.parent, &lookup);
    let name = &field.field.name;
    let _t = vm_try!(res_ctx.get_field_type(field));

    let _ordering = if volatile {
        AtomicOrdering::SeqCst
    } else {
        AtomicOrdering::Release
    };

    let layout = vm_try!(LayoutFactory::instance_field_layout_cached(
        field.parent,
        &res_ctx,
        Some(&ctx.shared().metrics),
    ));
    let field_layout = layout.get_field(field.parent, name.as_ref()).unwrap();

    let offset = base_offset + field_layout.position;
    // SAFETY: write_unaligned handles GC-safe writing to the heap if an owner is provided.
    // It also performs bounds checking and write barriers.
    match unsafe { ctx.write_unaligned(origin, offset, value, &field_layout.layout) } {
        Ok(_) => {}
        Err(_) => {
            if offset.0 == 0 {
                return ctx
                    .throw_by_name_with_message("System.NullReferenceException", NULL_REF_MSG);
            }
            return ctx.throw_by_name_with_message(
                "System.AccessViolationException",
                ACCESS_VIOLATION_MSG,
            );
        }
    }

    StepResult::Continue
}

#[dotnet_instruction(LoadStaticField { param0, volatile })]
pub fn ldsfld<'gc, 'm: 'gc, T: VesOps<'gc, 'm>>(
    ctx: &mut T,
    param0: &FieldSource,
    volatile: bool,
) -> StepResult {
    let (field, lookup): (_, dotnet_types::generics::GenericLookup) =
        vm_try!(ctx.locate_field(*param0));

    // Special fields check (intrinsic fields)
    if ctx.is_intrinsic_field_cached(field) {
        let type_generics = ctx.current_context().generics.type_generics.clone();
        return ctx.execute_intrinsic_field(field, type_generics, false);
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
    let value = vm_try!(res_ctx.read_cts_value(
        &t,
        &val_bytes,
        ctx.gc_with_token(&dotnet_utils::NoActiveBorrows::new())
    ));

    ctx.push(value.into_stack());
    StepResult::Continue
}

#[dotnet_instruction(StoreStaticField { param0, volatile })]
pub fn stsfld<'gc, 'm: 'gc, T: VesOps<'gc, 'm>>(
    ctx: &mut T,
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
    let mut val_bytes = vec![0u8; field_layout.layout.size().as_usize()];
    vm_try!(ctx.new_cts_value(&t, value)).write(&mut val_bytes);
    storage
        .storage
        .set_field_atomic(field.parent, name, &val_bytes, ordering);

    StepResult::Continue
}

#[dotnet_instruction(LoadFieldAddress(param0))]
pub fn ldflda<'gc, 'm: 'gc, T: VesOps<'gc, 'm>>(ctx: &mut T, param0: &FieldSource) -> StepResult {
    let (field, lookup) = vm_try!(ctx.locate_field(*param0));

    // Special fields check (intrinsic fields)
    if ctx.is_intrinsic_field_cached(field) {
        let type_generics = ctx.current_context().generics.type_generics.clone();
        return ctx.execute_intrinsic_field(field, type_generics, true);
    }

    let parent = ctx.pop();

    if let StackValue::ObjectRef(ObjectRef(Some(h))) = &parent {
        if field.parent.type_name() == "System.Runtime.CompilerServices.RawData" {
            let data = h.borrow();
            let ptr = match &data.storage {
                HeapStorage::Obj(o) | HeapStorage::Boxed(o) => unsafe {
                    o.instance_storage.raw_data_ptr()
                },
                HeapStorage::Vec(v) => unsafe { v.raw_data_ptr() },
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
                    Some(dotnet_value::ByteOffset(0)),
                )));
                return StepResult::Continue;
            }
        }

        if field.parent.type_name() == "System.Runtime.CompilerServices.RawArrayData" {
            let data = h.borrow();
            if let HeapStorage::Vec(ref vector) = data.storage {
                let ptr = if field.field.name == "Data" {
                    unsafe { vector.raw_data_ptr() }
                } else if field.field.name == "Length" {
                    (&vector.layout.length as *const usize) as *mut u8
                } else {
                    ptr::null_mut()
                };

                if !ptr.is_null() {
                    let target_type = vm_try!(ctx.current_context().get_field_desc(field));
                    let offset = if field.field.name == "Data" {
                        dotnet_value::ByteOffset(0)
                    } else {
                        // Length is at the beginning of the ObjectInner? No, Vector struct.
                        // Actually, Length field in RawArrayData is special.
                        // If it's not the data, we should probably still calculate offset correctly if we want it to be stable.
                        // But RawArrayData is a hack anyway.
                        dotnet_value::ByteOffset(
                            (ptr as usize)
                                .wrapping_sub(unsafe { data.storage.raw_data_ptr() } as usize),
                        )
                    };
                    drop(data);
                    ctx.push(StackValue::ManagedPtr(ManagedPtr::new(
                        NonNull::new(ptr),
                        target_type,
                        Some(ObjectRef(Some(*h))),
                        false,
                        Some(offset),
                    )));
                    return StepResult::Continue;
                }
            }
        }
    }

    if parent.is_null() {
        return ctx.throw_by_name_with_message("System.NullReferenceException", NULL_REF_MSG);
    }
    let (origin, base_offset) = match get_ptr_context(ctx, &parent) {
        Ok(v) => v,
        Err(e) => return e,
    };

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

    let field_offset = if field.parent.type_name() == "System.String" && name == "_firstChar" {
        base_offset
    } else {
        base_offset + field_layout.position
    };
    let t = vm_try!(res_ctx.get_field_type(field));
    let target_type = vm_try!(ctx.loader().find_concrete_type(t));

    // Check if this is a ref field (ManagedPtr layout)
    if matches!(
        &*field_layout.layout,
        LayoutManager::Scalar(Scalar::ManagedPtr)
    ) {
        // This is a ref field - we need to deserialize the ManagedPtr from the field bytes
        // to get its actual origin, not use the parent's origin!
        // We must read the raw bytes and call ManagedPtr::read_branded directly.

        use dotnet_value::pointer::ManagedPtr as MP;

        let mut ptr_bytes = MP::serialization_buffer();
        if let Err(e) = unsafe { ctx.read_bytes(origin.clone(), field_offset, &mut ptr_bytes) } {
            return StepResult::Error(crate::error::ExecutionError::InternalError(e).into());
        }

        // Deserialize to get the actual origin
        let info = match unsafe {
            MP::read_branded(
                &ptr_bytes,
                &ctx.gc_with_token(&dotnet_utils::NoActiveBorrows::new()),
            )
        } {
            Ok(i) => i,
            Err(e) => {
                return StepResult::Error(
                    crate::error::ExecutionError::InternalError(format!(
                        "ManagedPtr deserialization failed: {:?}",
                        e
                    ))
                    .into(),
                );
            }
        };
        let managed_ptr = MP::from_info_full(info, target_type, false);

        ctx.push(StackValue::ManagedPtr(managed_ptr));
    } else {
        // Regular field - use parent's origin + field offset
        let field_ptr = ctx.resolve_address(origin.clone(), field_offset);
        let info = dotnet_value::pointer::ManagedPtrInfo {
            address: Some(field_ptr),
            origin,
            offset: field_offset,
        };
        ctx.push(StackValue::ManagedPtr(ManagedPtr::from_info_full(
            info,
            target_type,
            false,
        )));
    }

    StepResult::Continue
}

#[dotnet_instruction(LoadStaticFieldAddress(param0))]
pub fn ldsflda<'gc, 'm: 'gc, T: VesOps<'gc, 'm>>(ctx: &mut T, param0: &FieldSource) -> StepResult {
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
    let base_ptr = unsafe { storage.storage.raw_data_ptr() };

    let layout = storage.layout();
    let field_layout = layout.get_field(field.parent, name.as_ref()).unwrap();
    let field_ptr = unsafe { base_ptr.add(field_layout.position.as_usize()) };

    let t = vm_try!(res_ctx.get_field_type(field));
    let target_type = vm_try!(ctx.loader().find_concrete_type(t));

    ctx.push(StackValue::ManagedPtr(ManagedPtr::new_static(
        NonNull::new(field_ptr),
        target_type,
        field.parent,
        lookup,
        false,
        field_layout.position,
    )));

    StepResult::Continue
}

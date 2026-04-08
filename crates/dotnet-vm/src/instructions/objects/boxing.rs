use crate::{
    ExceptionOps, StepResult,
    instructions::NULL_REF_MSG,
    resolution::{TypeResolutionExt, ValueResolution},
    stack::ops::{EvalStackOps, LoaderOps, MemoryOps, ReflectionOps, ResolutionOps},
};
const INVALID_PROGRAM_MSG: &str = "Common Language Runtime detected an invalid program.";
const INVALID_CAST_MSG: &str = "Specified cast is not valid.";
use dotnet_macros::dotnet_instruction;
use dotnet_types::{
    comparer::decompose_type_source,
    generics::{ConcreteType, GenericLookup},
};
use dotnet_value::{
    ByteOffset, StackValue,
    layout::HasLayout,
    object::{HeapStorage, ObjectRef},
    pointer::ManagedPtr,
};
use dotnetdll::prelude::*;
use std::ptr::NonNull;

#[dotnet_instruction(BoxValue(param0))]
pub fn box_value<'gc, T: ResolutionOps<'gc> + EvalStackOps<'gc> + MemoryOps<'gc>>(
    ctx: &mut T,
    param0: &MethodType,
) -> StepResult {
    let res_ctx = ctx.current_context();
    let t = vm_try!(res_ctx.make_concrete(param0));
    let value = vm_pop!(ctx);

    if let StackValue::ObjectRef(_) = value {
        // boxing is a noop for all reference types
        ctx.push(value);
    } else {
        let obj = vm_try!(ctx.box_value(&t, value));
        ctx.push(StackValue::ObjectRef(obj));
    }
    StepResult::Continue
}

#[dotnet_instruction(UnboxIntoValue(param0))]
pub fn unbox_any<
    'gc,
    T: ResolutionOps<'gc>
        + LoaderOps
        + ExceptionOps<'gc>
        + EvalStackOps<'gc>
        + MemoryOps<'gc>
        + ReflectionOps<'gc>,
>(
    ctx: &mut T,
    param0: &MethodType,
) -> StepResult {
    let val = ctx.pop();
    let res_ctx = ctx.current_context();
    let target_ct = vm_try!(res_ctx.make_concrete(param0));

    let is_vt = match target_ct.get() {
        BaseType::Type { .. } => {
            let td = vm_try!(ctx.loader().find_concrete_type(target_ct.clone()));
            vm_try!(td.is_value_type(&res_ctx))
        }
        BaseType::Vector(_, _) | BaseType::Array(_, _) | BaseType::Object | BaseType::String => {
            false
        }
        _ => true, // Primitives, IntPtr, etc are value types
    };

    if is_vt {
        let is_nullable = target_ct.is_nullable(ctx.loader().as_ref());
        // If it's a value type, unbox it.
        let StackValue::ObjectRef(obj) = val else {
            return ctx
                .throw_by_name_with_message("System.InvalidProgramException", INVALID_PROGRAM_MSG);
        };
        if obj.0.is_none() {
            if is_nullable {
                // ECMA-335 §III.4.33: unbox.any Nullable<T> on null produces a Nullable<T> with HasValue=false.
                let td = vm_try!(ctx.loader().find_concrete_type(target_ct.clone()));
                let BaseType::Type { source, .. } = target_ct.get() else {
                    unreachable!("Nullable must be a type");
                };
                let (_, generics) = decompose_type_source(source);
                let lookup = GenericLookup::new(generics.clone());
                let ctx_with_generics = res_ctx.for_type_with_generics(td.clone(), &lookup);
                let instance = vm_try!(ctx_with_generics.new_object(td));

                ctx.push(StackValue::ValueType(instance));
                return StepResult::Continue;
            } else {
                // unbox.any on null value type throws NullReferenceException (III.4.33)
                return ctx
                    .throw_by_name_with_message("System.NullReferenceException", NULL_REF_MSG);
            }
        }

        let result = obj.as_heap_storage(|storage| -> Result<StackValue<'gc>, ()> {
            match storage {
                HeapStorage::Boxed(o) | HeapStorage::Obj(o) => {
                    // Boxed value or struct is an Object.
                    let td = o.description.clone();

                    if is_nullable {
                        // ECMA-335 §III.4.33: unbox.any Nullable<T> on boxed T.
                        // Extract T (the inner type).
                        let BaseType::Type { source, .. } = target_ct.get() else {
                            unreachable!("Nullable must be a type");
                        };
                        let (_ut, generics) = decompose_type_source(source);
                        let inner_t: &ConcreteType = generics.first().ok_or(())?;
                        let concrete_inner_t =
                            res_ctx.normalize_type(inner_t.clone()).map_err(|_| ())?;

                        // Check if obj is indeed a boxed T.
                        let obj_type = res_ctx
                            .get_heap_description(obj.0.unwrap())
                            .map_err(|_| ())?;
                        let obj_ct = res_ctx.normalize_type(obj_type.into()).map_err(|_| ())?;

                        if !res_ctx
                            .is_a(obj_ct, concrete_inner_t.clone())
                            .map_err(|_| ())?
                        {
                            return Err(());
                        }

                        // Create Nullable<T> instance.
                        let target_td = ctx
                            .loader()
                            .find_concrete_type(target_ct.clone())
                            .map_err(|_| ())?;

                        let (_, generics) = decompose_type_source(source);
                        let lookup = GenericLookup::new(generics.clone());
                        let ctx_with_generics =
                            res_ctx.for_type_with_generics(target_td.clone(), &lookup);

                        let nullable_instance =
                            ctx_with_generics.new_object(target_td).map_err(|_| ())?;

                        // Set HasValue = true.
                        let layout = nullable_instance.instance_storage.layout().clone();
                        let has_value_field = layout
                            .fields
                            .iter()
                            .find(|(k, _)| k.name == "hasValue" || k.name == "_hasValue")
                            .map(|(_, v)| v)
                            .ok_or(())?;

                        nullable_instance.instance_storage.with_data_mut(|d| {
                            d[has_value_field.position.as_usize()] = 1;
                        });

                        // Set Value = data from boxed object.
                        let value_field = layout
                            .fields
                            .iter()
                            .find(|(k, _)| k.name == "value" || k.name == "_value")
                            .map(|(_, v)| v)
                            .ok_or(())?;

                        let value_pos = value_field.position.as_usize();
                        let value_size = value_field.layout.size().as_usize();

                        nullable_instance
                            .instance_storage
                            .with_data_mut(|nullable_data| {
                                o.instance_storage.with_data(|boxed_data| {
                                    nullable_data[value_pos..value_pos + value_size]
                                        .copy_from_slice(&boxed_data[..value_size]);
                                })
                            });

                        return Ok(StackValue::ValueType(nullable_instance));
                    }

                    // If the target is a primitive, we need to extract it.
                    if let Some(e) = td.is_enum() {
                        let enum_type = res_ctx.make_concrete(e).map_err(|_| ())?;
                        let cts = o
                            .instance_storage
                            .with_data(|d| ctx.read_cts_value(&enum_type, d))
                            .map_err(|_| ())?;
                        Ok(cts.into_stack())
                    } else if td.definition().namespace.as_deref() == Some("System") {
                        let name = &td.definition().name;
                        match name.as_ref() {
                            "Boolean" | "Char" | "SByte" | "Byte" | "Int16" | "UInt16"
                            | "Int32" | "UInt32" | "Int64" | "UInt64" | "Single" | "Double"
                            | "IntPtr" | "UIntPtr" => {
                                let cts = o
                                    .instance_storage
                                    .with_data(|d| ctx.read_cts_value(&target_ct, d))
                                    .map_err(|_| ())?;
                                Ok(cts.into_stack())
                            }
                            _ => Ok(StackValue::ValueType(o.clone())),
                        }
                    } else {
                        Ok(StackValue::ValueType(o.clone()))
                    }
                }
                _ => Err(()),
            }
        });
        match result {
            Ok(v) => ctx.push(v),
            Err(_) => {
                return ctx
                    .throw_by_name_with_message("System.InvalidCastException", INVALID_CAST_MSG);
            }
        }
    } else {
        // Reference type: identical to castclass.
        let StackValue::ObjectRef(target_obj) = val else {
            return ctx
                .throw_by_name_with_message("System.InvalidProgramException", INVALID_PROGRAM_MSG);
        };
        if let ObjectRef(Some(o)) = target_obj {
            let obj_type = vm_try!(res_ctx.get_heap_description(o));
            if vm_try!(res_ctx.is_a(obj_type.into(), target_ct)) {
                ctx.push(StackValue::ObjectRef(target_obj));
            } else {
                return ctx
                    .throw_by_name_with_message("System.InvalidCastException", INVALID_CAST_MSG);
            }
        } else {
            ctx.push(StackValue::ObjectRef(ObjectRef(None)));
        }
    }
    StepResult::Continue
}

#[dotnet_instruction(UnboxIntoAddress { param0 })]
pub fn unbox<
    'gc,
    T: ResolutionOps<'gc> + ExceptionOps<'gc> + EvalStackOps<'gc> + LoaderOps + MemoryOps<'gc>,
>(
    ctx: &mut T,
    param0: &MethodType,
) -> StepResult {
    let value = vm_pop!(ctx);
    let res_ctx = ctx.current_context();
    let target_ct = vm_try!(res_ctx.make_concrete(param0));

    let StackValue::ObjectRef(obj) = value else {
        return ctx
            .throw_by_name_with_message("System.InvalidProgramException", INVALID_PROGRAM_MSG);
    };

    let is_nullable = target_ct.is_nullable(ctx.loader().as_ref());

    if obj.0.is_none() && !is_nullable {
        return ctx.throw_by_name_with_message("System.NullReferenceException", NULL_REF_MSG);
    }

    if is_nullable {
        // ECMA-335 §III.4.32: Manufacture a new Nullable<T> on the heap.
        let td = vm_try!(ctx.loader().find_concrete_type(target_ct.clone()));

        let BaseType::Type { source, .. } = target_ct.get() else {
            unreachable!("Nullable must be a type");
        };
        let (_, generics) = decompose_type_source(source);
        let lookup = GenericLookup::new(generics.clone());
        let res_ctx_with_generics = res_ctx.for_type_with_generics(td.clone(), &lookup);

        let instance = vm_try!(res_ctx_with_generics.new_object(td));

        if let Some(h) = obj.0 {
            // Boxed T exists.
            let inner_t: &ConcreteType = generics
                .first()
                .expect("Nullable must have generic parameter");
            let concrete_inner_t = vm_try!(res_ctx.normalize_type(inner_t.clone()));

            // Check if obj is indeed a boxed T.
            let obj_type = vm_try!(res_ctx.get_heap_description(h));
            let obj_ct = vm_try!(res_ctx.normalize_type(obj_type.into()));
            if !vm_try!(res_ctx.is_a(obj_ct, concrete_inner_t)) {
                return ctx
                    .throw_by_name_with_message("System.InvalidCastException", INVALID_CAST_MSG);
            }

            // Set HasValue = true.
            let layout = instance.instance_storage.layout().clone();
            let has_value_field = layout
                .fields
                .iter()
                .find(|(k, _)| k.name == "hasValue" || k.name == "_hasValue")
                .map(|(_, v)| v)
                .expect("Nullable must have hasValue");

            instance.instance_storage.with_data_mut(|d| {
                d[has_value_field.position.as_usize()] = 1;
            });

            // Set Value = data from boxed object.
            let value_field = layout
                .fields
                .iter()
                .find(|(k, _)| k.name == "value" || k.name == "_value")
                .map(|(_, v)| v)
                .expect("Nullable must have value");

            let value_pos = value_field.position.as_usize();
            let value_size = value_field.layout.size().as_usize();

            instance.instance_storage.with_data_mut(|nullable_data| {
                h.borrow().storage.with_data(|boxed_data| {
                    nullable_data[value_pos..value_pos + value_size]
                        .copy_from_slice(&boxed_data[..value_size]);
                })
            });
        }
        // If obj is null, HasValue remains false (zero-initialized).

        // Wrap the manufactured Nullable<T> into a boxed object on the heap.
        let boxed_nullable = ObjectRef::new(
            ctx.gc_with_token(&ctx.no_active_borrows_token()),
            HeapStorage::Boxed(instance),
        );
        ctx.register_new_object(&boxed_nullable);

        let h = boxed_nullable.0.unwrap();
        let ptr = unsafe { h.borrow().storage.raw_data_ptr() };
        let target_type = vm_try!(ctx.loader().find_concrete_type(target_ct));
        ctx.push(StackValue::ManagedPtr(ManagedPtr::new(
            NonNull::new(ptr),
            target_type,
            Some(boxed_nullable),
            false,
            Some(ByteOffset(0)),
        )));
        return StepResult::Continue;
    }

    // Normal unbox
    let h = obj.0.unwrap(); // is_none case handled above
    let inner = h.borrow();
    let ptr = match &inner.storage {
        HeapStorage::Boxed(_) | HeapStorage::Obj(_) => unsafe { inner.storage.raw_data_ptr() },
        _ => {
            return ctx.throw_by_name_with_message("System.InvalidCastException", INVALID_CAST_MSG);
        }
    };

    let target_type = vm_try!(ctx.loader().find_concrete_type(target_ct));
    ctx.push(StackValue::ManagedPtr(ManagedPtr::new(
        NonNull::new(ptr),
        target_type,
        Some(obj),
        false,
        Some(ByteOffset(0)),
    )));

    StepResult::Continue
}

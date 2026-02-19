use crate::{StepResult, resolution::TypeResolutionExt, stack::ops::VesOps};
use dotnet_macros::dotnet_instruction;
use dotnet_value::{
    StackValue,
    object::{HeapStorage, ObjectRef},
    pointer::ManagedPtr,
};
use dotnetdll::prelude::*;
use std::ptr::NonNull;

#[dotnet_instruction(BoxValue(param0))]
pub fn box_value<'gc, 'm: 'gc, T: VesOps<'gc, 'm> + ?Sized>(
    ctx: &mut T,
    param0: &MethodType,
) -> StepResult {
    let res_ctx = ctx.current_context();
    let t = vm_try!(res_ctx.make_concrete(param0));
    let value = ctx.pop();

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
pub fn unbox_any<'gc, 'm: 'gc, T: VesOps<'gc, 'm> + ?Sized>(
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
        // If it's a value type, unbox it.
        let StackValue::ObjectRef(obj) = val else {
            return ctx.throw_by_name("System.InvalidProgramException");
        };
        if obj.0.is_none() {
            // unbox.any on null value type throws NullReferenceException (III.4.33)
            return ctx.throw_by_name("System.NullReferenceException");
        }

        let result = obj.as_heap_storage(|storage| -> Result<StackValue<'gc>, ()> {
            match storage {
                HeapStorage::Boxed(o) | HeapStorage::Obj(o) => {
                    // Boxed value or struct is an Object.
                    // If the target is a primitive, we need to extract it.
                    let td = o.description;
                    if let Some(e) = td.is_enum() {
                        let enum_type = res_ctx.make_concrete(e).map_err(|_| ())?;
                        let cts = ctx
                            .read_cts_value(&enum_type, &o.instance_storage.get())
                            .map_err(|_| ())?;
                        Ok(cts.into_stack())
                    } else if td.definition().namespace.as_deref() == Some("System") {
                        let name = &td.definition().name;
                        match name.as_ref() as &str {
                            "Boolean" | "Char" | "SByte" | "Byte" | "Int16" | "UInt16"
                            | "Int32" | "UInt32" | "Int64" | "UInt64" | "Single" | "Double"
                            | "IntPtr" | "UIntPtr" => {
                                let cts = ctx
                                    .read_cts_value(&target_ct, &o.instance_storage.get())
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
            Err(_) => return ctx.throw_by_name("System.InvalidCastException"),
        }
    } else {
        // Reference type: identical to castclass.
        let StackValue::ObjectRef(target_obj) = val else {
            return ctx.throw_by_name("System.InvalidProgramException");
        };
        if let ObjectRef(Some(o)) = target_obj {
            let obj_type = vm_try!(res_ctx.get_heap_description(o));
            if vm_try!(res_ctx.is_a(obj_type.into(), target_ct)) {
                ctx.push(StackValue::ObjectRef(target_obj));
            } else {
                return ctx.throw_by_name("System.InvalidCastException");
            }
        } else {
            ctx.push(StackValue::ObjectRef(ObjectRef(None)));
        }
    }
    StepResult::Continue
}

#[dotnet_instruction(UnboxIntoAddress { param0 })]
pub fn unbox<'gc, 'm: 'gc, T: VesOps<'gc, 'm> + ?Sized>(
    ctx: &mut T,
    param0: &MethodType,
) -> StepResult {
    let value = ctx.pop();
    let res_ctx = ctx.current_context();
    let target_ct = vm_try!(res_ctx.make_concrete(param0));

    let StackValue::ObjectRef(obj) = value else {
        return ctx.throw_by_name("System.InvalidProgramException");
    };
    let Some(h) = obj.0 else {
        return ctx.throw_by_name("System.NullReferenceException");
    };

    let inner = h.borrow();
    let ptr = match &inner.storage {
        HeapStorage::Boxed(o) | HeapStorage::Obj(o) => o.instance_storage.get().as_ptr() as *mut u8,
        _ => return ctx.throw_by_name("System.InvalidCastException"),
    };

    let target_type = vm_try!(ctx.loader().find_concrete_type(target_ct));
    ctx.push(StackValue::ManagedPtr(ManagedPtr::new(
        NonNull::new(ptr),
        target_type,
        Some(obj),
        false,
        Some(dotnet_value::ByteOffset(0)),
    )));

    StepResult::Continue
}

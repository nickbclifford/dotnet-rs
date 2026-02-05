use crate::{
    StepResult,
    resolution::{TypeResolutionExt, ValueResolution},
    stack::VesContext,
};
use dotnet_macros::dotnet_instruction;
use dotnet_utils::gc::GCHandle;
use dotnet_value::{
    StackValue,
    object::{HeapStorage, ObjectRef},
    pointer::ManagedPtr,
};
use dotnetdll::prelude::*;
use std::ptr::NonNull;

#[dotnet_instruction(BoxValue(param0))]
pub fn box_value<'gc, 'm: 'gc>(
    ctx: &mut VesContext<'_, 'gc, 'm>,
    gc: GCHandle<'gc>,
    param0: &MethodType,
) -> StepResult {
    let res_ctx = ctx.current_context();
    let t = res_ctx.make_concrete(param0);
    let value = ctx.pop(gc);

    if let StackValue::ObjectRef(_) = value {
        // boxing is a noop for all reference types
        ctx.push(gc, value);
    } else {
        let res_ctx = ctx.current_context();
        let obj = ObjectRef::new(gc, HeapStorage::Boxed(res_ctx.new_value_type(&t, value)));
        ctx.register_new_object(&obj);
        ctx.push(gc, StackValue::ObjectRef(obj));
    }
    StepResult::Continue
}

#[dotnet_instruction(UnboxIntoValue(param0))]
pub fn unbox_any<'gc, 'm: 'gc>(
    ctx: &mut VesContext<'_, 'gc, 'm>,
    gc: GCHandle<'gc>,
    param0: &MethodType,
) -> StepResult {
    let val = ctx.pop(gc);
    let res_ctx = ctx.current_context();
    let target_ct = res_ctx.make_concrete(param0);

    let is_vt = match target_ct.get() {
        BaseType::Type { .. } => {
            let td = ctx.loader().find_concrete_type(target_ct.clone());
            td.is_value_type(&res_ctx)
        }
        BaseType::Vector(_, _) | BaseType::Array(_, _) | BaseType::Object | BaseType::String => {
            false
        }
        _ => true, // Primitives, IntPtr, etc are value types
    };

    if is_vt {
        // If it's a value type, unbox it.
        let StackValue::ObjectRef(obj) = val else {
            panic!("unbox.any: expected object on stack, got {:?}", val);
        };
        if obj.0.is_none() {
            // unbox.any on null value type throws NullReferenceException (III.4.33)
            return ctx.throw_by_name(gc, "System.NullReferenceException");
        }

        let result = obj.as_heap_storage(|storage| {
            match storage {
                HeapStorage::Boxed(v) => {
                    dotnet_value::object::CTSValue::Value(v.clone()).into_stack(gc)
                }
                HeapStorage::Obj(o) => {
                    // Boxed struct is just an Object of that struct type.
                    StackValue::ValueType(o.clone())
                }
                _ => panic!("unbox.any: expected boxed value, got {:?}", storage),
            }
        });
        ctx.push(gc, result);
    } else {
        // Reference type: identical to castclass.
        let StackValue::ObjectRef(target_obj) = val else {
            panic!("unbox.any: expected object on stack, got {:?}", val);
        };
        if let ObjectRef(Some(o)) = target_obj {
            let obj_type = res_ctx.get_heap_description(o);
            if res_ctx.is_a(obj_type.into(), target_ct) {
                ctx.push(gc, StackValue::ObjectRef(target_obj));
            } else {
                return ctx.throw_by_name(gc, "System.InvalidCastException");
            }
        } else {
            ctx.push(gc, StackValue::ObjectRef(ObjectRef(None)));
        }
    }
    StepResult::Continue
}

#[dotnet_instruction(UnboxIntoAddress { param0 })]
pub fn unbox<'gc, 'm: 'gc>(
    ctx: &mut VesContext<'_, 'gc, 'm>,
    gc: GCHandle<'gc>,
    param0: &MethodType,
) -> StepResult {
    let value = ctx.pop(gc);
    let res_ctx = ctx.current_context();
    let target_ct = res_ctx.make_concrete(param0);

    let StackValue::ObjectRef(obj) = value else {
        panic!("unbox on non-object: {:?}", value);
    };
    let Some(h) = obj.0 else {
        return ctx.throw_by_name(gc, "System.NullReferenceException");
    };

    let inner = h.borrow();
    let ptr = match &inner.storage {
        HeapStorage::Boxed(dotnet_value::object::ValueType::Struct(s)) => {
            s.instance_storage.get().as_ptr() as *mut u8
        }
        _ => panic!("unbox on non-boxed struct"),
    };

    let target_type = ctx.loader().find_concrete_type(target_ct);
    ctx.push(
        gc,
        StackValue::ManagedPtr(ManagedPtr::new(
            NonNull::new(ptr),
            target_type,
            Some(obj),
            false,
        )),
    );

    StepResult::Continue
}

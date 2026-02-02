use crate::{
    resolution::{TypeResolutionExt, ValueResolution},
    CallStack, StepResult,
};
use dotnet_macros::dotnet_instruction;
use dotnet_utils::gc::GCHandle;
use dotnet_value::{
    object::{HeapStorage, ObjectRef},
    pointer::ManagedPtr,
    StackValue,
};
use dotnetdll::prelude::*;
use std::ptr::NonNull;

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
    StepResult::Continue
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
            return stack.throw_by_name(gc, "System.NullReferenceException");
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
    StepResult::Continue
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

    StepResult::Continue
}

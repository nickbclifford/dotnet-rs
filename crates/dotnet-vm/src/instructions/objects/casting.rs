use crate::{CallStack, StepResult};
use dotnet_macros::dotnet_instruction;
use dotnet_utils::gc::GCHandle;
use dotnet_value::{object::ObjectRef, StackValue};
use dotnetdll::prelude::*;

#[dotnet_instruction(CastClass)]
pub fn castclass<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    param0: &MethodType,
) -> StepResult {
    let target_obj_val = stack.pop(gc);
    let StackValue::ObjectRef(target_obj) = target_obj_val else {
        panic!(
            "castclass: expected object on stack, got {:?}",
            target_obj_val
        );
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
    StepResult::Continue
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
    StepResult::Continue
}

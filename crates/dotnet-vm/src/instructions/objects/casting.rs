use crate::{StepResult, stack::ops::VesOps};
use dotnet_macros::dotnet_instruction;
use dotnet_utils::gc::GCHandle;
use dotnet_value::{StackValue, object::ObjectRef};
use dotnetdll::prelude::*;

#[dotnet_instruction(CastClass { param0 })]
pub fn castclass<'gc, 'm: 'gc, T: VesOps<'gc, 'm> + ?Sized>(
    ctx: &mut T,
    gc: GCHandle<'gc>,
    param0: &MethodType,
) -> StepResult {
    let target_obj_val = ctx.pop(gc);
    let StackValue::ObjectRef(target_obj) = target_obj_val else {
        panic!(
            "castclass: expected object on stack, got {:?}",
            target_obj_val
        );
    };

    if let ObjectRef(Some(o)) = target_obj {
        let res_ctx = ctx.current_context();
        let obj_type = res_ctx.get_heap_description(o);
        let target_ct = res_ctx.make_concrete(param0);

        if res_ctx.is_a(obj_type.into(), target_ct) {
            ctx.push(gc, StackValue::ObjectRef(target_obj));
        } else {
            return ctx.throw_by_name(gc, "System.InvalidCastException");
        }
    } else {
        // castclass returns null for null (III.4.3)
        ctx.push(gc, StackValue::ObjectRef(ObjectRef(None)));
    }
    StepResult::Continue
}

#[dotnet_instruction(IsInstance(param0))]
pub fn isinst<'gc, 'm: 'gc, T: VesOps<'gc, 'm> + ?Sized>(
    ctx: &mut T,
    gc: GCHandle<'gc>,
    param0: &MethodType,
) -> StepResult {
    let target_obj_val = ctx.pop(gc);
    let StackValue::ObjectRef(target_obj) = target_obj_val else {
        panic!("isinst: expected object on stack, got {:?}", target_obj_val);
    };

    if let ObjectRef(Some(o)) = target_obj {
        let res_ctx = ctx.current_context();
        let obj_type = res_ctx.get_heap_description(o);
        let target_ct = res_ctx.make_concrete(param0);

        if res_ctx.is_a(obj_type.into(), target_ct) {
            ctx.push(gc, StackValue::ObjectRef(target_obj));
        } else {
            ctx.push(gc, StackValue::ObjectRef(ObjectRef(None)));
        }
    } else {
        // isinst returns null for null inputs (III.4.6)
        ctx.push(gc, StackValue::ObjectRef(ObjectRef(None)));
    }
    StepResult::Continue
}

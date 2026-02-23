use crate::{StepResult, stack::ops::VesOps};

const INVALID_PROGRAM_MSG: &str = "Common Language Runtime detected an invalid program.";
const INVALID_CAST_MSG: &str = "Specified cast is not valid.";
use dotnet_macros::dotnet_instruction;
use dotnet_value::{StackValue, object::ObjectRef};
use dotnetdll::prelude::*;

#[dotnet_instruction(CastClass { param0 })]
pub fn castclass<'gc, 'm: 'gc, T: VesOps<'gc, 'm> + ?Sized>(
    ctx: &mut T,
    param0: &MethodType,
) -> StepResult {
    let target_obj_val = ctx.pop();
    let StackValue::ObjectRef(target_obj) = target_obj_val else {
        return ctx.throw_by_name_with_message("System.InvalidProgramException", INVALID_PROGRAM_MSG);
    };

    if let ObjectRef(Some(o)) = target_obj {
        let res_ctx = ctx.current_context();
        let obj_type = vm_try!(res_ctx.get_heap_description(o));
        let target_ct = vm_try!(res_ctx.make_concrete(param0));

        if vm_try!(res_ctx.is_a(obj_type.into(), target_ct)) {
            ctx.push(StackValue::ObjectRef(target_obj));
        } else {
            return ctx.throw_by_name_with_message("System.InvalidCastException", INVALID_CAST_MSG);
        }
    } else {
        // castclass returns null for null (III.4.3)
        ctx.push(StackValue::ObjectRef(ObjectRef(None)));
    }
    StepResult::Continue
}

#[dotnet_instruction(IsInstance(param0))]
pub fn isinst<'gc, 'm: 'gc, T: VesOps<'gc, 'm> + ?Sized>(
    ctx: &mut T,
    param0: &MethodType,
) -> StepResult {
    let target_obj_val = ctx.pop();
    let StackValue::ObjectRef(target_obj) = target_obj_val else {
        return ctx.throw_by_name_with_message("System.InvalidProgramException", INVALID_PROGRAM_MSG);
    };

    if let ObjectRef(Some(o)) = target_obj {
        let res_ctx = ctx.current_context();
        let obj_type = vm_try!(res_ctx.get_heap_description(o));
        let target_ct = vm_try!(res_ctx.make_concrete(param0));

        if vm_try!(res_ctx.is_a(obj_type.into(), target_ct)) {
            ctx.push(StackValue::ObjectRef(target_obj));
        } else {
            ctx.push(StackValue::ObjectRef(ObjectRef(None)));
        }
    } else {
        // isinst returns null for null inputs (III.4.6)
        ctx.push(StackValue::ObjectRef(ObjectRef(None)));
    }
    StepResult::Continue
}

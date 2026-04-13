use crate::{
    StepResult,
    stack::ops::{EvalStackOps, ExceptionOps, ReflectionOps, ResolutionOps},
};

const INVALID_PROGRAM_MSG: &str = "Common Language Runtime detected an invalid program.";
const INVALID_CAST_MSG: &str = "Specified cast is not valid.";
use dotnet_macros::dotnet_instruction;
use dotnet_value::{StackValue, object::ObjectRef};
use dotnetdll::prelude::*;

#[dotnet_instruction(CastClass { param0 })]
pub fn castclass<
    'gc,
    T: ResolutionOps<'gc> + ReflectionOps<'gc> + ExceptionOps<'gc> + EvalStackOps<'gc>,
>(
    ctx: &mut T,
    param0: &MethodType,
) -> StepResult {
    let target_obj_val = vm_pop!(ctx);
    let StackValue::ObjectRef(target_obj) = target_obj_val else {
        return ctx
            .throw_by_name_with_message("System.InvalidProgramException", INVALID_PROGRAM_MSG);
    };

    if let ObjectRef(Some(_)) = target_obj {
        let obj_type = dotnet_vm_ops::vm_try!(ctx.get_heap_description(target_obj));
        let target_ct = dotnet_vm_ops::vm_try!(ctx.make_concrete(param0));

        if dotnet_vm_ops::vm_try!(ctx.is_a(obj_type.into(), target_ct)) {
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
pub fn isinst<
    'gc,
    T: ResolutionOps<'gc> + ReflectionOps<'gc> + ExceptionOps<'gc> + EvalStackOps<'gc>,
>(
    ctx: &mut T,
    param0: &MethodType,
) -> StepResult {
    let target_obj_val = vm_pop!(ctx);
    let StackValue::ObjectRef(target_obj) = target_obj_val else {
        return ctx
            .throw_by_name_with_message("System.InvalidProgramException", INVALID_PROGRAM_MSG);
    };

    if let ObjectRef(Some(_)) = target_obj {
        let obj_type = dotnet_vm_ops::vm_try!(ctx.get_heap_description(target_obj));
        let target_ct = dotnet_vm_ops::vm_try!(ctx.make_concrete(param0));

        if dotnet_vm_ops::vm_try!(ctx.is_a(obj_type.into(), target_ct)) {
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

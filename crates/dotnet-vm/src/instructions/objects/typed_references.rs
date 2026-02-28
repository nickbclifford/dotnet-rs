use crate::{StepResult, stack::ops::VesOps};

const INVALID_PROGRAM_MSG: &str = "Common Language Runtime detected an invalid program.";
const INVALID_CAST_MSG: &str = "Specified cast is not valid.";
use dotnet_macros::dotnet_instruction;
use dotnet_value::StackValue;
use dotnetdll::prelude::*;
use std::sync::Arc;

#[dotnet_instruction(MakeTypedReference(class))]
pub fn mkrefany<'gc, 'm: 'gc, T: VesOps<'gc, 'm>>(
    ctx: &mut T,
    class: &MethodType,
) -> StepResult {
    let ptr = ctx.pop();
    let StackValue::ManagedPtr(m) = ptr else {
        return ctx
            .throw_by_name_with_message("System.InvalidProgramException", INVALID_PROGRAM_MSG);
    };
    let target_type = vm_try!(ctx.make_concrete(class));
    let target_td = vm_try!(ctx.loader().find_concrete_type(target_type));
    ctx.push(StackValue::TypedRef(m, Arc::new(target_td)));
    StepResult::Continue
}

#[dotnet_instruction(ReadTypedReferenceType)]
pub fn refanytype<'gc, 'm: 'gc, T: VesOps<'gc, 'm>>(ctx: &mut T) -> StepResult {
    let tr = ctx.pop();
    let StackValue::TypedRef(_, td) = tr else {
        return ctx
            .throw_by_name_with_message("System.InvalidProgramException", INVALID_PROGRAM_MSG);
    };
    // refanytype pushes a RuntimeTypeHandle (which is a pointer to the type)
    ctx.push(StackValue::NativeInt(Arc::as_ptr(&td) as isize));
    StepResult::Continue
}

#[dotnet_instruction(ReadTypedReferenceValue(class))]
pub fn refanyval<'gc, 'm: 'gc, T: VesOps<'gc, 'm>>(
    ctx: &mut T,
    class: &MethodType,
) -> StepResult {
    let tr = ctx.pop();
    let StackValue::TypedRef(m, td) = tr else {
        return ctx
            .throw_by_name_with_message("System.InvalidProgramException", INVALID_PROGRAM_MSG);
    };
    let target_type = vm_try!(ctx.make_concrete(class));
    let target_td = vm_try!(ctx.loader().find_concrete_type(target_type));

    if *td != target_td {
        return ctx.throw_by_name_with_message("System.InvalidCastException", INVALID_CAST_MSG);
    }

    ctx.push(StackValue::ManagedPtr(m));
    StepResult::Continue
}

use crate::{
    StepResult,
    stack::ops::{ExceptionOps, TypedStackOps},
};
use dotnet_macros::dotnet_instruction;

#[dotnet_instruction(Leave(jump_target))]
pub fn leave<'gc, T: ExceptionOps<'gc>>(ctx: &mut T, jump_target: usize) -> StepResult {
    ctx.leave(jump_target)
}

#[dotnet_instruction(EndFinally)]
pub fn endfinally<'gc, T: ExceptionOps<'gc>>(ctx: &mut T) -> StepResult {
    ctx.endfinally()
}

#[dotnet_instruction(EndFilter)]
pub fn endfilter<'gc, T: TypedStackOps<'gc> + ExceptionOps<'gc>>(ctx: &mut T) -> StepResult {
    let result_val = ctx.pop_i32();
    ctx.endfilter(result_val)
}

#[dotnet_instruction(Throw)]
pub fn throw<'gc, T: TypedStackOps<'gc> + ExceptionOps<'gc>>(ctx: &mut T) -> StepResult {
    let exc = ctx.pop_obj();
    if exc.0.is_none() {
        return ctx.throw_by_name_with_message(
            "System.NullReferenceException",
            "Object reference not set to an instance of an object.",
        );
    }
    ctx.throw(exc)
}

#[dotnet_instruction(Rethrow)]
pub fn rethrow<'gc, T: ExceptionOps<'gc>>(ctx: &mut T) -> StepResult {
    ctx.rethrow()
}

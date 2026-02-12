use crate::{
    StepResult,
    stack::ops::{ExceptionOps, StackOps},
};
use dotnet_macros::dotnet_instruction;
use dotnetdll::prelude::*;

#[dotnet_instruction(Leave(jump_target))]
pub fn leave<'gc, 'm: 'gc, T: ExceptionOps<'gc> + ?Sized>(
    ctx: &mut T,
    jump_target: usize,
) -> StepResult {
    ctx.leave(jump_target)
}

#[dotnet_instruction(EndFinally)]
pub fn endfinally<'gc, 'm: 'gc, T: ExceptionOps<'gc> + ?Sized>(
    ctx: &mut T,
) -> StepResult {
    ctx.endfinally()
}

#[dotnet_instruction(EndFilter)]
pub fn endfilter<'gc, 'm: 'gc, T: StackOps<'gc, 'm> + ExceptionOps<'gc> + ?Sized>(
    ctx: &mut T,
) -> StepResult {
    let result_val = ctx.pop_i32();
    ctx.endfilter(result_val)
}

#[dotnet_instruction(Throw)]
pub fn throw<'gc, 'm: 'gc, T: StackOps<'gc, 'm> + ExceptionOps<'gc> + ?Sized>(
    ctx: &mut T,
) -> StepResult {
    let exc = ctx.pop_obj();
    if exc.0.is_none() {
        return ctx.throw_by_name("System.NullReferenceException");
    }
    ctx.throw(exc)
}

#[dotnet_instruction(Rethrow)]
pub fn rethrow<'gc, 'm: 'gc, T: ExceptionOps<'gc> + ?Sized>(
    ctx: &mut T,
) -> StepResult {
    ctx.rethrow()
}

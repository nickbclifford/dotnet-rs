use crate::{
    StepResult,
    stack::ops::{ExceptionOps, StackOps},
};
use dotnet_macros::dotnet_instruction;
use dotnet_utils::gc::GCHandle;
use dotnetdll::prelude::*;

#[dotnet_instruction(Leave(jump_target))]
pub fn leave<'gc, 'm: 'gc, T: ExceptionOps<'gc> + ?Sized>(
    ctx: &mut T,
    gc: GCHandle<'gc>,
    jump_target: usize,
) -> StepResult {
    ctx.leave(gc, jump_target)
}

#[dotnet_instruction(EndFinally)]
pub fn endfinally<'gc, 'm: 'gc, T: ExceptionOps<'gc> + ?Sized>(
    ctx: &mut T,
    gc: GCHandle<'gc>,
) -> StepResult {
    ctx.endfinally(gc)
}

#[dotnet_instruction(EndFilter)]
pub fn endfilter<'gc, 'm: 'gc, T: StackOps<'gc, 'm> + ExceptionOps<'gc> + ?Sized>(
    ctx: &mut T,
    gc: GCHandle<'gc>,
) -> StepResult {
    let result_val = ctx.pop_i32(gc);
    ctx.endfilter(gc, result_val)
}

#[dotnet_instruction(Throw)]
pub fn throw<'gc, 'm: 'gc, T: StackOps<'gc, 'm> + ExceptionOps<'gc> + ?Sized>(
    ctx: &mut T,
    gc: GCHandle<'gc>,
) -> StepResult {
    let exc = ctx.pop_obj(gc);
    if exc.0.is_none() {
        return ctx.throw_by_name(gc, "System.NullReferenceException");
    }
    ctx.throw(gc, exc)
}

#[dotnet_instruction(Rethrow)]
pub fn rethrow<'gc, 'm: 'gc, T: ExceptionOps<'gc> + ?Sized>(
    ctx: &mut T,
    gc: GCHandle<'gc>,
) -> StepResult {
    ctx.rethrow(gc)
}

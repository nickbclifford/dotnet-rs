use crate::{StepResult, instructions::macros::*, stack::VesContext};
use dotnet_macros::dotnet_instruction;
use dotnet_utils::gc::GCHandle;
use dotnetdll::prelude::*;
use std::cmp::Ordering as CmpOrdering;

#[dotnet_instruction(CompareEqual)]
pub fn ceq<'gc, 'm: 'gc>(ctx: &mut VesContext<'_, 'gc, 'm>, gc: GCHandle<'gc>) -> StepResult {
    let v2 = ctx.pop(gc);
    let v1 = ctx.pop(gc);
    let val = (v1 == v2) as i32;
    ctx.push_i32(gc, val);
    StepResult::Continue
}

comparison_op!(
    #[dotnet_instruction(CompareGreater(sgn))]
    cgt,
    CmpOrdering::Greater
);
comparison_op!(
    #[dotnet_instruction(CompareLess(sgn))]
    clt,
    CmpOrdering::Less
);

#[dotnet_instruction(CheckFinite)]
pub fn ckfinite<'gc, 'm: 'gc>(ctx: &mut VesContext<'_, 'gc, 'm>, gc: GCHandle<'gc>) -> StepResult {
    let f = ctx.pop_f64(gc);
    if f.is_infinite() || f.is_nan() {
        return ctx.throw_by_name(gc, "System.ArithmeticException");
    }
    ctx.push_f64(gc, f);
    StepResult::Continue
}

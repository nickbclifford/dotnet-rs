use crate::{
    StepResult,
    stack::ops::{EvalStackOps, ExceptionOps, TypedStackOps},
};
use dotnet_macros::dotnet_instruction;
use dotnetdll::prelude::*;
use std::cmp::Ordering as CmpOrdering;

#[dotnet_instruction(CompareEqual)]
pub fn ceq<'gc, T: EvalStackOps<'gc> + TypedStackOps<'gc>>(ctx: &mut T) -> StepResult {
    let v2 = vm_pop!(ctx);
    let v1 = vm_pop!(ctx);
    let val = (v1 == v2) as i32;
    ctx.push_i32(val);
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
pub fn ckfinite<'gc, T: TypedStackOps<'gc> + ExceptionOps<'gc>>(ctx: &mut T) -> StepResult {
    let f = ctx.pop_f64();
    if f.is_infinite() || f.is_nan() {
        return ctx.throw_by_name_with_message(
            "System.ArithmeticException",
            "Overflow or underflow in the arithmetic operation.",
        );
    }
    ctx.push_f64(f);
    StepResult::Continue
}

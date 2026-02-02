use crate::{
    instructions::{macros::*, StepResult},
    CallStack,
};
use dotnet_macros::dotnet_instruction;
use dotnet_utils::gc::GCHandle;
use dotnetdll::prelude::*;
use std::cmp::Ordering as CmpOrdering;

#[dotnet_instruction(CompareEqual)]
pub fn ceq<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
) -> StepResult {
    let v2 = stack.pop(gc);
    let v1 = stack.pop(gc);
    let val = (v1 == v2) as i32;
    stack.push_i32(gc, val);
    StepResult::Continue
}

comparison_op!(
    #[dotnet_instruction(CompareGreater)]
    cgt,
    CmpOrdering::Greater
);
comparison_op!(
    #[dotnet_instruction(CompareLess)]
    clt,
    CmpOrdering::Less
);

#[dotnet_instruction(CheckFinite)]
pub fn ckfinite<'gc, 'm: 'gc>(gc: GCHandle<'gc>, stack: &mut CallStack<'gc, 'm>) -> StepResult {
    let f = stack.pop_f64(gc);
    if f.is_infinite() || f.is_nan() {
        return stack.throw_by_name(gc, "System.ArithmeticException");
    }
    stack.push_f64(gc, f);
    StepResult::Continue
}

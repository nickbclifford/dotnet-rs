use crate::{instructions::StepResult, CallStack};
use dotnet_macros::dotnet_instruction;
use dotnet_utils::gc::GCHandle;
use dotnetdll::prelude::*;
use std::cmp::Ordering as CmpOrdering;

#[dotnet_instruction(CompareEqual)]
pub fn ceq<'gc, 'm: 'gc>(gc: GCHandle<'gc>, stack: &mut CallStack<'gc, 'm>) -> StepResult {
    let value2 = stack.pop(gc);
    let value1 = stack.pop(gc);
    let val = (value1 == value2) as i32;
    stack.push_i32(gc, val);
    StepResult::InstructionStepped
}

#[dotnet_instruction(CompareGreater)]
pub fn cgt<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    sgn: NumberSign,
) -> StepResult {
    let value2 = stack.pop(gc);
    let value1 = stack.pop(gc);
    let val = matches!(value1.compare(&value2, sgn), Some(CmpOrdering::Greater)) as i32;
    stack.push_i32(gc, val);
    StepResult::InstructionStepped
}

#[dotnet_instruction(CompareLess)]
pub fn clt<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    sgn: NumberSign,
) -> StepResult {
    let value2 = stack.pop(gc);
    let value1 = stack.pop(gc);
    let val = matches!(value1.compare(&value2, sgn), Some(CmpOrdering::Less)) as i32;
    stack.push_i32(gc, val);
    StepResult::InstructionStepped
}

#[dotnet_instruction(CheckFinite)]
pub fn ckfinite<'gc, 'm: 'gc>(gc: GCHandle<'gc>, stack: &mut CallStack<'gc, 'm>) -> StepResult {
    let f = stack.pop_f64(gc);
    if f.is_infinite() || f.is_nan() {
        return stack.throw_by_name(gc, "System.ArithmeticException");
    }
    stack.push_f64(gc, f);
    StepResult::InstructionStepped
}

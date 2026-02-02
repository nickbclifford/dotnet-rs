use crate::{instructions::StepResult, vm_expect_stack, vm_pop, vm_push, CallStack};
use dotnet_macros::dotnet_instruction;
use dotnet_utils::gc::GCHandle;
use dotnet_value::StackValue;
use dotnetdll::prelude::*;
use std::cmp::Ordering as CmpOrdering;

#[dotnet_instruction(CompareEqual)]
pub fn ceq<'gc, 'm: 'gc>(gc: GCHandle<'gc>, stack: &mut CallStack<'gc, 'm>) -> StepResult {
    let value2 = vm_pop!(stack, gc);
    let value1 = vm_pop!(stack, gc);
    let val = (value1 == value2) as i32;
    vm_push!(stack, gc, StackValue::Int32(val));
    StepResult::InstructionStepped
}

#[dotnet_instruction(CompareGreater)]
pub fn cgt<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    sgn: NumberSign,
) -> StepResult {
    let value2 = vm_pop!(stack, gc);
    let value1 = vm_pop!(stack, gc);
    let val = matches!(value1.compare(&value2, sgn), Some(CmpOrdering::Greater)) as i32;
    vm_push!(stack, gc, StackValue::Int32(val));
    StepResult::InstructionStepped
}

#[dotnet_instruction(CompareLess)]
pub fn clt<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    sgn: NumberSign,
) -> StepResult {
    let value2 = vm_pop!(stack, gc);
    let value1 = vm_pop!(stack, gc);
    let val = matches!(value1.compare(&value2, sgn), Some(CmpOrdering::Less)) as i32;
    vm_push!(stack, gc, StackValue::Int32(val));
    StepResult::InstructionStepped
}

#[dotnet_instruction(CheckFinite)]
pub fn ckfinite<'gc, 'm: 'gc>(gc: GCHandle<'gc>, stack: &mut CallStack<'gc, 'm>) -> StepResult {
    vm_expect_stack!(let NativeFloat(f) = vm_pop!(stack, gc));
    if f.is_infinite() || f.is_nan() {
        return stack.throw_by_name(gc, "System.ArithmeticException");
    }
    vm_push!(stack, gc, StackValue::NativeFloat(f));
    StepResult::InstructionStepped
}

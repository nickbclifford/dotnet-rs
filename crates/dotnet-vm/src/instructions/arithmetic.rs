use crate::{CallStack, StepResult};
use dotnet_macros::dotnet_instruction;
use dotnet_utils::gc::GCHandle;
use dotnetdll::prelude::*;

#[dotnet_instruction(Add)]
pub fn add<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
) -> StepResult {
    let v2 = stack.pop(gc);
    let v1 = stack.pop(gc);
    stack.push(gc, v1 + v2);
    StepResult::InstructionStepped
}

#[dotnet_instruction(AddOverflow)]
pub fn add_ovf<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    sgn: NumberSign,
) -> StepResult {
    let v2 = stack.pop(gc);
    let v1 = stack.pop(gc);
    match v1.checked_add(v2, sgn) {
        Ok(v) => {
            stack.push(gc, v);
            StepResult::InstructionStepped
        }
        Err(e) => stack.throw_by_name(gc, e),
    }
}

#[dotnet_instruction(And)]
pub fn and<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
) -> StepResult {
    let v2 = stack.pop(gc);
    let v1 = stack.pop(gc);
    stack.push(gc, v1 & v2);
    StepResult::InstructionStepped
}

#[dotnet_instruction(Divide)]
pub fn divide<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    sgn: NumberSign,
) -> StepResult {
    let v2 = stack.pop(gc);
    let v1 = stack.pop(gc);
    match v1.div(v2, sgn) {
        Ok(v) => {
            stack.push(gc, v);
            StepResult::InstructionStepped
        }
        Err(e) => stack.throw_by_name(gc, e),
    }
}

#[dotnet_instruction(Multiply)]
pub fn multiply<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
) -> StepResult {
    let v2 = stack.pop(gc);
    let v1 = stack.pop(gc);
    stack.push(gc, v1 * v2);
    StepResult::InstructionStepped
}

#[dotnet_instruction(MultiplyOverflow)]
pub fn multiply_ovf<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    sgn: NumberSign,
) -> StepResult {
    let v2 = stack.pop(gc);
    let v1 = stack.pop(gc);
    match v1.checked_mul(v2, sgn) {
        Ok(v) => {
            stack.push(gc, v);
            StepResult::InstructionStepped
        }
        Err(e) => stack.throw_by_name(gc, e),
    }
}

#[dotnet_instruction(Negate)]
pub fn negate<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
) -> StepResult {
    let v = stack.pop(gc);
    stack.push(gc, -v);
    StepResult::InstructionStepped
}

#[dotnet_instruction(Not)]
pub fn not<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
) -> StepResult {
    let v = stack.pop(gc);
    stack.push(gc, !v);
    StepResult::InstructionStepped
}

#[dotnet_instruction(Or)]
pub fn or<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
) -> StepResult {
    let v2 = stack.pop(gc);
    let v1 = stack.pop(gc);
    stack.push(gc, v1 | v2);
    StepResult::InstructionStepped
}

#[dotnet_instruction(Remainder)]
pub fn remainder<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    sgn: NumberSign,
) -> StepResult {
    let v2 = stack.pop(gc);
    let v1 = stack.pop(gc);
    match v1.rem(v2, sgn) {
        Ok(v) => {
            stack.push(gc, v);
            StepResult::InstructionStepped
        }
        Err(e) => stack.throw_by_name(gc, e),
    }
}

#[dotnet_instruction(ShiftLeft)]
pub fn shl<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
) -> StepResult {
    let v2 = stack.pop(gc);
    let v1 = stack.pop(gc);
    stack.push(gc, v1 << v2);
    StepResult::InstructionStepped
}

#[dotnet_instruction(ShiftRight)]
pub fn shr<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    sgn: NumberSign,
) -> StepResult {
    let v2 = stack.pop(gc);
    let v1 = stack.pop(gc);
    stack.push(gc, v1.shr(v2, sgn));
    StepResult::InstructionStepped
}

#[dotnet_instruction(Subtract)]
pub fn subtract<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
) -> StepResult {
    let v2 = stack.pop(gc);
    let v1 = stack.pop(gc);
    stack.push(gc, v1 - v2);
    StepResult::InstructionStepped
}

#[dotnet_instruction(SubtractOverflow)]
pub fn subtract_ovf<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    sgn: NumberSign,
) -> StepResult {
    let v2 = stack.pop(gc);
    let v1 = stack.pop(gc);
    match v1.checked_sub(v2, sgn) {
        Ok(v) => {
            stack.push(gc, v);
            StepResult::InstructionStepped
        }
        Err(e) => stack.throw_by_name(gc, e),
    }
}

#[dotnet_instruction(Xor)]
pub fn xor<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
) -> StepResult {
    let v2 = stack.pop(gc);
    let v1 = stack.pop(gc);
    stack.push(gc, v1 ^ v2);
    StepResult::InstructionStepped
}

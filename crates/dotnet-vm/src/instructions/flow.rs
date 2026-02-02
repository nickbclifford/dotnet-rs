use crate::exceptions::{ExceptionState, HandlerAddress, UnwindTarget};
use crate::{CallStack, StepResult};
use dotnet_macros::dotnet_instruction;
use dotnet_utils::gc::GCHandle;
use dotnet_value::StackValue;
use dotnetdll::prelude::*;

#[dotnet_instruction(Branch)]
pub fn br<'gc, 'm: 'gc>(
    _gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    target: usize,
) -> StepResult {
    stack.branch(target);
    StepResult::InstructionJumped
}

#[dotnet_instruction(BranchEqual)]
pub fn beq<'gc, 'm: 'gc>(
    _gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    target: usize,
) -> StepResult {
    let v2 = stack.pop(_gc);
    let v1 = stack.pop(_gc);
    if stack.conditional_branch(v1 == v2, target) {
        StepResult::InstructionJumped
    } else {
        StepResult::InstructionStepped
    }
}

#[dotnet_instruction(BranchGreaterOrEqual)]
pub fn bge<'gc, 'm: 'gc>(
    _gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    sgn: NumberSign,
    target: usize,
) -> StepResult {
    let v2 = stack.pop(_gc);
    let v1 = stack.pop(_gc);
    let cond = matches!(v1.compare(&v2, sgn), Some(std::cmp::Ordering::Greater) | Some(std::cmp::Ordering::Equal));
    if stack.conditional_branch(cond, target) {
        StepResult::InstructionJumped
    } else {
        StepResult::InstructionStepped
    }
}

#[dotnet_instruction(BranchGreater)]
pub fn bgt<'gc, 'm: 'gc>(
    _gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    sgn: NumberSign,
    target: usize,
) -> StepResult {
    let v2 = stack.pop(_gc);
    let v1 = stack.pop(_gc);
    let cond = matches!(v1.compare(&v2, sgn), Some(std::cmp::Ordering::Greater));
    if stack.conditional_branch(cond, target) {
        StepResult::InstructionJumped
    } else {
        StepResult::InstructionStepped
    }
}

#[dotnet_instruction(BranchLessOrEqual)]
pub fn ble<'gc, 'm: 'gc>(
    _gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    sgn: NumberSign,
    target: usize,
) -> StepResult {
    let v2 = stack.pop(_gc);
    let v1 = stack.pop(_gc);
    let cond = matches!(v1.compare(&v2, sgn), Some(std::cmp::Ordering::Less) | Some(std::cmp::Ordering::Equal));
    if stack.conditional_branch(cond, target) {
        StepResult::InstructionJumped
    } else {
        StepResult::InstructionStepped
    }
}

#[dotnet_instruction(BranchLess)]
pub fn blt<'gc, 'm: 'gc>(
    _gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    sgn: NumberSign,
    target: usize,
) -> StepResult {
    let v2 = stack.pop(_gc);
    let v1 = stack.pop(_gc);
    let cond = matches!(v1.compare(&v2, sgn), Some(std::cmp::Ordering::Less));
    if stack.conditional_branch(cond, target) {
        StepResult::InstructionJumped
    } else {
        StepResult::InstructionStepped
    }
}

#[dotnet_instruction(BranchNotEqual)]
pub fn bne<'gc, 'm: 'gc>(
    _gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    target: usize,
) -> StepResult {
    let v2 = stack.pop(_gc);
    let v1 = stack.pop(_gc);
    if stack.conditional_branch(v1 != v2, target) {
        StepResult::InstructionJumped
    } else {
        StepResult::InstructionStepped
    }
}

#[dotnet_instruction(BranchTruthy)]
pub fn brtrue<'gc, 'm: 'gc>(
    _gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    target: usize,
) -> StepResult {
    let v = stack.pop(_gc);
    if stack.conditional_branch(!is_nullish(v), target) {
        StepResult::InstructionJumped
    } else {
        StepResult::InstructionStepped
    }
}

#[dotnet_instruction(BranchFalsy)]
pub fn brfalse<'gc, 'm: 'gc>(
    _gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    target: usize,
) -> StepResult {
    let v = stack.pop(_gc);
    if stack.conditional_branch(is_nullish(v), target) {
        StepResult::InstructionJumped
    } else {
        StepResult::InstructionStepped
    }
}

#[dotnet_instruction(Return)]
pub fn ret<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
) -> StepResult {
    let frame_index = stack.execution.frames.len() - 1;
    if let ExceptionState::ExecutingHandler {
        exception, cursor, ..
    } = stack.execution.exception_mode
    {
        if cursor.frame_index == frame_index {
            stack.execution.exception_mode = ExceptionState::Unwinding {
                exception,
                target: UnwindTarget::Instruction(usize::MAX),
                cursor,
            };
            return stack.handle_exception(gc);
        }
    }

    let has_finally_blocks = !stack.state().info_handle.exceptions.is_empty();

    if has_finally_blocks {
        stack.execution.exception_mode = ExceptionState::Unwinding {
            exception: None,
            target: UnwindTarget::Instruction(usize::MAX),
            cursor: HandlerAddress {
                frame_index,
                section_index: 0,
                handler_index: 0,
            },
        };
        return stack.handle_exception(gc);
    }

    StepResult::MethodReturned
}

fn is_nullish(val: StackValue) -> bool {
    match val {
        StackValue::Int32(i) => i == 0,
        StackValue::Int64(i) => i == 0,
        StackValue::NativeInt(i) => i == 0,
        StackValue::ObjectRef(r) => r.0.is_none(),
        StackValue::UnmanagedPtr(_) | StackValue::ManagedPtr(_) => false,
        v => panic!("invalid type on stack ({:?}) for truthiness check", v),
    }
}

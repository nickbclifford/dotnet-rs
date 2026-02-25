use crate::{
    StepResult,
    stack::ops::{EvalStackOps, ExceptionOps},
};

const INVALID_PROGRAM_MSG: &str = "Common Language Runtime detected an invalid program.";
use dotnet_macros::dotnet_instruction;
use dotnet_value::StackValue;
use dotnetdll::prelude::*;

#[dotnet_instruction(Branch(target))]
pub fn br<T: ?Sized>(_ctx: &mut T, target: usize) -> StepResult {
    StepResult::Jump(target)
}

#[dotnet_instruction(BranchEqual(target))]
pub fn beq<'gc, T: EvalStackOps<'gc> + ?Sized>(ctx: &mut T, target: usize) -> StepResult {
    let v2 = ctx.pop();
    let v1 = ctx.pop();
    if v1 == v2 {
        StepResult::Jump(target)
    } else {
        StepResult::Continue
    }
}

#[dotnet_instruction(BranchGreaterOrEqual(sgn, target))]
pub fn bge<'gc, T: EvalStackOps<'gc> + ?Sized>(
    ctx: &mut T,

    sgn: NumberSign,
    target: usize,
) -> StepResult {
    let v2 = ctx.pop();
    let v1 = ctx.pop();
    let cond = matches!(
        v1.compare(&v2, sgn),
        Some(std::cmp::Ordering::Greater) | Some(std::cmp::Ordering::Equal)
    );
    if cond {
        StepResult::Jump(target)
    } else {
        StepResult::Continue
    }
}

#[dotnet_instruction(BranchGreater(sgn, target))]
pub fn bgt<'gc, T: EvalStackOps<'gc> + ?Sized>(
    ctx: &mut T,

    sgn: NumberSign,
    target: usize,
) -> StepResult {
    let v2 = ctx.pop();
    let v1 = ctx.pop();
    let cond = matches!(v1.compare(&v2, sgn), Some(std::cmp::Ordering::Greater));
    if cond {
        StepResult::Jump(target)
    } else {
        StepResult::Continue
    }
}

#[dotnet_instruction(BranchLessOrEqual(sgn, target))]
pub fn ble<'gc, T: EvalStackOps<'gc> + ?Sized>(
    ctx: &mut T,

    sgn: NumberSign,
    target: usize,
) -> StepResult {
    let v2 = ctx.pop();
    let v1 = ctx.pop();
    let cond = matches!(
        v1.compare(&v2, sgn),
        Some(std::cmp::Ordering::Less) | Some(std::cmp::Ordering::Equal)
    );
    if cond {
        StepResult::Jump(target)
    } else {
        StepResult::Continue
    }
}

#[dotnet_instruction(BranchLess(sgn, target))]
pub fn blt<'gc, T: EvalStackOps<'gc> + ?Sized>(
    ctx: &mut T,

    sgn: NumberSign,
    target: usize,
) -> StepResult {
    let v2 = ctx.pop();
    let v1 = ctx.pop();
    let cond = matches!(v1.compare(&v2, sgn), Some(std::cmp::Ordering::Less));
    if cond {
        StepResult::Jump(target)
    } else {
        StepResult::Continue
    }
}

#[dotnet_instruction(BranchNotEqual(target))]
pub fn bne<'gc, T: EvalStackOps<'gc> + ?Sized>(ctx: &mut T, target: usize) -> StepResult {
    let v2 = ctx.pop();
    let v1 = ctx.pop();
    if v1 != v2 {
        StepResult::Jump(target)
    } else {
        StepResult::Continue
    }
}

#[dotnet_instruction(BranchTruthy(target))]
pub fn brtrue<'gc, T: EvalStackOps<'gc> + ExceptionOps<'gc> + ?Sized>(
    ctx: &mut T,
    target: usize,
) -> StepResult {
    let v = ctx.pop();
    let cond = match v {
        StackValue::Int32(i) => i != 0,
        StackValue::Int64(i) => i != 0,
        StackValue::NativeInt(i) => i != 0,
        StackValue::ObjectRef(r) => r.0.is_some(),
        StackValue::UnmanagedPtr(_) | StackValue::ManagedPtr(_) => true,
        _ => {
            return ctx
                .throw_by_name_with_message("System.InvalidProgramException", INVALID_PROGRAM_MSG);
        }
    };
    if cond {
        StepResult::Jump(target)
    } else {
        StepResult::Continue
    }
}

#[dotnet_instruction(BranchFalsy(target))]
pub fn brfalse<'gc, T: EvalStackOps<'gc> + ExceptionOps<'gc> + ?Sized>(
    ctx: &mut T,
    target: usize,
) -> StepResult {
    let v = ctx.pop();
    let cond = match v {
        StackValue::Int32(i) => i == 0,
        StackValue::Int64(i) => i == 0,
        StackValue::NativeInt(i) => i == 0,
        StackValue::ObjectRef(r) => r.0.is_none(),
        StackValue::UnmanagedPtr(_) | StackValue::ManagedPtr(_) => false,
        _ => {
            return ctx
                .throw_by_name_with_message("System.InvalidProgramException", INVALID_PROGRAM_MSG);
        }
    };
    if cond {
        StepResult::Jump(target)
    } else {
        StepResult::Continue
    }
}

#[dotnet_instruction(Switch(targets))]
pub fn switch<'gc, T: EvalStackOps<'gc> + ExceptionOps<'gc> + ?Sized>(
    ctx: &mut T,
    targets: &[usize],
) -> StepResult {
    let index = match ctx.pop() {
        StackValue::Int32(i) => i as u32 as usize,
        StackValue::NativeInt(i) => i as usize,
        _ => {
            return ctx
                .throw_by_name_with_message("System.InvalidProgramException", INVALID_PROGRAM_MSG);
        }
    };

    if index < targets.len() {
        StepResult::Jump(targets[index])
    } else {
        StepResult::Continue
    }
}

#[dotnet_instruction(Return)]
pub fn ret<'gc, T: ExceptionOps<'gc> + ?Sized>(ctx: &mut T) -> StepResult {
    ctx.ret()
}

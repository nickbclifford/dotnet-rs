use crate::{
    StepResult,
    stack::ops::{ExceptionOps, StackOps},
};
use dotnet_macros::dotnet_instruction;
use dotnet_utils::gc::GCHandle;
use dotnet_value::StackValue;
use dotnetdll::prelude::*;

#[dotnet_instruction(Branch(target))]
pub fn br<'gc, 'm: 'gc, T: StackOps<'gc, 'm> + ?Sized>(
    _ctx: &mut T,
    _gc: GCHandle<'gc>,
    target: usize,
) -> StepResult {
    StepResult::Jump(target)
}

#[dotnet_instruction(BranchEqual(target))]
pub fn beq<'gc, 'm: 'gc, T: StackOps<'gc, 'm> + ?Sized>(
    ctx: &mut T,
    gc: GCHandle<'gc>,
    target: usize,
) -> StepResult {
    let v2 = ctx.pop(gc);
    let v1 = ctx.pop(gc);
    if v1 == v2 {
        StepResult::Jump(target)
    } else {
        StepResult::Continue
    }
}

#[dotnet_instruction(BranchGreaterOrEqual(sgn, target))]
pub fn bge<'gc, 'm: 'gc, T: StackOps<'gc, 'm> + ?Sized>(
    ctx: &mut T,
    gc: GCHandle<'gc>,
    sgn: NumberSign,
    target: usize,
) -> StepResult {
    let v2 = ctx.pop(gc);
    let v1 = ctx.pop(gc);
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
pub fn bgt<'gc, 'm: 'gc, T: StackOps<'gc, 'm> + ?Sized>(
    ctx: &mut T,
    gc: GCHandle<'gc>,
    sgn: NumberSign,
    target: usize,
) -> StepResult {
    let v2 = ctx.pop(gc);
    let v1 = ctx.pop(gc);
    let cond = matches!(v1.compare(&v2, sgn), Some(std::cmp::Ordering::Greater));
    if cond {
        StepResult::Jump(target)
    } else {
        StepResult::Continue
    }
}

#[dotnet_instruction(BranchLessOrEqual(sgn, target))]
pub fn ble<'gc, 'm: 'gc, T: StackOps<'gc, 'm> + ?Sized>(
    ctx: &mut T,
    gc: GCHandle<'gc>,
    sgn: NumberSign,
    target: usize,
) -> StepResult {
    let v2 = ctx.pop(gc);
    let v1 = ctx.pop(gc);
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
pub fn blt<'gc, 'm: 'gc, T: StackOps<'gc, 'm> + ?Sized>(
    ctx: &mut T,
    gc: GCHandle<'gc>,
    sgn: NumberSign,
    target: usize,
) -> StepResult {
    let v2 = ctx.pop(gc);
    let v1 = ctx.pop(gc);
    let cond = matches!(v1.compare(&v2, sgn), Some(std::cmp::Ordering::Less));
    if cond {
        StepResult::Jump(target)
    } else {
        StepResult::Continue
    }
}

#[dotnet_instruction(BranchNotEqual(target))]
pub fn bne<'gc, 'm: 'gc, T: StackOps<'gc, 'm> + ?Sized>(
    ctx: &mut T,
    gc: GCHandle<'gc>,
    target: usize,
) -> StepResult {
    let v2 = ctx.pop(gc);
    let v1 = ctx.pop(gc);
    if v1 != v2 {
        StepResult::Jump(target)
    } else {
        StepResult::Continue
    }
}

#[dotnet_instruction(BranchTruthy(target))]
pub fn brtrue<'gc, 'm: 'gc, T: StackOps<'gc, 'm> + ?Sized>(
    ctx: &mut T,
    gc: GCHandle<'gc>,
    target: usize,
) -> StepResult {
    let v = ctx.pop(gc);
    if !is_nullish(v) {
        StepResult::Jump(target)
    } else {
        StepResult::Continue
    }
}

#[dotnet_instruction(BranchFalsy(target))]
pub fn brfalse<'gc, 'm: 'gc, T: StackOps<'gc, 'm> + ?Sized>(
    ctx: &mut T,
    gc: GCHandle<'gc>,
    target: usize,
) -> StepResult {
    let v = ctx.pop(gc);
    if is_nullish(v) {
        StepResult::Jump(target)
    } else {
        StepResult::Continue
    }
}

#[dotnet_instruction(Switch(targets))]
pub fn switch<'gc, 'm: 'gc, T: StackOps<'gc, 'm> + ?Sized>(
    ctx: &mut T,
    gc: GCHandle<'gc>,
    targets: &[usize],
) -> StepResult {
    let index = match ctx.pop(gc) {
        StackValue::Int32(i) => i as u32 as usize,
        StackValue::NativeInt(i) => i as usize,
        v => panic!("invalid type on stack ({:?}) for switch instruction", v),
    };

    if index < targets.len() {
        StepResult::Jump(targets[index])
    } else {
        StepResult::Continue
    }
}

#[dotnet_instruction(Return)]
pub fn ret<'gc, 'm: 'gc, T: ExceptionOps<'gc> + ?Sized>(
    ctx: &mut T,
    gc: GCHandle<'gc>,
) -> StepResult {
    ctx.ret(gc)
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

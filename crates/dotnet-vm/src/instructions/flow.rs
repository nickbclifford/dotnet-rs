use crate::{
    StepResult,
    exceptions::{ExceptionState, HandlerAddress, HandlerKind, UnwindState, UnwindTarget},
    stack::VesContext,
};
use dotnet_macros::dotnet_instruction;
use dotnet_utils::gc::GCHandle;
use dotnet_value::StackValue;
use dotnetdll::prelude::*;

#[dotnet_instruction(Branch(target))]
pub fn br<'gc, 'm: 'gc>(
    _ctx: &mut VesContext<'_, 'gc, 'm>,
    _gc: GCHandle<'gc>,
    target: usize,
) -> StepResult {
    StepResult::Jump(target)
}

#[dotnet_instruction(BranchEqual(target))]
pub fn beq<'gc, 'm: 'gc>(
    ctx: &mut VesContext<'_, 'gc, 'm>,
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
pub fn bge<'gc, 'm: 'gc>(
    ctx: &mut VesContext<'_, 'gc, 'm>,
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
pub fn bgt<'gc, 'm: 'gc>(
    ctx: &mut VesContext<'_, 'gc, 'm>,
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
pub fn ble<'gc, 'm: 'gc>(
    ctx: &mut VesContext<'_, 'gc, 'm>,
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
pub fn blt<'gc, 'm: 'gc>(
    ctx: &mut VesContext<'_, 'gc, 'm>,
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
pub fn bne<'gc, 'm: 'gc>(
    ctx: &mut VesContext<'_, 'gc, 'm>,
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
pub fn brtrue<'gc, 'm: 'gc>(
    ctx: &mut VesContext<'_, 'gc, 'm>,
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
pub fn brfalse<'gc, 'm: 'gc>(
    ctx: &mut VesContext<'_, 'gc, 'm>,
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
pub fn switch<'gc, 'm: 'gc>(
    ctx: &mut VesContext<'_, 'gc, 'm>,
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
pub fn ret<'gc, 'm: 'gc>(ctx: &mut VesContext<'_, 'gc, 'm>, gc: GCHandle<'gc>) -> StepResult {
    let frame_index = ctx.frame_stack.len() - 1;
    if let ExceptionState::ExecutingHandler(state) = *ctx.exception_mode
        && state.cursor.frame_index == frame_index
    {
        *ctx.exception_mode = ExceptionState::Unwinding(UnwindState {
            exception: state.exception,
            target: UnwindTarget::Instruction(usize::MAX),
            cursor: state.cursor,
        });
        return ctx.handle_exception(gc);
    }

    let ip = ctx.state().ip;
    let mut has_finally_blocks = false;
    for section in &ctx.state().info_handle.exceptions {
        if section.instructions.contains(&ip)
            && section
                .handlers
                .iter()
                .any(|h| matches!(h.kind, HandlerKind::Finally | HandlerKind::Fault))
        {
            has_finally_blocks = true;
            break;
        }
    }

    if has_finally_blocks {
        *ctx.exception_mode = ExceptionState::Unwinding(UnwindState {
            exception: None,
            target: UnwindTarget::Instruction(usize::MAX),
            cursor: HandlerAddress {
                frame_index,
                section_index: 0,
                handler_index: 0,
            },
        });
        return ctx.handle_exception(gc);
    }

    StepResult::Return
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

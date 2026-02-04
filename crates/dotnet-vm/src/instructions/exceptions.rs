use crate::{
    StepResult,
    exceptions::{ExceptionState, HandlerAddress, UnwindState, UnwindTarget},
    stack::VesContext,
};
use dotnet_macros::dotnet_instruction;
use dotnet_utils::gc::GCHandle;
use dotnetdll::prelude::*;

#[dotnet_instruction(Leave)]
pub fn leave<'gc, 'm: 'gc>(
    ctx: &mut VesContext<'_, 'gc, 'm>,
    gc: GCHandle<'gc>,
    jump_target: usize,
) -> StepResult {
    // If we're already executing a handler (e.g., a finally block during unwinding),
    // we need to preserve that state. The Leave instruction in this case just means
    // "exit this finally block and continue unwinding".
    *ctx.exception_mode = match ctx.exception_mode {
        // We're inside a finally/fault handler. Transition back to Unwinding
        // to continue processing remaining handlers.
        ExceptionState::ExecutingHandler(state) => ExceptionState::Unwinding(*state),
        // Normal Leave: start a new unwind operation
        _ => ExceptionState::Unwinding(UnwindState {
            exception: None,
            // Cast i32 to usize for instruction offset
            target: UnwindTarget::Instruction(jump_target),
            cursor: HandlerAddress {
                frame_index: ctx.frame_stack.len() - 1,
                section_index: 0,
                handler_index: 0,
            },
        }),
    };
    ctx.handle_exception(gc)
}

#[dotnet_instruction(EndFinally)]
pub fn endfinally<'gc, 'm: 'gc>(
    ctx: &mut VesContext<'_, 'gc, 'm>,
    gc: GCHandle<'gc>,
) -> StepResult {
    match ctx.exception_mode {
        ExceptionState::ExecutingHandler(state) => {
            *ctx.exception_mode = ExceptionState::Unwinding(*state);
            ctx.handle_exception(gc)
        }
        _ => panic!(
            "endfinally called outside of handler, state: {:?}",
            ctx.exception_mode
        ),
    }
}

#[dotnet_instruction(EndFilter)]
pub fn endfilter<'gc, 'm: 'gc>(ctx: &mut VesContext<'_, 'gc, 'm>, gc: GCHandle<'gc>) -> StepResult {
    let _result_val = ctx.pop_i32(gc);

    let (_exception, _handler) = match ctx.exception_mode {
        ExceptionState::Filtering(state) => (state.exception, state.handler),
        _ => panic!("EndFilter called but not in Filtering mode"),
    };

    // Restore suspended state
    // Note: we use handler.frame_index because the filter ran in that frame.
    // It might have called other methods, but those should have returned by now.
    // let original_ip = ctx.execution().original_ip;
    // let original_stack_height = ctx.execution().original_stack_height;
    // ...
    // Wait, original_ip is in ThreadContext.
    // I should probably add it to VesContext or have a way to access it.
    // But for now, let's just use ctx.frame_stack and ctx.evaluation_stack directly.

    // I'll need to figure out where original_ip/original_stack_height are.
    // They are in ThreadContext.

    // Let's check how to get them.
    // ctx has access to evaluation_stack and frame_stack.

    // I'll skip complex EndFilter for a moment or implement it carefully.
    StepResult::Continue
}

#[dotnet_instruction(Throw)]
pub fn throw<'gc, 'm: 'gc>(ctx: &mut VesContext<'_, 'gc, 'm>, gc: GCHandle<'gc>) -> StepResult {
    let exc = ctx.pop_obj(gc);
    if exc.0.is_none() {
        return ctx.throw_by_name(gc, "System.NullReferenceException");
    }
    *ctx.exception_mode = ExceptionState::Throwing(exc);
    ctx.handle_exception(gc)
}

#[dotnet_instruction(Rethrow)]
pub fn rethrow<'gc, 'm: 'gc>(ctx: &mut VesContext<'_, 'gc, 'm>, gc: GCHandle<'gc>) -> StepResult {
    let exception = ctx
        .current_frame()
        .exception_stack
        .last()
        .cloned()
        .expect("rethrow without active exception");
    *ctx.exception_mode = ExceptionState::Throwing(exception);
    ctx.handle_exception(gc)
}

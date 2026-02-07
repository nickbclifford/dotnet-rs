use crate::{
    StepResult,
    exceptions::{ExceptionState, HandlerAddress, UnwindState, UnwindTarget},
    stack::VesContext,
};
use dotnet_macros::dotnet_instruction;
use dotnet_utils::gc::GCHandle;
use dotnetdll::prelude::*;

#[dotnet_instruction(Leave(jump_target))]
pub fn leave<'gc, 'm: 'gc>(
    ctx: &mut VesContext<'_, 'gc, 'm>,
    gc: GCHandle<'gc>,
    jump_target: usize,
) -> StepResult {
    // Always start unwinding from the beginning of the current frame to ensure
    // that any nested handlers (e.g. finally blocks within the current handler)
    // are properly discovered and executed.
    let cursor = HandlerAddress {
        frame_index: ctx.frame_stack.len() - 1,
        section_index: 0,
        handler_index: 0,
    };

    *ctx.exception_mode = ExceptionState::Unwinding(UnwindState {
        exception: None,
        target: UnwindTarget::Instruction(jump_target),
        cursor,
    });
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
    use crate::exceptions::SearchState;

    let result_val = ctx.pop_i32(gc);

    let (exception, handler) = match ctx.exception_mode {
        ExceptionState::Filtering(state) => (state.exception, state.handler),
        _ => panic!("EndFilter called but not in Filtering mode"),
    };

    // Restore state of the frame where filter ran
    let frame = &mut ctx.frame_stack.frames[handler.frame_index];
    frame.state.ip = *ctx.original_ip;
    frame.stack_height = *ctx.original_stack_height;
    frame.exception_stack.pop();

    // Restore suspended evaluation and frame stacks
    ctx.evaluation_stack.restore_suspended();
    ctx.frame_stack.restore_suspended();

    if result_val == 1 {
        // Result is 1: This handler is the one. Begin unwinding towards it.
        *ctx.exception_mode = ExceptionState::Unwinding(UnwindState {
            exception: Some(exception),
            target: UnwindTarget::Handler(handler),
            cursor: HandlerAddress {
                frame_index: ctx.frame_stack.len() - 1,
                section_index: 0,
                handler_index: 0,
            },
        });
    } else {
        // Result is 0: Continue searching after this handler.
        *ctx.exception_mode = ExceptionState::Searching(SearchState {
            exception,
            cursor: HandlerAddress {
                frame_index: handler.frame_index,
                section_index: handler.section_index,
                handler_index: handler.handler_index + 1,
            },
        });
    }

    StepResult::Exception
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

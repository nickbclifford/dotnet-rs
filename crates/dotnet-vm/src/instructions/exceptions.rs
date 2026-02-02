use crate::instructions::StepResult;
use crate::{CallStack, vm_pop};
use dotnet_utils::gc::GCHandle;
use dotnet_value::StackValue;
use dotnet_macros::dotnet_instruction;
use dotnetdll::prelude::*;
use crate::exceptions::{ExceptionState, HandlerAddress, UnwindTarget};

#[dotnet_instruction(Leave)]
pub fn leave<'gc, 'm>(gc: GCHandle<'gc>, stack: &mut CallStack<'gc, 'm>, jump_target: usize) -> StepResult
where 'm: 'gc {
    // If we're already executing a handler (e.g., a finally block during unwinding),
    // we need to preserve that state. The Leave instruction in this case just means
    // "exit this finally block and continue unwinding".
    stack.execution.exception_mode = match stack.execution.exception_mode {
        // We're inside a finally/fault handler. Transition back to Unwinding
        // to continue processing remaining handlers.
        ExceptionState::ExecutingHandler {
            exception,
            target,
            cursor,
        } => ExceptionState::Unwinding {
            exception,
            target,
            cursor,
        },
        // Normal Leave: start a new unwind operation
        _ => ExceptionState::Unwinding {
            exception: None,
            // Cast i32 to usize for instruction offset
            target: UnwindTarget::Instruction(jump_target),
            cursor: HandlerAddress {
                frame_index: stack.execution.frames.len() - 1,
                section_index: 0,
                handler_index: 0,
            },
        },
    };
    stack.handle_exception(gc)
}

#[dotnet_instruction(EndFinally)]
pub fn endfinally<'gc, 'm>(gc: GCHandle<'gc>, stack: &mut CallStack<'gc, 'm>) -> StepResult
where 'm: 'gc {
    match stack.execution.exception_mode {
        ExceptionState::ExecutingHandler {
            exception,
            target,
            cursor,
        } => {
            stack.execution.exception_mode = ExceptionState::Unwinding {
                exception,
                target,
                cursor,
            };
            stack.handle_exception(gc)
        }
        _ => panic!(
            "endfinally called outside of handler, state: {:?}",
            stack.execution.exception_mode
        ),
    }
}

#[dotnet_instruction(EndFilter)]
pub fn endfilter<'gc, 'm>(gc: GCHandle<'gc>, stack: &mut CallStack<'gc, 'm>) -> StepResult
where 'm: 'gc {
    let result = vm_pop!(stack, gc);
    let result_val = match result {
        StackValue::Int32(i) => i,
        _ => panic!("EndFilter expected Int32, found {:?}", result),
    };

    let (exception, handler) = match stack.execution.exception_mode {
        ExceptionState::Filtering { exception, handler } => (exception, handler),
        _ => panic!("EndFilter called but not in Filtering mode"),
    };

    // Restore suspended state
    // Note: we use handler.frame_index because the filter ran in that frame.
    // It might have called other methods, but those should have returned by now.
    stack.execution
        .stack
        .truncate(stack.execution.frames[handler.frame_index].base.stack);
    stack.execution
        .stack
        .append(&mut stack.execution.suspended_stack);
    stack.execution
        .frames
        .append(&mut stack.execution.suspended_frames);

    let frame = &mut stack.execution.frames[handler.frame_index];
    frame.exception_stack.pop();
    frame.state.ip = stack.execution.original_ip;
    frame.stack_height = stack.execution.original_stack_height;

    if result_val != 0 {
        // Filter matched!
        stack.execution.exception_mode = ExceptionState::Unwinding {
            exception: Some(exception),
            target: UnwindTarget::Handler(handler),
            cursor: HandlerAddress {
                frame_index: stack.execution.frames.len() - 1,
                section_index: 0,
                handler_index: 0,
            },
        };
    } else {
        // Filter did not match, continue searching
        let mut next_cursor = handler;
        next_cursor.handler_index += 1;
        stack.execution.exception_mode = ExceptionState::Searching {
            exception,
            cursor: next_cursor,
        };
    }
    stack.handle_exception(gc)
}

#[dotnet_instruction(Throw)]
pub fn throw<'gc, 'm>(gc: GCHandle<'gc>, stack: &mut CallStack<'gc, 'm>) -> StepResult
where 'm: 'gc {
    let exc = vm_pop!(stack, gc);
    let StackValue::ObjectRef(exc) = exc else {
        panic!(
            "Throw expects an object reference on the stack, received {:?}",
            exc
        )
    };
    if exc.0.is_none() {
        return stack.throw_by_name(gc, "System.NullReferenceException");
    }
    stack.execution.exception_mode = ExceptionState::Throwing(exc);
    stack.handle_exception(gc)
}

#[dotnet_instruction(Rethrow)]
pub fn rethrow<'gc, 'm>(gc: GCHandle<'gc>, stack: &mut CallStack<'gc, 'm>) -> StepResult
where 'm: 'gc {
    let exception = stack
        .current_frame()
        .exception_stack
        .last()
        .cloned()
        .expect("rethrow without active exception");
    stack.execution.exception_mode = ExceptionState::Throwing(exception);
    stack.handle_exception(gc)
}

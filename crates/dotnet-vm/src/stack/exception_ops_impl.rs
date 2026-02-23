use crate::{
    StepResult,
    exceptions::{
        ExceptionState, HandlerAddress, HandlerKind, SearchState, UnwindState, UnwindTarget,
    },
    memory::ops::MemoryOps,
    stack::{
        context::VesContext,
        ops::{ExceptionOps, StackOps},
    },
};
use dotnet_value::{
    StackValue,
    object::{HeapStorage, ObjectRef},
    string::CLRString,
};

impl<'a, 'gc, 'm: 'gc> ExceptionOps<'gc> for VesContext<'a, 'gc, 'm> {
    #[inline]
    fn throw_by_name(&mut self, name: &str) -> StepResult {
        self.throw_by_name_with_message(name, "")
    }

    #[inline]
    fn throw_by_name_with_message(&mut self, name: &str, message: &str) -> StepResult {
        let gc = self.gc;
        let exception_type = vm_try!(self.shared.loader.corlib_type(name));
        let instance = vm_try!(self.new_object(exception_type));
        let obj_ref = ObjectRef::new(gc, HeapStorage::Obj(instance));
        self.register_new_object(&obj_ref);

        if !message.is_empty() {
            let base_exception_type = vm_try!(self.shared.loader.corlib_type("System.Exception"));
            let message_ref = StackValue::string(gc, CLRString::from(message)).as_object_ref();
            self.register_new_object(&message_ref);
            obj_ref.as_object_mut(gc, |obj| {
                if obj
                    .instance_storage
                    .has_field(base_exception_type, "_message")
                {
                    let mut field = obj
                        .instance_storage
                        .get_field_mut_local(base_exception_type, "_message");
                    message_ref.write(&mut field);
                }
            });
        }

        *self.exception_mode = ExceptionState::Throwing(obj_ref, false);
        StepResult::Exception
    }

    #[inline]
    fn throw(&mut self, exception: ObjectRef<'gc>) -> StepResult {
        *self.exception_mode = ExceptionState::Throwing(exception, false);
        self.handle_exception()
    }

    #[inline]
    fn rethrow(&mut self) -> StepResult {
        let exception = self
            .current_frame()
            .exception_stack
            .last()
            .cloned()
            .expect("rethrow without active exception");
        *self.exception_mode = ExceptionState::Throwing(exception, true);
        self.handle_exception()
    }

    #[inline]
    fn leave(&mut self, target_ip: usize) -> StepResult {
        let cursor = HandlerAddress {
            frame_index: self.frame_stack.len() - 1,
            section_index: 0,
            handler_index: 0,
        };

        *self.exception_mode = ExceptionState::Unwinding(UnwindState {
            exception: None,
            target: UnwindTarget::Instruction(target_ip),
            cursor,
        });
        self.handle_exception()
    }

    #[inline]
    fn endfinally(&mut self) -> StepResult {
        match self.exception_mode {
            ExceptionState::ExecutingHandler(state) => {
                *self.exception_mode = ExceptionState::Unwinding(*state);
                self.handle_exception()
            }
            _ => panic!(
                "endfinally called outside of handler, state: {:?}",
                self.exception_mode
            ),
        }
    }

    #[inline]
    fn endfilter(&mut self, result: i32) -> StepResult {
        let (exception, handler) = match self.exception_mode {
            ExceptionState::Filtering(state) => (state.exception, state.handler),
            _ => panic!("EndFilter called but not in Filtering mode"),
        };

        // Restore state of the frame where filter ran
        let frame = &mut self.frame_stack.frames[handler.frame_index];
        frame.state.ip = *self.original_ip;
        frame.stack_height = *self.original_stack_height;
        frame.exception_stack.pop();

        // Restore suspended evaluation and frame stacks
        self.evaluation_stack.restore_suspended();
        self.frame_stack.restore_suspended();

        if result == 1 {
            // Result is 1: This handler is the one. Begin unwinding towards it.
            *self.exception_mode = ExceptionState::Unwinding(UnwindState {
                exception: Some(exception),
                target: UnwindTarget::Handler(handler),
                cursor: HandlerAddress {
                    frame_index: self.frame_stack.len() - 1,
                    section_index: 0,
                    handler_index: 0,
                },
            });
        } else {
            // Result is 0: Continue searching after this handler.
            *self.exception_mode = ExceptionState::Searching(SearchState {
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

    #[inline]
    fn ret(&mut self) -> StepResult {
        let frame_index = self.frame_stack.len() - 1;
        if let ExceptionState::ExecutingHandler(state) = *self.exception_mode
            && state.cursor.frame_index == frame_index
        {
            *self.exception_mode = ExceptionState::Unwinding(UnwindState {
                exception: state.exception,
                target: UnwindTarget::Instruction(usize::MAX),
                cursor: state.cursor,
            });
            return self.handle_exception();
        }

        let ip = self.current_frame().state.ip;
        let mut has_finally_blocks = false;
        for section in &self.current_frame().state.info_handle.exceptions {
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
            *self.exception_mode = ExceptionState::Unwinding(UnwindState {
                exception: None,
                target: UnwindTarget::Instruction(usize::MAX),
                cursor: HandlerAddress {
                    frame_index,
                    section_index: 0,
                    handler_index: 0,
                },
            });
            return self.handle_exception();
        }

        StepResult::Return
    }
}

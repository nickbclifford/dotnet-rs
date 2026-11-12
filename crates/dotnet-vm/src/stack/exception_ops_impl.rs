use crate::{
    StepResult,
    error::{ExecutionError, VmError},
    stack::{
        context::VesContext,
        ops::{ExceptionOps, VmStackOps},
    },
};
use dotnet_runtime_memory::ops::BaseMemoryOps;
use dotnet_types::WellKnown;
use dotnet_value::{StackValue, object::ObjectRef, string::CLRString};
use dotnet_vm_data::{
    ExceptionState, HandlerAddress, SearchState, UnwindState, UnwindTarget, exceptions::HandlerKind,
};
use dotnet_vm_ops::VesInternals;

impl<'a, 'gc> VesContext<'a, 'gc> {
    /// Throws the `System.TypeInitializationException` that reports a failed `.cctor` of
    /// `description` (ECMA-335 §II.10.5.3.3), optionally wrapping the exception the `.cctor`
    /// threw. The exception's `TypeName` is the failing type's name, as in the CLR.
    pub(crate) fn throw_type_initialization_exception(
        &mut self,
        description: &dotnet_types::TypeDescription,
        inner: Option<ObjectRef<'gc>>,
    ) -> StepResult {
        const NAME: &str = "System.TypeInitializationException";
        let type_name = description.type_name();
        let message = format!("The type initializer for '{type_name}' threw an exception.");
        let result = match inner {
            Some(inner) => self.throw_by_name_with_inner(NAME, &message, inner),
            None => self.throw_by_name_with_message(NAME, &message),
        };
        if let (StepResult::Exception, ExceptionState::Throwing(exception, _)) =
            (&result, *self.exception_mode)
        {
            let gc = self.gc;
            let exception_type = dotnet_vm_ops::vm_try!(self.shared.loader.corlib_type(NAME));
            let name_ref =
                StackValue::string(gc, CLRString::from(type_name.as_str())).as_object_ref();
            self.register_new_object(&name_ref);
            // `_typeName` is private BCL layout, so it is looked up by name and skipped if absent.
            exception.as_object_mut(gc, |obj| {
                if let Some(field) = obj
                    .instance_storage
                    .field::<ObjectRef>(exception_type, "_typeName")
                {
                    field.write(name_ref);
                }
            });
        }
        result
    }
}

impl<'a, 'gc> ExceptionOps<'gc> for VesContext<'a, 'gc> {
    fn throw_argument_out_of_range(&mut self, param_name: &str) -> StepResult {
        let argument_type =
            dotnet_vm_ops::vm_try!(self.shared.loader.corlib_type("System.ArgumentException"));
        let result =
            self.throw_by_name_with_message("System.ArgumentOutOfRangeException", param_name);
        if let (StepResult::Exception, ExceptionState::Throwing(exception, _)) =
            (&result, *self.exception_mode)
        {
            let gc = self.gc;
            let param_ref = StackValue::string(gc, CLRString::from(param_name)).as_object_ref();
            self.register_new_object(&param_ref);
            exception.as_object_mut(gc, |obj| {
                if obj
                    .instance_storage
                    .has_field(argument_type.clone(), "_paramName")
                {
                    let mut field = obj
                        .instance_storage
                        .get_field_mut_local(argument_type, "_paramName");
                    param_ref.write(&mut field);
                }
            });
        }
        result
    }

    #[inline]
    fn throw_by_name_with_message(&mut self, name: &str, message: &str) -> StepResult {
        let gc = self.gc;
        let exception_type = dotnet_vm_ops::vm_try!(self.shared.loader.corlib_type(name));
        let instance = dotnet_vm_ops::vm_try!(self.new_object(exception_type));
        let obj_ref = self.alloc_obj_ref(gc, instance);

        let base_exception_type =
            dotnet_vm_ops::vm_try!(self.shared.loader.corlib_wkt(WellKnown::Exception));
        if !message.is_empty() {
            let message_ref = StackValue::string(gc, CLRString::from(message)).as_object_ref();
            self.register_new_object(&message_ref);
            // `Exception._message` is private BCL layout, so it is looked up by name and skipped if absent.
            obj_ref.as_object_mut(gc, |obj| {
                if let Some(field) = obj
                    .instance_storage
                    .field::<ObjectRef>(base_exception_type, "_message")
                {
                    field.write(message_ref);
                }
            });
        }

        if let ExceptionState::ExecutingHandler(state) = *self.exception_mode {
            self.push_handler_unwind(state);
        }
        *self.exception_mode = ExceptionState::Throwing(obj_ref, false);
        StepResult::Exception
    }

    #[inline]
    fn throw_by_name_with_inner(
        &mut self,
        name: &str,
        message: &str,
        inner: ObjectRef<'gc>,
    ) -> StepResult {
        let gc = self.gc;
        let exception_type = dotnet_vm_ops::vm_try!(self.shared.loader.corlib_type(name));
        let instance = dotnet_vm_ops::vm_try!(self.new_object(exception_type));
        let obj_ref = self.alloc_obj_ref(gc, instance);

        let base_exception_type =
            dotnet_vm_ops::vm_try!(self.shared.loader.corlib_wkt(WellKnown::Exception));
        if !message.is_empty() {
            let message_ref = StackValue::string(gc, CLRString::from(message)).as_object_ref();
            self.register_new_object(&message_ref);
            obj_ref.as_object_mut(gc, |obj| {
                if let Some(field) = obj
                    .instance_storage
                    .field::<ObjectRef>(base_exception_type.clone(), "_message")
                {
                    field.write(message_ref);
                }
            });
        }

        obj_ref.as_object_mut(gc, |obj| {
            if let Some(field) = obj
                .instance_storage
                .field::<ObjectRef>(base_exception_type, "_innerException")
            {
                field.write(inner);
            }
        });

        if let ExceptionState::ExecutingHandler(state) = *self.exception_mode {
            self.push_handler_unwind(state);
        }
        *self.exception_mode = ExceptionState::Throwing(obj_ref, false);
        StepResult::Exception
    }

    #[inline]
    fn throw(&mut self, exception: ObjectRef<'gc>) -> StepResult {
        if let ExceptionState::ExecutingHandler(state) = *self.exception_mode {
            self.push_handler_unwind(state);
        }
        *self.exception_mode = ExceptionState::Throwing(exception, false);
        self.handle_exception()
    }

    #[inline]
    fn rethrow(&mut self) -> StepResult {
        let Some(exception) = self.current_frame().exception_stack.last().cloned() else {
            return StepResult::Error(VmError::Execution(ExecutionError::InvalidCil(
                "rethrow without active exception".into(),
            )));
        };
        if let ExceptionState::ExecutingHandler(state) = *self.exception_mode {
            self.push_handler_unwind(state);
        }
        *self.exception_mode = ExceptionState::Throwing(exception, true);
        self.handle_exception()
    }

    #[inline]
    fn leave(&mut self, target_ip: usize) -> StepResult {
        let frame_index = self.frame_stack.len() - 1;
        // ECMA-335 III.3.46, III.3.35 and I.12.4.2.8.2.8 require a leave
        // inside a finally or fault to remain within that handler.
        if let ExceptionState::ExecutingHandler(state) = *self.exception_mode
            && state.cursor.frame_index <= frame_index
        {
            self.push_handler_unwind(state);
        }

        let cursor = HandlerAddress {
            frame_index,
            section_index: 0,
            handler_index: 0,
        };

        *self.exception_mode = ExceptionState::Unwinding(UnwindState {
            exception: None,
            target: UnwindTarget::Instruction(target_ip),
            cursor,
            nested_filter: None,
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
            _ => StepResult::Error(VmError::Execution(ExecutionError::InvalidCil(
                format!(
                    "endfinally called outside of handler, state: {:?}",
                    self.exception_mode
                )
                .into(),
            ))),
        }
    }

    #[inline]
    fn endfilter(&mut self, result: i32) -> StepResult {
        let (exception, handler, original_ip, original_stack_height) = match self.exception_mode {
            ExceptionState::Filtering(state) => (
                state.exception,
                state.handler,
                state.original_ip,
                state.original_stack_height,
            ),
            _ => {
                return StepResult::Error(VmError::Execution(ExecutionError::InvalidCil(
                    "EndFilter called but not in Filtering mode".into(),
                )));
            }
        };

        if let ExceptionState::Filtering(state) = *self.exception_mode {
            assert_eq!(self.frame_stack.suspension_depth(), state.suspension_depth);
            assert_eq!(
                self.evaluation_stack.suspension_depth(),
                state.suspension_depth
            );
        }

        // Restore state of the frame where filter ran
        let frame = &mut self.frame_stack.frames[handler.frame_index];
        frame.state.ip = original_ip;
        frame.stack_height = original_stack_height.saturating_sub_idx(frame.base.stack);
        frame.exception_stack.pop();

        // Restore suspended evaluation and frame stacks
        self.frame_stack
            .active_filters
            .pop()
            .expect("active filter missing");
        self.evaluation_stack.restore_suspended();
        self.frame_stack.restore_suspended();
        self.restore_suspended_native();

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
                nested_filter: None,
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
                nested_filter: self.frame_stack.active_filters.last().copied(),
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
            self.discard_handler_unwinds_from(frame_index);
            *self.exception_mode = ExceptionState::Unwinding(UnwindState {
                exception: state.exception,
                target: UnwindTarget::Instruction(usize::MAX),
                cursor: state.cursor,
                nested_filter: state.nested_filter,
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
                nested_filter: None,
            });
            return self.handle_exception();
        }

        StepResult::Return
    }
}

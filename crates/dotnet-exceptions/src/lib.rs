use dotnet_types::{
    error::{TypeResolutionError, VmError},
    members::MethodDescription,
};
use dotnet_utils::{StackSlotIndex, gc::GCHandle};
use dotnet_value::{
    StackValue,
    object::{HeapStorage, ObjectRef},
    string::CLRString,
};
use dotnetdll::prelude::*;
use std::{cmp::Reverse, collections::HashMap, ops::Range};

pub use dotnet_vm_ops::{
    StepResult,
    exceptions::*,
    ops::{ExceptionContext, ResolutionOps},
};

/// Extracts human-readable information from a managed exception object.
pub fn extract_managed_exception<'gc>(
    gc: &GCHandle<'gc>,
    exception: ObjectRef<'gc>,
    ctx: &dyn ExceptionContext<'gc>,
) -> ManagedException {
    let mut message = None;
    let mut stack_trace = None;

    let _exc_ref = exception
        .0
        .expect("Extracting information from a null exception reference");
    let exc_type = ctx
        .get_heap_description(exception)
        .expect("Failed to get type description for exception object");
    let type_name = exc_type.type_name();

    let exception_type = ctx
        .loader()
        .corlib_type("System.Exception")
        .expect("Failed to resolve System.Exception type");

    exception.as_object(|obj| {
        if obj.instance_storage.has_field(exception_type, "_message") {
            let message_bytes = obj
                .instance_storage
                .get_field_local(exception_type, "_message");
            // SAFETY: message_bytes contains a valid ObjectRef from the object's storage.
            let message_ref = unsafe { ObjectRef::read_branded(&message_bytes, gc) };
            if let Some(msg_inner) = message_ref.0 {
                let storage = &msg_inner.borrow().storage;
                if let HeapStorage::Str(clr_str) = storage {
                    message = Some(clr_str.as_string());
                }
            }
        }
        if obj
            .instance_storage
            .has_field(exception_type, "_stackTraceString")
        {
            let st_bytes = obj
                .instance_storage
                .get_field_local(exception_type, "_stackTraceString");
            // SAFETY: st_bytes contains a valid ObjectRef from the object's storage.
            let st_ref = unsafe { ObjectRef::read_branded(&st_bytes, gc) };
            if let Some(st_inner) = st_ref.0 {
                let storage = &st_inner.borrow().storage;
                if let HeapStorage::Str(clr_str) = storage {
                    stack_trace = Some(clr_str.as_string());
                }
            }
        }
    });

    ManagedException {
        type_name,
        message,
        stack_trace,
    }
}

/// Parses exception handler metadata from an assembly into a structured format.
pub fn parse<'a>(
    source: impl IntoIterator<Item = &'a body::Exception>,
    ctx: &dyn ResolutionOps<'_>,
) -> Result<Vec<ProtectedSection>, TypeResolutionError> {
    let mut sections: HashMap<Range<usize>, Vec<Handler>> = HashMap::new();
    for exc in source {
        use body::ExceptionKind::*;
        let try_range = exc.try_offset..exc.try_offset + exc.try_length;
        let handler_range = exc.handler_offset..exc.handler_offset + exc.handler_length;

        let kind = match &exc.kind {
            TypedException(t) => HandlerKind::Catch(ctx.make_concrete(t)?),
            Filter { offset } => HandlerKind::Filter {
                clause_offset: *offset,
            },
            Finally => HandlerKind::Finally,
            Fault => HandlerKind::Fault,
        };

        sections.entry(try_range).or_default().push(Handler {
            instructions: handler_range,
            kind,
        });
    }

    let mut v: Vec<_> = sections
        .into_iter()
        .map(|(instructions, handlers)| ProtectedSection {
            instructions,
            handlers,
        })
        .collect();

    // Sort sections such that inner blocks come before outer blocks.
    // This ensures that when searching for a handler, we find the most specific one first.
    v.sort_by_key(|s| (Reverse(s.instructions.start), s.instructions.end));
    Ok(v)
}

pub struct ExceptionHandlingSystem;

impl ExceptionHandlingSystem {
    pub fn handle_exception<'gc>(
        &self,
        ctx: &mut dyn ExceptionContext<'gc>,
        gc: GCHandle<'gc>,
    ) -> StepResult {
        match *ctx.exception_mode() {
            // IMPORTANT: Never return Continue from exception handling states.
            // Returning Continue allows the optimized batch loop to keep executing
            // instructions without re-checking the current frame/exception state,
            // which can lead to corruption after stack or state transitions.
            ExceptionState::None => StepResult::Exception,
            ExceptionState::Throwing(exception, preserve) => {
                self.begin_throwing(ctx, exception, gc, preserve)
            }
            ExceptionState::Searching(state) => {
                self.search_for_handler(ctx, gc, state.exception, state.cursor)
            }
            ExceptionState::Unwinding(state) => {
                self.unwind(
                    ctx,
                    gc,
                    state.exception,
                    state.target,
                    state.cursor,
                    state.nested_filter,
                )
            }
            ExceptionState::Filtering(_) | ExceptionState::ExecutingHandler(_) => {
                StepResult::Exception
            }
        }
    }

    fn begin_throwing<'gc>(
        &self,
        ctx: &mut dyn ExceptionContext<'gc>,
        exception: ObjectRef<'gc>,
        gc: GCHandle<'gc>,
        preserve_stack_trace: bool,
    ) -> StepResult {
        let frame = ctx.frame_stack().current_frame();
        if ctx.tracer_enabled() {
            ctx.tracer().trace_exception(
                ctx.indent(),
                &format!("{:?}", exception),
                &format!(
                    "{:?} at IP {}",
                    frame.state.info_handle.source, frame.state.ip
                ),
            );
        }

        // Capture and store stack trace
        let exception_type = match ctx.loader().corlib_type("System.Exception") {
            Ok(t) => t,
            Err(e) => return StepResult::Error(e.into()),
        };

        let mut existing_trace = None;
        if preserve_stack_trace {
            exception.as_object(|obj| {
                if obj
                    .instance_storage
                    .has_field(exception_type, "_stackTraceString")
                {
                    let st_bytes = obj
                        .instance_storage
                        .get_field_local(exception_type, "_stackTraceString");
                    // SAFETY: st_bytes contains a valid ObjectRef from the object's storage.
                    let st_ref = unsafe { ObjectRef::read_branded(&st_bytes, &gc) };
                    if let Some(st_inner) = st_ref.0 {
                        let storage = &st_inner.borrow().storage;
                        if let HeapStorage::Str(clr_str) = storage {
                            existing_trace = Some(clr_str.as_string());
                        }
                    }
                }
            });
        }

        if !preserve_stack_trace || existing_trace.is_none() {
            let mut trace = String::new();

            let format_params = |method: &MethodDescription| {
                let mut params_str = String::new();
                for (i, param) in method.method().signature.parameters.iter().enumerate() {
                    if i > 0 {
                        params_str.push_str(", ");
                    }
                    params_str.push_str(&param.1.show(method.resolution().definition()));
                    let Some(meta) = method.method().parameter_metadata.get(i) else {
                        continue;
                    };
                    if let Some(name) = meta.as_ref().and_then(|m| m.name.as_ref()) {
                        params_str.push(' ');
                        params_str.push_str(name);
                    }
                }
                params_str
            };

            // If we're in an intrinsic, record it first
            if let Some(intrinsic) = ctx.current_intrinsic() {
                let type_name = intrinsic.parent.type_name();
                let method_name = intrinsic.method().name.to_string();
                let params = format_params(&intrinsic);
                trace.push_str(&format!(
                    "   at {}.{}({}) [Intrinsic]\n",
                    type_name, method_name, params
                ));
            }

            for frame in ctx.frame_stack().frames.iter().rev() {
                let method = &frame.state.info_handle.source;
                let type_name = method.parent.type_name();
                let method_name = method.method().name.to_string();
                let params = format_params(method);
                let ip = frame.state.ip;
                trace.push_str(&format!(
                    "   at {}.{}({}) in IP {}\n",
                    type_name, method_name, params, ip
                ));
            }

            exception.as_object(|obj| {
                if obj
                    .instance_storage
                    .has_field(exception_type, "_stackTraceString")
                {
                    let clr_str = CLRString::from(trace);
                    let str_obj = ObjectRef::new(gc, HeapStorage::Str(clr_str));
                    ctx.register_new_object(&str_obj);

                    let mut field_data = obj
                        .instance_storage
                        .get_field_mut_local(exception_type, "_stackTraceString");
                    let val = StackValue::ObjectRef(str_obj);
                    // SAFETY: field_data is a valid mutable slice of the object's instance storage,
                    // and val is a valid StackValue::ObjectRef.
                    unsafe {
                        val.store(field_data.as_mut_ptr(), StoreType::Object);
                    }
                }
            });
        }

        // Preempt any existing exception handling state (nested exceptions).
        // If we are already in the middle of a search or filter, we MUST NOT clear the suspended state,
        // as it contains the state of the outer exception we need to resume later.
        if matches!(
            *ctx.exception_mode(),
            ExceptionState::None | ExceptionState::Throwing(_, _)
        ) {
            ctx.evaluation_stack_mut().clear_suspended();
            ctx.frame_stack_mut().clear_suspended();
        }

        let nested_filter = match ctx.exception_mode() {
            ExceptionState::Filtering(state) => Some(*state),
            ExceptionState::Searching(state) => state.nested_filter,
            _ => None,
        };

        *ctx.exception_mode_mut() = ExceptionState::Searching(SearchState {
            exception,
            cursor: HandlerAddress {
                frame_index: ctx.frame_stack().len() - 1,
                section_index: 0,
                handler_index: 0,
            },
            nested_filter,
        });
        self.handle_exception(ctx, gc)
    }

    fn search_for_handler<'gc>(
        &self,
        ctx: &mut dyn ExceptionContext<'gc>,
        gc: GCHandle<'gc>,
        exception: ObjectRef<'gc>,
        cursor: HandlerAddress,
    ) -> StepResult {
        let stop_frame = if let ExceptionState::Searching(state) = ctx.exception_mode() {
            if let Some(nested) = state.nested_filter {
                if nested.handler.frame_index == 0 {
                    panic!("Nested filter handler frame index is 0! (len={})", ctx.frame_stack().len());
                }
                nested.handler.frame_index
            } else {
                0
            }
        } else {
            0
        };

        // Search from the cursor's frame down to the stop frame
        for frame_index in (stop_frame..=cursor.frame_index).rev() {
            let frame = &ctx.frame_stack().frames[frame_index];
            let ip = frame.state.ip;
            let exceptions = frame.state.info_handle.exceptions.clone();

            let section_start = if frame_index == cursor.frame_index {
                cursor.section_index
            } else {
                0
            };

            for (section_index, section) in exceptions.iter().enumerate().skip(section_start) {
                // Only consider sections that protect the current instruction pointer
                if !section.instructions.contains(&ip) {
                    continue;
                }

                let handler_start =
                    if frame_index == cursor.frame_index && section_index == cursor.section_index {
                        cursor.handler_index
                    } else {
                        0
                    };

                for (handler_index, handler) in
                    section.handlers.iter().enumerate().skip(handler_start)
                {
                    match &handler.kind {
                        HandlerKind::Catch(t) => {
                            let _exc_ref = match exception.0 {
                                Some(v) => v,
                                None => {
                                    return StepResult::Error(VmError::from(
                                        TypeResolutionError::InvalidHandle,
                                    ));
                                }
                            };
                            let exc_type = match ctx.get_heap_description(exception) {
                                Ok(v) => v,
                                Err(e) => return StepResult::Error(e.into()),
                            };
                            let catch_type = t.clone();

                            match ctx.is_a(exc_type.into(), catch_type) {
                                Ok(true) => {
                                    // Match found! Start the unwind phase towards this handler.
                                    *ctx.exception_mode_mut() =
                                        ExceptionState::Unwinding(UnwindState {
                                            exception: Some(exception),
                                            target: UnwindTarget::Handler(HandlerAddress {
                                                frame_index,
                                                section_index,
                                                handler_index,
                                            }),
                                            cursor: HandlerAddress {
                                                frame_index: ctx.frame_stack().len() - 1,
                                                section_index: 0,
                                                handler_index: 0,
                                            },
                                            nested_filter: None,
                                        });
                                    return StepResult::Exception;
                                }
                                Ok(false) => {}
                                Err(e) => return StepResult::Error(e.into()),
                            }
                        }
                        HandlerKind::Filter { clause_offset } => {
                            // Filter found! Suspend current execution and run the filter block.
                            let handler_addr = HandlerAddress {
                                frame_index,
                                section_index,
                                handler_index,
                            };
                            let original_ip = ctx.original_ip();
                            let original_stack_height = ctx.original_stack_height();

                            *ctx.exception_mode_mut() = ExceptionState::Filtering(FilterState {
                                exception,
                                handler: handler_addr,
                                original_ip,
                                original_stack_height,
                            });

                            // To run the filter, we must suspend the frames and stack above it.
                            let stack_base = ctx.frame_stack().frames[frame_index].base.stack;
                            ctx.evaluation_stack_mut().suspend_above(stack_base);
                            ctx.frame_stack_mut().suspend_above(frame_index);

                            let (ip, height) = {
                                let frame = &ctx.frame_stack().frames[frame_index];
                                (frame.state.ip, frame.stack_height)
                            };
                            *ctx.original_ip_mut() = ip;
                            *ctx.original_stack_height_mut() = height;

                            let frame = &mut ctx.frame_stack_mut().frames[frame_index];
                            frame.state.ip = *clause_offset;
                            frame.stack_height = StackSlotIndex(0);
                            frame.exception_stack.push(exception);
                            ctx.push_obj(exception);

                            return StepResult::Exception;
                        }
                        _ => {} // finally and fault are ignored during the search phase
                    }
                }
            }
        }

        // No handler found - the exception is unhandled.
        if let ExceptionState::Searching(search_state) = *ctx.exception_mode()
            && let Some(nested) = search_state.nested_filter
        {
            // If this exception occurred during a filter, it is swallowed by the CLI.
            // We start unwinding the filter's call stack to execute any finally blocks.
            // When the unwind completes (reaching the filter's entry point), we will
            // resume the original exception search.
            *ctx.exception_mode_mut() = ExceptionState::Unwinding(UnwindState {
                exception: Some(exception),
                target: UnwindTarget::Instruction(usize::MAX),
                cursor: HandlerAddress {
                    frame_index: ctx.frame_stack().len() - 1,
                    section_index: 0,
                    handler_index: 0,
                },
                nested_filter: Some(nested),
            });
            return self.handle_exception(ctx, gc);
        }

        let managed_exc = extract_managed_exception(&gc, exception, ctx);

        *ctx.exception_mode_mut() = ExceptionState::None;
        ctx.frame_stack_mut().clear();
        ctx.evaluation_stack_mut().clear();
        StepResult::MethodThrew(managed_exc)
    }

    fn unwind<'gc>(
        &self,
        ctx: &mut dyn ExceptionContext<'gc>,
        gc: GCHandle<'gc>,
        exception: Option<ObjectRef<'gc>>,
        target: UnwindTarget,
        cursor: HandlerAddress,
        nested_filter: Option<FilterState<'gc>>,
    ) -> StepResult {
        let target_frame = match target {
            UnwindTarget::Handler(h) => h.frame_index,
            UnwindTarget::Instruction(_) => {
                if let Some(nested) = nested_filter {
                    nested.handler.frame_index
                } else {
                    cursor.frame_index
                }
            }
        };

        // Unwind from the cursor's frame down to the target frame
        for frame_index in (target_frame..=cursor.frame_index).rev() {
            let (ip, exceptions) = {
                let frame = &ctx.frame_stack().frames[frame_index];
                (frame.state.ip, frame.state.info_handle.exceptions.clone())
            };

            let section_start = if frame_index == cursor.frame_index {
                cursor.section_index
            } else {
                0
            };

            for (section_index, section) in exceptions.iter().enumerate().skip(section_start) {
                // If we are in the target frame, stop before processing the target section or anything beyond it.
                if let UnwindTarget::Handler(target_h) = target
                    && frame_index == target_h.frame_index
                    && section_index >= target_h.section_index
                {
                    break;
                }

                let in_try = section.instructions.contains(&ip);

                // Determine if we are currently exiting this protected section.
                let exiting = match target {
                    // When unwinding to a catch/filter, any 'try' block we were in is being exited.
                    UnwindTarget::Handler(_) => in_try,
                    // When unwinding due to 'leave', we check if the target is outside the try block.
                    UnwindTarget::Instruction(target_ip) => {
                        let mut jumping_within_handler = false;
                        let in_handler = section.handlers.iter().any(|h| {
                            let in_h = h.instructions.contains(&ip);
                            if in_h && h.instructions.contains(&target_ip) {
                                jumping_within_handler = true;
                            }
                            in_h
                        });

                        if jumping_within_handler {
                            false
                        } else {
                            (in_try || in_handler) && !section.instructions.contains(&target_ip)
                        }
                    }
                };

                if !exiting {
                    continue;
                }

                let handler_start =
                    if frame_index == cursor.frame_index && section_index == cursor.section_index {
                        cursor.handler_index
                    } else {
                        0
                    };

                for (handler_index, handler) in
                    section.handlers.iter().enumerate().skip(handler_start)
                {
                    let handler_kind = handler.kind.clone();
                    let handler_instructions = handler.instructions.clone();

                    // If we are currently inside this handler, we are either re-throwing
                    // or leaving it. In both cases, we don't run it again.
                    if handler_instructions.contains(&ip) {
                        // If we are leaving a catch or filter, it's no longer on the active exception stack.
                        if matches!(
                            handler_kind,
                            HandlerKind::Catch(_) | HandlerKind::Filter { .. }
                        ) {
                            ctx.frame_stack_mut().frames[frame_index]
                                .exception_stack
                                .pop();
                        }
                        continue;
                    }

                    // Finally blocks always run during unwind.
                    // Fault blocks only run if an exception is active.
                    let should_run = match &handler_kind {
                        HandlerKind::Finally => true,
                        HandlerKind::Fault => exception.is_some(),
                        _ => false,
                    };

                    if should_run {
                        let handler_start_ip = handler_instructions.start;

                        // Set up the cursor to resume unwinding after this handler finishes.
                        let next_cursor = if handler_index + 1 < section.handlers.len() {
                            HandlerAddress {
                                frame_index,
                                section_index,
                                handler_index: handler_index + 1,
                            }
                        } else {
                            HandlerAddress {
                                frame_index,
                                section_index: section_index + 1,
                                handler_index: 0,
                            }
                        };

                        *ctx.exception_mode_mut() = ExceptionState::ExecutingHandler(UnwindState {
                            exception,
                            target,
                            cursor: next_cursor,
                            nested_filter,
                        });

                        let frame = &mut ctx.frame_stack_mut().frames[frame_index];
                        frame.state.ip = handler_start_ip;
                        frame.stack_height = StackSlotIndex(0);

                        return StepResult::Exception;
                    }
                }
            }

            // If we have finished all sections in this frame and it's not the target frame,
            // we pop it and continue unwinding in the caller.
            if frame_index > target_frame {
                ctx.unwind_frame();

                // If unwinding the frame triggered a new exception (e.g. wrapping a .cctor failure),
                // we must stop unwinding and start processing the new exception.
                if matches!(ctx.exception_mode(), ExceptionState::Throwing(_, _)) {
                    return self.handle_exception(ctx, gc);
                }
            }
        }

        // We have successfully unwound to the target!
        if let UnwindTarget::Instruction(usize::MAX) = target
            && let Some(nested) = nested_filter
        {
            // Resume original search
            // Restore suspended evaluation and frame stacks
            ctx.evaluation_stack_mut().restore_suspended();
            ctx.frame_stack_mut().restore_suspended();

            // Restore state of the frame where filter ran
            let frame = &mut ctx.frame_stack_mut().frames[nested.handler.frame_index];
            frame.state.ip = nested.original_ip;
            frame.stack_height = nested
                .original_stack_height
                .saturating_sub_idx(frame.base.stack);
            frame.exception_stack.pop();

            // Result is 0 (false): Continue searching after this filter handler.
            *ctx.exception_mode_mut() = ExceptionState::Searching(SearchState {
                exception: nested.exception,
                cursor: HandlerAddress {
                    frame_index: nested.handler.frame_index,
                    section_index: nested.handler.section_index,
                    handler_index: nested.handler.handler_index + 1,
                },
                nested_filter: None,
            });

            return self.handle_exception(ctx, gc);
        }

        *ctx.exception_mode_mut() = ExceptionState::None;
        match target {
            UnwindTarget::Handler(target_h) => {
                let handler_start_ip = {
                    let section = &ctx.frame_stack().frames[target_h.frame_index]
                        .state
                        .info_handle
                        .exceptions[target_h.section_index];
                    let handler = &section.handlers[target_h.handler_index];
                    handler.instructions.start
                };

                let frame = &mut ctx.frame_stack_mut().frames[target_h.frame_index];
                frame.state.ip = handler_start_ip;
                frame.stack_height = StackSlotIndex(0);

                // Push the exception object onto the stack for the catch/filter handler.
                let exception = exception.expect("Target handler reached but no exception present");
                frame.exception_stack.push(exception);
                ctx.push_obj(exception);

                // Continue execution at the handler
                StepResult::Exception
            }
            UnwindTarget::Instruction(target_ip) => {
                if target_ip == usize::MAX {
                    return StepResult::Return;
                }

                let frame = &mut ctx.frame_stack_mut().frames[target_frame];
                frame.state.ip = target_ip;
                frame.stack_height = StackSlotIndex(0);

                StepResult::Exception
            }
        }
    }
}

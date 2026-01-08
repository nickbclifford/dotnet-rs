use crate::{
    types::generics::ConcreteType,
    utils::DebugStr,
    value::object::ObjectRef,
    vm::{context::ResolutionContext, CallStack, GCHandle, StepResult},
    vm_error, vm_push,
};
use dotnetdll::prelude::*;
use gc_arena::Collect;
use std::{
    cmp::Reverse,
    collections::HashMap,
    fmt::{self, Debug, Formatter},
    ops::Range,
};

/// Represents the location of an exception handler within the call stack.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Collect)]
#[collect(require_static)]
pub struct HandlerAddress {
    /// Index into the `CallStack::frames` vector.
    pub frame_index: usize,
    /// Index into the `MethodInfo::exceptions` vector for that frame.
    pub section_index: usize,
    /// Index into the `ProtectedSection::handlers` vector for that section.
    pub handler_index: usize,
}

/// The current state of the exception handling mechanism.
///
/// Exception handling in the CLI is a two-pass process:
/// 1. **Search Phase**: The runtime searches for a matching `catch` block or a `filter` that returns true.
/// 2. **Unwind Phase**: The runtime executes `finally` and `fault` blocks from the throw point
///    up to the found handler (or the target of a `leave` instruction).
#[derive(Collect, Debug)]
#[collect(no_drop)]
pub enum ExceptionState<'gc> {
    /// No exception is currently being processed.
    None,
    /// An exception has just been thrown. The next step is to begin the search phase.
    Throwing(ObjectRef<'gc>),
    /// Currently searching for a matching handler (catch or filter).
    Searching {
        exception: ObjectRef<'gc>,
        cursor: HandlerAddress,
    },
    /// A filter block is currently executing.
    Filtering {
        exception: ObjectRef<'gc>,
        handler: HandlerAddress,
    },
    /// Currently in the unwind phase, executing `finally` and `fault` blocks.
    Unwinding {
        exception: Option<ObjectRef<'gc>>,
        target: UnwindTarget,
        cursor: HandlerAddress,
    },
    /// A `finally`, `fault`, or `catch` handler is currently executing.
    ExecutingHandler {
        exception: Option<ObjectRef<'gc>>,
        target: UnwindTarget,
        cursor: HandlerAddress,
    },
}

/// The destination of the current unwind operation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Collect)]
#[collect(require_static)]
pub enum UnwindTarget {
    /// Unwinding to a specific `catch` or `filter` handler.
    Handler(HandlerAddress),
    /// Unwinding because of a `leave` instruction to a specific IP.
    Instruction(usize),
}

/// A protected block of code (a `try` block) and its associated handlers.
#[derive(Clone)]
pub struct ProtectedSection {
    /// The range of instruction offsets protected by this section.
    pub instructions: Range<usize>,
    /// The exception handlers associated with this protected block.
    pub handlers: Vec<Handler>,
}

impl Debug for ProtectedSection {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_set()
            .entry(&DebugStr(format!("try {{ {:?} }}", self.instructions)))
            .entries(self.handlers.iter())
            .finish()
    }
}

/// An exception handler (catch, filter, finally, or fault).
#[derive(Clone)]
pub struct Handler {
    /// The range of instruction offsets for the handler's body.
    pub instructions: Range<usize>,
    /// The type of this handler.
    pub kind: HandlerKind,
}

impl Debug for Handler {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "{:?} {{ {:?} }}", self.kind, self.instructions)
    }
}

/// Specifies the behavior and trigger conditions of an exception handler.
#[derive(Clone)]
pub enum HandlerKind {
    /// Triggers only when the thrown exception is of the specified type (or a subtype).
    Catch(ConcreteType),
    /// Triggers when the filter clause at the specified offset returns true.
    Filter { clause_offset: usize },
    /// Always executes when exiting the protected block (whether normally or via exception).
    Finally,
    /// Executes only when exiting the protected block via an exception.
    Fault,
}

impl Debug for HandlerKind {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        use HandlerKind::*;
        match self {
            Catch(t) => write!(f, "catch({t:?})"),
            Filter { clause_offset } => write!(f, "filter({clause_offset}..)"),
            Finally => write!(f, "finally"),
            Fault => write!(f, "fault"),
        }
    }
}

/// Parses exception handler metadata from an assembly into a structured format.
pub fn parse<'a>(
    source: impl IntoIterator<Item = &'a body::Exception>,
    ctx: &ResolutionContext,
) -> Vec<ProtectedSection> {
    let mut sections: HashMap<Range<usize>, Vec<Handler>> = HashMap::new();
    for exc in source {
        use body::ExceptionKind::*;
        let try_range = exc.try_offset..exc.try_offset + exc.try_length;
        let handler_range = exc.handler_offset..exc.handler_offset + exc.handler_length;

        let kind = match &exc.kind {
            TypedException(t) => HandlerKind::Catch(ctx.make_concrete(t)),
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
    v
}

impl<'gc, 'm: 'gc> CallStack<'gc, 'm> {
    pub fn handle_exception(&mut self, gc: GCHandle<'gc>) -> StepResult {
        match self.execution.exception_mode {
            ExceptionState::None => StepResult::InstructionStepped,
            ExceptionState::Throwing(exception) => self.begin_throwing(exception, gc),
            ExceptionState::Searching { exception, cursor } => {
                self.search_for_handler(gc, exception, cursor)
            }
            ExceptionState::Unwinding {
                exception,
                target,
                cursor,
            } => self.unwind(gc, exception, target, cursor),
            ExceptionState::Filtering { .. } | ExceptionState::ExecutingHandler { .. } => {
                StepResult::InstructionStepped
            }
        }
    }

    fn begin_throwing(&mut self, exception: ObjectRef<'gc>, gc: GCHandle<'gc>) -> StepResult {
        let frame = self.execution.frames.last().unwrap();
        if self.tracer_enabled() {
            self.tracer().trace_exception(
                self.indent(),
                &format!("{:?}", exception),
                &format!(
                    "{:?} at IP {}",
                    frame.state.info_handle.source, frame.state.ip
                ),
            );
        }
        // Preempt any existing exception handling state (nested exceptions)
        self.execution.suspended_stack.clear();
        self.execution.suspended_frames.clear();

        self.execution.exception_mode = ExceptionState::Searching {
            exception,
            cursor: HandlerAddress {
                frame_index: self.execution.frames.len() - 1,
                section_index: 0,
                handler_index: 0,
            },
        };
        self.handle_exception(gc)
    }

    fn search_for_handler(
        &mut self,
        gc: GCHandle<'gc>,
        exception: ObjectRef<'gc>,
        cursor: HandlerAddress,
    ) -> StepResult {
        // Search from the cursor's frame down to the bottom of the stack
        for frame_index in (0..=cursor.frame_index).rev() {
            let frame = &self.execution.frames[frame_index];
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
                            let exc_type = self
                                .current_context()
                                .get_heap_description(exception.0.expect("throwing null"));
                            let catch_type = self.loader().find_concrete_type(t.clone());

                            if self.is_a(exc_type, catch_type) {
                                // Match found! Start the unwind phase towards this handler.
                                self.execution.exception_mode = ExceptionState::Unwinding {
                                    exception: Some(exception),
                                    target: UnwindTarget::Handler(HandlerAddress {
                                        frame_index,
                                        section_index,
                                        handler_index,
                                    }),
                                    cursor: HandlerAddress {
                                        frame_index: self.execution.frames.len() - 1,
                                        section_index: 0,
                                        handler_index: 0,
                                    },
                                };
                                return StepResult::InstructionStepped;
                            }
                        }
                        HandlerKind::Filter { clause_offset } => {
                            // Filter found! Suspend current execution and run the filter block.
                            let handler_addr = HandlerAddress {
                                frame_index,
                                section_index,
                                handler_index,
                            };
                            self.execution.exception_mode = ExceptionState::Filtering {
                                exception,
                                handler: handler_addr,
                            };

                            // To run the filter, we must suspend the frames and stack above it.
                            let stack_base = self.execution.frames[frame_index].base.stack;
                            self.execution.suspended_stack =
                                self.execution.stack.split_off(stack_base);
                            self.execution.suspended_frames =
                                self.execution.frames.split_off(frame_index + 1);

                            let frame = &mut self.execution.frames[frame_index];
                            self.execution.original_ip = frame.state.ip;
                            self.execution.original_stack_height = frame.stack_height;

                            frame.state.ip = *clause_offset;
                            frame.stack_height = 0;
                            frame.exception_stack.push(exception);
                            vm_push!(self, gc, ObjectRef(exception));

                            return StepResult::InstructionStepped;
                        }
                        _ => {} // finally and fault are ignored during the search phase
                    }
                }
            }
        }

        // No handler found - the exception is unhandled.
        // In a real VM this might trigger a debugger or a default handler.
        // Log the exception and full backtrace before clearing the stack.

        let mut message = None;
        let exception_type = self.loader().corlib_type("System.Exception");
        exception.as_object(|obj| {
            if obj.instance_storage.has_field(exception_type, "_message") {
                let message_bytes = obj
                    .instance_storage
                    .get_field_local(exception_type, "_message");
                let message_ref = ObjectRef::read(message_bytes);
                if let Some(msg_inner) = message_ref.0 {
                    let storage = &msg_inner.borrow().storage;
                    if let crate::value::object::HeapStorage::Str(clr_str) = storage {
                        message = Some(clr_str.as_string());
                    }
                }
            }
        });

        // Always log to stderr (regardless of tracer configuration)
        eprintln!("╔═══════════════════════════════════════════════════════");
        eprintln!("║ UNHANDLED EXCEPTION");
        if let Some(msg) = &message {
            eprintln!("║ Message: {}", msg);
        }
        eprintln!("╠═══════════════════════════════════════════════════════");
        eprintln!("║ Exception Object: {:?}", exception);
        eprintln!("╠═══════════════════════════════════════════════════════");
        eprintln!("║ Call Stack (backtrace):");
        eprintln!("╠═══════════════════════════════════════════════════════");

        // Log the full call stack backtrace
        for (frame_idx, frame) in self.execution.frames.iter().enumerate() {
            eprintln!(
                "║   Frame #{}: {:?}",
                frame_idx, frame.state.info_handle.source
            );
            eprintln!(
                "║     at IP {} (method has {} instructions total)",
                frame.state.ip,
                frame.state.info_handle.instructions.len()
            );
        }

        eprintln!("╚═══════════════════════════════════════════════════════");

        // Also log to tracer if enabled
        if let Some(msg) = &message {
            vm_error!(
                self,
                "UNHANDLED EXCEPTION: {} ({:?}) - No matching exception handler found",
                msg,
                exception
            );
        } else {
            vm_error!(
                self,
                "UNHANDLED EXCEPTION: {:?} - No matching exception handler found",
                exception
            );
        }
        for (frame_idx, frame) in self.execution.frames.iter().enumerate() {
            vm_error!(
                self,
                "  Frame #{}: {:?} at IP {}",
                frame_idx,
                frame.state.info_handle.source,
                frame.state.ip
            );
        }

        self.execution.exception_mode = ExceptionState::None;
        self.execution.frames.clear();
        self.execution.stack.clear();
        StepResult::MethodThrew
    }

    fn unwind(
        &mut self,
        gc: GCHandle<'gc>,
        exception: Option<ObjectRef<'gc>>,
        target: UnwindTarget,
        cursor: HandlerAddress,
    ) -> StepResult {
        let target_frame = match target {
            UnwindTarget::Handler(h) => h.frame_index,
            UnwindTarget::Instruction(_) => cursor.frame_index,
        };

        // Unwind from the cursor's frame down to the target frame
        for frame_index in (target_frame..=cursor.frame_index).rev() {
            let (ip, exceptions) = {
                let frame = &self.execution.frames[frame_index];
                (frame.state.ip, frame.state.info_handle.exceptions.clone())
            };

            let section_start = if frame_index == cursor.frame_index {
                cursor.section_index
            } else {
                0
            };

            for (section_index, section) in exceptions.iter().enumerate().skip(section_start) {
                // If we are in the target frame, stop before processing the target section or anything beyond it.
                if let UnwindTarget::Handler(target_h) = target {
                    if frame_index == target_h.frame_index
                        && section_index >= target_h.section_index
                    {
                        break;
                    }
                }

                let in_try = section.instructions.contains(&ip);

                // Determine if we are currently exiting this protected section.
                let exiting = match target {
                    // When unwinding to a catch/filter, any 'try' block we were in is being exited.
                    UnwindTarget::Handler(_) => in_try,
                    // When unwinding due to 'leave', we check if the target is outside the try block.
                    UnwindTarget::Instruction(target_ip) => {
                        let in_handler = section
                            .handlers
                            .iter()
                            .any(|h| h.instructions.contains(&ip));
                        (in_try || in_handler) && !section.instructions.contains(&target_ip)
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
                        // This is required so that 'rethrow' instructions correctly identify
                        // the currently active exception.
                        if matches!(
                            handler_kind,
                            HandlerKind::Catch(_) | HandlerKind::Filter { .. }
                        ) {
                            self.execution.frames[frame_index].exception_stack.pop();
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

                        self.execution.exception_mode = ExceptionState::ExecutingHandler {
                            exception,
                            target,
                            cursor: next_cursor,
                        };

                        let frame = &mut self.execution.frames[frame_index];
                        frame.state.ip = handler_start_ip;
                        frame.stack_height = 0;

                        return StepResult::InstructionStepped;
                    }
                }
            }

            // If we have finished all sections in this frame and it's not the target frame,
            // we pop it and continue unwinding in the caller.
            if frame_index > target_frame {
                self.unwind_frame(gc);
            }
        }

        // We have successfully unwound to the target!
        self.execution.exception_mode = ExceptionState::None;
        match target {
            UnwindTarget::Handler(target_h) => {
                let handler_start_ip = {
                    let section = &self.execution.frames[target_h.frame_index]
                        .state
                        .info_handle
                        .exceptions[target_h.section_index];
                    let handler = &section.handlers[target_h.handler_index];
                    handler.instructions.start
                };

                let frame = &mut self.execution.frames[target_h.frame_index];
                frame.state.ip = handler_start_ip;
                frame.stack_height = 0;

                // Push the exception object onto the stack for the catch/filter handler.
                let exception = exception.expect("Target handler reached but no exception present");
                frame.exception_stack.push(exception);
                vm_push!(self, gc, ObjectRef(exception));
            }
            UnwindTarget::Instruction(target_ip) => {
                // Special case: usize::MAX indicates we should return from the method
                // after executing finally blocks
                if target_ip == usize::MAX {
                    return StepResult::MethodReturned;
                }

                let frame = &mut self.execution.frames[target_frame];
                frame.state.ip = target_ip;
                frame.stack_height = 0;
            }
        }

        StepResult::InstructionStepped
    }
}

//! Execution engine and instruction dispatch.
//!
//! This module implements the main VM execution loop in [`ExecutionEngine`],
//! which orchestrates the fetch-decode-execute cycle. It also provides
//! the [`InstructionRegistry`] for looking up instruction handlers.
use crate::{
    ExceptionState, ResolutionContext, StackSlotIndex, StepResult,
    stack::{CallStack, ops::*},
    threading::ThreadManagerOps,
};
use dotnet_types::{TypeDescription, generics::GenericLookup, members::MethodDescription};
use dotnet_utils::{gc::GCHandle, sync::Ordering};
use dotnet_value::{layout::HasLayout, object::ObjectRef};
use dotnet_vm_ops::prepared_call::PreparedCall;
use dotnetdll::prelude::*;
use gc_arena::{Collect, collect::Trace};
use std::sync::OnceLock;

#[cfg(feature = "bench-instrumentation")]
use dotnet_metrics::OpcodeCategory;

pub mod registry;
pub mod ring_buffer;

pub struct InstructionRegistry;

impl InstructionRegistry {
    #[inline(always)]
    pub fn dispatch<'gc, T: VesOps<'gc>>(ctx: &mut T, instr: &Instruction) -> StepResult {
        #[cfg(feature = "instruction-dispatch-jump-table")]
        {
            return registry::dispatch_jump_table(ctx, instr);
        }
        #[cfg(not(feature = "instruction-dispatch-jump-table"))]
        registry::dispatch_monomorphic(ctx, instr)
    }
}

const DISPATCH_BATCH_SIZE: usize = 128;
const DEFAULT_SAFE_POINT_POLL_INTERVAL: usize = 32;

#[cold]
#[inline(never)]
fn invalid_ip_step_result(ip: usize) -> StepResult {
    StepResult::Error(crate::error::VmError::Execution(
        crate::error::ExecutionError::InvalidIP(ip),
    ))
}

fn dispatch_safe_point_poll_interval() -> usize {
    static POLL_INTERVAL: OnceLock<usize> = OnceLock::new();
    *POLL_INTERVAL.get_or_init(|| match std::env::var("DOTNET_SAFE_POINT_POLL_INTERVAL") {
        Ok(raw) => match raw.trim().parse::<usize>() {
            Ok(interval) if matches!(interval, 32 | 64 | 128) => interval,
            Ok(interval) => {
                tracing::warn!(
                    "invalid DOTNET_SAFE_POINT_POLL_INTERVAL={interval}; expected one of 32, 64, 128; using {}",
                    DEFAULT_SAFE_POINT_POLL_INTERVAL
                );
                DEFAULT_SAFE_POINT_POLL_INTERVAL
            }
            Err(err) => {
                tracing::warn!(
                    "failed to parse DOTNET_SAFE_POINT_POLL_INTERVAL={raw:?}: {err}; using {}",
                    DEFAULT_SAFE_POINT_POLL_INTERVAL
                );
                DEFAULT_SAFE_POINT_POLL_INTERVAL
            }
        },
        Err(_) => DEFAULT_SAFE_POINT_POLL_INTERVAL,
    })
}

impl<'gc> CallStack<'gc> {
    #[inline]
    #[must_use]
    pub fn check_gc_safe_point(&self) -> bool {
        let thread_manager = &self.shared.thread_manager;
        thread_manager.is_gc_stop_requested()
    }
}

pub struct ExecutionEngine<'gc> {
    pub stack: CallStack<'gc>,
    pub ring_buffer: ring_buffer::InstructionRingBuffer,
}

// SAFETY: `ExecutionEngine` correctly traces its GC-managed fields (`stack`) in its
// `trace` implementation. `ring_buffer` does not contain GC pointers.
unsafe impl<'gc> Collect<'gc> for ExecutionEngine<'gc> {
    fn trace<Tr: Trace<'gc>>(&self, cc: &mut Tr) {
        self.stack.trace(cc);
    }
}

impl<'gc> ExecutionEngine<'gc> {
    pub fn new(stack: CallStack<'gc>) -> Self {
        Self {
            stack,
            ring_buffer: ring_buffer::InstructionRingBuffer::new(),
        }
    }

    pub fn ves_context(&mut self, gc: GCHandle<'gc>) -> crate::stack::VesContext<'_, 'gc> {
        self.stack.ves_context(gc)
    }

    pub fn step(&mut self, gc: GCHandle<'gc>) -> StepResult {
        match self.stack.execution.exception_mode {
            ExceptionState::None
            | ExceptionState::ExecutingHandler(_)
            | ExceptionState::Filtering(_) => self.step_normal(gc),
            _ => self.ves_context(gc).handle_exception(),
        }
    }

    /// Executes a single instruction in the current frame.
    ///
    /// This method:
    /// 1. Fetches the instruction at the current IP.
    /// 2. Records the original IP and stack height for suspension/retry.
    /// 3. Traces the instruction if enabled.
    /// 4. Dispatches the instruction to its handler.
    /// 5. Handles the result (Continue, Jump, Call, Return, etc.).
    pub fn step_normal(&mut self, gc: GCHandle<'gc>) -> StepResult {
        if self.stack.execution.frame_stack.is_empty() {
            return StepResult::Return;
        }

        if self.stack.current_frame().multicast_state.is_some() {
            return self.handle_multicast_step(gc);
        }

        let ip = self.stack.state().ip;
        let instructions = self.stack.state().info_handle.instructions;
        if vm_unlikely!(ip >= instructions.len()) {
            return invalid_ip_step_result(ip);
        }
        let i = &instructions[ip];

        // Synchronize original IP and stack height for suspension/retry logic (e.g. Yield)
        self.stack.execution.original_ip = ip;
        self.stack.execution.original_stack_height =
            self.stack.execution.evaluation_stack.top_of_stack();

        self.ring_buffer.push(ip, i.name());

        let _ = self
            .stack
            .tracer()
            .enabled_emit(self.stack.indent(), |trace| {
                let resolution = self.stack.state().info_handle.source.resolution();
                let instr_text = i.show(resolution.definition());
                trace.instruction(ip, &instr_text);
            });

        tracing::debug!(
            "step_normal: frame={}, ip={}, instr={:?}",
            self.stack.execution.frame_stack.len(),
            ip,
            i.opcode()
        );

        #[cfg(feature = "bench-instrumentation")]
        self.stack
            .shared
            .metrics
            .record_opcode_dispatch(opcode_category_for(i));

        let mut ctx = self.ves_context(gc);
        let res = InstructionRegistry::dispatch(&mut ctx, i);

        tracing::debug!("step_normal: res={:?}", res);

        match res {
            StepResult::Continue => {
                self.stack.increment_ip();
                StepResult::Continue
            }
            StepResult::Jump(target) => {
                self.stack.branch(target);
                StepResult::Continue
            }
            StepResult::Yield => StepResult::Yield,
            StepResult::Exception => StepResult::Exception,
            StepResult::Return => {
                let res = ctx.handle_return();
                tracing::debug!("step_normal: handle_return result={:?}", res);
                res
            }
            _ => res,
        }
    }

    #[inline(always)]
    fn step_batch_instruction(
        &mut self,
        gc: GCHandle<'gc>,
        _remaining_budget: usize,
    ) -> (StepResult, usize) {
        (self.step_normal(gc), 1)
    }

    /// Run the engine until it needs to yield, returns from the entry point, or throws an unhandled exception.
    pub fn run(&mut self, gc: GCHandle<'gc>) -> StepResult {
        loop {
            if self.stack.shared.abort_requested.load(Ordering::Relaxed) {
                // Final snapshot before aborting to ensures correct dump in test harness
                *self.stack.shared.last_instructions.lock() = self.ring_buffer.clone();
                return StepResult::Yield;
            }

            if self.stack.check_gc_safe_point() {
                // Snapshot before yielding
                *self.stack.shared.last_instructions.lock() = self.ring_buffer.clone();
                return StepResult::Yield;
            }
            let res = match self.stack.execution.exception_mode {
                ExceptionState::None
                | ExceptionState::ExecutingHandler(_)
                | ExceptionState::Filtering(_) => {
                    // Optimized path for normal execution: run multiple instructions
                    // without checking exception mode every time, relying on instructions
                    // to return StepResult::Exception if they throw.
                    let mut last_res = StepResult::Continue;
                    let poll_interval = dispatch_safe_point_poll_interval();
                    let mut executed_in_batch = 0usize;
                    let mut since_poll = 0usize;
                    // Batch multiple instructions to reduce dispatch overhead.
                    // Safe because any state-changing instruction must return a non-Continue result
                    // (e.g., Jump/Return/Exception), which breaks out of this loop.
                    while executed_in_batch < DISPATCH_BATCH_SIZE {
                        let remaining = DISPATCH_BATCH_SIZE - executed_in_batch;
                        let (res, consumed) = self.step_batch_instruction(gc, remaining);
                        last_res = res;
                        let consumed = consumed.min(remaining);
                        executed_in_batch += consumed;

                        if last_res != StepResult::Continue {
                            break;
                        }

                        since_poll += consumed;
                        // Check abort/safe-point every N instructions within a batch.
                        if since_poll >= poll_interval {
                            if self.stack.shared.abort_requested.load(Ordering::Relaxed) {
                                last_res = StepResult::Yield;
                                break;
                            }
                            if self.stack.check_gc_safe_point() {
                                last_res = StepResult::Yield;
                                break;
                            }
                            since_poll = 0;
                        }
                    }

                    // Snapshot the ring buffer only on non-normal exits or periodically
                    // for timeout diagnostics. Normal Continue paths skip the snapshot
                    // to avoid mutex + clone overhead on every batch.
                    if last_res != StepResult::Continue {
                        *self.stack.shared.last_instructions.lock() = self.ring_buffer.clone();
                    }

                    last_res
                }
                _ => {
                    let res = self.ves_context(gc).handle_exception();
                    match res {
                        StepResult::Jump(target) => {
                            self.stack.branch(target);
                            StepResult::Continue
                        }
                        StepResult::Return => self.ves_context(gc).handle_return(),
                        _ => res,
                    }
                }
            };

            if res == StepResult::Exception {
                continue;
            }

            return res;
        }
    }

    pub fn unified_dispatch(
        &mut self,
        gc: GCHandle<'gc>,
        source: &MethodSource,
        this_type: Option<TypeDescription>,
        ctx: Option<&ResolutionContext<'_>>,
    ) -> StepResult {
        self.ves_context(gc)
            .unified_dispatch(source, this_type, ctx)
    }

    pub fn dispatch_method(
        &mut self,
        gc: GCHandle<'gc>,
        method: MethodDescription,
        lookup: GenericLookup,
    ) -> StepResult {
        let mut ctx = self.stack.ves_context(gc);
        tracing::debug!(
            "dispatch_method: method={:?}, pinvoke={:?}, internal_call={}",
            method,
            method.method().pinvoke,
            method.method().internal_call
        );
        ctx.dispatch_resolved_method_for_engine_api(method, lookup)
    }

    fn handle_multicast_step(&mut self, gc: GCHandle<'gc>) -> StepResult {
        let targets_len = {
            let frame = self.stack.current_frame();
            let state = frame.multicast_state.as_ref().unwrap();
            ObjectRef(Some(state.targets)).as_vector(|v| v.layout.length)
        };

        let mut ctx = self.stack.ves_context(gc);

        // 1. Handle return from previous target
        if ctx.frame_stack.current_frame().stack_height > StackSlotIndex(0) {
            let frame = ctx.frame_stack.current_frame();
            let invoke_method = frame.state.info_handle.source.clone();
            let has_return_value = invoke_method.signature().return_type.1.is_some();

            let next_index = frame.multicast_state.as_ref().unwrap().next_index;
            if next_index < targets_len && has_return_value {
                let _ = ctx.pop();
            }
        }

        let (target_delegate, args, _next_index) = {
            let frame = ctx.frame_stack.current_frame_mut();
            let state = frame.multicast_state.as_mut().unwrap();
            let index = state.next_index;
            if index < targets_len {
                let target = ObjectRef(Some(state.targets)).as_vector(|v| {
                    let offset = (v.layout.element_layout.as_ref().size() * index).as_usize();
                    unsafe { ObjectRef::read_branded(&v.get()[offset..], &gc) }
                });
                state.next_index += 1;
                (Some(target), state.args.clone(), index)
            } else {
                (None, vec![], index)
            }
        };

        if let Some(target_delegate) = target_delegate {
            let invoke_method = ctx
                .frame_stack
                .current_frame()
                .state
                .info_handle
                .source
                .clone();
            let lookup = ctx.frame_stack.current_frame().generic_inst.clone();

            let prepared_call =
                PreparedCall::for_multicast_step(invoke_method, lookup, target_delegate, args);
            let call_target = prepared_call.push_arguments(&mut ctx);
            let (invoke_method, lookup) = call_target.into_parts();

            ctx.dispatch_method(invoke_method, lookup)
        } else {
            // All targets called.
            ctx.handle_return()
        }
    }
}

#[cfg(feature = "bench-instrumentation")]
fn opcode_category_for(instr: &Instruction) -> OpcodeCategory {
    use OpcodeCategory::*;
    let name = format!("{:?}", instr);

    if name.starts_with("Add")
        || name.starts_with("Sub")
        || name.starts_with("Mul")
        || name.starts_with("Div")
        || name.starts_with("Rem")
        || name.starts_with("Neg")
        || name.starts_with("And")
        || name.starts_with("Or")
        || name.starts_with("Xor")
        || name.starts_with("Shift")
    {
        Arithmetic
    } else if name.starts_with("Call")
        || name.starts_with("Jump")
        || name.starts_with("NewObject")
        || name.starts_with("LoadFunction")
    {
        Calls
    } else if name.starts_with("Compare") {
        Comparisons
    } else if name.starts_with("Convert") {
        Conversions
    } else if name.starts_with("Throw") || name.starts_with("EndFinally") || name == "Rethrow" {
        Exceptions
    } else if name.starts_with("Branch") || name.starts_with("Leave") || name.starts_with("Return")
    {
        Flow
    } else if name.starts_with("LoadIndirect")
        || name.starts_with("StoreIndirect")
        || name.starts_with("Initialize")
        || name.starts_with("Copy")
        || name.starts_with("LoadToken")
        || name.starts_with("LoadLength")
        || name.starts_with("LocalMemory")
    {
        Memory
    } else if name.starts_with("LoadElement")
        || name.starts_with("StoreElement")
        || name.starts_with("LoadField")
        || name.starts_with("StoreField")
        || name.starts_with("LoadStaticField")
        || name.starts_with("StoreStaticField")
        || name.starts_with("NewArray")
        || name.starts_with("Box")
        || name.starts_with("Unbox")
        || name.starts_with("CastClass")
        || name.starts_with("IsInstance")
    {
        Objects
    } else if name.starts_with("LoadMethodPointer") || name.starts_with("LoadVirtualMethodPointer")
    {
        Reflection
    } else if name.starts_with("Load")
        || name.starts_with("Store")
        || name.starts_with("Duplicate")
        || name == "Pop"
        || name == "NoOperation"
    {
        Stack
    } else {
        Other
    }
}

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
#[cfg(feature = "bench-instrumentation")]
use dotnet_metrics::OpcodeCategory;
use dotnet_types::{TypeDescription, generics::GenericLookup, members::MethodDescription};
use dotnet_utils::{gc::GCHandle, sync::Ordering};
use dotnet_value::{StackValue, layout::HasLayout, object::ObjectRef};
use dotnetdll::prelude::*;
use gc_arena::{Collect, collect::Trace};
use std::sync::OnceLock;

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

vm_cold_panic!(fn panic_intrinsic_not_found(method: &MethodDescription) => "intrinsic not found: {:?}", method);
vm_cold_panic!(
    fn panic_no_body_in_method(method: &MethodDescription) =>
        "no body in executing method: {}.{}",
        method.parent.type_name(),
        method.method().name
);

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

        // Record for local ring buffer (provides more context on timeout/panic)
        let resolution = self.stack.state().info_handle.source.resolution();
        self.ring_buffer.push(ip, i.show(resolution.definition()));

        if self.stack.shared.tracer_enabled.load(Ordering::Relaxed) {
            let instr_text = i.show(resolution.definition());
            vm_trace_instruction!(self.stack, ip, &instr_text);
        }

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
        remaining_budget: usize,
    ) -> (StepResult, usize) {
        #[cfg(not(feature = "dispatch-super-instruction-prototype"))]
        let _ = remaining_budget;

        #[cfg(feature = "dispatch-super-instruction-prototype")]
        if let Some((res, consumed)) =
            self.try_step_super_load_const_i32_1_add(gc, remaining_budget)
        {
            return (res, consumed);
        }

        (self.step_normal(gc), 1)
    }

    #[cfg(feature = "dispatch-super-instruction-prototype")]
    fn try_step_super_load_const_i32_1_add(
        &mut self,
        gc: GCHandle<'gc>,
        remaining_budget: usize,
    ) -> Option<(StepResult, usize)> {
        if remaining_budget < 2
            || self.stack.execution.frame_stack.is_empty()
            || self.stack.current_frame().multicast_state.is_some()
        {
            return None;
        }

        let ip = self.stack.state().ip;
        let instructions = self.stack.state().info_handle.instructions;
        if ip + 1 >= instructions.len()
            || !matches!(
                (&instructions[ip], &instructions[ip + 1]),
                (Instruction::LoadConstantInt32(1), Instruction::Add)
            )
        {
            return None;
        }

        let first = instructions[ip].clone();
        let second = instructions[ip + 1].clone();
        let resolution = self.stack.state().info_handle.source.resolution();

        // Keep suspension/retry metadata aligned with each fused instruction.
        self.stack.execution.original_ip = ip;
        self.stack.execution.original_stack_height =
            self.stack.execution.evaluation_stack.top_of_stack();

        self.ring_buffer
            .push(ip, first.show(resolution.definition()));
        if self.stack.shared.tracer_enabled.load(Ordering::Relaxed) {
            let first_text = first.show(resolution.definition());
            vm_trace_instruction!(self.stack, ip, &first_text);
        }

        #[cfg(feature = "bench-instrumentation")]
        self.stack
            .shared
            .metrics
            .record_opcode_dispatch(opcode_category_for(&first));

        tracing::debug!(
            "step_super: frame={}, ip={}, first={:?}, second={:?}",
            self.stack.execution.frame_stack.len(),
            ip,
            first.opcode(),
            second.opcode()
        );

        let first_res = {
            let mut ctx = self.ves_context(gc);
            InstructionRegistry::dispatch(&mut ctx, &first)
        };
        let mut consumed = 1usize;

        match first_res {
            StepResult::Continue => {
                self.stack.increment_ip();
            }
            StepResult::Jump(target) => {
                self.stack.branch(target);
                return Some((StepResult::Continue, consumed));
            }
            StepResult::Yield => return Some((StepResult::Yield, consumed)),
            StepResult::Exception => return Some((StepResult::Exception, consumed)),
            StepResult::Return => {
                let res = self.ves_context(gc).handle_return();
                tracing::debug!("step_super: first handle_return result={:?}", res);
                return Some((res, consumed));
            }
            _ => return Some((first_res, consumed)),
        }

        let second_ip = self.stack.state().ip;
        if second_ip != ip + 1 {
            return Some((StepResult::Continue, consumed));
        }

        self.stack.execution.original_ip = second_ip;
        self.stack.execution.original_stack_height =
            self.stack.execution.evaluation_stack.top_of_stack();
        self.ring_buffer
            .push(second_ip, second.show(resolution.definition()));
        if self.stack.shared.tracer_enabled.load(Ordering::Relaxed) {
            let second_text = second.show(resolution.definition());
            vm_trace_instruction!(self.stack, second_ip, &second_text);
        }

        #[cfg(feature = "bench-instrumentation")]
        self.stack
            .shared
            .metrics
            .record_opcode_dispatch(opcode_category_for(&second));

        consumed += 1;
        let second_res = {
            let mut ctx = self.ves_context(gc);
            InstructionRegistry::dispatch(&mut ctx, &second)
        };
        let result = match second_res {
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
                let res = self.ves_context(gc).handle_return();
                tracing::debug!("step_super: second handle_return result={:?}", res);
                res
            }
            _ => second_res,
        };

        Some((result, consumed))
    }

    /// Run the engine until it needs to yield, returns from the entry point, or throws an unhandled exception.
    pub fn run(&mut self, gc: GCHandle<'gc>) -> StepResult {
        loop {
            if self.stack.shared.abort_requested.load(Ordering::Relaxed) {
                // Final snapshot before aborting to ensures correct dump in test harness
                *self.stack.shared.last_instructions.lock() = self.ring_buffer.clone();
                return StepResult::Yield;
            }

            if self.stack.shared.thread_manager.is_gc_stop_requested() {
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
                            if self.stack.shared.thread_manager.is_gc_stop_requested() {
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
        // Instrumented benchmarks for current workloads report intrinsic_call_total=0, so this
        // branch is expected to be cold on the measured hot path.
        if vm_unlikely!(ctx.is_intrinsic_cached(method.clone())) {
            crate::intrinsics::intrinsic_call(&mut ctx, method, &lookup)
        } else if method.method().pinvoke.is_some() {
            let shared = ctx.shared().clone();
            dotnet_pinvoke::external_call(&mut ctx, method, &shared.pinvoke)
        } else {
            if method.method().internal_call {
                panic_intrinsic_not_found(&method);
            }

            if method.method().body.is_none() {
                if let Some(result) = dotnet_intrinsics_delegates::try_delegate_dispatch(
                    &mut ctx,
                    method.clone(),
                    &lookup,
                ) {
                    return result;
                }

                panic_no_body_in_method(&method);
            }

            vm_try!(ctx.call_frame(
                vm_try!(ctx.shared().caches.get_method_info(method, &lookup, ctx.shared().clone())),
                lookup,
            ));
            StepResult::FramePushed
        }
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
            let has_return_value = invoke_method.method().signature.return_type.1.is_some();

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
            ctx.push(StackValue::ObjectRef(target_delegate));
            for arg in args {
                ctx.push(arg);
            }

            let invoke_method = ctx
                .frame_stack
                .current_frame()
                .state
                .info_handle
                .source
                .clone();
            let lookup = ctx.frame_stack.current_frame().generic_inst.clone();

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

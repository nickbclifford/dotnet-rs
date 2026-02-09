//! Execution engine and instruction dispatch.
//!
//! This module implements the main VM execution loop in [`ExecutionEngine`],
//! which orchestrates the fetch-decode-execute cycle. It also provides
//! the [`InstructionRegistry`] for looking up instruction handlers.
use crate::{
    MethodInfo, ResolutionContext, StepResult,
    exceptions::ExceptionState,
    stack::{
        CallStack,
        ops::{CallOps, ReflectionOps, StackOps, VesOps},
    },
    threading::ThreadManagerOps,
    vm_trace_instruction,
};
use dotnet_types::{TypeDescription, generics::GenericLookup, members::MethodDescription};
use dotnet_utils::gc::GCHandle;
use dotnet_value::{StackValue, layout::HasLayout, object::ObjectRef};
use dotnetdll::prelude::*;
use gc_arena::Collect;

pub mod registry;

pub struct InstructionRegistry;

impl InstructionRegistry {
    pub fn dispatch<'gc, 'm>(
        gc: GCHandle<'gc>,
        ctx: &mut dyn VesOps<'gc, 'm>,
        instr: &Instruction,
    ) -> Option<StepResult> {
        let handler = registry::get_handler(instr.opcode())?;
        Some(handler(ctx, gc, instr))
    }
}

impl<'gc, 'm: 'gc> CallStack<'gc, 'm> {
    #[inline]
    pub fn check_gc_safe_point(&self) {
        let thread_manager = &self.shared.thread_manager;
        if thread_manager.is_gc_stop_requested() {
            let managed_id = self.thread_id.get();
            if managed_id != 0 {
                #[cfg(feature = "multithreaded-gc")]
                thread_manager.safe_point(managed_id, &self.shared.gc_coordinator);
                #[cfg(not(feature = "multithreaded-gc"))]
                thread_manager.safe_point(managed_id, &Default::default());
            }
        }
    }
}

pub struct ExecutionEngine<'gc, 'm: 'gc> {
    pub stack: CallStack<'gc, 'm>,
}

// SAFETY: `ExecutionEngine` correctly traces its only GC-managed field (`stack`) in its
// `trace` implementation. This implementation is safe because it delegates to the `Collect`
// implementation of `CallStack`, which correctly traces all GC-managed references.
unsafe impl<'gc, 'm: 'gc> Collect for ExecutionEngine<'gc, 'm> {
    fn trace(&self, cc: &gc_arena::Collection) {
        self.stack.trace(cc);
    }
}

impl<'gc, 'm: 'gc> ExecutionEngine<'gc, 'm> {
    pub fn new(stack: CallStack<'gc, 'm>) -> Self {
        Self { stack }
    }

    pub fn ves_context(&mut self) -> crate::stack::VesContext<'_, 'gc, 'm> {
        self.stack.ves_context()
    }

    pub fn step(&mut self, gc: GCHandle<'gc>) -> StepResult {
        match self.stack.execution.exception_mode {
            ExceptionState::None
            | ExceptionState::ExecutingHandler(_)
            | ExceptionState::Filtering(_) => self.step_normal(gc),
            _ => self.ves_context().handle_exception(gc),
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
        let i = &self.stack.state().info_handle.instructions[ip];

        // Synchronize original IP and stack height for suspension/retry logic (e.g. Yield)
        self.stack.execution.original_ip = ip;
        self.stack.execution.original_stack_height =
            self.stack.execution.evaluation_stack.top_of_stack();

        vm_trace_instruction!(
            self.stack,
            ip,
            &i.show(
                self.stack
                    .state()
                    .info_handle
                    .source
                    .resolution()
                    .definition()
            )
        );

        tracing::debug!(
            "step_normal: frame={}, ip={}, instr={:?}",
            self.stack.execution.frame_stack.len(),
            ip,
            i.opcode()
        );

        let mut ctx = self.ves_context();
        let res = if let Some(res) = InstructionRegistry::dispatch(gc, &mut ctx, i) {
            res
        } else {
            panic!("Unregistered instruction: {:?}", i);
        };

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
                let res = self.stack.handle_return(gc);
                tracing::debug!("step_normal: handle_return result={:?}", res);
                res
            }
            _ => res,
        }
    }

    /// Run the engine until it needs to yield, returns from the entry point, or throws an unhandled exception.
    pub fn run(&mut self, gc: GCHandle<'gc>) -> StepResult {
        loop {
            let res = match self.stack.execution.exception_mode {
                ExceptionState::None
                | ExceptionState::ExecutingHandler(_)
                | ExceptionState::Filtering(_) => {
                    // Optimized path for normal execution: run multiple instructions
                    // without checking exception mode every time, relying on instructions
                    // to return StepResult::Exception if they throw.
                    let mut last_res = StepResult::Continue;
                    // Batch multiple instructions to reduce dispatch overhead.
                    // Safe because any state-changing instruction must return a non-Continue result
                    // (e.g., Jump/Return/Exception), which breaks out of this loop.
                    for _ in 0..128 {
                        last_res = self.step_normal(gc);
                        if last_res != StepResult::Continue {
                            break;
                        }
                    }
                    last_res
                }
                _ => {
                    let res = self.ves_context().handle_exception(gc);
                    match res {
                        StepResult::Jump(target) => {
                            self.stack.branch(target);
                            StepResult::Continue
                        }
                        StepResult::Return => self.stack.handle_return(gc),
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
        ctx: Option<&ResolutionContext<'_, 'm>>,
    ) -> StepResult {
        self.ves_context()
            .unified_dispatch(gc, source, this_type, ctx)
    }

    pub fn dispatch_method(
        &mut self,
        gc: GCHandle<'gc>,
        method: MethodDescription,
        lookup: GenericLookup,
    ) -> StepResult {
        let mut ctx = self.stack.ves_context();
        tracing::debug!(
            "dispatch_method: method={:?}, pinvoke={:?}, internal_call={}",
            method,
            method.method.pinvoke,
            method.method.internal_call
        );
        if ctx.is_intrinsic_cached(method) {
            crate::intrinsics::intrinsic_call(gc, &mut ctx, method, &lookup)
        } else if method.method.pinvoke.is_some() {
            crate::pinvoke::external_call(&mut ctx, method, gc);
            if matches!(*ctx.exception_mode, ExceptionState::Throwing(_)) {
                StepResult::Exception
            } else {
                StepResult::Continue
            }
        } else {
            if method.method.internal_call {
                panic!("intrinsic not found: {:?}", method);
            }

            if method.method.body.is_none() {
                if let Some(result) = crate::intrinsics::delegates::try_delegate_dispatch(
                    &mut ctx, gc, method, &lookup,
                ) {
                    return result;
                }

                panic!(
                    "no body in executing method: {}.{}",
                    method.parent.type_name(),
                    method.method.name
                );
            }

            ctx.call_frame(
                gc,
                MethodInfo::new(method, &lookup, ctx.shared.clone()),
                lookup,
            );
            StepResult::FramePushed
        }
    }

    fn handle_multicast_step(&mut self, gc: GCHandle<'gc>) -> StepResult {
        let targets_len = {
            let frame = self.stack.current_frame();
            let state = frame.multicast_state.as_ref().unwrap();
            ObjectRef(Some(state.targets)).as_vector(|v| v.layout.length)
        };

        let mut ctx = self.stack.ves_context();

        // 1. Handle return from previous target
        if ctx.frame_stack.current_frame().stack_height > 0 {
            let frame = ctx.frame_stack.current_frame();
            let invoke_method = frame.state.info_handle.source;
            let has_return_value = invoke_method.method.signature.return_type.1.is_some();

            let next_index = frame.multicast_state.as_ref().unwrap().next_index;
            if next_index < targets_len && has_return_value {
                let _ = ctx.pop(gc);
            }
        }

        let (target_delegate, args, _next_index) = {
            let frame = ctx.frame_stack.current_frame_mut();
            let state = frame.multicast_state.as_mut().unwrap();
            let index = state.next_index;
            if index < targets_len {
                let target = ObjectRef(Some(state.targets)).as_vector(|v| {
                    let offset = index * v.layout.element_layout.as_ref().size();
                    unsafe { ObjectRef::read_unchecked(&v.get()[offset..]) }
                });
                state.next_index += 1;
                (Some(target), state.args.clone(), index)
            } else {
                (None, vec![], index)
            }
        };

        if let Some(target_delegate) = target_delegate {
            ctx.push(gc, StackValue::ObjectRef(target_delegate));
            for arg in args {
                ctx.push(gc, arg);
            }

            let invoke_method = ctx.frame_stack.current_frame().state.info_handle.source;
            let lookup = ctx.frame_stack.current_frame().generic_inst.clone();

            ctx.dispatch_method(gc, invoke_method, lookup)
        } else {
            // All targets called.
            self.stack.handle_return(gc)
        }
    }
}

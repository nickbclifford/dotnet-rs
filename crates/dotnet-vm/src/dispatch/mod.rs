use crate::vm_trace_instruction;
use dotnet_types::{TypeDescription, generics::GenericLookup, members::MethodDescription};
use dotnet_utils::gc::GCHandle;
use dotnetdll::prelude::*;
use gc_arena::Collect;

pub mod registry;

use super::{
    CallStack, MethodInfo, ResolutionContext, StepResult, exceptions::ExceptionState,
    threading::ThreadManagerOps,
};

pub struct InstructionRegistry;

impl InstructionRegistry {
    pub fn dispatch<'gc, 'm>(
        gc: GCHandle<'gc>,
        interp: &mut ExecutionEngine<'gc, 'm>,
        instr: &Instruction,
    ) -> Option<StepResult> {
        let handler = registry::get_handler(instr.opcode())?;
        let mut ctx = interp.ves_context();
        Some(handler(&mut ctx, gc, instr))
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

    pub fn step_normal(&mut self, gc: GCHandle<'gc>) -> StepResult {
        let i = &self.stack.state().info_handle.instructions[self.stack.state().ip];

        if self.stack.tracer_enabled() {
            let ip = self.stack.state().ip;
            let i_res = self.stack.state().info_handle.source.resolution();
            vm_trace_instruction!(self.stack, ip, &i.show(i_res.definition()));
        }

        let res = if let Some(res) = InstructionRegistry::dispatch(gc, self, i) {
            res
        } else {
            panic!("Unregistered instruction: {:?}", i);
        };

        match res {
            StepResult::Continue => {
                self.stack.increment_ip();
                StepResult::Continue
            }
            StepResult::Jump(target) => {
                self.stack.branch(target);
                StepResult::Continue
            }
            StepResult::Yield => {
                self.stack.increment_ip();
                StepResult::Yield
            }
            StepResult::Return => self.stack.handle_return(gc),
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
        ctx: Option<&ResolutionContext<'gc, 'm>>,
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
        if ctx.is_intrinsic_cached(method) {
            crate::intrinsics::intrinsic_call(gc, &mut ctx, method, &lookup)
        } else if method.method.pinvoke.is_some() {
            ctx.external_call(method, gc);
            StepResult::Continue
        } else {
            if method.method.internal_call {
                panic!("intrinsic not found: {:?}", method);
            }
            ctx.call_frame(
                gc,
                MethodInfo::new(method, &lookup, ctx.shared.clone()),
                lookup,
            );
            StepResult::FramePushed
        }
    }
}

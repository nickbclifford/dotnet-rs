use crate::{
    MethodState, StepResult,
    dispatch::ExecutionEngine,
    exceptions::ExceptionState,
    state::{ArenaLocalState, SharedGlobalState},
    sync::{Arc, MutexGuard, Ordering},
    tracer::Tracer,
};
use dotnet_utils::gc::GCHandle;
use dotnet_value::StackValue;
use gc_arena::{Arena, Collect, Rootable};
use std::cell::Cell;

pub mod context;
pub mod evaluation_stack;
pub mod frames;

pub use context::*;
pub use evaluation_stack::*;
pub use frames::*;

pub struct CallStack<'gc, 'm> {
    pub execution: ThreadContext<'gc, 'm>,
    pub shared: Arc<SharedGlobalState<'m>>,
    pub local: ArenaLocalState<'gc>,
    pub thread_id: Cell<u64>,
}

unsafe impl<'gc, 'm: 'gc> Collect for CallStack<'gc, 'm> {
    fn trace(&self, cc: &gc_arena::Collection) {
        // NOTE: tracing_id is managed by execute_gc_command_for_current_thread,
        // NOT here. Setting/clearing it here would interfere with gc-arena's
        // deferred work list processing (StackValue traces happen AFTER this returns).
        self.execution.trace(cc);
        self.local.trace(cc);
        self.shared.statics.trace(cc);
    }
}

pub type GCArena = Arena<Rootable!['gc => ExecutionEngine<'gc, 'static>]>;

impl<'gc, 'm: 'gc> CallStack<'gc, 'm> {
    pub fn new(shared: Arc<SharedGlobalState<'m>>, local: ArenaLocalState<'gc>) -> Self {
        Self {
            execution: ThreadContext {
                evaluation_stack: EvaluationStack::new(),
                frame_stack: FrameStack::new(),
                exception_mode: ExceptionState::None,
                original_ip: 0,
                original_stack_height: 0,
            },
            shared,
            local,
            thread_id: Cell::new(0),
        }
    }

    pub fn ves_context(&mut self) -> VesContext<'_, 'gc, 'm> {
        VesContext {
            evaluation_stack: &mut self.execution.evaluation_stack,
            frame_stack: &mut self.execution.frame_stack,
            shared: &self.shared,
            local: &mut self.local,
            exception_mode: &mut self.execution.exception_mode,
            thread_id: &self.thread_id,
            original_ip: &mut self.execution.original_ip,
            original_stack_height: &mut self.execution.original_stack_height,
        }
    }

    pub(super) fn get_slot(&self, index: usize) -> StackValue<'gc> {
        self.execution.evaluation_stack.stack[index].clone()
    }

    pub fn handle_return(&mut self, gc: GCHandle<'gc>) -> StepResult {
        let tracer_enabled = self.tracer_enabled();
        self.execution.frame_stack.handle_return(
            gc,
            &mut self.execution.evaluation_stack,
            &self.shared,
            &self.local.heap,
            tracer_enabled,
        )
    }

    pub fn return_frame(&mut self, gc: GCHandle<'gc>) {
        let tracer_enabled = self.tracer_enabled();
        self.execution.frame_stack.return_frame(
            gc,
            &mut self.execution.evaluation_stack,
            &self.shared,
            &self.local.heap,
            tracer_enabled,
        );
    }

    pub fn unwind_frame(&mut self, gc: GCHandle<'gc>) {
        self.execution.frame_stack.unwind_frame(
            gc,
            &mut self.execution.evaluation_stack,
            &self.shared,
            &self.local.heap,
        );
    }

    pub fn current_frame(&self) -> &StackFrame<'gc, 'm> {
        self.execution.frame_stack.current_frame()
    }

    pub fn current_frame_mut(&mut self) -> &mut StackFrame<'gc, 'm> {
        self.execution.frame_stack.current_frame_mut()
    }

    pub fn state(&self) -> &MethodState<'m> {
        self.execution.frame_stack.state()
    }

    pub fn state_mut(&mut self) -> &mut MethodState<'m> {
        self.execution.frame_stack.state_mut()
    }

    pub fn branch(&mut self, target: usize) {
        crate::vm_trace_branch!(self, "BR", target, true);
        self.execution.frame_stack.branch(target);
    }

    pub fn conditional_branch(&mut self, condition: bool, target: usize) -> bool {
        crate::vm_trace_branch!(self, "BR_COND", target, condition);
        self.execution
            .frame_stack
            .conditional_branch(condition, target)
    }

    pub fn increment_ip(&mut self) {
        self.execution.frame_stack.increment_ip();
    }

    pub fn tracer_enabled(&self) -> bool {
        self.shared.tracer_enabled.load(Ordering::Relaxed)
    }

    pub fn indent(&self) -> usize {
        self.execution.frame_stack.len().saturating_sub(1)
    }

    pub fn tracer(&self) -> MutexGuard<'_, Tracer> {
        self.shared.tracer.lock()
    }
}

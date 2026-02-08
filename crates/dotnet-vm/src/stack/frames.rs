use crate::{
    MethodState, StepResult, memory::heap::HeapManager, stack::context::StackFrame,
    state::SharedGlobalState,
};
use dotnet_utils::gc::GCHandle;
use dotnet_value::StackValue;
use dotnetdll::prelude::ReturnType;
use gc_arena::Collect;

use super::evaluation_stack::EvaluationStack;

#[derive(Collect, Default)]
#[collect(no_drop)]
pub struct FrameStack<'gc, 'm> {
    pub(crate) frames: Vec<StackFrame<'gc, 'm>>,
    pub(crate) suspended_frames: Vec<StackFrame<'gc, 'm>>,
}

impl<'gc, 'm> FrameStack<'gc, 'm> {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, frame: StackFrame<'gc, 'm>) {
        self.frames.push(frame);
    }

    pub fn pop(&mut self) -> Option<StackFrame<'gc, 'm>> {
        self.frames.pop()
    }

    pub fn current_frame(&self) -> &StackFrame<'gc, 'm> {
        self.frames.last().expect("Frame stack underflow")
    }

    pub fn current_frame_opt(&self) -> Option<&StackFrame<'gc, 'm>> {
        self.frames.last()
    }

    pub fn current_frame_opt_mut(&mut self) -> Option<&mut StackFrame<'gc, 'm>> {
        self.frames.last_mut()
    }

    pub fn current_frame_mut(&mut self) -> &mut StackFrame<'gc, 'm> {
        self.frames.last_mut().expect("Frame stack underflow")
    }

    pub fn state(&self) -> &MethodState<'m> {
        &self.current_frame().state
    }

    pub fn state_mut(&mut self) -> &mut MethodState<'m> {
        &mut self.current_frame_mut().state
    }

    pub fn branch(&mut self, target: usize) {
        self.state_mut().ip = target;
    }

    pub fn conditional_branch(&mut self, condition: bool, target: usize) -> bool {
        if condition {
            self.state_mut().ip = target;
            true
        } else {
            false
        }
    }

    pub fn increment_ip(&mut self) {
        self.state_mut().ip += 1;
    }

    pub fn is_empty(&self) -> bool {
        self.frames.is_empty()
    }

    pub fn len(&self) -> usize {
        self.frames.len()
    }

    pub fn clear(&mut self) {
        self.frames.clear();
        self.suspended_frames.clear();
    }

    pub fn clear_suspended(&mut self) {
        self.suspended_frames.clear();
    }

    pub fn suspend_above(&mut self, index: usize) {
        self.suspended_frames = self.frames.split_off(index + 1);
    }

    pub fn restore_suspended(&mut self) {
        self.frames.append(&mut self.suspended_frames);
    }

    pub fn unwind_frame(
        &mut self,
        gc: GCHandle<'gc>,
        evaluation_stack: &mut EvaluationStack<'gc>,
        shared: &SharedGlobalState<'m>,
        heap: &HeapManager<'gc>,
    ) {
        let frame = self.pop().expect("unwind_frame called with empty stack");

        if frame.state.info_handle.is_cctor {
            let type_desc = frame.state.info_handle.source.parent;
            shared.statics.mark_failed(type_desc, &frame.generic_inst);
        }

        if frame.is_finalizer {
            heap.processing_finalizer.set(false);
        }
        for i in frame.base.arguments..evaluation_stack.top_of_stack() {
            evaluation_stack.set_slot(gc, i, StackValue::null());
        }
        evaluation_stack.truncate(frame.base.arguments);
    }

    pub fn return_frame(
        &mut self,
        gc: GCHandle<'gc>,
        evaluation_stack: &mut EvaluationStack<'gc>,
        shared: &SharedGlobalState<'m>,
        heap: &HeapManager<'gc>,
        tracer_enabled: bool,
    ) {
        let frame = self.pop().unwrap();

        if tracer_enabled {
            let method_name = format!("{:?}", frame.state.info_handle.source);
            shared
                .tracer
                .lock()
                .trace_method_exit(self.len(), &method_name);
        }

        if frame.state.info_handle.is_cctor {
            let type_desc = frame.state.info_handle.source.parent;
            shared
                .statics
                .mark_initialized(type_desc, &frame.generic_inst);
        }

        if frame.is_finalizer {
            heap.processing_finalizer.set(false);
        }

        let signature = frame.state.info_handle.signature;
        let return_value = if let ReturnType(_, Some(_)) = &signature.return_type {
            let return_slot_index = frame.base.stack + frame.stack_height - 1;
            Some(evaluation_stack.get_slot(return_slot_index))
        } else {
            None
        };

        for i in frame.base.arguments..evaluation_stack.top_of_stack() {
            evaluation_stack.set_slot(gc, i, StackValue::null());
        }
        evaluation_stack.truncate(frame.base.arguments);

        if let Some(return_value) = return_value {
            if !self.is_empty() {
                // push to evaluation stack
                // wait, how to push back to evaluation stack?
                // we need to call push on evaluation_stack
                evaluation_stack.push(gc, return_value);
                self.current_frame_mut().stack_height += 1;
            } else {
                evaluation_stack.push(gc, return_value);
            }
        }
    }

    pub fn handle_return(
        &mut self,
        gc: GCHandle<'gc>,
        evaluation_stack: &mut EvaluationStack<'gc>,
        shared: &SharedGlobalState<'m>,
        heap: &HeapManager<'gc>,
        tracer_enabled: bool,
    ) -> StepResult {
        if self.is_empty() {
            tracing::debug!("handle_return: stack empty, returning Return");
            return StepResult::Return;
        }

        let was_auto_invoked = {
            let frame = self.current_frame();
            frame.state.info_handle.is_cctor || frame.is_finalizer
        };

        tracing::debug!(
            "handle_return: popping frame, was_auto_invoked={}",
            was_auto_invoked
        );

        self.return_frame(gc, evaluation_stack, shared, heap, tracer_enabled);

        if self.is_empty() {
            tracing::debug!("handle_return: stack empty after pop, returning Return");
            return StepResult::Return;
        }

        if !was_auto_invoked {
            tracing::debug!("handle_return: incrementing IP of caller");
            self.increment_ip();
            let out_of_bounds = {
                let frame = self.current_frame();
                frame.state.ip >= frame.state.info_handle.instructions.len()
            };
            if out_of_bounds {
                tracing::debug!("handle_return: out of bounds, recursive return");
                return self.handle_return(gc, evaluation_stack, shared, heap, tracer_enabled);
            }
        } else {
            tracing::debug!("handle_return: NOT incrementing IP (was auto-invoked)");
        }

        StepResult::Continue
    }
}

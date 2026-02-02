use dotnet_types::{
    generics::GenericLookup,
    resolution::ResolutionS,
};
use dotnet_value::{
    object::ObjectRef,
    StackValue,
};
use gc_arena::Collect;
use crate::{exceptions::ExceptionState, MethodInfo, MethodState};

#[derive(Collect)]
#[collect(no_drop)]
pub struct ThreadContext<'gc, 'm> {
    pub stack: Vec<StackValue<'gc>>,
    pub frames: Vec<StackFrame<'gc, 'm>>,
    pub exception_mode: ExceptionState<'gc>,
    pub suspended_frames: Vec<StackFrame<'gc, 'm>>,
    pub suspended_stack: Vec<StackValue<'gc>>,
    pub original_ip: usize,
    pub original_stack_height: usize,
}

pub struct StackFrame<'gc, 'm> {
    pub stack_height: usize,
    pub base: BasePointer,
    pub state: MethodState<'m>,
    pub generic_inst: GenericLookup,
    pub source_resolution: ResolutionS,
    /// The exceptions currently being handled by catch blocks in this frame (required for rethrow).
    pub exception_stack: Vec<ObjectRef<'gc>>,
    pub pinned_locals: Vec<bool>,
    pub is_finalizer: bool,
}

unsafe impl<'gc, 'm> Collect for StackFrame<'gc, 'm> {
    fn trace(&self, cc: &gc_arena::Collection) {
        self.exception_stack.trace(cc);
        self.state.trace(cc);
        self.generic_inst.trace(cc);
    }
}

impl<'gc, 'm> StackFrame<'gc, 'm> {
    pub fn new(
        base_pointer: BasePointer,
        method: MethodInfo<'m>,
        generic_inst: GenericLookup,
        pinned_locals: Vec<bool>,
    ) -> Self {
        Self {
            stack_height: 0,
            base: base_pointer,
            source_resolution: method.source.resolution(),
            state: MethodState::new(method),
            generic_inst,
            exception_stack: Vec::new(),
            pinned_locals,
            is_finalizer: false,
        }
    }
}

#[derive(Debug)]
pub struct BasePointer {
    pub arguments: usize,
    pub locals: usize,
    pub stack: usize,
}

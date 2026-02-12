use crate::{
    MethodInfo, MethodState, ResolutionContext, StepResult,
    exceptions::ExceptionState,
    memory::ops::MemoryOps,
    resolution::{TypeResolutionExt, ValueResolution},
    stack::{evaluation_stack::EvaluationStack, frames::FrameStack, ops::*},
    state::{ArenaLocalState, SharedGlobalState},
    sync::{Arc, MutexGuard},
    tracer::Tracer,
};
use dotnet_types::{
    TypeDescription,
    comparer::decompose_type_source,
    error::TypeResolutionError,
    generics::{ConcreteType, GenericLookup},
    members::MethodDescription,
    resolution::ResolutionS,
    runtime::RuntimeType,
};
use dotnet_utils::{gc::GCHandle, sync::Ordering};
use dotnet_value::{
    StackValue,
    object::{HeapStorage, ObjectHandle, ObjectRef},
    storage::FieldStorage,
};
use dotnetdll::prelude::*;
use gc_arena::Collect;
use std::ptr::NonNull;

pub struct VesContext<'a, 'gc, 'm> {
    pub(crate) gc: GCHandle<'gc>,
    pub(crate) evaluation_stack: &'a mut EvaluationStack<'gc>,
    pub(crate) frame_stack: &'a mut FrameStack<'gc, 'm>,
    pub(crate) shared: &'a Arc<SharedGlobalState<'m>>,
    pub(crate) local: &'a mut ArenaLocalState<'gc>,
    pub(crate) exception_mode: &'a mut ExceptionState<'gc>,
    pub(crate) thread_id: &'a std::cell::Cell<u64>,
    pub(crate) original_ip: &'a mut usize,
    pub(crate) original_stack_height: &'a mut usize,
}

impl<'a, 'gc, 'm: 'gc> VesContext<'a, 'gc, 'm> {
    #[inline]
    pub(crate) fn on_push(&mut self) {
        if let Some(frame) = self.frame_stack.current_frame_opt_mut() {
            frame.stack_height += 1;
        }
    }

    #[inline]
    pub(crate) fn trace_push(&self, value: &StackValue<'gc>) {
        if self.tracer_enabled() {
            let val_str = format!("{:?}", value);
            self.tracer()
                .trace_stack_op(self.indent(), "PUSH", &val_str);
        }
    }

    #[inline]
    pub(crate) fn trace_pop(&self, value: &StackValue<'gc>) {
        if self.tracer_enabled() {
            let val_str = format!("{:?}", value);
            self.tracer().trace_stack_op(self.indent(), "POP", &val_str);
        }
    }

    #[inline]
    pub(crate) fn on_pop(&mut self) {
        if let Some(frame) = self.frame_stack.current_frame_opt_mut() {
            if frame.stack_height == 0 {
                panic!(
                    "Stack height underflow in frame {:?}",
                    frame.state.info_handle.source
                );
            }
            frame.stack_height -= 1;
        }
    }

    #[inline]
    pub(crate) fn on_pop_safe(&mut self) -> Result<(), crate::error::VmError> {
        if let Some(frame) = self.frame_stack.current_frame_opt_mut() {
            if frame.stack_height == 0 {
                return Err(crate::error::VmError::Execution(
                    crate::error::ExecutionError::StackUnderflow,
                ));
            }
            frame.stack_height -= 1;
        }
        Ok(())
    }

    pub fn top_of_stack_address(&self) -> NonNull<u8> {
        self.evaluation_stack
            .get_slot_address(self.evaluation_stack.top_of_stack().saturating_sub(1))
    }

    pub(crate) fn init_locals(
        &mut self,
        method: MethodDescription,
        locals: &'m [LocalVariable],
        generics: &GenericLookup,
    ) -> Result<(Vec<StackValue<'gc>>, Vec<bool>), TypeResolutionError> {
        let mut values = vec![];
        let mut pinned_locals = vec![];

        for l in locals {
            use BaseType::*;
            use LocalVariable::*;
            match l {
                TypedReference => {
                    values.push(StackValue::null());
                    pinned_locals.push(false);
                }
                Variable {
                    by_ref: _,
                    var_type,
                    pinned,
                    ..
                } => {
                    pinned_locals.push(*pinned);
                    let ctx = ResolutionContext {
                        generics,
                        loader: self.shared.loader,
                        resolution: method.resolution(),
                        type_owner: Some(method.parent),
                        method_owner: Some(method),
                        caches: self.shared.caches.clone(),
                        shared: Some(self.shared.clone()),
                    };

                    let v = match ctx.make_concrete(var_type)?.get() {
                        Type { source, .. } => {
                            let (ut, type_generics) = decompose_type_source::<ConcreteType>(source);
                            let desc = ctx.locate_type(ut)?;

                            if desc.is_value_type(&ctx)? {
                                let new_lookup = GenericLookup {
                                    type_generics: type_generics.into(),
                                    ..generics.clone()
                                };
                                let new_ctx = ctx.with_generics(&new_lookup);
                                let instance = new_ctx.new_object(desc)?;
                                StackValue::ValueType(instance)
                            } else {
                                StackValue::null()
                            }
                        }
                        _ => StackValue::null(),
                    };
                    values.push(v);
                }
            }
        }
        Ok((values, pinned_locals))
    }

    pub(crate) fn handle_return(&mut self) -> StepResult {
        let tracer_enabled = self.tracer_enabled();
        let shared = &self.shared;
        let heap = &self.local.heap;
        let res =
            self.frame_stack
                .handle_return(self.evaluation_stack, shared, heap, tracer_enabled);

        if let Some(return_type) = self
            .frame_stack
            .current_frame_opt()
            .and_then(|f| f.awaiting_invoke_return.clone())
        {
            self.frame_stack.current_frame_mut().awaiting_invoke_return = None;

            let is_void = matches!(return_type, RuntimeType::Void);
            if is_void {
                self.push(StackValue::null());
            } else {
                let val = self.pop();
                let return_concrete = return_type.to_concrete(self.loader());
                let td = self
                    .loader()
                    .find_concrete_type(return_concrete.clone())
                    .expect("Type must exist for init_locals");
                if td
                    .is_value_type(&self.current_context())
                    .expect("Failed to check if return type is value type")
                {
                    let boxed = ObjectRef::new(
                        self.gc,
                        HeapStorage::Boxed(
                            self.new_value_type(&return_concrete, val)
                                .expect("Failed to create value type for return"),
                        ),
                    );
                    self.register_new_object(&boxed);
                    self.push_obj(boxed);
                } else {
                    // already a reference type (or null)
                    self.push(val);
                }
            }
        }

        res
    }

    #[inline]
    pub(crate) fn handle_exception(&mut self) -> StepResult {
        let gc = self.gc;
        crate::exceptions::ExceptionHandlingSystem.handle_exception(self, gc)
    }

    #[inline]
    pub(crate) fn unwind_frame(&mut self) {
        self.frame_stack
            .unwind_frame(self.evaluation_stack, self.shared, &self.local.heap);
    }

    #[inline]
    pub(crate) fn tracer_enabled(&self) -> bool {
        self.shared.tracer_enabled.load(Ordering::Relaxed)
    }

    #[inline]
    pub(crate) fn tracer(&self) -> MutexGuard<'_, Tracer> {
        self.shared.tracer.lock()
    }

    #[inline]
    pub(crate) fn indent(&self) -> usize {
        self.frame_stack.len()
    }

    pub fn locate_type(&self, handle: UserType) -> TypeDescription {
        let f = self.current_frame();
        self.resolver()
            .locate_type(f.source_resolution, handle)
            .expect("Type resolution failed")
    }

    pub fn new_instance_fields(
        &self,
        td: TypeDescription,
    ) -> Result<FieldStorage, TypeResolutionError> {
        self.resolver()
            .new_instance_fields(td, &self.current_context())
    }

    pub fn new_static_fields(
        &self,
        td: TypeDescription,
    ) -> Result<FieldStorage, TypeResolutionError> {
        self.resolver()
            .new_static_fields(td, &self.current_context())
    }

    #[inline]
    pub(crate) fn register_new_object(&self, instance: &ObjectRef<'gc>) {
        if let Some(ptr) = instance.0 {
            let addr = gc_arena::Gc::as_ptr(ptr) as usize;
            self.local
                .heap
                ._all_objs
                .borrow_mut()
                .insert(addr, *instance);

            // Add to finalization queue if it has a finalizer
            if let HeapStorage::Obj(o) = &ptr.borrow().storage
                && (o.description.static_initializer().is_some()
                    || o.description
                        .definition()
                        .methods
                        .iter()
                        .any(|m| m.name == "Finalize"))
            {
                self.local
                    .heap
                    .finalization_queue
                    .borrow_mut()
                    .push(*instance);
            }
        }
    }
}

impl<'a, 'gc, 'm: 'gc> VesOps<'gc, 'm> for VesContext<'a, 'gc, 'm> {
    #[inline]
    fn run(&mut self) -> StepResult {
        let _gc = self.gc;
        loop {
            let res = match self.exception_mode {
                ExceptionState::None
                | ExceptionState::ExecutingHandler(_)
                | ExceptionState::Filtering(_) => {
                    let mut last_res = StepResult::Continue;
                    for _ in 0..128 {
                        let ip = self.state().ip;
                        let i = &self.state().info_handle.instructions[ip];

                        last_res = if let Some(res) =
                            crate::dispatch::InstructionRegistry::dispatch(self, i)
                        {
                            res
                        } else {
                            panic!("Unregistered instruction: {:?}", i);
                        };

                        match last_res {
                            StepResult::Continue => {
                                self.increment_ip();
                            }
                            StepResult::Jump(target) => {
                                self.branch(target);
                                last_res = StepResult::Continue;
                            }
                            StepResult::Yield => {
                                break;
                            }
                            StepResult::Return => {
                                last_res = self.handle_return();
                                break;
                            }
                            _ => break,
                        }
                    }
                    last_res
                }
                _ => {
                    let res = self.handle_exception();
                    match res {
                        StepResult::Jump(target) => {
                            self.branch(target);
                            StepResult::Continue
                        }
                        StepResult::Return => self.handle_return(),
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

    #[inline]
    fn handle_return(&mut self) -> StepResult {
        self.handle_return()
    }

    #[inline]
    fn handle_exception(&mut self) -> StepResult {
        self.handle_exception()
    }

    #[inline]
    fn branch(&mut self, target: usize) {
        crate::vm_trace_branch!(self, "BR", target, true);
        self.frame_stack.branch(target);
    }

    #[inline]
    fn conditional_branch(&mut self, condition: bool, target: usize) -> bool {
        crate::vm_trace_branch!(self, "BR_COND", target, condition);
        self.frame_stack.conditional_branch(condition, target)
    }

    #[inline]
    fn increment_ip(&mut self) {
        self.frame_stack.increment_ip();
    }

    #[inline]
    fn state(&self) -> &crate::MethodState<'m> {
        self.frame_stack.state()
    }

    #[inline]
    fn state_mut(&mut self) -> &mut crate::MethodState<'m> {
        self.frame_stack.state_mut()
    }

    #[inline]
    fn exception_mode(&self) -> &ExceptionState<'gc> {
        self.exception_mode
    }

    #[inline]
    fn exception_mode_mut(&mut self) -> &mut ExceptionState<'gc> {
        self.exception_mode
    }

    #[inline]
    fn evaluation_stack(&self) -> &EvaluationStack<'gc> {
        self.evaluation_stack
    }

    #[inline]
    fn evaluation_stack_mut(&mut self) -> &mut EvaluationStack<'gc> {
        self.evaluation_stack
    }

    #[inline]
    fn frame_stack(&self) -> &FrameStack<'gc, 'm> {
        self.frame_stack
    }

    #[inline]
    fn frame_stack_mut(&mut self) -> &mut FrameStack<'gc, 'm> {
        self.frame_stack
    }

    #[inline]
    fn original_ip(&self) -> usize {
        *self.original_ip
    }

    #[inline]
    fn original_ip_mut(&mut self) -> &mut usize {
        self.original_ip
    }

    #[inline]
    fn original_stack_height(&self) -> usize {
        *self.original_stack_height
    }

    #[inline]
    fn original_stack_height_mut(&mut self) -> &mut usize {
        self.original_stack_height
    }

    #[inline]
    fn unwind_frame(&mut self) {
        self.unwind_frame()
    }

    #[inline]
    fn tracer_enabled(&self) -> bool {
        self.tracer_enabled()
    }

    #[inline]
    fn tracer(&self) -> MutexGuard<'_, Tracer> {
        self.tracer()
    }

    #[inline]
    fn indent(&self) -> usize {
        self.indent()
    }

    #[inline]
    fn process_pending_finalizers(&mut self) -> StepResult {
        let _gc = self.gc;
        if self.local.heap.processing_finalizer.get() {
            return StepResult::Continue;
        }
        let instance = self.local.heap.pending_finalization.borrow_mut().pop();

        // Only run finalizers if we are at the bottom of the stack or in a safe state
        // For now, let's just run them one by one
        if let Some(instance) = instance {
            self.local.heap.processing_finalizer.set(true);
            let ctx = self.current_context();

            if self.tracer_enabled() {
                let ptr = instance.0.unwrap();
                let obj_type = match &ptr.borrow().storage {
                    HeapStorage::Obj(o) => o.description,
                    _ => unreachable!(),
                };
                let type_name = format!("{:?}", obj_type);
                let addr = gc_arena::Gc::as_ptr(ptr) as usize;
                self.shared
                    .tracer
                    .lock()
                    .trace_gc_finalization(self.indent(), &type_name, addr);
            }

            let finalizer: Option<MethodDescription> = instance.as_heap_storage(|storage| {
                if let HeapStorage::Obj(o) = storage {
                    let obj_type = o.description;
                    let object_type = self
                        .shared
                        .loader
                        .corlib_type("System.Object")
                        .expect("System.Object must exist in corlib");
                    let base_finalize = object_type
                        .definition()
                        .methods
                        .iter()
                        .find(|m| {
                            m.name == "Finalize"
                                && m.virtual_member
                                && m.signature.parameters.is_empty()
                        })
                        .expect("System.Object::Finalize not found");

                    let method_desc = MethodDescription {
                        parent: object_type,
                        method: base_finalize,
                        method_resolution: object_type.resolution,
                    };

                    Some(
                        self.resolver()
                            .resolve_virtual_method(
                                method_desc,
                                obj_type,
                                &GenericLookup::default(),
                                &ctx,
                            )
                            .expect("Failed to resolve finalizer"),
                    )
                } else {
                    None
                }
            });

            let Some(finalizer) = finalizer else {
                self.local.heap.processing_finalizer.set(false);
                return StepResult::Continue;
            };

            let method_info = vm_try!(MethodInfo::new(
                finalizer,
                &self.shared.empty_generics,
                self.shared.clone()
            ));

            // Push the object as 'this'
            self.push(StackValue::ObjectRef(instance));

            vm_try!(self.call_frame(method_info, self.shared.empty_generics.clone()));
            self.current_frame_mut().is_finalizer = true;
            return StepResult::FramePushed;
        }

        StepResult::Continue
    }

    #[inline]
    fn back_up_ip(&mut self) {
        self.frame_stack.current_frame_mut().state.ip = *self.original_ip;
        self.evaluation_stack.truncate(*self.original_stack_height);
        if let Some(frame) = self.frame_stack.current_frame_opt_mut() {
            frame.stack_height = (*self.original_stack_height).saturating_sub(frame.base.stack);
        }
    }
}

#[derive(Collect)]
#[collect(no_drop)]
pub struct ThreadContext<'gc, 'm> {
    pub evaluation_stack: EvaluationStack<'gc>,
    pub frame_stack: FrameStack<'gc, 'm>,
    pub exception_mode: ExceptionState<'gc>,
    pub original_ip: usize,
    pub original_stack_height: usize,
}

#[derive(Collect)]
#[collect(no_drop)]
pub struct MulticastState<'gc> {
    pub targets: ObjectHandle<'gc>,
    pub next_index: usize,
    pub args: Vec<StackValue<'gc>>,
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
    pub multicast_state: Option<MulticastState<'gc>>,
    pub awaiting_invoke_return: Option<RuntimeType>,
}

// SAFETY: `StackFrame` correctly traces all GC-managed fields (`exception_stack`, `state`, `generic_inst`, and `multicast_state`)
// in its `trace` implementation. All other fields are either primitives or non-GC references
// and do not need tracing. This implementation is safe because it delegates to the `Collect`
// implementations of its components.
unsafe impl<'gc, 'm> Collect for StackFrame<'gc, 'm> {
    fn trace(&self, cc: &gc_arena::Collection) {
        self.exception_stack.trace(cc);
        self.state.trace(cc);
        self.generic_inst.trace(cc);
        self.multicast_state.trace(cc);
        self.awaiting_invoke_return.trace(cc);
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
            multicast_state: None,
            awaiting_invoke_return: None,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct BasePointer {
    pub arguments: usize,
    pub locals: usize,
    pub stack: usize,
}

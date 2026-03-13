use crate::{
    MethodState, ResolutionContext, StepResult,
    resolution::{TypeResolutionExt, ValueResolution},
    stack::ops::*,
    state::{ArenaLocalState, SharedGlobalState},
    sync::Arc,
    tracer::Tracer,
};
use dotnet_types::{
    TypeDescription,
    comparer::decompose_type_source,
    error::TypeResolutionError,
    generics::{ConcreteType, GenericLookup},
    members::MethodDescription,
    runtime::RuntimeType,
};
use dotnet_utils::{gc::GCHandle, sync::Ordering};
use dotnet_value::{
    StackValue,
    object::{HeapStorage, ObjectRef},
    storage::FieldStorage,
};
use dotnetdll::prelude::*;
use gc_arena::Collect;
use std::ptr::NonNull;

pub use dotnet_vm_ops::{BasePointer, EvaluationStack, ExceptionState, FrameStack, StackFrame};

pub struct VesContext<'a, 'gc> {
    pub(crate) gc: GCHandle<'gc>,
    pub(crate) evaluation_stack: &'a mut EvaluationStack<'gc>,
    pub(crate) frame_stack: &'a mut FrameStack<'gc>,
    pub(crate) shared: &'a Arc<SharedGlobalState>,
    pub(crate) local: &'a mut ArenaLocalState<'gc>,
    pub(crate) exception_mode: &'a mut ExceptionState<'gc>,
    pub(crate) current_intrinsic: &'a mut Option<crate::CollectableMethodDescription>,
    pub(crate) thread_id: &'a std::cell::Cell<dotnet_utils::ArenaId>,
    pub(crate) original_ip: &'a mut usize,
    pub(crate) original_stack_height: &'a mut crate::StackSlotIndex,
}

impl<'a, 'gc> VesContext<'a, 'gc> {
    #[inline]
    pub(crate) fn on_push(&mut self) {
        if let Some(frame) = self.frame_stack.current_frame_opt_mut() {
            frame.stack_height += 1usize;
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
            if frame.stack_height == crate::StackSlotIndex(0) {
                panic!(
                    "Stack height underflow in frame {:?}",
                    frame.state.info_handle.source
                );
            }
            frame.stack_height -= 1usize;
        }
    }

    #[inline]
    pub(crate) fn on_pop_safe(&mut self) -> Result<(), crate::error::VmError> {
        if let Some(frame) = self.frame_stack.current_frame_opt_mut() {
            if frame.stack_height == crate::StackSlotIndex(0) {
                return Err(crate::error::VmError::Execution(
                    crate::error::ExecutionError::StackUnderflow,
                ));
            }
            frame.stack_height -= 1usize;
        }
        Ok(())
    }

    pub fn top_of_stack_address(&self) -> NonNull<u8> {
        self.evaluation_stack
            .get_slot_address(self.evaluation_stack.top_of_stack().saturating_sub(1usize))
    }

    pub(crate) fn init_locals(
        &mut self,
        method: MethodDescription,
        locals: &[LocalVariable],
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
                        state: self.shared.resolution_shared(),
                        resolution: method.resolution(),
                        type_owner: Some(method.parent),
                        method_owner: Some(method.clone()),
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
        if self.frame_stack.is_empty() {
            tracing::debug!("handle_return: stack empty, returning Return");
            return StepResult::Return;
        }

        let was_auto_invoked = {
            let frame = self.frame_stack.current_frame();
            frame.state.info_handle.is_cctor || frame.is_finalizer
        };

        tracing::debug!(
            "handle_return: popping frame, was_auto_invoked={}",
            was_auto_invoked
        );

        let _res = self.return_frame();

        if self.frame_stack.is_empty() {
            tracing::debug!("handle_return: stack empty after pop, returning Return");
            return StepResult::Return;
        }

        if !was_auto_invoked {
            if self.frame_stack.current_frame().multicast_state.is_some() {
                tracing::debug!("handle_return: in multicast state, stopping return");
                return StepResult::Continue;
            }

            tracing::debug!("handle_return: incrementing IP of caller");
            self.frame_stack.increment_ip();
            let out_of_bounds = {
                let frame = self.frame_stack.current_frame();
                frame.state.ip >= frame.state.info_handle.instructions.len()
            };
            if out_of_bounds {
                tracing::debug!("handle_return: out of bounds, recursive return");
                return self.handle_return();
            }
        } else {
            tracing::debug!("handle_return: NOT incrementing IP (was auto-invoked)");
        }

        StepResult::Continue
    }

    pub fn return_frame(&mut self) -> StepResult {
        let frame = self.frame_stack.pop().unwrap();

        if self.tracer_enabled() {
            let method_name = format!("{:?}", frame.state.info_handle.source);
            self.shared
                .tracer
                .trace_method_exit(self.frame_stack.len(), &method_name);
        }

        if frame.state.info_handle.is_cctor {
            let type_desc = frame.state.info_handle.source.parent;
            self.shared
                .statics
                .mark_initialized(type_desc, &frame.generic_inst);
        }

        if frame.is_finalizer {
            self.local.heap.processing_finalizer.set(false);
        }

        let signature = frame.state.info_handle.signature;
        let return_value = if let ReturnType(_, Some(_)) = &signature.return_type {
            let return_slot_index = frame.base.stack + frame.stack_height - 1usize;
            Some(self.evaluation_stack.get_slot(return_slot_index))
        } else {
            None
        };

        for i in frame.base.arguments.as_usize()..self.evaluation_stack.top_of_stack().as_usize() {
            self.evaluation_stack
                .set_slot(crate::StackSlotIndex(i), StackValue::null());
        }
        self.evaluation_stack.truncate(frame.base.arguments);

        if let Some(return_value) = return_value {
            // Note: self.push() calls on_push() which increments stack_height,
            // so we must NOT manually increment stack_height here.
            self.push(return_value);
        }

        if let Some(return_type) = frame.awaiting_invoke_return {
            let is_void = matches!(return_type, RuntimeType::Void);
            if is_void {
                self.push(StackValue::null());
            } else {
                let val = self.pop();
                let return_concrete = return_type.to_concrete(&*self.shared.loader);
                let td = self
                    .shared
                    .loader
                    .find_concrete_type(return_concrete.clone())
                    .expect("Type must exist for return_frame");
                if td
                    .is_value_type(&self.current_context())
                    .expect("Failed to check if return type is value type")
                {
                    let boxed =
                        dotnet_vm_ops::ops::MemoryOps::box_value(self, &return_concrete, val)
                            .expect("Failed to box return value");
                    self.push_obj(boxed);
                } else {
                    // already a reference type (or null)
                    self.push(val);
                }
            }
        }

        StepResult::Continue
    }

    #[inline]
    pub(crate) fn handle_exception(&mut self) -> StepResult {
        let gc = self.gc;
        crate::exceptions::ExceptionHandlingSystem.handle_exception(self, gc)
    }

    pub(crate) fn unwind_frame(&mut self) {
        let frame = self
            .frame_stack
            .pop()
            .expect("unwind_frame called with empty stack");

        if frame.state.info_handle.is_cctor {
            let type_desc = frame.state.info_handle.source.parent;
            self.shared
                .statics
                .mark_failed(type_desc, &frame.generic_inst);

            // Wrap exception in TypeInitializationException per ECMA-335 §II.10.5.3.3
            let current_exception = match *self.exception_mode {
                ExceptionState::Throwing(ex, _) => Some(ex),
                ExceptionState::Searching(s) => Some(s.exception),
                ExceptionState::Unwinding(u) => u.exception,
                ExceptionState::ExecutingHandler(u) => u.exception,
                ExceptionState::Filtering(f) => Some(f.exception),
                ExceptionState::None => None,
            };

            if let Some(inner) = current_exception {
                let type_name = type_desc.type_name();
                let message = format!("The type initializer for '{}' threw an exception.", type_name);
                // We use throw_by_name_with_inner which updates self.exception_mode to Throwing.
                // This is safe because unwind_frame is called during the exception handling loop
                // in dotnet-exceptions, and the next iteration will see the new Throwing state.
                let _ = self.throw_by_name_with_inner(
                    "System.TypeInitializationException",
                    &message,
                    inner,
                );
            }
        }

        if frame.is_finalizer {
            self.local.heap.processing_finalizer.set(false);
        }
        for i in frame.base.arguments.as_usize()..self.evaluation_stack.top_of_stack().as_usize() {
            self.evaluation_stack
                .set_slot(crate::StackSlotIndex(i), StackValue::null());
        }
        self.evaluation_stack.truncate(frame.base.arguments);
    }

    #[inline]
    pub(crate) fn tracer_enabled(&self) -> bool {
        self.shared.tracer_enabled.load(Ordering::Relaxed)
    }

    #[inline]
    pub(crate) fn tracer(&self) -> &Tracer {
        &self.shared.tracer
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

            // Add to finalization queue if it has a finalizer.
            // Note: We skip System.Object because it defines a virtual Finalize() method
            // but we don't want every single object in the VM to be added to the
            // finalization queue. Only objects that override it or have their own
            // finalizers should be queued.
            if let HeapStorage::Obj(o) = &ptr.borrow().storage {
                let has_finalizer = o.description.static_initializer().is_some()
                    || o.description
                        .definition()
                        .methods
                        .iter()
                        .any(|m| m.name == "Finalize");

                if has_finalizer {
                    let system_object = self
                        .shared
                        .loader
                        .corlib_type("System.Object")
                        .ok();

                    if Some(o.description) != system_object {
                        self.local
                            .heap
                            .finalization_queue
                            .borrow_mut()
                            .push(*instance);
                    }
                }
            }
        }
    }

    #[inline]
    pub(crate) fn pin_object(&mut self, object: ObjectRef<'gc>) {
        self.local.heap.pinned_objects.borrow_mut().insert(object);
    }

    #[inline]
    pub(crate) fn unpin_object(&mut self, object: ObjectRef<'gc>) {
        self.local.heap.pinned_objects.borrow_mut().remove(&object);
    }
}

impl<'a, 'gc> BaseLoaderOps for VesContext<'a, 'gc> {
    #[inline]
    fn loader(&self) -> &Arc<dotnet_assemblies::AssemblyLoader> {
        &self.shared.loader
    }
}

impl<'a, 'gc> BaseCallOps<'gc> for VesContext<'a, 'gc> {
    #[inline]
    fn return_frame(&mut self) -> StepResult {
        VesContext::return_frame(self)
    }
}

impl<'a, 'gc> BaseResolutionOps<'gc> for VesContext<'a, 'gc> {
    #[inline]
    fn stack_value_type(
        &self,
        val: &StackValue<'gc>,
    ) -> Result<TypeDescription, TypeResolutionError> {
        self.resolver().stack_value_type(val)
    }

    #[inline]
    fn make_concrete(
        &self,
        t: &dotnetdll::prelude::MethodType,
    ) -> Result<ConcreteType, TypeResolutionError> {
        let f = self.frame_stack.current_frame();
        self.resolver()
            .make_concrete(f.source_resolution, &f.generic_inst, t)
    }

    #[inline]
    fn is_a(
        &self,
        value: ConcreteType,
        ancestor: ConcreteType,
    ) -> Result<bool, TypeResolutionError> {
        self.current_context().is_a(value, ancestor)
    }

    #[inline]
    fn instance_field_layout(
        &self,
        td: TypeDescription,
    ) -> Result<dotnet_value::layout::FieldLayoutManager, TypeResolutionError> {
        self.resolver().instance_fields(td, &self.current_context())
    }
}

impl<'a, 'gc> dotnet_vm_ops::ops::StackOps<'gc> for VesContext<'a, 'gc> {}

impl<'a, 'gc> BaseMemoryOps<'gc> for VesContext<'a, 'gc> {
    #[inline]
    fn gc_with_token(&self, _token: &dotnet_utils::NoActiveBorrows<'_>) -> GCHandle<'gc> {
        self.gc
    }

    #[inline]
    fn new_vector(
        &self,
        element: ConcreteType,
        size: usize,
    ) -> Result<dotnet_value::object::Vector<'gc>, TypeResolutionError> {
        self.resolver()
            .new_vector(element, size, &self.current_context())
    }

    #[inline]
    fn new_object(
        &self,
        td: TypeDescription,
    ) -> Result<dotnet_value::object::Object<'gc>, TypeResolutionError> {
        self.resolver().new_object(td, &self.current_context())
    }

    #[inline]
    fn new_value_type(
        &self,
        t: &ConcreteType,
        data: StackValue<'gc>,
    ) -> Result<dotnet_value::object::ValueType<'gc>, TypeResolutionError> {
        self.resolver()
            .new_value_type(t, data, &self.current_context())
    }

    #[inline]
    fn new_cts_value(
        &self,
        t: &ConcreteType,
        data: StackValue<'gc>,
    ) -> Result<dotnet_value::object::CTSValue<'gc>, TypeResolutionError> {
        self.resolver()
            .new_cts_value(t, data, &self.current_context())
    }

    #[inline]
    fn read_cts_value(
        &self,
        t: &ConcreteType,
        data: &[u8],
    ) -> Result<dotnet_value::object::CTSValue<'gc>, TypeResolutionError> {
        self.resolver()
            .read_cts_value(t, data, self.gc, &self.current_context())
    }

    #[inline]
    fn box_value(
        &self,
        t: &ConcreteType,
        data: StackValue<'gc>,
    ) -> Result<ObjectRef<'gc>, TypeResolutionError> {
        self.resolver()
            .box_value(t, data, self.gc, &self.current_context())
    }

    #[inline]
    fn clone_object(&self, obj: ObjectRef<'gc>) -> ObjectRef<'gc> {
        let gc = self.gc;
        let h = obj.0.expect("cannot clone null object");
        let inner = h.borrow();
        let cloned_storage = inner.storage.clone();

        let new_inner = dotnet_value::object::ObjectInner {
            magic: inner.magic,
            owner_id: self.thread_id.get(),
            storage: cloned_storage,
        };

        let new_h = gc_arena::Gc::new(&gc, dotnet_utils::gc::ThreadSafeLock::new(new_inner));
        let new_ref = ObjectRef(Some(new_h));
        self.register_new_object(&new_ref);
        new_ref
    }

    #[inline]
    fn register_new_object(&self, instance: &ObjectRef<'gc>) {
        VesContext::register_new_object(self, instance)
    }
}

impl<'a, 'gc> BaseReflectionOps<'gc> for VesContext<'a, 'gc> {
    #[inline]
    fn get_heap_description(
        &self,
        object: ObjectRef<'gc>,
    ) -> Result<TypeDescription, TypeResolutionError> {
        if let Some(handle) = object.0 {
            self.resolver().get_heap_description(handle)
        } else {
            self.shared.loader.corlib_type("System.Object")
        }
    }
}

impl<'a, 'gc> BaseStaticsOps<'gc> for VesContext<'a, 'gc> {
    #[inline]
    fn initialize_static_storage(
        &mut self,
        description: TypeDescription,
        generics: GenericLookup,
    ) -> StepResult {
        // Static storage is keyed per-type-instantiation, so only TYPE generics matter.
        // Strip method generics to ensure consistent keying regardless of call context.
        let type_generics = GenericLookup::new(generics.type_generics.to_vec());
        let thread_id = self.thread_id.get();
        // Use the type instantiation generics (not the calling frame's generics) so that
        // static field layouts for generic types like Task<T> resolve TypeGeneric(0) correctly.
        let context = self.with_generics(&type_generics);
        let metrics = Some(&self.shared.metrics);

        match self
            .shared
            .statics
            .init(description, &context, thread_id, metrics)
        {
            Ok(crate::statics::StaticInitResult::Execute(cctor)) => {
                self.dispatch_method(cctor, type_generics)
            }
            Ok(crate::statics::StaticInitResult::Initialized)
            | Ok(crate::statics::StaticInitResult::Recursive) => StepResult::Continue,
            Ok(crate::statics::StaticInitResult::Waiting) => {
                #[cfg(feature = "multithreading")]
                if self.shared.statics.wait_for_init(
                    description,
                    &type_generics,
                    self.shared.thread_manager.as_ref(),
                    thread_id,
                    &self.shared.gc_coordinator,
                ) {
                    self.back_up_ip();
                    return StepResult::Yield;
                }
                StepResult::Continue
            }
            Ok(crate::statics::StaticInitResult::Failed) => {
                let type_name = description.type_name();
                let message =
                    format!("The type initializer for '{}' threw an exception.", type_name);
                self.throw_by_name_with_message("System.TypeInitializationException", &message)
            }
            Err(e) => StepResult::Error(e.into()),
        }
    }
}

impl<'a, 'gc> BaseVesInternals<'gc> for VesContext<'a, 'gc> {
    #[inline]
    fn back_up_ip(&mut self) {
        self.frame_stack.current_frame_mut().state.ip = *self.original_ip;
        self.evaluation_stack.truncate(*self.original_stack_height);
        if let Some(frame) = self.frame_stack.current_frame_opt_mut() {
            frame.stack_height = self
                .original_stack_height
                .saturating_sub_idx(frame.base.stack);
        }
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
    fn state(&self) -> &MethodState {
        self.frame_stack.state()
    }

    #[inline]
    fn state_mut(&mut self) -> &mut MethodState {
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
    fn original_ip(&self) -> usize {
        *self.original_ip
    }

    #[inline]
    fn original_ip_mut(&mut self) -> &mut usize {
        self.original_ip
    }

    #[inline]
    fn original_stack_height(&self) -> crate::StackSlotIndex {
        *self.original_stack_height
    }

    #[inline]
    fn original_stack_height_mut(&mut self) -> &mut crate::StackSlotIndex {
        self.original_stack_height
    }

    fn current_intrinsic(&self) -> Option<MethodDescription> {
        self.current_intrinsic.as_ref().map(|m| m.0.clone())
    }

    #[inline]
    fn set_current_intrinsic(&mut self, method: Option<MethodDescription>) {
        *self.current_intrinsic = method.map(crate::CollectableMethodDescription);
    }

    #[inline]
    fn unwind_frame(&mut self) {
        self.unwind_frame()
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
    fn frame_stack(&self) -> &FrameStack<'gc> {
        self.frame_stack
    }

    #[inline]
    fn frame_stack_mut(&mut self) -> &mut FrameStack<'gc> {
        self.frame_stack
    }
}

impl<'a, 'gc> VesInternals<'gc> for VesContext<'a, 'gc> {}

impl<'a, 'gc> BaseVesBaseOps for VesContext<'a, 'gc> {
    #[inline]
    fn tracer_enabled(&self) -> bool {
        VesContext::tracer_enabled(self)
    }

    #[inline]
    fn tracer(&self) -> &Tracer {
        VesContext::tracer(self)
    }

    #[inline]
    fn indent(&self) -> usize {
        VesContext::indent(self)
    }
}

impl<'a, 'gc> VesBaseOps for VesContext<'a, 'gc> {}

impl<'a, 'gc> BaseExceptionContext<'gc> for VesContext<'a, 'gc> {}

impl<'a, 'gc> ExceptionContext<'gc> for VesContext<'a, 'gc> {}

impl<'a, 'gc> BasePInvokeContext<'gc> for VesContext<'a, 'gc> {
    #[inline]
    fn pin_object(&mut self, object: ObjectRef<'gc>) {
        VesContext::pin_object(self, object)
    }

    #[inline]
    fn unpin_object(&mut self, object: ObjectRef<'gc>) {
        VesContext::unpin_object(self, object)
    }
}

impl<'a, 'gc> PInvokeContext<'gc> for VesContext<'a, 'gc> {}

impl<'a, 'gc> VesOps<'gc> for VesContext<'a, 'gc> {
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

                        last_res = crate::dispatch::InstructionRegistry::dispatch(self, i);

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
                    // Periodic check for safe point after each instruction chunk
                    let _ = self.check_gc_safe_point();
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
                    .trace_gc_finalization(self.indent(), &type_name, addr);
            }

            let finalizer_data: Option<(MethodDescription, GenericLookup)> = instance
                .as_heap_storage(|storage| {
                    if let HeapStorage::Obj(o) = storage {
                        let obj_type = o.description;
                        let generics = o.generics.clone();
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

                        let method_desc = MethodDescription::new(
                            object_type,
                            GenericLookup::default(),
                            object_type.resolution,
                            base_finalize,
                        );

                        Some((
                            self.resolver()
                                .resolve_virtual_method(method_desc, obj_type, &generics, &ctx)
                                .expect("Failed to resolve finalizer"),
                            generics,
                        ))
                    } else {
                        None
                    }
                });

            let Some((finalizer, generics)) = finalizer_data else {
                self.local.heap.processing_finalizer.set(false);
                return StepResult::Continue;
            };

            let method_info = vm_try!(self.shared.caches.get_method_info(
                finalizer,
                &generics,
                self.shared.clone()
            ));

            // Push the object as 'this'
            self.push(StackValue::ObjectRef(instance));

            vm_try!(self.call_frame(method_info, generics));
            self.current_frame_mut().is_finalizer = true;
            return StepResult::FramePushed;
        }

        StepResult::Continue
    }
}

#[derive(Collect)]
#[collect(no_drop, gc_lifetime = 'gc)]
pub struct ThreadContext<'gc> {
    pub evaluation_stack: EvaluationStack<'gc>,
    pub frame_stack: FrameStack<'gc>,
    pub exception_mode: ExceptionState<'gc>,
    pub current_intrinsic: Option<crate::CollectableMethodDescription>,
    pub original_ip: usize,
    pub original_stack_height: crate::StackSlotIndex,
}

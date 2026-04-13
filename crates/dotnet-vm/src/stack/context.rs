use crate::{
    MethodState, ResolutionContext, StepResult,
    resolution::{TypeResolutionExt, ValueResolution},
    stack::ops::*,
    state::{ArenaLocalState, SharedGlobalState},
    sync::{Arc, LockResult, SyncBlockOps, SyncManagerOps},
};
use dotnet_tracer::Tracer;
use dotnet_types::{
    TypeDescription,
    error::{IntrinsicError, TypeResolutionError},
    generics::{ConcreteType, GenericLookup},
    members::{FieldDescription, MethodDescription},
    runtime::RuntimeType,
};
use dotnet_utils::{BorrowScopeOps, gc::GCHandle, sync::Ordering};
use dotnet_value::{
    StackValue,
    object::{HeapStorage, Object, ObjectRef},
    storage::FieldStorage,
};
use dotnetdll::prelude::*;
use gc_arena::Collect;
use std::ptr::NonNull;

pub use dotnet_vm_ops::{BasePointer, EvaluationStack, ExceptionState, FrameStack, PinnedLocals};

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
    pub(crate) call_args_buffer: &'a mut Vec<StackValue<'gc>>,
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
        locals_base: crate::StackSlotIndex,
    ) -> Result<PinnedLocals, TypeResolutionError> {
        let mut pinned_locals = PinnedLocals::with_capacity(locals.len());
        let ctx = ResolutionContext {
            generics,
            state: self.shared.resolution_shared(),
            resolution: method.resolution(),
            type_owner: Some(method.parent.clone()),
            method_owner: Some(method),
        };

        for (i, l) in locals.iter().enumerate() {
            use BaseType::*;
            use LocalVariable::*;
            let value = match l {
                TypedReference => {
                    pinned_locals.push(false);
                    StackValue::null()
                }
                Variable {
                    by_ref: _,
                    var_type,
                    pinned,
                    ..
                } => {
                    pinned_locals.push(*pinned);
                    let concrete_local = ctx.make_concrete(var_type)?;
                    match concrete_local.get() {
                        Boolean | Char | Int8 | UInt8 | Int16 | UInt16 | Int32 | UInt32 => {
                            StackValue::Int32(0)
                        }
                        Int64 | UInt64 => StackValue::Int64(0),
                        Float32 | Float64 => StackValue::NativeFloat(0.0),
                        IntPtr | UIntPtr | FunctionPointer(_) | ValuePointer(_, _) => {
                            StackValue::NativeInt(0)
                        }
                        Object | String | Array(_, _) | Vector(_, _) => StackValue::null(),
                        Type { .. } => {
                            let desc = self.loader().find_concrete_type(concrete_local.clone())?;
                            if desc.is_value_type(&ctx)? {
                                let mut new_lookup = concrete_local.make_lookup();
                                new_lookup.method_generics = generics.method_generics.clone();
                                let new_ctx = ctx.with_generics(&new_lookup);
                                let instance = new_ctx.new_object(desc)?;
                                StackValue::ValueType(instance)
                            } else {
                                StackValue::null()
                            }
                        }
                    }
                }
            };
            self.evaluation_stack.set_slot_at(locals_base + i, value);
        }
        Ok(pinned_locals)
    }

    #[inline]
    pub(crate) fn pop_call_args_into_buffer(&mut self, count: usize) {
        self.call_args_buffer.clear();
        self.call_args_buffer.reserve(count);
        for _ in 0..count {
            let value = self.pop();
            self.call_args_buffer.push(value);
        }
        self.call_args_buffer.reverse();
    }

    #[inline]
    pub(crate) fn copy_slots_into_call_args_buffer(
        &mut self,
        start: crate::StackSlotIndex,
        count: usize,
    ) {
        self.evaluation_stack
            .copy_slots_into(start, count, self.call_args_buffer);
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
            let type_desc = frame.state.info_handle.source.parent.clone();
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

        if let Some(return_type) = frame.awaiting_invoke_return.clone() {
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

        self.frame_stack.recycle_frame(frame);
        StepResult::Continue
    }

    #[inline]
    pub(crate) fn handle_exception(&mut self) -> StepResult {
        let gc = self.gc;
        dotnet_exceptions::ExceptionHandlingSystem.handle_exception(self, gc)
    }

    pub(crate) fn unwind_frame(&mut self) {
        let frame = self
            .frame_stack
            .pop()
            .expect("unwind_frame called with empty stack");

        if frame.state.info_handle.is_cctor {
            let type_desc = frame.state.info_handle.source.parent.clone();
            self.shared
                .statics
                .mark_failed(type_desc.clone(), &frame.generic_inst);

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
                let message = format!(
                    "The type initializer for '{}' threw an exception.",
                    type_name
                );
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
        self.frame_stack.recycle_frame(frame);
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
            .locate_type(f.source_resolution.clone(), handle)
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
            self.local.heap.register_object(*instance);

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
                    let system_object = self.shared.loader.corlib_type("System.Object").ok();

                    if Some(o.description.clone()) != system_object {
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
        self.local.heap.pin_object(object);
    }

    #[inline]
    pub(crate) fn unpin_object(&mut self, object: ObjectRef<'gc>) {
        self.local.heap.unpin_object(object);
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
    fn make_concrete(&self, t: &MethodType) -> Result<ConcreteType, TypeResolutionError> {
        let f = self.frame_stack.current_frame();
        self.resolver()
            .make_concrete(f.source_resolution.clone(), &f.generic_inst, t)
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
    fn no_active_borrows_token(&self) -> dotnet_utils::GcReadyToken<'_> {
        self.gc_ready_token()
    }

    #[inline]
    fn gc_with_token(&self, token: &dotnet_utils::GcReadyToken<'_>) -> GCHandle<'gc> {
        assert!(
            token.belongs_to(self),
            "gc_with_token requires a token issued by this VesContext",
        );
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
    fn new_object_with_lookup(
        &self,
        td: TypeDescription,
        lookup: &GenericLookup,
    ) -> Result<dotnet_value::object::Object<'gc>, TypeResolutionError> {
        let ctx = self.current_context().with_generics(lookup);
        self.resolver().new_object(td, &ctx)
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

        let new_inner =
            dotnet_value::object::ObjectInner::new(cloned_storage, self.thread_id.get());

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

impl<'a, 'gc> dotnet_intrinsics_delegates::DelegateInvokeHost<'gc> for VesContext<'a, 'gc> {
    fn delegate_method_info(
        &self,
        method: MethodDescription,
        lookup: &GenericLookup,
    ) -> Result<dotnet_vm_ops::MethodInfo<'static>, TypeResolutionError> {
        self.shared
            .caches
            .get_method_info(method, lookup, self.shared.clone())
    }

    #[inline]
    fn delegate_call_frame(
        &mut self,
        method: dotnet_vm_ops::MethodInfo<'static>,
        generic_inst: GenericLookup,
    ) -> Result<(), TypeResolutionError> {
        self.call_frame(method, generic_inst)
    }

    #[inline]
    fn delegate_lookup_method_by_index(&self, index: usize) -> (MethodDescription, GenericLookup) {
        self.lookup_method_by_index(index)
    }

    #[inline]
    fn delegate_runtime_method_obj(
        &mut self,
        method: MethodDescription,
        lookup: GenericLookup,
    ) -> ObjectRef<'gc> {
        self.get_runtime_method_obj(method, lookup)
    }

    #[inline]
    fn delegate_dispatch_method(
        &mut self,
        method: MethodDescription,
        lookup: GenericLookup,
    ) -> StepResult {
        self.dispatch_method(method, lookup)
    }
}

impl<'a, 'gc> dotnet_intrinsics_string::IntrinsicStringHost<'gc> for VesContext<'a, 'gc> {
    #[inline]
    fn string_intrinsic_as_span(
        &mut self,
        method: MethodDescription,
        generics: &GenericLookup,
    ) -> StepResult {
        dotnet_intrinsics_span::conversions::intrinsic_as_span(self, method, generics)
    }

    #[inline]
    fn string_with_span_data<'span, R, F: FnOnce(&[u8]) -> R>(
        &self,
        span: Object<'span>,
        element_type: TypeDescription,
        element_size: usize,
        f: F,
    ) -> Result<R, IntrinsicError> {
        dotnet_intrinsics_span::helpers::with_span_data(self, span, element_type, element_size, f)
    }
}

impl<'a, 'gc> dotnet_intrinsics_span::LayoutQueryHost for VesContext<'a, 'gc> {
    #[inline]
    fn span_type_layout(
        &self,
        t: ConcreteType,
    ) -> Result<std::sync::Arc<dotnet_value::layout::LayoutManager>, TypeResolutionError> {
        crate::layout::type_layout(t, &self.current_context())
    }
}

impl<'a, 'gc> dotnet_intrinsics_span::SpanPointerIntrospectionHost<'gc> for VesContext<'a, 'gc> {
    #[inline]
    fn span_ptr_info(
        &mut self,
        val: &StackValue<'gc>,
    ) -> Result<
        (
            dotnet_value::pointer::PointerOrigin<'gc>,
            dotnet_utils::ByteOffset,
        ),
        StepResult,
    > {
        crate::instructions::objects::get_ptr_info(self, val)
    }
}

impl<'a, 'gc> dotnet_intrinsics_span::SpanObjectFactoryHost<'gc> for VesContext<'a, 'gc> {
    #[inline]
    fn span_new_object_with_type_generics(
        &self,
        td: TypeDescription,
        type_generics: Vec<ConcreteType>,
    ) -> Result<Object<'gc>, TypeResolutionError> {
        let lookup = GenericLookup::new(type_generics);
        let current = self.current_context();
        current.with_generics(&lookup).new_object(td)
    }
}

impl<'a, 'gc> dotnet_intrinsics_span::SpanRuntimeHost<'gc> for VesContext<'a, 'gc> {
    #[inline]
    fn span_resolve_runtime_field(&self, obj: ObjectRef<'gc>) -> (FieldDescription, GenericLookup) {
        self.resolve_runtime_field(obj)
    }

    #[inline]
    fn span_resolve_runtime_type(&self, obj: ObjectRef<'gc>) -> RuntimeType {
        self.resolve_runtime_type(obj)
    }

    #[inline]
    fn span_dispatch_method(
        &mut self,
        method: MethodDescription,
        lookup: GenericLookup,
    ) -> StepResult {
        self.dispatch_method(method, lookup)
    }
}

impl<'a, 'gc> dotnet_intrinsics_threading::MonitorHost<'gc> for VesContext<'a, 'gc> {
    type SyncBlock = Arc<crate::sync::SyncBlock>;

    #[inline]
    fn monitor_get_sync_block_for_object(&self, object: ObjectRef<'gc>) -> Option<Self::SyncBlock> {
        object
            .sync_block_index()
            .and_then(|index| self.shared().sync_blocks.get_sync_block(index))
    }

    #[inline]
    fn monitor_get_or_create_sync_block_for_object(
        &self,
        object: ObjectRef<'gc>,
        gc: GCHandle<'gc>,
    ) -> Self::SyncBlock {
        let current_index = object.sync_block_index();
        let (new_index, new_block) = self
            .shared()
            .sync_blocks
            .get_or_create_sync_block(current_index);

        match current_index {
            Some(existing_index) if existing_index != new_index => {
                if let Some(existing_block) =
                    self.shared().sync_blocks.get_sync_block(existing_index)
                {
                    return existing_block;
                }
                object.set_sync_block_index(gc, new_index);
                new_block
            }
            None => {
                object.set_sync_block_index(gc, new_index);
                new_block
            }
            _ => new_block,
        }
    }

    #[inline]
    fn monitor_try_enter(
        &self,
        sync_block: &Self::SyncBlock,
        thread_id: dotnet_utils::ArenaId,
    ) -> bool {
        sync_block.try_enter(thread_id)
    }

    #[inline]
    fn monitor_enter_safe(
        &self,
        sync_block: &Self::SyncBlock,
        thread_id: dotnet_utils::ArenaId,
    ) -> dotnet_intrinsics_threading::MonitorLockResult {
        match sync_block.enter_safe(
            thread_id,
            &self.shared().metrics,
            self.shared().thread_manager.as_ref(),
            &self.shared().gc_coordinator,
        ) {
            LockResult::Success => dotnet_intrinsics_threading::MonitorLockResult::Success,
            LockResult::Timeout => dotnet_intrinsics_threading::MonitorLockResult::Timeout,
            LockResult::Yield => dotnet_intrinsics_threading::MonitorLockResult::Yield,
        }
    }

    #[inline]
    fn monitor_enter_with_timeout_safe(
        &self,
        sync_block: &Self::SyncBlock,
        thread_id: dotnet_utils::ArenaId,
        deadline: std::time::Instant,
    ) -> dotnet_intrinsics_threading::MonitorLockResult {
        match sync_block.enter_with_timeout_safe(
            thread_id,
            deadline,
            &self.shared().metrics,
            self.shared().thread_manager.as_ref(),
            &self.shared().gc_coordinator,
        ) {
            LockResult::Success => dotnet_intrinsics_threading::MonitorLockResult::Success,
            LockResult::Timeout => dotnet_intrinsics_threading::MonitorLockResult::Timeout,
            LockResult::Yield => dotnet_intrinsics_threading::MonitorLockResult::Yield,
        }
    }

    #[inline]
    fn monitor_exit(&self, sync_block: &Self::SyncBlock, thread_id: dotnet_utils::ArenaId) -> bool {
        sync_block.exit(thread_id)
    }
}

impl<'a, 'gc> dotnet_intrinsics_threading::StackSlotWriteHost<'gc> for VesContext<'a, 'gc> {
    #[inline]
    fn threading_set_stack_slot(&mut self, index: crate::StackSlotIndex, value: StackValue<'gc>) {
        self.set_slot(index, value);
    }
}

impl<'a, 'gc> dotnet_intrinsics_unsafe::UnsafeIntrinsicHost<'gc> for VesContext<'a, 'gc> {
    #[inline]
    fn unsafe_type_layout(
        &self,
        t: ConcreteType,
    ) -> Result<std::sync::Arc<dotnet_value::layout::LayoutManager>, TypeResolutionError> {
        crate::layout::type_layout(t, &self.current_context())
    }

    #[inline]
    fn unsafe_check_read_safety(
        &self,
        result_layout: &dotnet_value::layout::LayoutManager,
        src_layout: Option<&dotnet_value::layout::LayoutManager>,
        src_ptr_offset: usize,
    ) -> Result<(), dotnet_types::error::MemoryAccessError> {
        dotnet_runtime_memory::check_read_safety(result_layout, src_layout, src_ptr_offset)
    }

    #[inline]
    fn unsafe_lookup_owner_layout_and_base(
        &self,
        ptr: *mut u8,
        origin: &dotnet_value::pointer::PointerOrigin<'gc>,
    ) -> (Option<dotnet_value::layout::LayoutManager>, Option<usize>) {
        let memory = dotnet_runtime_memory::RawMemoryAccess::new(&self.local.heap);
        let owner = match origin {
            dotnet_value::pointer::PointerOrigin::Heap(o) => Some(*o),
            _ => self.local.heap.find_object(ptr as usize),
        };

        if let Some(owner) = owner {
            let layout =
                memory.get_layout_from_owner(dotnet_runtime_memory::MemoryOwner::Local(owner));
            let (base, _) = memory.get_storage_base(owner);
            let base_addr = if base.is_null() {
                None
            } else {
                Some(base as usize)
            };
            (layout, base_addr)
        } else {
            (None, None)
        }
    }

    #[inline]
    fn unsafe_get_last_pinvoke_error(&self) -> i32 {
        unsafe { dotnet_pinvoke::LAST_ERROR }
    }

    #[inline]
    fn unsafe_set_last_pinvoke_error(&mut self, value: i32) {
        unsafe {
            dotnet_pinvoke::LAST_ERROR = value;
        }
    }

    #[inline]
    fn unsafe_resolve_runtime_type(&self, obj: ObjectRef<'gc>) -> ConcreteType {
        self.resolve_runtime_type(obj)
            .to_concrete(self.loader().as_ref())
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
            .init(description.clone(), &context, thread_id, metrics)
        {
            Ok(crate::statics::StaticInitResult::Execute(cctor)) => {
                self.dispatch_method(cctor, type_generics)
            }
            Ok(crate::statics::StaticInitResult::Initialized)
            | Ok(crate::statics::StaticInitResult::Recursive) => StepResult::Continue,
            Ok(crate::statics::StaticInitResult::Waiting) => {
                #[cfg(feature = "multithreading")]
                {
                    if self.shared.statics.wait_for_init(
                        description.clone(),
                        &type_generics,
                        self.shared.thread_manager.as_ref(),
                        thread_id,
                        &self.shared.gc_coordinator,
                    ) {
                        self.back_up_ip();
                        return StepResult::Yield;
                    }
                    // Re-check initialization state after waiting.
                    // If the .cctor failed in another thread, we must throw TypeInitializationException.
                    let state = self
                        .shared
                        .statics
                        .get_init_state(description.clone(), &type_generics);
                    match state {
                        crate::statics::INIT_STATE_INITIALIZED => {
                            // Success - continue with the operation
                        }
                        crate::statics::INIT_STATE_FAILED => {
                            let type_name = description.type_name();
                            let message = format!(
                                "The type initializer for '{}' threw an exception.",
                                type_name
                            );
                            return self.throw_by_name_with_message(
                                "System.TypeInitializationException",
                                &message,
                            );
                        }
                        _ => {
                            // Unexpected state - this should not happen
                            return StepResult::internal_error(format!(
                                "Type initialization for '{}' ended in unexpected state {}",
                                description.type_name(),
                                state
                            ));
                        }
                    }
                }
                StepResult::Continue
            }
            Ok(crate::statics::StaticInitResult::Failed) => {
                let type_name = description.type_name();
                let message = format!(
                    "The type initializer for '{}' threw an exception.",
                    type_name
                );
                self.throw_by_name_with_message("System.TypeInitializationException", &message)
            }
            Err(e) => StepResult::Error(e.into()),
        }
    }
}

impl<'a, 'gc> VesInternals<'gc> for VesContext<'a, 'gc> {
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

impl<'a, 'gc> VesBaseOps for VesContext<'a, 'gc> {
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
                    HeapStorage::Obj(o) => o.description.clone(),
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
                        let obj_type = o.description.clone();
                        let generics = o.generics.clone();
                        let object_type = self
                            .shared
                            .loader
                            .corlib_type("System.Object")
                            .expect("System.Object must exist in corlib");
                        let (idx, _) = object_type
                            .definition()
                            .methods
                            .iter()
                            .enumerate()
                            .find(|(_, m)| {
                                m.name == "Finalize"
                                    && m.virtual_member
                                    && m.signature.parameters.is_empty()
                            })
                            .expect("System.Object::Finalize not found");

                        let method_desc = MethodDescription::new(
                            object_type.clone(),
                            GenericLookup::default(),
                            object_type.resolution.clone(),
                            MethodMemberIndex::Method(idx),
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

            let method_info = dotnet_vm_ops::vm_try!(self.shared.caches.get_method_info(
                finalizer,
                &generics,
                self.shared.clone()
            ));

            // Push the object as 'this'
            self.push(StackValue::ObjectRef(instance));

            dotnet_vm_ops::vm_try!(self.call_frame(method_info, generics));
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
    pub call_args_buffer: Vec<StackValue<'gc>>,
}

#[cfg(test)]
mod host_adapter_trait_tests {
    use super::VesContext;
    use crate::StepResult;
    use dotnet_types::{
        generics::{ConcreteType, GenericLookup},
        members::{FieldDescription, MethodDescription},
    };
    use dotnet_vm_ops::ops as vm_ops;
    use std::sync::Arc;

    #[test]
    fn ves_context_implements_vm_ops_host_traits() {
        fn assert_impls()
        where
            for<'a, 'gc> VesContext<'a, 'gc>: vm_ops::StringIntrinsicHost<'gc>
                + dotnet_intrinsics_string::IntrinsicStringHost<'gc>
                + vm_ops::DelegateIntrinsicHost<'gc>
                + vm_ops::SpanIntrinsicHost<'gc>
                + dotnet_intrinsics_span::SpanIntrinsicHost<'gc>
                + vm_ops::UnsafeIntrinsicHost<'gc>
                + dotnet_intrinsics_unsafe::UnsafeIntrinsicHost<'gc>
                + vm_ops::ThreadingIntrinsicHost<'gc>
                + dotnet_intrinsics_threading::ThreadingIntrinsicHost<'gc>
                + dotnet_intrinsics_reflection::ReflectionIntrinsicHost<'gc>
                + vm_ops::ReflectionIntrinsicHost<'gc>,
        {
        }

        assert_impls();
    }

    #[test]
    fn vm_ops_hosts_cover_intrinsic_handler_bounds() {
        fn assert_string_handler<'gc, T: vm_ops::StringIntrinsicHost<'gc>>() {
            let _handler: fn(&mut T, FieldDescription, Arc<[ConcreteType]>, bool) -> StepResult =
                dotnet_intrinsics_string::accessors::intrinsic_field_string_empty::<T>;
        }

        fn assert_delegate_handler<
            'gc,
            T: vm_ops::DelegateIntrinsicHost<'gc>
                + dotnet_intrinsics_delegates::DelegateInvokeHost<'gc>,
        >() {
            let _handler: fn(&mut T, MethodDescription, &GenericLookup) -> Option<StepResult> =
                dotnet_intrinsics_delegates::helpers::try_delegate_dispatch::<T>;
        }

        fn assert_span_handler<
            'gc,
            T: vm_ops::SpanIntrinsicHost<'gc> + dotnet_intrinsics_span::SpanIntrinsicHost<'gc>,
        >() {
            let _handler: fn(&mut T, MethodDescription, &GenericLookup) -> StepResult =
                dotnet_intrinsics_span::equality::intrinsic_memory_extensions_sequence_equal::<T>;
        }

        fn assert_unsafe_handler<
            'gc,
            T: vm_ops::UnsafeIntrinsicHost<'gc> + dotnet_intrinsics_unsafe::UnsafeIntrinsicHost<'gc>,
        >() {
            let _handler: fn(&mut T, MethodDescription, &GenericLookup) -> StepResult =
                dotnet_intrinsics_unsafe::marshal::intrinsic_marshal_offset_of::<T>;
        }

        fn assert_threading_handler<
            'gc,
            T: vm_ops::ThreadingIntrinsicHost<'gc>
                + dotnet_intrinsics_threading::ThreadingIntrinsicHost<'gc>,
        >() {
            let _handler: fn(&mut T, MethodDescription, &GenericLookup) -> StepResult =
                dotnet_intrinsics_threading::monitor::intrinsic_monitor_reliable_enter::<T>;
        }

        fn assert_reflection_handler<
            'gc,
            T: vm_ops::ReflectionIntrinsicHost<'gc>
                + dotnet_intrinsics_reflection::ReflectionIntrinsicHost<'gc>,
        >() {
            let _handler: fn(&mut T, MethodDescription, &GenericLookup) -> StepResult =
                dotnet_intrinsics_reflection::methods::runtime_method_info_intrinsic_call::<T>;
        }

        type TestContext = VesContext<'static, 'static>;
        assert_string_handler::<'static, TestContext>();
        assert_delegate_handler::<'static, TestContext>();
        assert_span_handler::<'static, TestContext>();
        assert_unsafe_handler::<'static, TestContext>();
        assert_threading_handler::<'static, TestContext>();
        assert_reflection_handler::<'static, TestContext>();
    }
}

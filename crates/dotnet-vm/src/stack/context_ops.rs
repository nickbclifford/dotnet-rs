use super::VesContext;
use crate::{
    MethodState, StepResult,
    resolution::ValueResolution,
    stack::ops::*,
    sync::{Arc, LockResult, SyncBlockOps, SyncManagerOps},
};
use dotnet_tracer::Tracer;
use dotnet_types::{
    TypeDescription,
    error::{ExecutionError, IntrinsicError, TypeResolutionError},
    generics::{ConcreteType, GenericLookup},
    members::{FieldDescription, MethodDescription},
    runtime::RuntimeType,
};
use dotnet_utils::{BorrowScopeOps, gc::GCHandle};
use dotnet_value::{
    StackValue,
    object::{HeapStorage, Object, ObjectRef},
};
use dotnetdll::prelude::{MethodMemberIndex, MethodType};

impl<'a, 'gc> LoaderOps for VesContext<'a, 'gc> {
    #[inline]
    fn loader(&self) -> &Arc<dotnet_assemblies::AssemblyLoader> {
        &self.shared.loader
    }
}

impl<'a, 'gc> SimdCapabilityOps for VesContext<'a, 'gc> {
    #[inline]
    fn simd_vector128_is_hardware_accelerated(&self) -> bool {
        dotnet_intrinsics_simd::vector128_is_hardware_accelerated()
    }

    #[inline]
    fn simd_vector256_is_hardware_accelerated(&self) -> bool {
        dotnet_intrinsics_simd::vector256_is_hardware_accelerated()
    }
}

impl<'a, 'gc> ResolutionOps<'gc> for VesContext<'a, 'gc> {
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

impl<'a, 'gc> dotnet_runtime_memory::ops::BaseMemoryOps<'gc> for VesContext<'a, 'gc> {
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

impl<'a, 'gc> ReflectionOps<'gc> for VesContext<'a, 'gc> {
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
    ) -> Result<dotnet_vm_data::MethodInfo<'static>, TypeResolutionError> {
        self.shared
            .caches
            .get_method_info(method, lookup, self.shared.clone())
    }

    #[inline]
    fn delegate_call_frame(
        &mut self,
        method: dotnet_vm_data::MethodInfo<'static>,
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
    fn span_resolve_runtime_field(
        &self,
        obj: ObjectRef<'gc>,
    ) -> Result<(FieldDescription, GenericLookup), ExecutionError> {
        self.resolve_runtime_field(obj)
    }

    #[inline]
    fn span_resolve_runtime_type(
        &self,
        obj: ObjectRef<'gc>,
    ) -> Result<RuntimeType, ExecutionError> {
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
    fn unsafe_resolve_runtime_type(
        &self,
        obj: ObjectRef<'gc>,
    ) -> Result<ConcreteType, ExecutionError> {
        Ok(self
            .resolve_runtime_type(obj)?
            .to_concrete(self.loader().as_ref()))
    }
}

impl<'a, 'gc> StaticsOps<'gc> for VesContext<'a, 'gc> {
    #[inline]
    fn initialize_static_storage(
        &mut self,
        description: TypeDescription,
        generics: GenericLookup,
    ) -> StepResult {
        // Static storage is keyed per-type-instantiation, so only parent type generics matter.
        // Some call sites may carry extra generics from the invoking method context; trim to the
        // declaring type arity to keep .cctor/stateful static storage keying stable.
        let type_arity = description.definition().generic_parameters.len();
        let type_generics = GenericLookup::new(
            generics
                .type_generics
                .iter()
                .take(type_arity)
                .cloned()
                .collect(),
        );
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
                            // VM invariant: once `wait_for_init` returns without yielding, the
                            // state machine must have converged to INITIALIZED or FAILED.
                            // Any other state is an internal synchronization bug.
                            unreachable!(
                                "Type initialization for '{}' ended in unexpected state {}",
                                description.type_name(),
                                state
                            );
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
    fn exception_mode(&self) -> &dotnet_vm_ops::ExceptionState<'gc> {
        self.exception_mode
    }

    #[inline]
    fn exception_mode_mut(&mut self) -> &mut dotnet_vm_ops::ExceptionState<'gc> {
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
    fn evaluation_stack_mut(&mut self) -> &mut dotnet_vm_ops::EvaluationStack<'gc> {
        self.evaluation_stack
    }

    #[inline]
    fn frame_stack(&self) -> &dotnet_vm_ops::FrameStack<'gc> {
        self.frame_stack
    }

    #[inline]
    fn frame_stack_mut(&mut self) -> &mut dotnet_vm_ops::FrameStack<'gc> {
        self.frame_stack
    }

    #[inline]
    fn suspended_handler_unwinds_mut(&mut self) -> &mut Vec<dotnet_vm_ops::UnwindState<'gc>> {
        self.suspended_handler_unwinds
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

impl<'a, 'gc> ExceptionContext<'gc> for VesContext<'a, 'gc> {}

impl<'a, 'gc> VmExceptionContext<'gc> for VesContext<'a, 'gc> {}

impl<'a, 'gc> PInvokeContext<'gc> for VesContext<'a, 'gc> {
    #[inline]
    fn pin_object(&mut self, object: ObjectRef<'gc>) {
        VesContext::pin_object(self, object)
    }

    #[inline]
    fn unpin_object(&mut self, object: ObjectRef<'gc>) {
        VesContext::unpin_object(self, object)
    }
}

impl<'a, 'gc> VmPInvokeContext<'gc> for VesContext<'a, 'gc> {}

impl<'a, 'gc> VesOps<'gc> for VesContext<'a, 'gc> {
    #[inline]
    fn run(&mut self) -> StepResult {
        let _gc = self.gc;
        loop {
            let res = match self.exception_mode {
                dotnet_vm_ops::ExceptionState::None
                | dotnet_vm_ops::ExceptionState::ExecutingHandler(_)
                | dotnet_vm_ops::ExceptionState::Filtering(_) => {
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
                    // Poll safe-point between instruction chunks and cooperatively yield.
                    if last_res == StepResult::Continue && self.check_gc_safe_point() {
                        StepResult::Yield
                    } else {
                        last_res
                    }
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
                        let method_desc = object_type
                            .definition()
                            .methods
                            .iter()
                            .enumerate()
                            .find_map(|(i, m)| {
                                if !m.virtual_member || m.name != "Finalize" {
                                    return None;
                                }
                                let d = MethodDescription::new(
                                    object_type.clone(),
                                    GenericLookup::default(),
                                    object_type.resolution.clone(),
                                    MethodMemberIndex::Method(i),
                                );
                                d.signature().parameters.is_empty().then_some(d)
                            })
                            .expect("System.Object::Finalize not found");

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

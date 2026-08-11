use crate::{
    MethodInfo, ResolutionContext, StepResult,
    resolution::{TypeResolutionExt, ValueResolution},
    stack::ops::*,
    state::{ArenaLocalState, SharedGlobalState, StaticStorageManager},
    sync::Arc,
};
use dotnet_tracer::Tracer;
use dotnet_types::{
    TypeDescription,
    error::{ExecutionError, TypeResolutionError},
    generics::GenericLookup,
    members::MethodDescription,
    runtime::RuntimeType,
};
use dotnet_utils::{gc::GCHandle, sync::Ordering};
use dotnet_value::{
    ManagedPtr, StackValue,
    object::{HeapStorage, ObjectRef},
    pointer::{ManagedPtrInfo, ManagedPtrResolver, StaticMetadata},
    storage::FieldStorage,
};
use dotnet_vm_data::FrameReturnAction;
use dotnetdll::prelude::*;
use gc_arena::{Collect, Mutation};
use std::ptr::NonNull;

pub use dotnet_vm_data::{BasePointer, EvaluationStack, ExceptionState, FrameStack, PinnedLocals};

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
    pub(crate) continuation: &'a mut dotnet_vm_data::VmContinuation<'gc>,
    pub(crate) call_args_buffer: &'a mut Vec<StackValue<'gc>>,
    pub(crate) heap_managed_ptr_decode_cache: &'a mut super::HeapManagedPtrDecodeCache<'gc>,
}

struct VesManagedPtrResolver<'a, 'gc> {
    evaluation_stack: &'a EvaluationStack<'gc>,
    statics: &'a StaticStorageManager,
}

impl<'a, 'gc, 'resolver> ManagedPtrResolver<'resolver> for VesManagedPtrResolver<'a, 'gc> {
    fn stack_slot_base(&self, slot: crate::StackSlotIndex) -> Option<NonNull<u8>> {
        Some(self.evaluation_stack.get_slot_address(slot))
    }

    fn static_storage_base(&self, metadata: &StaticMetadata) -> Option<NonNull<u8>> {
        let storage = self
            .statics
            .get(metadata.type_desc.clone(), &metadata.generics);
        storage
            .storage
            .with_data(|data| NonNull::new(data.as_ptr().cast_mut()))
    }
}

impl<'a, 'gc> VmManagedPtrDecodeCacheOps<'gc> for VesContext<'a, 'gc> {
    fn read_managed_ptr_with_heap_cache(
        &mut self,
        source: &[u8],
        gc: &Mutation<'gc>,
    ) -> Result<ManagedPtrInfo<'gc>, dotnet_types::error::PointerDeserializationError> {
        let resolver = VesManagedPtrResolver {
            evaluation_stack: &*self.evaluation_stack,
            statics: &self.shared.statics,
        };
        // SAFETY: F3.InteriorPointerRebased — The field reader supplies one complete ManagedPtr encoding
        // from live managed storage. The cache is rooted by this CallStack and
        // GCArena::finish_cycle invalidates its serialized-handle keys after
        // each collection epoch.
        unsafe {
            ManagedPtr::read_resolved_branded_with_heap_cache_unchecked(
                source,
                gc,
                &resolver,
                self.heap_managed_ptr_decode_cache,
            )
        }
    }
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
        let _ = self.tracer().enabled_emit(self.indent(), |trace| {
            let val_str = format!("{:?}", value);
            trace.stack_op("PUSH", &val_str);
        });
    }

    #[inline]
    pub(crate) fn trace_pop(&self, value: &StackValue<'gc>) {
        let _ = self.tracer().enabled_emit(self.indent(), |trace| {
            let val_str = format!("{:?}", value);
            trace.stack_op("POP", &val_str);
        });
    }

    #[inline]
    pub(crate) fn trace_method_entry_for_call_frame(&self, method: &MethodInfo<'static>) {
        // Managed-method tracing must follow frame lifecycle: entry is emitted at call setup,
        // and exits are emitted from frame pop/return/unwind paths.
        let _ = self.tracer().enabled_emit(self.indent(), |trace| {
            let method_desc = format!("{:?}", method.source);
            trace.method_entry(&method_desc, "");
        });
    }

    #[inline]
    pub(crate) fn trace_method_exit_for_frame(&self, frame: &dotnet_vm_data::StackFrame<'gc>) {
        let _ = self.tracer().enabled_emit(self.frame_stack.len(), |trace| {
            let method_name = format!("{:?}", frame.state.info_handle.source);
            trace.method_exit(&method_name);
        });
    }

    #[inline]
    pub(crate) fn pop_and_recycle_frame_with_trace(&mut self) -> bool {
        let Some(frame) = self.frame_stack.pop() else {
            return false;
        };

        self.trace_method_exit_for_frame(&frame);
        self.frame_stack.recycle_frame(frame);
        true
    }

    #[inline]
    fn cctor_type_desc_and_lookup(
        frame: &dotnet_vm_data::StackFrame<'gc>,
    ) -> (TypeDescription, GenericLookup) {
        let type_desc = frame.state.info_handle.source.parent.clone();
        // Reuse the existing Arc<[ConcreteType]> directly — no Vec allocation.
        let cctor_generics = GenericLookup::from_type_arc(frame.generic_inst.type_generics.clone());
        (type_desc, cctor_generics)
    }

    #[inline]
    pub(crate) fn on_pop_safe(&mut self) -> Result<(), crate::error::VmError> {
        if let Some(frame) = self.frame_stack.current_frame_opt_mut() {
            frame.stack_height =
                frame
                    .stack_height
                    .checked_sub(1usize)
                    .ok_or(crate::error::VmError::Execution(
                        crate::error::ExecutionError::StackUnderflow,
                    ))?;
        }
        Ok(())
    }

    pub fn top_of_stack_address(&self) -> NonNull<u8> {
        self.evaluation_stack
            .get_slot_address(self.evaluation_stack.top_of_stack().saturating_sub(1usize))
    }

    pub(crate) fn init_locals(
        &mut self,
        method: &MethodDescription,
        locals: &[LocalVariable],
        generics: &GenericLookup,
        locals_base: crate::StackSlotIndex,
    ) -> Result<PinnedLocals, TypeResolutionError> {
        let mut pinned_locals = PinnedLocals::with_capacity(locals.len());
        // Local initialization only resolves the declared local types and creates their
        // default values. It never exposes this context through reflection, the sole consumer
        // of descriptor owners, so do not clone the method/type descriptors just to carry
        // reflection ownership that this path cannot observe.
        let ctx = ResolutionContext {
            generics,
            state: self.shared.resolution_shared(),
            resolution: method.resolution(),
            type_owner: None,
            method_owner: None,
        };

        for (i, l) in locals.iter().enumerate() {
            use BaseType::*;
            use LocalVariable::*;
            let value = match l {
                TypedReference => {
                    pinned_locals.push(false);
                    StackValue::UninitializedTypedRef
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
                                new_lookup.set_method_generics(generics.method_generics.clone());
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

        let result = self.return_frame();
        if !matches!(result, StepResult::Continue) {
            return result;
        }

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

    fn clear_evaluation_stack_from(&mut self, start: crate::StackSlotIndex) {
        for i in start.as_usize()..self.evaluation_stack.top_of_stack().as_usize() {
            self.evaluation_stack
                .set_slot(crate::StackSlotIndex::new(i), StackValue::null());
        }
        self.evaluation_stack.truncate(start);
    }

    pub fn return_frame(&mut self) -> StepResult {
        if matches!(
            self.frame_stack
                .current_frame()
                .state
                .info_handle
                .signature
                .return_type
                .1,
            Some(ParameterType::TypedReference)
        ) {
            return StepResult::Error(
                ExecutionError::InvalidCil(
                    "ECMA-335 typedref values cannot be method return values".into(),
                )
                .into(),
            );
        }

        // VM invariant: `return_frame` is only reached while executing a method body, so the
        // dispatch loop must have an active frame. An empty stack here indicates a bug in frame
        // lifecycle management, not a recoverable runtime condition.
        let frame = self
            .frame_stack
            .pop()
            .expect("return_frame called with empty frame stack (ret/dispatch invariant violated)");

        self.trace_method_exit_for_frame(&frame);

        if frame.state.info_handle.is_cctor {
            let (type_desc, cctor_generics) = Self::cctor_type_desc_and_lookup(&frame);
            self.shared
                .statics
                .mark_initialized(type_desc, &cctor_generics);
        }

        if frame.is_finalizer {
            self.local.heap.processing_finalizer.set(false);
        }

        let signature = frame.state.info_handle.signature;
        let return_value = if let ReturnType(_, Some(_)) = &signature.return_type {
            let return_slot_index = (frame.base.stack + frame.stack_height)
                .checked_sub(1)
                .expect(
                    "non-void return frame lacks a return-value stack slot: VM invariant violated",
                );
            Some(self.evaluation_stack.get_slot(return_slot_index))
        } else {
            None
        };

        self.clear_evaluation_stack_from(frame.base.arguments);

        if let Some(return_value) = return_value {
            // Note: self.push() calls on_push() which increments stack_height,
            // so we must NOT manually increment stack_height here.
            self.push(return_value);
        }

        let frame_continuation = self
            .frame_stack
            .current_frame_opt_mut()
            .and_then(|caller| caller.frame_continuation.take());

        if let Some(FrameReturnAction::InvokeReturn(return_type)) = frame_continuation {
            if matches!(return_type, RuntimeType::Void) {
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
            let (type_desc, cctor_generics) = Self::cctor_type_desc_and_lookup(&frame);
            self.shared
                .statics
                .mark_failed(type_desc.clone(), &cctor_generics);

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
        self.clear_evaluation_stack_from(frame.base.arguments);
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
            if let HeapStorage::Obj(o) = &ptr.borrow().storage
                && o.has_finalizer
            {
                self.local
                    .heap
                    .finalization_queue
                    .borrow_mut()
                    .push(*instance);
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

#[derive(Collect)]
#[collect(no_drop, gc_lifetime = 'gc)]
pub struct ThreadContext<'gc> {
    pub evaluation_stack: EvaluationStack<'gc>,
    pub frame_stack: FrameStack<'gc>,
    pub exception_mode: ExceptionState<'gc>,
    pub current_intrinsic: Option<crate::CollectableMethodDescription>,
    pub original_ip: usize,
    pub original_stack_height: crate::StackSlotIndex,
    pub continuation: dotnet_vm_data::VmContinuation<'gc>,
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
                + dotnet_intrinsics_span::SpanIntrinsicHost<'gc>
                + vm_ops::SimdIntrinsicHost<'gc>
                + dotnet_intrinsics_unsafe::UnsafeIntrinsicHost<'gc>
                + dotnet_intrinsics_threading::ThreadingIntrinsicHost<'gc>
                + dotnet_intrinsics_reflection::ReflectionIntrinsicHost<'gc>,
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

        fn assert_span_handler<'gc, T: dotnet_intrinsics_span::SpanIntrinsicHost<'gc>>() {
            let _handler: fn(&mut T, MethodDescription, &GenericLookup) -> StepResult =
                dotnet_intrinsics_span::equality::intrinsic_memory_extensions_sequence_equal::<T>;
        }

        fn assert_simd_handler<'gc, T: vm_ops::SimdIntrinsicHost<'gc>>() {
            let _handler: fn(&mut T, MethodDescription, &GenericLookup) -> StepResult =
                dotnet_intrinsics_core::simd::intrinsic_vector128_is_hardware_accelerated::<T>;
        }

        fn assert_unsafe_handler<'gc, T: dotnet_intrinsics_unsafe::UnsafeIntrinsicHost<'gc>>() {
            let _handler: fn(&mut T, MethodDescription, &GenericLookup) -> StepResult =
                dotnet_intrinsics_unsafe::marshal::intrinsic_marshal_offset_of::<T>;
        }

        fn assert_threading_handler<
            'gc,
            T: dotnet_intrinsics_threading::ThreadingIntrinsicHost<'gc>,
        >() {
            let _handler: fn(&mut T, MethodDescription, &GenericLookup) -> StepResult =
                dotnet_intrinsics_threading::monitor::intrinsic_monitor_reliable_enter::<T>;
        }

        fn assert_reflection_handler<
            'gc,
            T: dotnet_intrinsics_reflection::ReflectionIntrinsicHost<'gc>,
        >() {
            let _handler: fn(&mut T, MethodDescription, &GenericLookup) -> StepResult =
                dotnet_intrinsics_reflection::methods::runtime_method_info_intrinsic_call::<T>;
        }

        type TestContext = VesContext<'static, 'static>;
        assert_string_handler::<'static, TestContext>();
        assert_delegate_handler::<'static, TestContext>();
        assert_span_handler::<'static, TestContext>();
        assert_simd_handler::<'static, TestContext>();
        assert_unsafe_handler::<'static, TestContext>();
        assert_threading_handler::<'static, TestContext>();
        assert_reflection_handler::<'static, TestContext>();
    }
}

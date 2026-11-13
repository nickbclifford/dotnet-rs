use super::VesContext;
use crate::{
    MethodState, StepResult,
    resolution::ValueResolution,
    stack::ops::*,
    sync::{Arc, SyncBlockOps, SyncManagerOps},
};
use dotnet_tracer::Tracer;
use dotnet_types::{
    TypeDescription, WellKnown,
    error::{ExecutionError, IntrinsicError, TypeResolutionError},
    generics::{ConcreteType, GenericLookup},
    members::{FieldDescription, MethodDescription},
    runtime::RuntimeType,
};
use dotnet_utils::{BorrowScopeOps, gc::GCHandle};
use dotnet_value::{
    StackValue,
    layout::LayoutManager,
    object::{HeapStorage, Object, ObjectRef},
    pointer::{ManagedPtrResolver, StaticMetadata},
};
use dotnet_vm_ops::MemoryOps;
use dotnet_vm_ops::ops::StackOps;
use dotnetdll::prelude::{MethodMemberIndex, MethodType};
use std::{
    ptr::NonNull,
    sync::{Arc as StdArc, atomic::AtomicBool},
};

impl<'a, 'gc> VesContext<'a, 'gc> {
    /// Save the executing handler before starting nested exception control flow.
    pub fn push_handler_unwind(&mut self, state: dotnet_vm_data::UnwindState<'gc>) {
        let frame_index = state.cursor.frame_index;
        let frame = &self.frame_stack.frames[frame_index];
        let ip = frame.state.ip;
        let handler = frame
            .state
            .info_handle
            .exceptions
            .iter()
            .enumerate()
            .flat_map(|(section_index, section)| {
                section
                    .handlers
                    .iter()
                    .enumerate()
                    .map(move |(handler_index, handler)| (section_index, handler_index, handler))
            })
            .filter(|(_, _, h)| {
                matches!(
                    h.kind,
                    dotnet_vm_data::exceptions::HandlerKind::Finally
                        | dotnet_vm_data::exceptions::HandlerKind::Fault
                ) && h.instructions.contains(&ip)
            })
            .min_by_key(|(_, _, h)| h.instructions.end - h.instructions.start)
            .map(
                |(section_index, handler_index, _)| dotnet_vm_data::HandlerAddress {
                    frame_index,
                    section_index,
                    handler_index,
                },
            )
            .expect("executing handler must contain its frame instruction pointer");
        let saved = dotnet_vm_data::SavedHandlerUnwind { state, handler };
        match self.continuation {
            dotnet_vm_data::VmContinuation::HandlerUnwinds(unwinds) => unwinds.push(saved),
            _ => *self.continuation = dotnet_vm_data::VmContinuation::HandlerUnwinds(vec![saved]),
        }
    }

    /// Discard suspended handlers in frames being removed.
    pub fn discard_handler_unwinds_from(&mut self, frame_index: usize) {
        if let dotnet_vm_data::VmContinuation::HandlerUnwinds(unwinds) = self.continuation {
            unwinds.retain(|saved| saved.handler.frame_index < frame_index);
            if unwinds.is_empty() {
                *self.continuation = dotnet_vm_data::VmContinuation::None;
            }
        }
    }

    /// Discard exited handlers and resume the innermost handler containing the destination.
    pub fn resume_handler_unwind(
        &mut self,
        destination: Option<(usize, usize)>,
    ) -> Option<dotnet_vm_data::UnwindState<'gc>> {
        let dotnet_vm_data::VmContinuation::HandlerUnwinds(unwinds) = self.continuation else {
            return None;
        };
        while let Some(saved) = unwinds.last() {
            let h = saved.handler;
            let contains = destination.is_some_and(|(frame, ip)| {
                h.frame_index < frame
                    || (h.frame_index == frame
                        && self
                            .frame_stack
                            .frames
                            .get(frame)
                            .and_then(|f| f.state.info_handle.exceptions.get(h.section_index))
                            .and_then(|s| s.handlers.get(h.handler_index))
                            .is_some_and(|handler| handler.instructions.contains(&ip)))
            });
            if contains {
                break;
            }
            unwinds.pop();
        }
        let state = unwinds.pop().map(|saved| saved.state);
        if unwinds.is_empty() {
            *self.continuation = dotnet_vm_data::VmContinuation::None;
        }
        state
    }
}

impl<'a, 'gc> LoaderOps for VesContext<'a, 'gc> {
    #[inline]
    fn loader(&self) -> &Arc<dotnet_assemblies::AssemblyLoader> {
        &self.shared.loader
    }
}

impl<'a, 'gc> SimdCapabilityOps for VesContext<'a, 'gc> {
    #[inline]
    fn simd_vector128_is_hardware_accelerated(&self) -> bool {
        dotnet_intrinsics_core::simd::vector128_is_hardware_accelerated()
    }

    #[inline]
    fn simd_vector256_is_hardware_accelerated(&self) -> bool {
        dotnet_intrinsics_core::simd::vector256_is_hardware_accelerated()
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

impl<'a, 'gc> StackOps<'gc> for VesContext<'a, 'gc> {}

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
    fn new_object(&self, td: TypeDescription) -> Result<Object<'gc>, TypeResolutionError> {
        self.resolver().new_object(td, &self.current_context())
    }

    #[inline]
    fn new_object_with_lookup(
        &self,
        td: TypeDescription,
        lookup: &GenericLookup,
    ) -> Result<Object<'gc>, TypeResolutionError> {
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
            self.shared.loader.corlib_wkt(WellKnown::Object)
        }
    }
}

impl<'a, 'gc> dotnet_intrinsics_delegates::DelegateInvokeHost<'gc> for VesContext<'a, 'gc> {
    #[inline]
    fn delegate_dispatch_kind(
        &self,
        method: &MethodDescription,
    ) -> Option<dotnet_intrinsics_delegates::DelegateDispatchKind> {
        self.shared.caches.delegate_dispatch_cache.get(method)
    }

    #[inline]
    fn cache_delegate_dispatch_kind(
        &self,
        method: MethodDescription,
        kind: dotnet_intrinsics_delegates::DelegateDispatchKind,
    ) {
        self.shared
            .caches
            .delegate_dispatch_cache
            .insert(method, kind);
    }

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

    fn delegate_dispatch_method_inline(
        &mut self,
        method: MethodDescription,
        lookup: GenericLookup,
    ) -> StepResult {
        self.dispatch_method_inline(method, lookup)
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
    fn string_with_span_data<R, F: FnOnce(&[u8]) -> R>(
        &self,
        span: Object<'_>,
        element_type: TypeDescription,
        element_size: usize,
        f: F,
    ) -> Result<R, IntrinsicError> {
        dotnet_intrinsics_span::helpers::with_span_data(self, span, element_type, element_size, f)
    }
}

impl<'a, 'gc> dotnet_vm_ops::ops::TypeLayoutOps for VesContext<'a, 'gc> {
    #[inline]
    fn type_layout(&self, t: ConcreteType) -> Result<StdArc<LayoutManager>, TypeResolutionError> {
        crate::layout::type_layout(t, &self.current_context())
    }
}

impl<'a, 'gc> dotnet_intrinsics_span::LayoutQueryHost for VesContext<'a, 'gc> {}

impl<'a, 'gc, 'resolver> ManagedPtrResolver<'resolver> for VesContext<'a, 'gc> {
    #[inline]
    fn static_storage_base(&self, metadata: &StaticMetadata) -> Option<NonNull<u8>> {
        let storage = self.statics().get_for_metadata(metadata);
        storage
            .storage
            .with_data(|data| NonNull::new(data.as_ptr().cast_mut()))
    }

    #[inline]
    fn frame_home_block(
        &self,
        serial: dotnet_value::pointer::FrameHomeSerial,
    ) -> Option<(NonNull<u8>, usize)> {
        self.frame_stack.home_block(serial).or_else(|| {
            self.parked_finalizer_caller
                .as_ref()?
                .frame_stack
                .home_block(serial)
        })
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
        let blocks = &self.shared().sync_blocks;
        let mut created = None;
        let mut create = || {
            let (index, block) = blocks.create_sync_block();
            created = Some(block);
            index
        };
        let index = object.get_or_assign_sync_block_index(gc, &mut create);
        match created {
            Some(block) => block,
            // Sync blocks are never removed, so an index published on an object always resolves.
            None => blocks
                .get_sync_block(index)
                .expect("published sync block index has no block"),
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
    fn monitor_exit(&self, sync_block: &Self::SyncBlock, thread_id: dotnet_utils::ArenaId) -> bool {
        sync_block.exit(thread_id)
    }
    fn monitor_is_owned(&self, block: &Self::SyncBlock, thread: dotnet_utils::ArenaId) -> bool {
        block.is_owned(thread)
    }
    fn monitor_pulse(
        &self,
        block: &Self::SyncBlock,
        thread: dotnet_utils::ArenaId,
        all: bool,
    ) -> bool {
        block.pulse(thread, all)
    }
}

impl<'a, 'gc> dotnet_intrinsics_threading::StackSlotWriteHost<'gc> for VesContext<'a, 'gc> {
    #[inline]
    fn threading_set_stack_slot(&mut self, index: crate::StackSlotIndex, value: StackValue<'gc>) {
        self.set_slot(index, value);
    }
}

impl<'a, 'gc> dotnet_intrinsics_threading::ManagedThreadHost<'gc> for VesContext<'a, 'gc> {
    fn threading_current_thread(&mut self, thread_type: TypeDescription) -> StepResult {
        let thread = match self.local.managed_threads.current() {
            Some(thread) => thread,
            None => {
                let object = dotnet_vm_ops::vm_try!(self.new_object(thread_type));
                let thread = ObjectRef::new(self.gc, HeapStorage::Obj(Box::new(object)));
                self.register_new_object(&thread);
                self.local.managed_threads.set_current(thread);
                #[cfg(feature = "multithreading")]
                self.shared.workers.register_managed_thread(
                    thread.as_ptr().unwrap(),
                    crate::threading::workers::ManagedThreadMetadata {
                        id: self.local.managed_threads.current_id(),
                        delegate: None,
                        parameterized: false,
                        started: true,
                        background: false,
                        completion_index: None,
                        retained: StdArc::new(AtomicBool::new(true)),
                    },
                );
                thread
            }
        };
        self.push_obj(thread);
        StepResult::Continue
    }

    fn threading_new_managed_thread(
        &mut self,
        thread_type: TypeDescription,
        thread_start: ObjectRef<'gc>,
        parameterized: bool,
    ) -> StepResult {
        if !crate::threading::managed_thread_workers_supported() {
            return self.throw_by_name_with_message(
                "System.PlatformNotSupportedException",
                "Managed Thread(ThreadStart) requires the multithreading feature.",
            );
        }

        let thread = dotnet_vm_ops::vm_try!(self.new_object(thread_type));
        let thread = ObjectRef::new(self.gc, HeapStorage::Obj(Box::new(thread)));
        self.register_new_object(&thread);
        let retained = StdArc::new(AtomicBool::new(true));
        self.local.managed_threads.records_write().insert(
            thread,
            crate::threading::local::ManagedThreadRecord::new(thread_start, retained.clone()),
        );
        #[cfg(feature = "multithreading")]
        self.shared.workers.register_managed_thread(
            thread.as_ptr().unwrap(),
            crate::threading::workers::ManagedThreadMetadata {
                id: crate::threading::local::next_managed_id(),
                delegate: thread_start.as_ptr(),
                parameterized,
                started: false,
                background: false,
                completion_index: None,
                retained,
            },
        );
        #[cfg(not(feature = "multithreading"))]
        let _ = parameterized;
        self.push_obj(thread);
        StepResult::Continue
    }

    fn threading_start_managed_thread(
        &mut self,
        thread: ObjectRef<'gc>,
        argument: ObjectRef<'gc>,
        has_argument: bool,
    ) -> StepResult {
        if thread.0.is_none() {
            return self.throw_by_name_with_message(
                "System.NullReferenceException",
                dotnet_vm_ops::NULL_REF_MSG,
            );
        }

        if !crate::threading::managed_thread_workers_supported() {
            return self.throw_by_name_with_message(
                "System.PlatformNotSupportedException",
                "Managed Thread(ThreadStart) requires the multithreading feature.",
            );
        }

        #[cfg(feature = "multithreading")]
        {
            let ptr = thread.as_ptr().expect("non-null thread");
            let Some(metadata) = self.shared.workers.managed_thread(ptr) else {
                return self.throw_by_name_with_message(
                    "System.Threading.ThreadStateException",
                    "Thread is not startable.",
                );
            };
            if metadata.started {
                return self.throw_by_name_with_message(
                    "System.Threading.ThreadStateException",
                    "Thread has already started.",
                );
            }
            if has_argument && !metadata.parameterized {
                return self.throw_by_name_with_message(
                    "System.InvalidOperationException",
                    "Thread was not created with a ParameterizedThreadStart.",
                );
            }
            let support = self
                .loader()
                .get_assembly(dotnet_assemblies::SUPPORT_ASSEMBLY)
                .expect("support assembly");
            let (index, _) = support
                .definition()
                .enumerate_type_definitions()
                .find(|(_, ty)| ty.type_name() == "DotnetRs.ThreadStartRunner")
                .expect("thread start runner type");
            let ty = TypeDescription::new(support.clone(), index);
            let method_name = if metadata.parameterized {
                "RunParameterized"
            } else {
                "Run"
            };
            let index = ty
                .definition()
                .methods
                .iter()
                .position(|method| method.name == method_name)
                .expect("thread start runner method");
            let runner = MethodDescription::new(
                ty,
                GenericLookup::default(),
                support,
                MethodMemberIndex::Method(index),
            );
            if self.shared.workers.begin_managed_thread(ptr).is_none() {
                return self.throw_by_name_with_message(
                    "System.Threading.ThreadStateException",
                    "Thread has already started.",
                );
            }
            let delegate_ptr = metadata.delegate.expect("start delegate");
            // SAFETY: F1.GcHandleRooted — the creating arena roots the delegate until this worker
            // completes.
            let delegate =
                unsafe { ObjectRef::from_storage_ptr(delegate_ptr.as_ptr().cast_mut().cast()) };
            {
                let mut records = self.local.managed_threads.records_write();
                let record = records.entry(thread).or_insert_with(|| {
                    crate::threading::local::ManagedThreadRecord::new(
                        delegate,
                        metadata.retained.clone(),
                    )
                });
                record.start_argument = argument;
            }
            let start = crate::threading::local::WorkerStart {
                thread: ptr,
                delegate: delegate_ptr,
                argument: argument.as_ptr(),
                id: metadata.id,
                retained: metadata.retained.clone(),
            };
            crate::threading::spawn_thread_start(
                self.shared.clone(),
                runner,
                GenericLookup::default(),
                start,
            )
            .expect("multithreaded worker spawn");
            StepResult::Continue
        }
        #[cfg(not(feature = "multithreading"))]
        {
            let _ = (argument, has_argument);
            unreachable!("single-threaded workers are not supported")
        }
    }

    fn threading_managed_id(&mut self, thread: ObjectRef<'gc>) -> StepResult {
        if thread.0.is_none() {
            return self.throw_by_name_with_message(
                "System.NullReferenceException",
                dotnet_vm_ops::NULL_REF_MSG,
            );
        }
        if self.local.managed_threads.current() == Some(thread) {
            self.push_i32(self.local.managed_threads.current_id());
            return StepResult::Continue;
        }
        #[cfg(feature = "multithreading")]
        if let Some(metadata) = self.shared.workers.managed_thread(thread.as_ptr().unwrap()) {
            self.push_i32(metadata.id);
            return StepResult::Continue;
        }
        self.throw_by_name_with_message(
            "System.Threading.ThreadStateException",
            "Unknown managed thread.",
        )
    }

    fn threading_background(&mut self, thread: ObjectRef<'gc>, value: Option<bool>) -> StepResult {
        if thread.0.is_none() {
            return self.throw_by_name_with_message(
                "System.NullReferenceException",
                dotnet_vm_ops::NULL_REF_MSG,
            );
        }
        #[cfg(feature = "multithreading")]
        {
            let ptr = thread.as_ptr().unwrap();
            if let Some(value) = value {
                if self.shared.workers.set_background(ptr, value) {
                    return StepResult::Continue;
                }
            } else if let Some(metadata) = self.shared.workers.managed_thread(ptr) {
                self.push_i32(i32::from(metadata.background));
                return StepResult::Continue;
            }
        }
        #[cfg(not(feature = "multithreading"))]
        if self.local.managed_threads.current() == Some(thread) {
            if let Some(value) = value {
                self.local.managed_threads.set_background(value);
            } else {
                self.push_i32(i32::from(self.local.managed_threads.background()));
            }
            return StepResult::Continue;
        }
        self.throw_by_name_with_message(
            "System.Threading.ThreadStateException",
            "Unknown managed thread.",
        )
    }
}

impl<'a, 'gc> dotnet_intrinsics_threading::PoolHost<'gc> for VesContext<'a, 'gc> {
    #[cfg(feature = "multithreading")]
    fn pool_request(&mut self) -> StepResult {
        let spawn = self.shared.pool.request();
        if spawn {
            let corlib = self
                .loader()
                .corlib_wkt(WellKnown::Object)
                .expect("CoreLib Object")
                .resolution;
            let resolve = |type_name: &str, method_name: &str| -> Option<MethodDescription> {
                let (index, _) = corlib
                    .definition()
                    .enumerate_type_definitions()
                    .find(|(_, ty)| ty.type_name() == type_name)?;
                let ty = TypeDescription::new(corlib.clone(), index);
                let index = ty
                    .definition()
                    .methods
                    .iter()
                    .position(|m| m.name == method_name)
                    .map(MethodMemberIndex::Method)
                    .or_else(|| {
                        ty.definition()
                            .properties
                            .iter()
                            .position(|p| p.setter.as_ref().is_some_and(|m| m.name == method_name))
                            .map(MethodMemberIndex::PropertySetter)
                    })?;
                Some(MethodDescription::new(
                    ty,
                    GenericLookup::default(),
                    corlib.clone(),
                    index,
                ))
            };
            let Some(dispatch) = resolve("System.Threading.ThreadPoolWorkQueue", "Dispatch") else {
                return StepResult::Error(
                    ExecutionError::InternalError("CoreLib pool dispatcher not found".into())
                        .into(),
                );
            };
            let signature = dispatch.signature();
            if signature.instance
                || !signature.parameters.is_empty()
                || !matches!(
                    &signature.return_type.1,
                    Some(dotnetdll::prelude::ParameterType::Value(MethodType::Base(base)))
                        if matches!(base.as_ref(), dotnetdll::prelude::BaseType::Boolean)
                )
            {
                return StepResult::Error(
                    ExecutionError::InternalError(
                        "CoreLib pool dispatcher has unexpected signature".into(),
                    )
                    .into(),
                );
            }
            let Some(setter) = resolve("System.Threading.Thread", "set_IsThreadPoolThread") else {
                return StepResult::Error(
                    ExecutionError::InternalError("CoreLib pool-thread setter not found".into())
                        .into(),
                );
            };
            let signature = setter.signature();
            if !signature.instance
                || signature.parameters.len() != 1
                || signature.return_type.1.is_some()
                || !matches!(&signature.parameters[0].1,
                    dotnetdll::prelude::ParameterType::Value(MethodType::Base(base))
                        if matches!(base.as_ref(), dotnetdll::prelude::BaseType::Boolean))
            {
                return StepResult::Error(
                    ExecutionError::InternalError(
                        "CoreLib pool-thread setter has unexpected signature".into(),
                    )
                    .into(),
                );
            }
            let shared = self.shared.clone();
            let worker_shared = shared.clone();
            shared.workers.spawn_pool_owner(move |running| {
                let fatal_shared = worker_shared.clone();
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    crate::Executor::run_pool_worker(worker_shared, dispatch, setter, running);
                }));
                if let Err(panic) = result {
                    fatal_shared.report_worker_error(
                        ExecutionError::InternalError("ThreadPool worker panicked".into()).into(),
                    );
                    std::panic::resume_unwind(panic);
                }
            });
        }
        StepResult::Continue
    }
    #[cfg(not(feature = "multithreading"))]
    fn pool_request(&mut self) -> StepResult {
        self.shared.pool.request();
        StepResult::Continue
    }
    fn pool_placeholder(&mut self) -> StepResult {
        let ty = self
            .loader()
            .corlib_wkt(WellKnown::Object)
            .expect("CoreLib Object");
        let object = dotnet_vm_ops::vm_try!(self.new_object(ty));
        let candidate = ObjectRef::new(self.gc, HeapStorage::Obj(Box::new(object)));
        self.register_new_object(&candidate);
        let placeholder = self.shared.statics.pool_placeholder(candidate);
        self.push_obj(placeholder);
        StepResult::Continue
    }
    fn pool_complete(&self) {
        self.shared.pool.completed();
    }
    fn pool_completed_count(&self) -> i64 {
        self.shared.pool.completed_count()
    }
    fn pool_worker_count(&self) -> i32 {
        self.shared.pool.worker_count()
    }
    fn pool_set_min(&self, worker: i32, io: i32) -> bool {
        self.shared.pool.set_min(worker, io)
    }
    fn pool_set_max(&self, worker: i32, io: i32) -> bool {
        self.shared.pool.set_max(worker, io)
    }
    fn pool_min(&self) -> (i32, i32) {
        self.shared.pool.min()
    }
    fn pool_max(&self) -> (i32, i32) {
        self.shared.pool.max()
    }
    fn pool_available(&self) -> (i32, i32) {
        self.shared.pool.available()
    }
}

impl<'a, 'gc> dotnet_intrinsics_unsafe::UnsafeIntrinsicHost<'gc> for VesContext<'a, 'gc> {
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
            _ => self.local.heap.find_object(ptr.addr()),
        };

        if let Some(owner) = owner {
            let layout = memory
                .get_layout_from_owner(dotnet_runtime_memory::MemoryOwner::Local(owner))
                .expect("local owner cannot fail arena lease acquisition");
            let (base, _) = memory.get_storage_base(owner);
            let base_addr = if base.is_null() {
                None
            } else {
                Some(base.addr())
            };
            (layout, base_addr)
        } else {
            (None, None)
        }
    }

    #[inline]
    fn unsafe_get_last_pinvoke_error(&self) -> i32 {
        self.local.pinvoke_last_error
    }

    #[inline]
    fn unsafe_set_last_pinvoke_error(&mut self, value: i32) {
        self.local.pinvoke_last_error = value;
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
        // `newobj` re-executes with its operands intact once the initializer completes.
        self.initialize_type_then(
            description,
            &generics,
            super::native_effects::InitializerResume::Retry,
        )
    }
}

impl<'a, 'gc> VesInternals<'gc> for VesContext<'a, 'gc> {
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
    fn exception_mode(&self) -> &dotnet_vm_data::ExceptionState<'gc> {
        self.exception_mode
    }

    #[inline]
    fn exception_mode_mut(&mut self) -> &mut dotnet_vm_data::ExceptionState<'gc> {
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

    fn native_boundaries(
        &self,
    ) -> Vec<(usize, dotnet_vm_data::NativeId, MethodDescription, usize)> {
        self.call_state
            .native
            .iter()
            .map(|work| {
                (
                    work.caller_depth,
                    work.id,
                    work.method.0.clone(),
                    work.caller_ip,
                )
            })
            .collect()
    }

    fn native_capture_at(&self, depth: usize) -> Option<dotnet_vm_data::NativeReturnDestination> {
        self.call_state.native.iter().rev().find_map(|work| {
            let pending = work.pending.as_ref()?;
            (work.caller_depth == depth && pending.capture).then_some(
                dotnet_vm_data::NativeReturnDestination {
                    id: work.id,
                    generation: pending.generation,
                },
            )
        })
    }

    fn capture_native_exception(
        &mut self,
        destination: dotnet_vm_data::NativeReturnDestination,
        exception: ObjectRef<'gc>,
    ) -> Result<(), dotnet_types::error::VmError> {
        self.deliver_native_exception(destination, exception)
    }

    fn unwind_native_at(&mut self, depth: usize) {
        self.unwind_native_at(depth);
    }

    fn suspend_native_above(&mut self, frame_index: usize) {
        let first = self
            .call_state
            .native
            .iter()
            .position(|work| work.caller_depth > frame_index)
            .unwrap_or(self.call_state.native.len());
        let segment = self.call_state.native.split_off(first);
        self.call_state
            .suspended_native
            .push((frame_index + 1, segment));
    }

    fn restore_suspended_native(&mut self) {
        let (base, segment) = self
            .call_state
            .suspended_native
            .pop()
            .expect("native suspension underflow");
        assert!(
            self.call_state
                .native
                .last()
                .is_none_or(|work| { work.caller_depth < base }),
            "filter native call still active on restore"
        );
        self.call_state.native.extend(segment);
    }

    #[inline]
    fn unwind_frame(&mut self) {
        self.unwind_frame()
    }

    #[inline]
    fn evaluation_stack_mut(&mut self) -> &mut dotnet_vm_data::EvaluationStack<'gc> {
        self.evaluation_stack
    }

    #[inline]
    fn frame_stack(&self) -> &dotnet_vm_data::FrameStack<'gc> {
        self.frame_stack
    }

    #[inline]
    fn frame_stack_mut(&mut self) -> &mut dotnet_vm_data::FrameStack<'gc> {
        self.frame_stack
    }

    #[inline]
    fn discard_handler_unwinds_from(&mut self, frame_index: usize) {
        VesContext::discard_handler_unwinds_from(self, frame_index)
    }

    fn resume_handler_unwind(
        &mut self,
        destination: Option<(usize, usize)>,
    ) -> Option<dotnet_vm_data::UnwindState<'gc>> {
        VesContext::resume_handler_unwind(self, destination)
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
                dotnet_vm_data::ExceptionState::None
                | dotnet_vm_data::ExceptionState::ExecutingHandler(_)
                | dotnet_vm_data::ExceptionState::Filtering(_) => {
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
}

impl<'a, 'gc> VesContext<'a, 'gc> {
    pub(crate) fn prepare_pending_finalizer(
        &mut self,
    ) -> Result<
        Option<(ObjectRef<'gc>, crate::MethodInfo<'static>, GenericLookup)>,
        crate::error::VmError,
    > {
        let _gc = self.gc;
        if self.local.heap.processing_finalizer.get() {
            return Ok(None);
        }
        let instance = self.local.heap.pending_finalization.borrow_mut().pop();

        if let Some(instance) = instance {
            self.local.heap.processing_finalizer.set(true);
            let ctx = self.current_context();

            if self.tracer_enabled() {
                let ptr = instance
                    .0
                    .expect("finalization queue only ever holds non-null object references");
                let obj_type = match &ptr.borrow().storage {
                    HeapStorage::Obj(o) => o.description.clone(),
                    _ => unreachable!(),
                };
                let type_name = format!("{:?}", obj_type);
                let addr = gc_arena::Gc::as_ptr(ptr).addr();
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
                            .corlib_wkt(WellKnown::Object)
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
                return Ok(None);
            };

            let method_info =
                self.shared
                    .caches
                    .get_method_info(finalizer, &generics, self.shared.clone())?;
            if let Some(err) = self.managed_frame_abort_error(&method_info) {
                return Err(err);
            }
            return Ok(Some((instance, method_info, generics)));
        }

        Ok(None)
    }
}

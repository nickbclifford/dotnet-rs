use crate::{
    error::MemoryAccessError,
    stack::{
        context::VesContext,
        ops::{ArgumentOps, EvalStackOps, LocalOps, ResolutionOps, TypedStackOps, VmStackOps},
    },
};
use dotnet_runtime_memory::RawMemoryAccess;
use dotnet_utils::ByteOffset;
use dotnet_value::{
    CLRString, StackValue,
    layout::{HasLayout, LayoutManager, Scalar},
    object::{HeapStorage, Object as ObjectInstance, ObjectRef},
    pointer::ManagedPtr,
    storage::FieldStorage,
};
use dotnet_vm_data::{
    StepResult,
    frame_homes::{FrameHomes, HomeSlot, VariableHome},
};
use std::ptr::NonNull;

impl<'a, 'gc> EvalStackOps<'gc> for VesContext<'a, 'gc> {
    #[inline]
    fn push(&mut self, value: StackValue<'gc>) {
        self.trace_push(&value);
        self.evaluation_stack.push(value);
        self.on_push();
    }

    #[inline]
    fn pop(&mut self) -> StackValue<'gc> {
        // invariant: infallible stack pops are only for call sites where stack availability was
        // proven by instruction metadata or prior VM checks; fallible paths should use pop_safe().
        self.pop_safe()
            .expect("infallible EvalStackOps::pop called with empty evaluation stack")
    }

    #[inline]
    fn pop_safe(&mut self) -> Result<StackValue<'gc>, crate::error::VmError> {
        self.on_pop_safe()?;
        let val = self.evaluation_stack.pop_safe()?;
        self.trace_pop(&val);
        Ok(val)
    }

    #[inline]
    fn pop_multiple(&mut self, count: usize) -> Vec<StackValue<'gc>> {
        // Pop per element to preserve frame-height accounting and tracer events.
        let mut results = Vec::with_capacity(count);
        for _ in 0..count {
            results.push(self.pop());
        }
        results.reverse();
        results
    }

    #[inline]
    fn peek(&self) -> Option<StackValue<'gc>> {
        self.evaluation_stack.stack.last().cloned()
    }

    #[inline]
    fn peek_stack(&self) -> StackValue<'gc> {
        self.evaluation_stack.peek_stack()
    }

    #[inline]
    fn peek_stack_at(&self, offset: usize) -> StackValue<'gc> {
        self.evaluation_stack.peek_stack_at(offset)
    }

    #[inline]
    fn top_of_stack(&self) -> crate::StackSlotIndex {
        self.evaluation_stack.top_of_stack()
    }
}

impl<'gc> VesContext<'_, 'gc> {
    /// Allocate (or find) the canonical object for `chars` in this context's arena and return
    /// the authoritative table winner. The caller is the service owner, a single-threaded
    /// executor, or an executor with no service; each of those arenas is retained until
    /// shutdown. A concurrent explicit `Intern` winner takes precedence and is returned as is.
    pub(crate) fn materialize_canonical_string(
        &self,
        chars: &[u16],
    ) -> dotnet_value::object::ObjectPtr {
        if let Some(entry) = self.shared.interned_strings.get(chars) {
            return *entry;
        }
        let object = ObjectRef::new(self.gc, HeapStorage::Str(CLRString::new(chars.to_vec())));
        self.register_new_object(&object);
        // New literals use the service arena when active. Explicit Intern winners retain their
        // original allocation and are not subject to the owner assertion.
        #[cfg(feature = "multithreading")]
        debug_assert!(
            !self.shared.canonical.is_active()
                || self.shared.canonical.is_active_owner(self.thread_id.get()),
            "canonical literal allocated outside the service arena"
        );
        *self
            .shared
            .interned_strings
            .entry(chars.to_vec())
            .or_insert(object.as_ptr().expect("interned string is non-null"))
    }

    /// Probe the canonical table for a queued wait. Returns `Ready` once the key is published.
    #[cfg(feature = "multithreading")]
    pub(crate) fn canonical_probe(
        &self,
        key: &dotnet_vm_ops::effects::CanonicalKey,
    ) -> Result<dotnet_vm_ops::effects::NativeCanonicalProbe, dotnet_types::error::VmError> {
        use dotnet_vm_ops::effects::{CanonicalKey, NativeCanonicalProbe};
        let published = match key {
            CanonicalKey::String(chars) => {
                self.shared.interned_strings.contains_key(chars.as_slice())
            }
            CanonicalKey::Type(ty) => self
                .shared
                .reflection_registry
                .runtime_type_objects
                .contains_key(ty),
            CanonicalKey::Method(method, lookup) => self
                .shared
                .reflection_registry
                .runtime_method_objects
                .contains_key(&(method.clone(), lookup.clone())),
            CanonicalKey::Field(field, lookup) => self
                .shared
                .reflection_registry
                .runtime_field_objects
                .contains_key(&(field.clone(), lookup.clone())),
            CanonicalKey::Property(accessor, lookup) => self
                .shared
                .reflection_registry
                .runtime_property_objects
                .contains_key(&(accessor.clone(), lookup.clone())),
            CanonicalKey::Assembly(resolution) => self
                .shared
                .reflection_registry
                .runtime_assembly_objects
                .contains_key(resolution),
        };
        if published {
            return Ok(NativeCanonicalProbe::Ready);
        }
        let needed = self
            .shared
            .canonical
            .request_needed(self.thread_id.get())
            .map_err(|error| {
                dotnet_types::error::ExecutionError::InternalError(
                    error.describe().into_boxed_str(),
                )
            })?;
        if !needed {
            // The owner, or an executor without a service, materializes directly on retry.
            return Ok(NativeCanonicalProbe::Ready);
        }
        match self.shared.canonical.request(key) {
            Ok(wait) => Ok(NativeCanonicalProbe::Waiting(Box::new(wait))),
            Err(error) => Err(dotnet_types::error::ExecutionError::InternalError(
                error.describe().into_boxed_str(),
            )
            .into()),
        }
    }

    /// Materialize at most `max` queued canonical requests. Runs inside the owner's active
    /// mutation; the queue lock is released before any allocation, and the shared table (not
    /// the completion) carries the result.
    #[cfg(feature = "multithreading")]
    pub(crate) fn service_canonical_batch(&mut self, max: usize) -> usize {
        use crate::threading::canonical::CanonicalError;
        use dotnet_vm_ops::effects::CanonicalKey;
        let batch = self.shared.canonical.take_batch(max);
        let served = batch.len();
        for request in batch {
            let outcome = match &request.key {
                CanonicalKey::String(chars) => {
                    self.materialize_canonical_string(chars);
                    Ok(())
                }
                CanonicalKey::Type(ty) => {
                    dotnet_intrinsics_reflection::common::materialize_runtime_type(self, ty.clone())
                        .map(|_| ())
                        .map_err(|error| CanonicalError::Materialization(error.to_string()))
                }
                CanonicalKey::Method(method, lookup) => {
                    dotnet_intrinsics_reflection::common::materialize_runtime_method_obj(
                        self,
                        method.clone(),
                        lookup.clone(),
                    )
                    .map(|_| ())
                    .map_err(|error| CanonicalError::Materialization(error.to_string()))
                }
                CanonicalKey::Field(field, lookup) => {
                    dotnet_intrinsics_reflection::common::materialize_runtime_field_obj(
                        self,
                        field.clone(),
                        lookup.clone(),
                    )
                    .map(|_| ())
                    .map_err(|error| CanonicalError::Materialization(error.to_string()))
                }
                CanonicalKey::Property(accessor, lookup) => {
                    dotnet_intrinsics_reflection::types::materialize_runtime_property_obj(
                        self,
                        accessor.clone(),
                        lookup.clone(),
                    )
                    .map(|_| ())
                    .map_err(|error| CanonicalError::Materialization(error.to_string()))
                }
                CanonicalKey::Assembly(resolution) => {
                    dotnet_intrinsics_reflection::types::materialize_runtime_assembly(
                        self,
                        resolution.clone(),
                    )
                    .map(|_| ())
                    .map_err(|error| {
                        CanonicalError::Materialization(format!(
                            "request {} failed: {error}",
                            request.id
                        ))
                    })
                }
            };
            self.shared.canonical.finish(request, outcome);
        }
        served
    }
}

impl<'a, 'gc> TypedStackOps<'gc> for VesContext<'a, 'gc> {
    #[inline]
    fn push_string(&mut self, value: CLRString) {
        let gc = self.gc;
        let in_heap = ObjectRef::new(gc, HeapStorage::Str(value));
        self.register_new_object(&in_heap);
        self.push(StackValue::ObjectRef(in_heap));
    }

    fn push_interned_string(&mut self, chars: &[u16]) -> StepResult {
        let hit = self.shared.interned_strings.get(chars).map(|entry| *entry);
        let ptr = if let Some(ptr) = hit {
            ptr
        } else {
            #[cfg(feature = "multithreading")]
            if self.shared.canonical.must_queue(self.thread_id.get()) {
                // The service allocates misses in its long-lived arena before the retry.
                return self.await_canonical_then_retry(
                    dotnet_vm_ops::effects::CanonicalKey::String(chars.to_vec()),
                );
            }
            self.materialize_canonical_string(chars)
        };
        // SAFETY: F1.GcHandleRooted; F6.NoEscapeAcrossArena — the shared table
        // roots this object, and its allocating arena is retained until shutdown.
        let object = ObjectRef(Some(unsafe { gc_arena::Gc::from_ptr(ptr.as_ptr().cast()) }));
        self.push(StackValue::ObjectRef(object));
        StepResult::Continue
    }

    fn push_empty_string(&mut self) {
        // The service installs the empty key before it accepts requests, and without a service
        // (single-threaded, or no finalizer thread) the local arena is the retained home.
        let ptr = self
            .shared
            .interned_strings
            .get(&[][..])
            .map(|entry| *entry)
            .unwrap_or_else(|| self.materialize_canonical_string(&[]));
        // SAFETY: F1.GcHandleRooted; F6.NoEscapeAcrossArena — the shared table
        // roots this object, and its allocating arena is retained until shutdown.
        let object = ObjectRef(Some(unsafe { gc_arena::Gc::from_ptr(ptr.as_ptr().cast()) }));
        self.push(StackValue::ObjectRef(object));
    }

    fn find_interned_string(&self, chars: &[u16]) -> ObjectRef<'gc> {
        let Some(entry) = self.shared.interned_strings.get(chars) else {
            return ObjectRef(None);
        };
        let ptr = *entry;
        // SAFETY: F1.GcHandleRooted; F6.NoEscapeAcrossArena — the table roots the
        // object and its allocating arena is retained until shutdown.
        ObjectRef(Some(unsafe { gc_arena::Gc::from_ptr(ptr.as_ptr().cast()) }))
    }

    fn push_interned_string_object(&mut self, chars: &[u16], object: ObjectRef<'gc>) {
        // Interning an existing object preserves the winning input's identity; its arena must
        // remain registered while the shared table roots it.
        let ptr = *self
            .shared
            .interned_strings
            .entry(chars.to_vec())
            .or_insert(object.as_ptr().expect("interned string is non-null"));
        // SAFETY: F1.GcHandleRooted; F6.NoEscapeAcrossArena — the table roots the
        // object and its allocating arena is retained until shutdown.
        let canonical = ObjectRef(Some(unsafe { gc_arena::Gc::from_ptr(ptr.as_ptr().cast()) }));
        self.push_obj(canonical);
    }
}

impl<'a, 'gc> LocalOps<'gc> for VesContext<'a, 'gc> {
    #[inline]
    fn get_local(&self, index: crate::LocalIndex) -> StackValue<'gc> {
        let frame = self.frame_stack.current_frame();
        if let Some(slot) = frame.homes.local(index.as_usize()) {
            return self.load_home(slot.address, slot.home);
        }
        self.evaluation_stack.get_slot(frame.base.locals + index)
    }

    #[inline]
    fn set_local(
        &mut self,
        index: crate::LocalIndex,
        value: StackValue<'gc>,
    ) -> Result<(), crate::error::VmError> {
        let frame = self.frame_stack.current_frame();
        if let Some(slot) = frame.homes.local(index.as_usize()) {
            return self.store_home(slot.address, slot.home, value);
        }
        self.account_value_type_write(&value);
        let bp = self.frame_stack.current_frame().base;
        self.evaluation_stack.set_slot(bp.locals + index, value);
        Ok(())
    }
}

impl<'a, 'gc> VesContext<'a, 'gc> {
    /// Account only stack-slot writes: frame-home writes do not allocate a stack payload.
    #[cfg(feature = "multithreading")]
    #[inline]
    fn account_value_type_write(&self, value: &StackValue<'gc>) {
        if matches!(value, StackValue::ValueType(..)) {
            let bytes = value.size_bytes();
            Self::record_value_type_write_metrics(bytes);
            self.gc.record_allocation(bytes);
        }
    }

    #[cfg(not(feature = "multithreading"))]
    #[inline]
    fn account_value_type_write(&self, _value: &StackValue<'gc>) {}

    #[cfg(all(feature = "multithreading", feature = "bench-instrumentation"))]
    #[inline]
    fn record_value_type_write_metrics(bytes: usize) {
        dotnet_metrics::record_active_allocation_pressure(
            dotnet_metrics::AllocationPressureSource::ValueTypeWrite,
            bytes,
        );
    }

    #[cfg(all(feature = "multithreading", not(feature = "bench-instrumentation")))]
    #[inline]
    fn record_value_type_write_metrics(_bytes: usize) {}

    /// Installs `homes` in the frame just pushed and copies its homed arguments from their
    /// evaluation-stack slots into the block, so the homes hold the values the caller passed.
    pub(crate) fn install_frame_homes(
        &mut self,
        homes: FrameHomes,
    ) -> Result<(), crate::error::TypeResolutionError> {
        let frame = self.frame_stack.current_frame_mut();
        frame.homes = homes;
        let Some(layout) = frame.homes.layout().cloned() else {
            return Ok(());
        };
        let arguments = frame.base.arguments;
        for (index, home) in layout.homed_arguments() {
            let slot = self
                .frame_stack
                .current_frame()
                .homes
                .argument(index)
                .expect("a homed argument has a home in its frame's block");
            let address = slot.address;
            let value = self
                .evaluation_stack
                .get_slot(arguments + crate::ArgumentIndex::new(index));
            self.store_home(address, home, value).map_err(|error| {
                crate::error::TypeResolutionError::InvalidLayout(
                    format!("argument {index} does not match its parameter type: {error}").into(),
                )
            })?;
        }
        Ok(())
    }

    /// A byref to the homed variable `slot` of the current frame.
    fn home_address(
        &self,
        slot: HomeSlot<'_>,
        pinned: bool,
    ) -> Result<StackValue<'gc>, crate::error::TypeResolutionError> {
        let live_type = if let Some(description) = &slot.home.scalar_type {
            description.clone()
        } else {
            match &slot.home.value_type {
                Some(value_type) => value_type.description.clone(),
                None => {
                    ResolutionOps::stack_value_type(self, &self.load_home(slot.address, slot.home))?
                }
            }
        };
        Ok(StackValue::ManagedPtr(
            ManagedPtr::new_frame_home(
                slot.address,
                live_type,
                slot.serial,
                ByteOffset::new(slot.home.offset),
                pinned,
            )
            .into(),
        ))
    }

    /// Decodes the value of a homed variable from its home bytes.
    fn load_home(&self, address: NonNull<u8>, home: &VariableHome) -> StackValue<'gc> {
        if let Some(value_type) = &home.value_type {
            let size = value_type.fields.total_size;
            let mut data = vec![0u8; size];
            // SAFETY: F10.RawMemoryAccessValid — `address` starts this local's `size`-byte home in
            // the current frame's live block, which `data` does not overlap.
            unsafe { std::ptr::copy_nonoverlapping(address.as_ptr(), data.as_mut_ptr(), size) };
            return StackValue::ValueType(ObjectInstance::new(
                value_type.description.clone(),
                value_type.generics.clone(),
                FieldStorage::new(value_type.fields.clone(), data),
            ));
        }
        let memory = RawMemoryAccess::new(&self.local.heap);
        // SAFETY: F10.RawMemoryAccessValid — `address` starts this local's home in the current
        // frame's live block; the home is `home.layout` bytes long, aligned for it, and holds
        // that layout's encoding (zeroed at allocation, written only by `store_home` and by
        // stores through its address).
        unsafe { memory.read_value_internal(self.gc, address.as_ptr(), None, &home.layout, None) }
            .expect("a local home holds a value of its layout")
    }

    /// Encodes `value` into a homed local, applying the implicit conversions ECMA-335 §III.1.6
    /// allows when storing to a local.
    fn store_home(
        &self,
        address: NonNull<u8>,
        home: &VariableHome,
        value: StackValue<'gc>,
    ) -> Result<(), crate::error::VmError> {
        let layout = &*home.layout;
        let value = if let (Some(expected), StackValue::ValueType(object)) =
            (&home.scalar_type, &value)
        {
            if &object.description != expected
                || object.instance_storage.layout().total_size != layout.size().as_usize()
            {
                return Err(MemoryAccessError::TypeMismatch(
                    "stored scalar value type does not match its home".into(),
                )
                .into());
            }
            let memory = RawMemoryAccess::new(&self.local.heap);
            object.with_data(|data| {
                if data.len() != layout.size().as_usize() {
                    return Err(MemoryAccessError::TypeMismatch(
                        "stored scalar value type has an invalid byte length".into(),
                    ));
                }
                // SAFETY: F10.RawMemoryAccessValid — the matching scalar value type owns
                // initialized bytes of the checked layout and buffer size. The read stays in
                // the borrow.
                unsafe { memory.read_value_internal(self.gc, data.as_ptr(), None, layout, None) }
            })?
        } else {
            value
        };
        let value = match (layout, value) {
            (LayoutManager::Scalar(Scalar::NativeInt), StackValue::Int32(value)) => {
                StackValue::NativeInt(value as isize)
            }
            (LayoutManager::Scalar(Scalar::NativeInt), StackValue::ManagedPtr(pointer)) => {
                let address = pointer
                    .try_observe_address()
                    .map_err(|_| MemoryAccessError::InvalidOrigin)?;
                StackValue::NativeInt(
                    address.map_or(0, |address| address.as_ptr().expose_provenance()) as isize,
                )
            }
            (
                LayoutManager::Scalar(
                    Scalar::Int8 | Scalar::UInt8 | Scalar::Int16 | Scalar::UInt16 | Scalar::Int32,
                ),
                StackValue::NativeInt(value),
            ) => StackValue::Int32(value as i32),
            (LayoutManager::Field(fields), StackValue::ValueType(value))
                if value.instance_storage.layout().total_size != fields.total_size =>
            {
                return Err(MemoryAccessError::TypeMismatch(
                    "stored value type does not match the local's layout".into(),
                )
                .into());
            }
            (_, value) => value,
        };
        let mut memory = RawMemoryAccess::new(&self.local.heap);
        // SAFETY: F10.RawMemoryAccessValid — `address` starts this local's home in the current
        // frame's live block, which is `layout` bytes long and aligned for it; the write encodes
        // a value of that layout (value-type sizes are checked above).
        unsafe { memory.write_to_unmanaged(self.gc, address.as_ptr(), value, layout) }
            .map_err(Into::into)
    }
}

impl<'a, 'gc> ArgumentOps<'gc> for VesContext<'a, 'gc> {
    #[inline]
    fn get_argument(&self, index: crate::ArgumentIndex) -> StackValue<'gc> {
        let frame = self.frame_stack.current_frame();
        if let Some(slot) = frame.homes.argument(index.as_usize()) {
            return self.load_home(slot.address, slot.home);
        }
        self.evaluation_stack.get_slot(frame.base.arguments + index)
    }

    #[inline]
    fn set_argument(
        &mut self,
        index: crate::ArgumentIndex,
        value: StackValue<'gc>,
    ) -> Result<(), crate::error::VmError> {
        let frame = self.frame_stack.current_frame();
        if let Some(slot) = frame.homes.argument(index.as_usize()) {
            return self.store_home(slot.address, slot.home, value);
        }
        self.account_value_type_write(&value);
        let bp = self.frame_stack.current_frame().base;
        self.evaluation_stack.set_slot(bp.arguments + index, value);
        Ok(())
    }
}

impl<'a, 'gc> VmStackOps<'gc> for VesContext<'a, 'gc> {
    #[inline]
    fn current_frame(&self) -> &crate::stack::StackFrame<'gc> {
        self.frame_stack.current_frame()
    }

    #[inline]
    fn current_frame_mut(&mut self) -> &mut crate::stack::StackFrame<'gc> {
        self.frame_stack.current_frame_mut()
    }

    #[inline]
    fn get_slot(&self, index: crate::StackSlotIndex) -> StackValue<'gc> {
        self.evaluation_stack.get_slot(index)
    }

    #[inline]
    fn get_slot_ref(&self, index: crate::StackSlotIndex) -> &StackValue<'gc> {
        self.evaluation_stack.get_slot_ref(index)
    }

    #[inline]
    fn set_slot(&mut self, index: crate::StackSlotIndex, value: StackValue<'gc>) {
        self.account_value_type_write(&value);
        self.evaluation_stack.set_slot(index, value)
    }

    fn local_address(
        &self,
        index: crate::LocalIndex,
    ) -> Result<StackValue<'gc>, crate::error::TypeResolutionError> {
        let frame = self.frame_stack.current_frame();
        let is_pinned = frame
            .pinned_locals
            .get(index.as_usize())
            .copied()
            .unwrap_or(false);
        if let Some(slot) = frame.homes.local(index.as_usize()) {
            return self.home_address(slot, is_pinned);
        }
        Err(crate::error::TypeResolutionError::InvalidLayout(
            "address-taking requires a supported frame home; TypedReference addresses are not supported"
                .into(),
        ))
    }

    fn argument_address(
        &self,
        index: crate::ArgumentIndex,
    ) -> Result<StackValue<'gc>, crate::error::TypeResolutionError> {
        let frame = self.frame_stack.current_frame();
        if let Some(slot) = frame.homes.argument(index.as_usize()) {
            return self.home_address(slot, true);
        }
        Err(crate::error::TypeResolutionError::InvalidLayout(
            "address-taking requires a supported frame home; TypedReference addresses are not supported"
                .into(),
        ))
    }
}

#[cfg(test)]
mod scalar_home_audit_tests {
    use super::*;
    use crate::{
        state::SharedGlobalState,
        sync::Arc,
        test_utils::{new_test_arena, test_gc_handle},
    };
    use dotnet_assemblies::AssemblyLoader;
    use dotnet_types::TypeDescription;
    use dotnet_value::layout::{FieldLayoutManager, GcDesc};
    use dotnetdll::prelude::*;

    #[allow(
        clippy::arc_with_non_send_sync,
        reason = "single-threaded test uses production shared-state types"
    )]
    #[test]
    fn scalar_home_rejects_wrong_identity_and_layout_size() {
        let loader = Arc::new(AssemblyLoader::new_bare("scalar_home_audit".into()).unwrap());
        let mut res = Resolution::new(Module::new("ScalarAudit.dll"));
        let expected_index = res.push_type_definition(TypeDefinition::new(None, "Scalar"));
        let wrong_index = res.push_type_definition(TypeDefinition::new(None, "OtherScalar"));
        let res = loader.register_owned_assembly(res);
        let expected = TypeDescription::new(res.clone(), expected_index);
        let wrong = TypeDescription::new(res, wrong_index);
        let shared = Arc::new(SharedGlobalState::new(loader));
        let mut arena = new_test_arena(&shared);
        arena.mutate_root(|gc, engine| {
            // SAFETY: F1.GcHandleRooted — engine remains the live root throughout this mutation.
            let gc = unsafe { test_gc_handle(gc, engine) };
            let ctx = engine.ves_context(gc);
            let home = VariableHome {
                offset: 0,
                layout: Arc::new(LayoutManager::Scalar(Scalar::Int32)),
                scalar_type: Some(expected.clone()),
                value_type: None,
            };
            for (description, wrong_size) in [(wrong, false), (expected.clone(), true)] {
                let size = if wrong_size { 8 } else { 4 };
                let layout = Arc::new(FieldLayoutManager {
                    fields: Default::default(),
                    total_size: size,
                    alignment: 4,
                    gc_desc: GcDesc::default(),
                    has_ref_fields: false,
                });
                let value = ObjectInstance::new(
                    description,
                    Default::default(),
                    FieldStorage::new(layout, vec![0; size]),
                );
                let mut destination = 0x12345678i32;
                assert!(
                    ctx.store_home(
                        NonNull::from(&mut destination).cast(),
                        &home,
                        StackValue::ValueType(value)
                    )
                    .is_err()
                );
                assert_eq!(destination, 0x12345678);
            }
        });
    }

    #[allow(
        clippy::arc_with_non_send_sync,
        reason = "single-threaded test uses production shared-state types"
    )]
    #[test]
    fn scalar_home_rejects_short_payload() {
        let loader = Arc::new(AssemblyLoader::new_bare("scalar_home_short".into()).unwrap());
        let mut res = Resolution::new(Module::new("ScalarShort.dll"));
        let index = res.push_type_definition(TypeDefinition::new(None, "Scalar"));
        let description = TypeDescription::new(loader.register_owned_assembly(res), index);
        let shared = Arc::new(SharedGlobalState::new(loader));
        let mut arena = new_test_arena(&shared);
        arena.mutate_root(|gc, engine| {
            // SAFETY: F1.GcHandleRooted — engine remains the live root throughout this mutation.
            let gc = unsafe { test_gc_handle(gc, engine) };
            let ctx = engine.ves_context(gc);
            let home = VariableHome {
                offset: 0,
                layout: Arc::new(LayoutManager::Scalar(Scalar::Int32)),
                scalar_type: Some(description.clone()),
                value_type: None,
            };
            for length in [0, 1, 2, 3, 5, 8] {
                let layout = Arc::new(FieldLayoutManager {
                    fields: Default::default(),
                    total_size: 4,
                    alignment: 4,
                    gc_desc: GcDesc::default(),
                    has_ref_fields: false,
                });
                let value = ObjectInstance::new(
                    description.clone(),
                    Default::default(),
                    FieldStorage::new(layout, vec![0; length]),
                );
                let mut destination = 0x12345678i32;
                assert!(
                    ctx.store_home(
                        NonNull::from(&mut destination).cast(),
                        &home,
                        StackValue::ValueType(value),
                    )
                    .is_err()
                );
                assert_eq!(destination, 0x12345678);
            }
        });
    }
}

use crate::{
    MethodInfo, MethodType, ResolutionContext, StepResult,
    resolver::VmResolverService,
    stack::{
        context::VesContext,
        ops::{
            EvalStackOps, IntrinsicDispatchOps, LoaderOps, ReflectionLookupOps, ReflectionOps,
            ThreadOps, VmCallOps, VmLoaderOps, VmReflectionOps, VmResolutionOps, VmStaticsOps,
        },
    },
    state::{ReflectionRegistry, SharedGlobalState, StaticStorageManager},
    sync::Arc,
};
use dotnet_types::{
    TypeDescription,
    error::{ExecutionError, TypeResolutionError},
    generics::{ConcreteType, GenericLookup},
    members::{FieldDescription, MethodDescription},
    resolution::ResolutionS,
    runtime::RuntimeType,
};
use dotnet_utils::ArenaId;
use dotnet_value::{
    StackValue,
    object::{Object, ObjectRef},
};
use dotnetdll::prelude::{FieldSource, MethodType as DllMethodType, UserType};

#[cfg(feature = "multithreading")]
use dashmap::{DashMap, mapref::entry::Entry};
#[cfg(feature = "multithreading")]
use dotnet_metrics::{CacheEvent, CacheKind};
#[cfg(feature = "multithreading")]
use dotnet_utils::sync::{AtomicUsize, Ordering};

#[cfg(feature = "multithreading")]
fn shared_reflection_index_get_or_insert<K, V, H, M>(
    map: &DashMap<K, usize>,
    rev_map: &DashMap<usize, V>,
    next_index: &AtomicUsize,
    key: K,
    value: V,
    on_hit: H,
    on_miss: M,
) -> usize
where
    K: Eq + std::hash::Hash,
    H: FnOnce(),
    M: FnOnce(),
{
    match map.entry(key) {
        Entry::Occupied(entry) => {
            on_hit();
            *entry.get()
        }
        Entry::Vacant(entry) => {
            on_miss();
            let idx = next_index.fetch_add(1, Ordering::Relaxed);
            rev_map.insert(idx, value);
            entry.insert(idx);
            idx
        }
    }
}

#[cfg(feature = "multithreading")]
fn shared_reflection_by_index<V: Clone>(
    rev_map: &DashMap<usize, V>,
    index: usize,
    invalid_index_message: &'static str,
) -> V {
    rev_map
        .get(&index)
        .map(|entry| entry.clone())
        .expect(invalid_index_message)
}

#[cfg(not(feature = "multithreading"))]
fn local_reflection_index_get_or_insert<V: PartialEq>(list: &mut Vec<V>, value: V) -> usize {
    if let Some(existing) = list.iter().position(|entry| entry == &value) {
        existing
    } else {
        list.push(value);
        list.len() - 1
    }
}

#[cfg(not(feature = "multithreading"))]
fn local_reflection_by_index<V: Clone>(list: &[V], index: usize) -> V {
    list[index].clone()
}

impl<'a> dotnet_intrinsics_reflection::RuntimeTypeContext for ResolutionContext<'a> {
    fn reflection_locate_type(
        &self,
        handle: UserType,
    ) -> Result<TypeDescription, TypeResolutionError> {
        self.locate_type(handle)
    }

    fn reflection_type_owner(&self) -> Option<TypeDescription> {
        self.type_owner.clone()
    }

    fn reflection_method_owner(&self) -> Option<MethodDescription> {
        self.method_owner.clone()
    }
}

impl<'a, 'gc> VmLoaderOps for VesContext<'a, 'gc> {
    #[inline]
    fn resolver(&self) -> VmResolverService {
        VmResolverService::new(self.shared.clone())
    }

    #[inline]
    fn shared(&self) -> &Arc<SharedGlobalState> {
        self.shared
    }
}

impl<'a, 'gc> VmStaticsOps<'gc> for VesContext<'a, 'gc> {
    #[inline]
    fn statics(&self) -> &StaticStorageManager {
        &self.shared.statics
    }

    fn start_static_field(&mut self, action: crate::stack::ops::StaticFieldAction) -> StepResult {
        self.start_static_field_effect(action)
    }
}

impl<'a, 'gc> ThreadOps for VesContext<'a, 'gc> {
    fn thread_id(&self) -> ArenaId {
        self.thread_id.get()
    }

    fn managed_thread_id(&self) -> i32 {
        self.local.managed_threads.current_id()
    }
}

impl<'a, 'gc> IntrinsicDispatchOps<'gc> for VesContext<'a, 'gc> {
    #[inline]
    fn is_intrinsic_field_cached(&self, field: FieldDescription) -> bool {
        self.resolver().is_intrinsic_field_cached(field)
    }

    #[inline]
    fn is_intrinsic_cached(&self, method: MethodDescription) -> bool {
        self.resolver().is_intrinsic_cached(method)
    }

    #[inline]
    fn execute_intrinsic_field(
        &mut self,
        field: FieldDescription,
        type_generics: Arc<[ConcreteType]>,
        is_address: bool,
    ) -> StepResult {
        crate::intrinsics::intrinsic_field(self, field, type_generics, is_address)
    }

    #[inline]
    fn execute_intrinsic_call(
        &mut self,
        method: MethodDescription,
        lookup: &GenericLookup,
    ) -> StepResult {
        crate::intrinsics::intrinsic_call(self, method, lookup)
    }
}

impl<'gc> VesContext<'_, 'gc> {
    pub(crate) fn retry_reflection_target(&mut self) -> StepResult {
        let Some(pending) = self
            .frame_stack
            .current_frame_mut()
            .reflection_pending
            .take()
        else {
            return StepResult::internal_error("missing pending reflection target");
        };
        if self.top_of_stack() < pending.base {
            return StepResult::internal_error("reflection argument base changed");
        }
        while self.top_of_stack() > pending.base {
            self.pop();
        }
        for argument in &pending.arguments {
            self.push(argument.clone());
        }
        let depth = self.frame_stack.len();
        let saved_ip = *self.original_ip;
        let saved_height = *self.original_stack_height;
        let ip_before = self.frame_stack.state().ip;
        *self.original_ip = ip_before;
        *self.original_stack_height = self.top_of_stack();
        let actions = std::mem::take(&mut self.frame_stack.current_frame_mut().frame_continuation);
        let arm_base = self.call_state.reflection_arming.len();
        self.call_state
            .reflection_arming
            .extend(actions.into_iter().rev().map(Some));
        let result = self.dispatch_method(pending.method.clone(), pending.generics.clone());
        let actions = self
            .call_state
            .reflection_arming
            .drain(arm_base..)
            .rev()
            .flatten()
            .collect::<Vec<_>>();
        self.frame_stack.frames[depth - 1]
            .frame_continuation
            .extend(actions);
        *self.original_ip = saved_ip;
        *self.original_stack_height = saved_height;
        let initializer = matches!(result, StepResult::FramePushed)
            && self.frame_stack.len() == depth + 1
            && self.frame_stack.current_frame().state.info_handle.is_cctor
            && self.frame_stack.current_frame().state.info_handle.source != pending.method;
        // A synchronous target that advanced this frame's IP and then yielded (`GC.Collect`)
        // has completed; only a yield that left the IP alone asks for a retry.
        let completed_yield = matches!(result, StepResult::Yield)
            && self.frame_stack.len() == depth
            && self.frame_stack.state().ip != ip_before;
        if (initializer || (matches!(result, StepResult::Yield) && !completed_yield))
            && self.frame_stack.frames[depth - 1]
                .reflection_pending
                .is_none()
        {
            self.frame_stack.frames[depth - 1].reflection_pending = Some(pending);
        }
        match result {
            StepResult::Continue | StepResult::Jump(_) => {
                let actions =
                    std::mem::take(&mut self.frame_stack.current_frame_mut().frame_continuation);
                match self.apply_frame_return_actions(actions) {
                    Ok(()) => result,
                    Err(error) => StepResult::Error(error),
                }
            }
            StepResult::Yield if completed_yield => {
                let actions =
                    std::mem::take(&mut self.frame_stack.current_frame_mut().frame_continuation);
                match self.apply_frame_return_actions(actions) {
                    Ok(()) => result,
                    Err(error) => StepResult::Error(error),
                }
            }
            StepResult::FramePushed | StepResult::NativeSuspended(_) | StepResult::Yield => result,
            _ => {
                self.frame_stack.frames[depth - 1]
                    .frame_continuation
                    .clear();
                self.frame_stack.frames[depth - 1].reflection_pending = None;
                result
            }
        }
    }
}

impl<'a, 'gc> dotnet_intrinsics_reflection::ResolutionContextHost<'gc> for VesContext<'a, 'gc> {
    fn reflection_make_runtime_type_with_lookup(
        &self,
        source: &DllMethodType,
        lookup: &GenericLookup,
    ) -> RuntimeType {
        let ctx = self.current_context().with_generics(lookup);
        dotnet_intrinsics_reflection::common::make_runtime_type(&ctx, source)
            .expect("failed to build runtime type")
    }

    fn reflection_make_runtime_type_for_method(
        &self,
        method: MethodDescription,
        lookup: &GenericLookup,
        source: &DllMethodType,
    ) -> RuntimeType {
        let ctx = ResolutionContext::for_method(
            method,
            self.loader_arc(),
            lookup,
            self.shared().caches.clone(),
            Some(Arc::downgrade(self.shared())),
        );

        if let Ok(concrete) = ctx.make_concrete(source)
            && let Some(rt) =
                dotnet_types::runtime::runtime_type_from_concrete(self.loader().as_ref(), &concrete)
        {
            return rt;
        }

        dotnet_intrinsics_reflection::common::make_runtime_type(&ctx, source)
            .expect("failed to build runtime type")
    }

    fn reflection_is_value_type_with_lookup(
        &self,
        td: TypeDescription,
        lookup: &GenericLookup,
    ) -> Result<bool, TypeResolutionError> {
        let ctx = self.current_context().with_generics(lookup);
        ctx.resolver().is_value_type(td)
    }

    fn reflection_new_object_with_lookup(
        &self,
        td: TypeDescription,
        lookup: &GenericLookup,
    ) -> Result<Object<'gc>, TypeResolutionError> {
        let ctx = self.current_context().with_generics(lookup);
        ctx.resolver().new_object(td, &ctx)
    }

    fn reflection_method_info(
        &self,
        method: MethodDescription,
        lookup: &GenericLookup,
    ) -> Result<MethodInfo<'static>, TypeResolutionError> {
        self.shared()
            .caches
            .get_method_info(method, lookup, self.shared().clone())
    }

    fn reflection_empty_generics(&self) -> GenericLookup {
        self.shared().empty_generics.clone()
    }

    fn reflection_dispatch_method(
        &mut self,
        method: MethodDescription,
        mut lookup: GenericLookup,
    ) -> StepResult {
        // The continuation belongs to this dispatch, including nested synchronous dispatches.
        let action = self
            .frame_stack
            .current_frame_mut()
            .frame_continuation
            .pop();
        tracing::debug!(reflection_invoke_path = "fallback");
        let mut dispatch_method = method;

        if dispatch_method.signature().instance && dispatch_method.method().virtual_member {
            let num_args = 1 + dispatch_method.signature().parameters.len();
            if let StackValue::ObjectRef(this_obj) = self.peek_stack_at(num_args - 1)
                && this_obj.0.is_some()
            {
                let this_type = dotnet_vm_ops::vm_try!(self.get_heap_description(this_obj));
                dispatch_method = dotnet_vm_ops::vm_try!(self.resolver().resolve_virtual_method(
                    dispatch_method,
                    this_type,
                    &lookup,
                    &self.current_context(),
                ));

                let parent_arity = dispatch_method.parent.definition().generic_parameters.len();
                if parent_arity == 0 {
                    lookup.set_type_generics(Vec::new().into());
                } else if dispatch_method.parent_generics.type_generics.len() == parent_arity {
                    lookup.set_type_generics(dispatch_method.parent_generics.type_generics.clone());
                } else {
                    let type_generics = lookup
                        .type_generics
                        .iter()
                        .take(parent_arity)
                        .cloned()
                        .collect::<Vec<_>>()
                        .into();
                    lookup.set_type_generics(type_generics);
                }
            }
        }

        let depth = self.frame_stack.len();
        let count = dispatch_method.signature().parameters.len()
            + usize::from(dispatch_method.signature().instance);
        let Some(base) = self.top_of_stack().checked_sub(count) else {
            return StepResult::internal_error("reflected target argument underflow");
        };
        let arguments = (0..count)
            .rev()
            .map(|offset| self.peek_stack_at(offset))
            .collect();
        let saved_ip = *self.original_ip;
        let saved_height = *self.original_stack_height;
        let ip_before = self.frame_stack.state().ip;
        *self.original_ip = ip_before;
        *self.original_stack_height = self.top_of_stack();
        self.call_state.reflection_arming.push(action);
        let result = self.dispatch_method(dispatch_method.clone(), lookup.clone());
        let action = self
            .call_state
            .reflection_arming
            .pop()
            .expect("reflection dispatch scope");
        *self.original_ip = saved_ip;
        *self.original_stack_height = saved_height;
        let initializer = matches!(result, StepResult::FramePushed)
            && self.frame_stack.len() == depth + 1
            && self.frame_stack.current_frame().state.info_handle.is_cctor
            && self.frame_stack.current_frame().state.info_handle.source != dispatch_method;
        // A synchronous target that advanced this frame's IP and then yielded (`GC.Collect`)
        // has completed; only a yield that left the IP alone asks for a retry.
        let completed_yield = matches!(result, StepResult::Yield)
            && self.frame_stack.len() == depth
            && self.frame_stack.state().ip != ip_before;
        let caller = &mut self.frame_stack.frames[depth - 1];
        match result {
            StepResult::Yield if completed_yield => {
                match self.apply_frame_return_actions(action.into_iter().collect()) {
                    Ok(()) => result,
                    Err(error) => StepResult::Error(error),
                }
            }
            StepResult::FramePushed | StepResult::NativeSuspended(_) | StepResult::Yield => {
                if let Some(action) = action {
                    caller.frame_continuation.push(action);
                }
                if (initializer || matches!(result, StepResult::Yield))
                    && !self.has_staged_native_call(depth)
                    && self
                        .call_state
                        .native
                        .last()
                        .is_none_or(|work| work.caller_depth != depth)
                    && self.frame_stack.frames[depth - 1]
                        .reflection_pending
                        .is_none()
                {
                    tracing::debug!(reflection_invoke_path = "retry");
                    self.frame_stack.frames[depth - 1].reflection_pending =
                        Some(Box::new(dotnet_vm_data::stack::ReflectionPending {
                            method: dispatch_method,
                            generics: lookup,
                            arguments,
                            base,
                        }));
                }
                result
            }
            StepResult::Continue | StepResult::Jump(_) => {
                match self.apply_frame_return_actions(action.into_iter().collect()) {
                    Ok(()) => result,
                    Err(error) => StepResult::Error(error),
                }
            }
            _ => result,
        }
    }

    fn set_frame_return_action(&mut self, action: dotnet_vm_data::FrameReturnAction) {
        VesContext::set_frame_return_action(self, action);
    }

    fn reflection_constructor_frame(
        &mut self,
        instance: Object<'gc>,
        method: MethodInfo<'static>,
        generic_inst: GenericLookup,
    ) -> Result<(), TypeResolutionError> {
        self.constructor_frame(instance, method, generic_inst)
    }
}

impl<'a, 'gc> dotnet_intrinsics_reflection::ReflectionRegistryHost<'gc> for VesContext<'a, 'gc> {
    fn reflection_cached_runtime_assembly(
        &self,
        resolution: ResolutionS,
    ) -> Option<ObjectRef<'gc>> {
        self.reflection().asms_read().get(&resolution).copied()
    }

    fn reflection_cache_runtime_assembly(&self, resolution: ResolutionS, object: ObjectRef<'gc>) {
        self.reflection()
            .asms_write()
            .insert(resolution.clone(), object);
        self.reflection()
            .asm_resolutions_write()
            .insert(object, resolution);
    }

    fn reflection_runtime_assembly_resolution(
        &self,
        object: ObjectRef<'gc>,
    ) -> Option<ResolutionS> {
        self.reflection()
            .asm_resolutions_read()
            .get(&object)
            .cloned()
    }

    fn reflection_cached_runtime_type(&self, target: &RuntimeType) -> Option<ObjectRef<'gc>> {
        if let Some(object) = self.reflection().types_read().get(target).copied() {
            return Some(object);
        }
        #[cfg(feature = "multithreading")]
        {
            let ptr = *self
                .shared()
                .reflection_registry
                .runtime_type_objects
                .get(target)?;
            // SAFETY: F1.GcHandleRooted; F6.NoEscapeAcrossArena — the shared table
            // roots the original object and its owner arena is retained until shutdown.
            let object = ObjectRef(Some(unsafe { gc_arena::Gc::from_ptr(ptr.as_ptr().cast()) }));
            self.reflection()
                .types_write()
                .insert(target.clone(), object);
            Some(object)
        }
        #[cfg(not(feature = "multithreading"))]
        None
    }

    fn reflection_cache_runtime_type(
        &self,
        target: RuntimeType,
        object: ObjectRef<'gc>,
    ) -> ObjectRef<'gc> {
        #[cfg(feature = "multithreading")]
        let object = {
            let ptr = *self
                .shared()
                .reflection_registry
                .runtime_type_objects
                .entry(target.clone())
                .or_insert(object.as_ptr().expect("type object is non-null"));
            // SAFETY: F1.GcHandleRooted; F6.NoEscapeAcrossArena — the published
            // winner is rooted by the table and its arena is retained until shutdown.
            ObjectRef(Some(unsafe { gc_arena::Gc::from_ptr(ptr.as_ptr().cast()) }))
        };
        self.reflection().types_write().insert(target, object);
        object
    }

    fn reflection_runtime_type_index_get_or_insert(&self, target: RuntimeType) -> usize {
        #[cfg(feature = "multithreading")]
        {
            let shared = self.shared();
            let registry = &shared.reflection_registry;
            match registry.runtime_types.entry(target.clone()) {
                Entry::Occupied(entry) => {
                    shared
                        .metrics
                        .record_cache(CacheKind::SharedRuntimeTypes, CacheEvent::Hit);
                    *entry.get()
                }
                Entry::Vacant(entry) => {
                    shared
                        .metrics
                        .record_cache(CacheKind::SharedRuntimeTypes, CacheEvent::Miss);
                    let idx = registry.next_type_index.fetch_add(1, Ordering::Relaxed);
                    registry.runtime_types_rev.insert(idx, target);
                    entry.insert(idx);
                    idx
                }
            }
        }

        #[cfg(not(feature = "multithreading"))]
        {
            let mut list = self.reflection().types_list_write();
            local_reflection_index_get_or_insert(&mut list, target)
        }
    }

    fn reflection_runtime_type_by_index(&self, index: usize) -> RuntimeType {
        #[cfg(feature = "multithreading")]
        {
            self.shared()
                .reflection_registry
                .runtime_types_rev
                .get(&index)
                .map(|e| e.clone())
                .expect("invalid runtime type index")
        }

        #[cfg(not(feature = "multithreading"))]
        {
            self.reflection().types_list_read()[index].clone()
        }
    }

    fn reflection_runtime_method_index_get_or_insert(
        &self,
        method: MethodDescription,
        lookup: GenericLookup,
    ) -> usize {
        #[cfg(feature = "multithreading")]
        {
            let shared = self.shared();
            let registry = &shared.reflection_registry;
            let key = (method.clone(), lookup.clone());
            let value = (method, lookup);
            shared_reflection_index_get_or_insert(
                &registry.runtime_methods,
                &registry.runtime_methods_rev,
                &registry.next_method_index,
                key,
                value,
                || {
                    shared
                        .metrics
                        .record_cache(CacheKind::SharedRuntimeMethods, CacheEvent::Hit)
                },
                || {
                    shared
                        .metrics
                        .record_cache(CacheKind::SharedRuntimeMethods, CacheEvent::Miss)
                },
            )
        }

        #[cfg(not(feature = "multithreading"))]
        {
            let mut methods = self.reflection().methods_write();
            local_reflection_index_get_or_insert(&mut methods, (method, lookup))
        }
    }

    fn reflection_runtime_method_by_index(
        &self,
        index: usize,
    ) -> (MethodDescription, GenericLookup) {
        #[cfg(feature = "multithreading")]
        {
            let shared = self.shared();
            shared_reflection_by_index(
                &shared.reflection_registry.runtime_methods_rev,
                index,
                "invalid runtime method index",
            )
        }

        #[cfg(not(feature = "multithreading"))]
        {
            let methods = self.reflection().methods_read();
            local_reflection_by_index(&methods, index)
        }
    }

    fn reflection_runtime_field_index_get_or_insert(
        &self,
        field: FieldDescription,
        lookup: GenericLookup,
    ) -> usize {
        #[cfg(feature = "multithreading")]
        {
            let shared = self.shared();
            let registry = &shared.reflection_registry;
            let key = (field.clone(), lookup.clone());
            let value = (field, lookup);
            shared_reflection_index_get_or_insert(
                &registry.runtime_fields,
                &registry.runtime_fields_rev,
                &registry.next_field_index,
                key,
                value,
                || {
                    shared
                        .metrics
                        .record_cache(CacheKind::SharedRuntimeFields, CacheEvent::Hit)
                },
                || {
                    shared
                        .metrics
                        .record_cache(CacheKind::SharedRuntimeFields, CacheEvent::Miss)
                },
            )
        }

        #[cfg(not(feature = "multithreading"))]
        {
            let mut fields = self.reflection().fields_write();
            local_reflection_index_get_or_insert(&mut fields, (field, lookup))
        }
    }

    fn reflection_runtime_field_by_index(&self, index: usize) -> (FieldDescription, GenericLookup) {
        #[cfg(feature = "multithreading")]
        {
            let shared = self.shared();
            shared_reflection_by_index(
                &shared.reflection_registry.runtime_fields_rev,
                index,
                "invalid runtime field index",
            )
        }

        #[cfg(not(feature = "multithreading"))]
        {
            let fields = self.reflection().fields_read();
            local_reflection_by_index(&fields, index)
        }
    }

    fn reflection_cached_runtime_method_obj(
        &self,
        method: &MethodDescription,
        lookup: &GenericLookup,
    ) -> Option<ObjectRef<'gc>> {
        self.reflection()
            .method_objs_read()
            .get(&(method.clone(), lookup.clone()))
            .copied()
    }

    fn reflection_cache_runtime_method_obj(
        &self,
        method: MethodDescription,
        lookup: GenericLookup,
        object: ObjectRef<'gc>,
    ) {
        self.reflection()
            .method_objs_write()
            .insert((method, lookup), object);
    }

    fn reflection_cached_runtime_field_obj(
        &self,
        field: &FieldDescription,
        lookup: &GenericLookup,
    ) -> Option<ObjectRef<'gc>> {
        self.reflection()
            .field_objs_read()
            .get(&(field.clone(), lookup.clone()))
            .copied()
    }

    fn reflection_cache_runtime_field_obj(
        &self,
        field: FieldDescription,
        lookup: GenericLookup,
        object: ObjectRef<'gc>,
    ) {
        self.reflection()
            .field_objs_write()
            .insert((field, lookup), object);
    }

    fn reflection_cached_runtime_property_obj(
        &self,
        accessor: &MethodDescription,
        lookup: &GenericLookup,
    ) -> Option<ObjectRef<'gc>> {
        self.reflection()
            .property_objs_read()
            .get(&(accessor.clone(), lookup.clone()))
            .copied()
    }

    fn reflection_cache_runtime_property_obj(
        &self,
        accessor: MethodDescription,
        lookup: GenericLookup,
        object: ObjectRef<'gc>,
    ) {
        self.reflection()
            .property_objs_write()
            .insert((accessor, lookup), object);
    }
}

impl<'a, 'gc> ReflectionLookupOps<'gc> for VesContext<'a, 'gc> {
    #[inline]
    fn get_runtime_method_index(
        &mut self,
        method: MethodDescription,
        lookup: GenericLookup,
    ) -> usize {
        dotnet_intrinsics_reflection::common::get_runtime_method_index(self, method, lookup)
            as usize
    }

    #[inline]
    fn get_runtime_type(&mut self, target: RuntimeType) -> ObjectRef<'gc> {
        dotnet_intrinsics_reflection::common::get_runtime_type(self, target)
    }

    #[inline]
    fn get_runtime_method_obj(
        &mut self,
        method: MethodDescription,
        lookup: GenericLookup,
    ) -> ObjectRef<'gc> {
        dotnet_intrinsics_reflection::common::get_runtime_method_obj(self, method, lookup)
    }

    #[inline]
    fn get_runtime_field_obj(
        &mut self,
        field: FieldDescription,
        lookup: GenericLookup,
    ) -> ObjectRef<'gc> {
        dotnet_intrinsics_reflection::common::get_runtime_field_obj(self, field, lookup)
    }
}

impl<'a, 'gc> VmReflectionOps<'gc> for VesContext<'a, 'gc> {
    #[inline]
    fn pre_initialize_reflection(&mut self) {
        dotnet_intrinsics_reflection::common::pre_initialize_reflection(self)
    }

    fn make_runtime_type(&self, ctx: &ResolutionContext<'_>, source: &MethodType) -> RuntimeType {
        // Resolve generic type/method parameters through the current lookup first.
        // Without this, `typeof(T)` inside a generic method returns an unresolved
        // TypeParameter that `to_concrete` maps to Object, breaking IsValueType etc.
        if let Ok(concrete) = ctx.generics.make_concrete(
            ctx.resolution.clone(),
            source.clone(),
            self.loader().as_ref(),
        ) && let Some(rt) =
            dotnet_types::runtime::runtime_type_from_concrete(self.loader().as_ref(), &concrete)
        {
            return rt;
        }
        dotnet_intrinsics_reflection::common::make_runtime_type(ctx, source)
            .expect("failed to build runtime type")
    }

    #[inline]
    fn locate_field(
        &self,
        handle: FieldSource,
    ) -> Result<(FieldDescription, GenericLookup), TypeResolutionError> {
        let context = self.current_context();
        self.resolver()
            .locate_field(context.resolution, handle, context.generics)
    }

    #[inline]
    fn resolve_runtime_type(&self, obj: ObjectRef<'gc>) -> Result<RuntimeType, ExecutionError> {
        dotnet_intrinsics_reflection::common::resolve_runtime_type(self, obj)
    }

    #[inline]
    fn resolve_runtime_method(
        &self,
        obj: ObjectRef<'gc>,
    ) -> Result<(MethodDescription, GenericLookup), ExecutionError> {
        dotnet_intrinsics_reflection::common::resolve_runtime_method(self, obj)
    }

    #[inline]
    fn resolve_runtime_field(
        &self,
        obj: ObjectRef<'gc>,
    ) -> Result<(FieldDescription, GenericLookup), ExecutionError> {
        dotnet_intrinsics_reflection::common::resolve_runtime_field(self, obj)
    }

    #[inline]
    fn lookup_method_by_index(&self, index: usize) -> (MethodDescription, GenericLookup) {
        <Self as dotnet_intrinsics_reflection::ReflectionRegistryHost<'gc>>::reflection_runtime_method_by_index(
            self, index,
        )
    }

    #[inline]
    fn reflection(&self) -> ReflectionRegistry<'_, 'gc> {
        ReflectionRegistry::new(&self.local.reflection)
    }
}

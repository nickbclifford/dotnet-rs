use crate::{
    MethodInfo, MethodType, ResolutionContext, StepResult,
    layout::type_layout,
    resolver::VmResolverService,
    stack::{
        context::VesContext,
        ops::{
            IntrinsicDispatchOps, LoaderOps, ReflectionLookupOps, ThreadOps, VmCallOps,
            VmLoaderOps, VmReflectionOps, VmResolutionOps, VmStaticsOps,
        },
    },
    state::{ReflectionRegistry, SharedGlobalState, StaticStorageManager},
    sync::Arc,
};
use dotnet_types::{
    TypeDescription,
    error::TypeResolutionError,
    generics::{ConcreteType, GenericLookup},
    members::{FieldDescription, MethodDescription},
    resolution::ResolutionS,
    runtime::RuntimeType,
};
use dotnet_utils::ArenaId;
use dotnet_value::object::{Object, ObjectRef};
use dotnetdll::prelude::{FieldSource, MethodType as DllMethodType, UserType};

#[cfg(feature = "multithreading")]
use dotnet_utils::sync::Ordering;

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
}

impl<'a, 'gc> ThreadOps for VesContext<'a, 'gc> {
    fn thread_id(&self) -> ArenaId {
        self.thread_id.get()
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
        lookup: GenericLookup,
    ) -> StepResult {
        self.dispatch_method(method, lookup)
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

impl<'a, 'gc> dotnet_intrinsics_reflection::LayoutQueryHost for VesContext<'a, 'gc> {
    fn reflection_type_layout(
        &self,
        t: ConcreteType,
    ) -> Result<std::sync::Arc<dotnet_value::layout::LayoutManager>, TypeResolutionError> {
        type_layout(t, &self.current_context())
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
        self.reflection().asms_write().insert(resolution, object);
    }

    fn reflection_cached_runtime_type(&self, target: &RuntimeType) -> Option<ObjectRef<'gc>> {
        self.reflection().types_read().get(target).copied()
    }

    fn reflection_cache_runtime_type(&self, target: RuntimeType, object: ObjectRef<'gc>) {
        self.reflection().types_write().insert(target, object);
    }

    fn reflection_runtime_type_index_get_or_insert(&self, target: RuntimeType) -> usize {
        #[cfg(feature = "multithreading")]
        {
            *self
                .shared()
                .shared_runtime_types
                .entry(target.clone())
                .or_insert_with(|| {
                    let idx = self
                        .shared()
                        .next_runtime_type_index
                        .fetch_add(1, Ordering::Relaxed);
                    self.shared().shared_runtime_types_rev.insert(idx, target);
                    idx
                })
        }

        #[cfg(not(feature = "multithreading"))]
        {
            let mut list = self.reflection().types_list_write();
            if let Some(existing) = list.iter().position(|t| t == &target) {
                existing
            } else {
                list.push(target);
                list.len() - 1
            }
        }
    }

    fn reflection_runtime_type_by_index(&self, index: usize) -> RuntimeType {
        #[cfg(feature = "multithreading")]
        {
            self.shared()
                .shared_runtime_types_rev
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
            *self
                .shared()
                .shared_runtime_methods
                .entry((method.clone(), lookup.clone()))
                .or_insert_with(|| {
                    let idx = self
                        .shared()
                        .next_runtime_method_index
                        .fetch_add(1, Ordering::Relaxed);
                    self.shared()
                        .shared_runtime_methods_rev
                        .insert(idx, (method, lookup));
                    idx
                })
        }

        #[cfg(not(feature = "multithreading"))]
        {
            let mut methods = self.reflection().methods_write();
            if let Some(existing) = methods
                .iter()
                .position(|(m, g)| *m == method && *g == lookup)
            {
                existing
            } else {
                methods.push((method, lookup));
                methods.len() - 1
            }
        }
    }

    fn reflection_runtime_method_by_index(
        &self,
        index: usize,
    ) -> (MethodDescription, GenericLookup) {
        #[cfg(feature = "multithreading")]
        {
            self.shared()
                .shared_runtime_methods_rev
                .get(&index)
                .map(|e| e.clone())
                .expect("invalid runtime method index")
        }

        #[cfg(not(feature = "multithreading"))]
        {
            self.reflection().methods_read()[index].clone()
        }
    }

    fn reflection_runtime_field_index_get_or_insert(
        &self,
        field: FieldDescription,
        lookup: GenericLookup,
    ) -> usize {
        #[cfg(feature = "multithreading")]
        {
            *self
                .shared()
                .shared_runtime_fields
                .entry((field.clone(), lookup.clone()))
                .or_insert_with(|| {
                    let idx = self
                        .shared()
                        .next_runtime_field_index
                        .fetch_add(1, Ordering::Relaxed);
                    self.shared()
                        .shared_runtime_fields_rev
                        .insert(idx, (field, lookup));
                    idx
                })
        }

        #[cfg(not(feature = "multithreading"))]
        {
            let mut fields = self.reflection().fields_write();
            if let Some(existing) = fields.iter().position(|(f, g)| *f == field && *g == lookup) {
                existing
            } else {
                fields.push((field, lookup));
                fields.len() - 1
            }
        }
    }

    fn reflection_runtime_field_by_index(&self, index: usize) -> (FieldDescription, GenericLookup) {
        #[cfg(feature = "multithreading")]
        {
            self.shared()
                .shared_runtime_fields_rev
                .get(&index)
                .map(|e| e.clone())
                .expect("invalid runtime field index")
        }

        #[cfg(not(feature = "multithreading"))]
        {
            self.reflection().fields_read()[index].clone()
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

    #[inline]
    fn make_runtime_type(&self, ctx: &ResolutionContext<'_>, source: &MethodType) -> RuntimeType {
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
    fn resolve_runtime_type(&self, obj: ObjectRef<'gc>) -> RuntimeType {
        dotnet_intrinsics_reflection::common::resolve_runtime_type(self, obj)
    }

    #[inline]
    fn resolve_runtime_method(&self, obj: ObjectRef<'gc>) -> (MethodDescription, GenericLookup) {
        dotnet_intrinsics_reflection::common::resolve_runtime_method(self, obj)
    }

    #[inline]
    fn resolve_runtime_field(&self, obj: ObjectRef<'gc>) -> (FieldDescription, GenericLookup) {
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
        ReflectionRegistry::new(self.local)
    }
}

use crate::{
    MethodType, ResolutionContext, StepResult,
    resolver::ResolverService,
    stack::{
        context::VesContext,
        ops::{
            IntrinsicDispatchOps, LoaderOps, ReflectionLookupOps, ReflectionOps, ResolutionOps,
            StaticsOps, ThreadOps,
        },
    },
    state::{ReflectionRegistry, SharedGlobalState, StaticStorageManager},
    sync::Arc,
};
use dotnet_assemblies::AssemblyLoader;
use dotnet_types::{
    error::TypeResolutionError,
    generics::{ConcreteType, GenericLookup},
    members::{FieldDescription, MethodDescription},
    runtime::RuntimeType,
};
use dotnet_utils::ArenaId;
use dotnet_value::object::ObjectRef;
use dotnetdll::prelude::FieldSource;

impl<'a, 'gc> LoaderOps for VesContext<'a, 'gc> {
    #[inline]
    fn loader_arc(&self) -> Arc<AssemblyLoader> {
        self.shared.loader.clone()
    }

    #[inline]
    fn resolver(&self) -> ResolverService {
        ResolverService::new(self.shared.clone())
    }

    #[inline]
    fn shared(&self) -> &Arc<SharedGlobalState> {
        self.shared
    }
}

impl<'a, 'gc> StaticsOps<'gc> for VesContext<'a, 'gc> {
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

impl<'a, 'gc> ReflectionLookupOps<'gc> for VesContext<'a, 'gc> {
    #[inline]
    fn get_runtime_method_index(
        &mut self,
        method: MethodDescription,
        lookup: GenericLookup,
    ) -> usize {
        crate::intrinsics::reflection::common::get_runtime_method_index(self, method, lookup)
            as usize
    }

    #[inline]
    fn get_runtime_type(&mut self, target: RuntimeType) -> ObjectRef<'gc> {
        crate::intrinsics::reflection::common::get_runtime_type(self, target)
    }

    #[inline]
    fn get_runtime_method_obj(
        &mut self,
        method: MethodDescription,
        lookup: GenericLookup,
    ) -> ObjectRef<'gc> {
        crate::intrinsics::reflection::common::get_runtime_method_obj(self, method, lookup)
    }

    #[inline]
    fn get_runtime_field_obj(
        &mut self,
        field: FieldDescription,
        lookup: GenericLookup,
    ) -> ObjectRef<'gc> {
        crate::intrinsics::reflection::common::get_runtime_field_obj(self, field, lookup)
    }
}

impl<'a, 'gc> ReflectionOps<'gc> for VesContext<'a, 'gc> {
    #[inline]
    fn pre_initialize_reflection(&mut self) {
        crate::intrinsics::reflection::common::pre_initialize_reflection(self)
    }

    #[inline]
    fn make_runtime_type(&self, ctx: &ResolutionContext<'_>, source: &MethodType) -> RuntimeType {
        crate::intrinsics::reflection::common::make_runtime_type(ctx, source)
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
        crate::intrinsics::reflection::common::resolve_runtime_type(self, obj)
    }

    #[inline]
    fn resolve_runtime_method(&self, obj: ObjectRef<'gc>) -> (MethodDescription, GenericLookup) {
        crate::intrinsics::reflection::common::resolve_runtime_method(self, obj)
    }

    #[inline]
    fn resolve_runtime_field(&self, obj: ObjectRef<'gc>) -> (FieldDescription, GenericLookup) {
        crate::intrinsics::reflection::common::resolve_runtime_field(self, obj)
    }

    #[inline]
    fn lookup_method_by_index(&self, index: usize) -> (MethodDescription, GenericLookup) {
        #[cfg(feature = "multithreading")]
        return self
            .shared
            .shared_runtime_methods_rev
            .get(&index)
            .map(|e| e.clone())
            .expect("invalid method index in delegate");

        #[cfg(not(feature = "multithreading"))]
        self.reflection().methods_read()[index].clone()
    }

    #[inline]
    fn reflection(&self) -> ReflectionRegistry<'_, 'gc> {
        ReflectionRegistry::new(self.local)
    }
}

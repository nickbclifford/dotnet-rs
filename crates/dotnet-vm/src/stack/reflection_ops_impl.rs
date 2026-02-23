use crate::{
    MethodType, ResolutionContext, StepResult,
    resolver::ResolverService,
    stack::{
        context::VesContext,
        ops::{
            CallOps, ExceptionOps, LoaderOps, RawMemoryOps, ReflectionOps, ResolutionOps, StackOps,
            StaticsOps, ThreadOps,
        },
    },
    state::{SharedGlobalState, StaticStorageManager},
    sync::Arc,
};
use dotnet_assemblies::AssemblyLoader;
use dotnet_types::{
    TypeDescription,
    generics::GenericLookup,
    members::{FieldDescription, MethodDescription},
    runtime::RuntimeType,
};
use dotnet_value::object::{ObjectHandle, ObjectRef};
use dotnetdll::prelude::FieldSource;

impl<'a, 'gc, 'm: 'gc> LoaderOps<'m> for VesContext<'a, 'gc, 'm> {
    #[inline]
    fn loader(&self) -> &'m AssemblyLoader {
        self.shared.loader
    }

    #[inline]
    fn resolver(&self) -> ResolverService<'m> {
        ResolverService::new(self.shared.clone())
    }

    #[inline]
    fn shared(&self) -> &Arc<SharedGlobalState<'m>> {
        self.shared
    }
}

impl<'a, 'gc, 'm: 'gc> StaticsOps<'gc> for VesContext<'a, 'gc, 'm> {
    #[inline]
    fn statics(&self) -> &StaticStorageManager {
        &self.shared.statics
    }

    #[inline]
    fn initialize_static_storage(
        &mut self,
        description: TypeDescription,
        generics: GenericLookup,
    ) -> StepResult {
        let _gc = self.gc;
        if self.check_gc_safe_point() { return StepResult::Yield; }

        let ctx = ResolutionContext {
            resolution: description.resolution,
            generics: &generics,
            loader: self.loader(),
            type_owner: Some(description),
            method_owner: None,
            caches: self.shared.caches.clone(),
            shared: Some(self.shared.clone()),
        };

        let tid = self.thread_id.get();
        let init_result = vm_try!(self.shared.statics.init(
            description,
            &ctx,
            tid,
            Some(&self.shared.metrics)
        ));

        use crate::statics::StaticInitResult::*;
        match init_result {
            Execute(m) => {
                crate::vm_trace!(
                    self,
                    "-- calling static constructor (will return to ip {}) --",
                    self.current_frame().state.ip
                );
                vm_try!(self.call_frame(
                    vm_try!(self.shared.caches.get_method_info(m, &generics, self.shared.clone())),
                    generics.clone(),
                ));
                StepResult::FramePushed
            }
            Initialized | Recursive => StepResult::Continue,
            Waiting => {
                if self.check_gc_safe_point() { return StepResult::Yield; }
                StepResult::Yield
            }
            Failed => self.throw_by_name_with_message(
                "System.TypeInitializationException",
                &format!(
                    "The type initializer for '{}' threw an exception.",
                    description.type_name()
                ),
            ),
        }
    }
}

impl<'a, 'gc, 'm: 'gc> ThreadOps for VesContext<'a, 'gc, 'm> {
    fn thread_id(&self) -> dotnet_utils::ArenaId {
        self.thread_id.get()
    }
}

impl<'a, 'gc, 'm: 'gc> ReflectionOps<'gc, 'm> for VesContext<'a, 'gc, 'm> {
    #[inline]
    fn pre_initialize_reflection(&mut self) {
        crate::intrinsics::reflection::common::pre_initialize_reflection(self)
    }

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

    #[inline]
    fn make_runtime_type(
        &self,
        ctx: &ResolutionContext<'_, 'm>,
        source: &MethodType,
    ) -> RuntimeType {
        crate::intrinsics::reflection::common::make_runtime_type(ctx, source)
    }

    #[inline]
    fn get_heap_description(
        &self,
        object: ObjectHandle<'gc>,
    ) -> Result<TypeDescription, dotnet_types::error::TypeResolutionError> {
        self.resolver().get_heap_description(object)
    }

    #[inline]
    fn locate_field(
        &self,
        handle: FieldSource,
    ) -> Result<(FieldDescription, GenericLookup), dotnet_types::error::TypeResolutionError> {
        let context = self.current_context();
        self.resolver()
            .locate_field(context.resolution, handle, context.generics)
    }

    #[inline]
    fn is_intrinsic_field_cached(&self, field: FieldDescription) -> bool {
        self.resolver().is_intrinsic_field_cached(field)
    }

    #[inline]
    fn is_intrinsic_cached(&self, method: MethodDescription) -> bool {
        self.resolver().is_intrinsic_cached(method)
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
        #[cfg(feature = "multithreaded-gc")]
        return self
            .shared
            .shared_runtime_methods_rev
            .get(&index)
            .map(|e| e.clone())
            .expect("invalid method index in delegate");

        #[cfg(not(feature = "multithreaded-gc"))]
        self.reflection().methods_read()[index].clone()
    }

    #[inline]
    fn reflection(&self) -> crate::state::ReflectionRegistry<'_, 'gc> {
        crate::state::ReflectionRegistry::new(self.local)
    }
}

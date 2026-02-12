use super::{context::VesContext, ops::{LoaderOps, ReflectionOps, StaticsOps, ThreadOps, RawMemoryOps, StackOps, CallOps, ExceptionOps, ResolutionOps}};
use crate::{
    StepResult, MethodInfo, MethodType, ResolutionContext,
    resolver::ResolverService,
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
use dotnet_value::object::{ObjectRef, ObjectHandle};
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
        self.check_gc_safe_point();

        let ctx = ResolutionContext {
            resolution: description.resolution,
            generics: &generics,
            loader: self.loader(),
            type_owner: Some(description),
            method_owner: None,
            caches: self.shared.caches.clone(),
            shared: Some(self.shared.clone()),
        };

        loop {
            let tid = self.thread_id.get();
            let init_result =
                self.shared
                    .statics
                    .init(description, &ctx, tid, Some(&self.shared.metrics));

            use crate::statics::StaticInitResult::*;
            match init_result {
                Execute(m) => {
                    crate::vm_trace!(
                        self,
                        "-- calling static constructor (will return to ip {}) --",
                        self.current_frame().state.ip
                    );
                    self.call_frame(
                        MethodInfo::new(m, &generics, self.shared.clone()),
                        generics.clone(),
                    );
                    return StepResult::FramePushed;
                }
                Initialized | Recursive => {
                    return StepResult::Continue;
                }
                Waiting => {
                    self.check_gc_safe_point();
                    std::thread::yield_now();
                }
                Failed => {
                    return self.throw_by_name("System.TypeInitializationException");
                }
            }
        }
    }
}

impl<'a, 'gc, 'm: 'gc> ThreadOps for VesContext<'a, 'gc, 'm> {
    #[inline]
    fn thread_id(&self) -> usize {
        self.thread_id.get() as usize
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
    fn get_heap_description(&self, object: ObjectHandle<'gc>) -> TypeDescription {
        self.resolver().get_heap_description(object)
    }

    #[inline]
    fn locate_field(&self, handle: FieldSource) -> (FieldDescription, GenericLookup) {
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


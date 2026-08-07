//! VM-side adapter over `dotnet-runtime-resolver`, wiring the resolver's generic
//! cache and layout parameters to the concrete VM implementations.
use crate::{
    intrinsics,
    state::{GlobalCaches, HIERARCHY_FRONT_CACHE, SharedGlobalState, VMT_FRONT_CACHE},
    sync::Arc,
};
use dotnet_assemblies::AssemblyLoader;
use dotnet_runtime_resolver::{
    IntrinsicCacheAdapter, ResolverLayoutAdapter, StaticConstrainedCacheAdapter,
    StaticConstrainedCacheKey, TypePropertyCacheAdapter, VmtCacheAdapter,
};
use dotnet_types::{
    TypeDescription,
    generics::{ConcreteType, GenericLookup},
    members::{FieldDescription, MethodDescription},
};
use dotnet_value::layout::{FieldLayoutManager, LayoutManager};
use std::{collections::HashMap, ops::Deref};

#[derive(Clone)]
pub struct VmResolverService {
    inner: dotnet_runtime_resolver::ResolverService<VmResolverCaches, VmResolverLayout>,
}

impl std::fmt::Debug for VmResolverService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VmResolverService").finish_non_exhaustive()
    }
}

impl Deref for VmResolverService {
    type Target = dotnet_runtime_resolver::ResolverService<VmResolverCaches, VmResolverLayout>;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl VmResolverService {
    pub fn new(shared: Arc<SharedGlobalState>) -> Self {
        #[allow(
            clippy::arc_with_non_send_sync,
            reason = "no-MT resolver caches are executor-confined; Arc preserves feature-neutral ownership"
        )]
        let caches = Arc::new(VmResolverCaches::new(shared.caches.clone()));
        let layout = VmResolverLayout::new(shared.caches.clone());
        let inner = dotnet_runtime_resolver::ResolverService::from_parts(
            shared.loader.clone(),
            caches,
            layout,
        );
        Self { inner }
    }

    pub fn from_parts(loader: Arc<AssemblyLoader>, caches: Arc<GlobalCaches>) -> Self {
        #[allow(
            clippy::arc_with_non_send_sync,
            reason = "no-MT resolver caches are executor-confined; Arc preserves feature-neutral ownership"
        )]
        let adapter = Arc::new(VmResolverCaches::new(caches.clone()));
        let layout = VmResolverLayout::new(caches);
        let inner = dotnet_runtime_resolver::ResolverService::from_parts(loader, adapter, layout);
        Self { inner }
    }

    pub fn loader(&self) -> &AssemblyLoader {
        self.inner.loader()
    }
}

#[derive(Clone)]
pub struct VmResolverCaches {
    caches: Arc<GlobalCaches>,
}

impl VmResolverCaches {
    fn new(caches: Arc<GlobalCaches>) -> Self {
        Self { caches }
    }
}

impl IntrinsicCacheAdapter for VmResolverCaches {
    fn get_intrinsic_cached(&self, method: &MethodDescription) -> Option<bool> {
        self.caches.intrinsic_cache.get(method)
    }

    fn set_intrinsic_cached(&self, method: MethodDescription, is_intrinsic: bool) {
        self.caches.intrinsic_cache.insert(method, is_intrinsic);
    }

    fn compute_is_intrinsic(&self, method: MethodDescription, loader: &AssemblyLoader) -> bool {
        intrinsics::is_intrinsic(method, loader, &self.caches.intrinsic_registry)
    }

    fn get_intrinsic_field_cached(&self, field: &FieldDescription) -> Option<bool> {
        self.caches.intrinsic_field_cache.get(field)
    }

    fn set_intrinsic_field_cached(&self, field: FieldDescription, is_intrinsic: bool) {
        self.caches
            .intrinsic_field_cache
            .insert(field, is_intrinsic);
    }

    fn compute_is_intrinsic_field(&self, field: FieldDescription, loader: &AssemblyLoader) -> bool {
        intrinsics::is_intrinsic_field(field, loader, &self.caches.intrinsic_registry)
    }
}

// VMT and hierarchy caching intentionally retain split get/set adapter methods. Their builds run
// in the `dotnet-runtime-resolver` consumer crate (for example, `resolve_virtual_method`), so a
// closure-based entry point here would invert that boundary and carry resolver error types into
// the adapter. Consolidation is therefore limited to their L1/L2 cache boilerplate.
impl VmtCacheAdapter for VmResolverCaches {
    fn get_vmt_cached(
        &self,
        base_method: &MethodDescription,
        this_type: &TypeDescription,
        generics: &GenericLookup,
    ) -> Option<MethodDescription> {
        self.caches.vmt_cache.record_key_clones(3);
        let key = (base_method.clone(), this_type.clone(), generics.clone());
        self.caches
            .vmt_cache
            .try_get_with_front(&key, &VMT_FRONT_CACHE)
    }

    fn set_vmt_cached(
        &self,
        base_method: MethodDescription,
        this_type: TypeDescription,
        generics: GenericLookup,
        method: MethodDescription,
    ) {
        self.caches.vmt_cache.insert_with_front(
            (base_method, this_type, generics),
            method,
            &VMT_FRONT_CACHE,
        );
    }

    fn record_vmt_key_clones(&self, count: u64) {
        self.caches.vmt_cache.record_key_clones(count);
    }

    fn get_overrides_cached(
        &self,
        key: &(TypeDescription, GenericLookup),
    ) -> Option<Arc<HashMap<MethodDescription, MethodDescription>>> {
        self.caches.overrides_cache.get(key)
    }

    fn set_overrides_cached(
        &self,
        key: (TypeDescription, GenericLookup),
        overrides: Arc<HashMap<MethodDescription, MethodDescription>>,
    ) {
        self.caches.overrides_cache.insert(key, overrides);
    }
}

impl StaticConstrainedCacheAdapter for VmResolverCaches {
    fn get_static_constrained_cached(
        &self,
        key: &StaticConstrainedCacheKey,
    ) -> Option<MethodDescription> {
        self.caches.static_constrained_cache.get(key)
    }

    fn set_static_constrained_cached(
        &self,
        key: StaticConstrainedCacheKey,
        method: MethodDescription,
    ) {
        self.caches.static_constrained_cache.insert(key, method);
    }

    fn record_static_constrained_key_clones(&self, count: u64) {
        self.caches
            .static_constrained_cache
            .record_key_clones(count);
    }
}

impl TypePropertyCacheAdapter for VmResolverCaches {
    fn get_hierarchy_cached(&self, child: &ConcreteType, parent: &ConcreteType) -> Option<bool> {
        self.caches.hierarchy_cache.record_key_clones(2);
        let key = (child.clone(), parent.clone());
        self.caches
            .hierarchy_cache
            .try_get_with_front(&key, &HIERARCHY_FRONT_CACHE)
    }

    fn set_hierarchy_cached(&self, child: ConcreteType, parent: ConcreteType, is_match: bool) {
        self.caches.hierarchy_cache.insert_with_front(
            (child, parent),
            is_match,
            &HIERARCHY_FRONT_CACHE,
        );
    }

    fn record_hierarchy_key_clones(&self, count: u64) {
        self.caches.hierarchy_cache.record_key_clones(count);
    }

    fn get_value_type_cached(&self, td: &TypeDescription) -> Option<bool> {
        self.caches.value_type_cache.get(td)
    }

    fn set_value_type_cached(&self, td: TypeDescription, is_value_type: bool) {
        self.caches.value_type_cache.insert(td, is_value_type);
    }

    fn get_has_finalizer_cached(&self, td: &TypeDescription) -> Option<bool> {
        self.caches.has_finalizer_cache.get(td)
    }

    fn set_has_finalizer_cached(&self, td: TypeDescription, has_finalizer: bool) {
        self.caches.has_finalizer_cache.insert(td, has_finalizer);
    }
}

#[derive(Clone)]
pub struct VmResolverLayout {
    caches: Arc<GlobalCaches>,
}

impl VmResolverLayout {
    fn new(caches: Arc<GlobalCaches>) -> Self {
        Self { caches }
    }
}

impl ResolverLayoutAdapter for VmResolverLayout {
    fn get_layout_cached(&self, key: &ConcreteType) -> Option<Arc<LayoutManager>> {
        self.caches.layout_cache.get(key)
    }

    fn set_layout_cached(&self, key: ConcreteType, layout: Arc<LayoutManager>) {
        self.caches.layout_cache.insert(key, layout);
    }

    fn get_instance_field_layout_cached(
        &self,
        key: &(TypeDescription, GenericLookup),
    ) -> Option<Arc<FieldLayoutManager>> {
        self.caches.instance_field_layout_cache.get(key)
    }

    fn set_instance_field_layout_cached(
        &self,
        key: (TypeDescription, GenericLookup),
        layout: Arc<FieldLayoutManager>,
    ) {
        self.caches.instance_field_layout_cache.insert(key, layout);
    }
}

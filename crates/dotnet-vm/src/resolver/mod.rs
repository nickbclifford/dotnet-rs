//! VM-side adapter over `dotnet-runtime-resolver`, wiring the resolver's generic
//! cache and layout parameters to the concrete VM implementations.
use crate::{
    intrinsics,
    state::{GlobalCaches, HIERARCHY_FRONT_CACHE, SharedGlobalState, VMT_FRONT_CACHE},
    sync::Arc,
};
use dotnet_assemblies::AssemblyLoader;
use dotnet_metrics::CacheEvent;
use dotnet_runtime_resolver::{
    IntrinsicCacheAdapter, ResolverLayoutAdapter, TypePropertyCacheAdapter, VmtCacheAdapter,
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
    #[allow(
        clippy::arc_with_non_send_sync,
        reason = "VmResolverCaches uses Arc uniformly and is thread-confined in the single-threaded configuration"
    )]
    pub fn new(shared: Arc<SharedGlobalState>) -> Self {
        let caches = Arc::new(VmResolverCaches::new(shared.caches.clone()));
        let layout = VmResolverLayout::new(shared.caches.clone());
        let inner = dotnet_runtime_resolver::ResolverService::from_parts(
            shared.loader.clone(),
            caches,
            layout,
        );
        Self { inner }
    }

    #[allow(
        clippy::arc_with_non_send_sync,
        reason = "VmResolverCaches uses Arc uniformly and is thread-confined in the single-threaded configuration"
    )]
    pub fn from_parts(loader: Arc<AssemblyLoader>, caches: Arc<GlobalCaches>) -> Self {
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

impl VmtCacheAdapter for VmResolverCaches {
    fn get_vmt_cached(
        &self,
        base_method: &MethodDescription,
        this_type: &TypeDescription,
        generics: &GenericLookup,
    ) -> Option<MethodDescription> {
        if self.caches.vmt_cache.front_cache_enabled() {
            let front_key = (base_method.clone(), this_type.clone(), generics.clone());
            if let Some(front_cached) =
                VMT_FRONT_CACHE.with(|cache| cache.borrow_mut().get(&front_key))
            {
                self.caches.vmt_cache.record_front_cache(CacheEvent::Hit);
                self.caches.vmt_cache.record_hit();
                return Some(front_cached);
            }
            self.caches.vmt_cache.record_front_cache(CacheEvent::Miss);
        }

        self.caches.vmt_cache.record_key_clones(3);
        let key = (base_method.clone(), this_type.clone(), generics.clone());
        if let Some(cached) = self.caches.vmt_cache.get(&key) {
            if self.caches.vmt_cache.front_cache_enabled() {
                VMT_FRONT_CACHE.with(|cache| {
                    cache.borrow_mut().insert(
                        key,
                        cached.clone(),
                        self.caches
                            .vmt_cache
                            .front_cache_capacity()
                            .expect("vmt front cache must have a configured capacity"),
                    );
                });
            }
            Some(cached)
        } else {
            None
        }
    }

    fn set_vmt_cached(
        &self,
        base_method: MethodDescription,
        this_type: TypeDescription,
        generics: GenericLookup,
        method: MethodDescription,
    ) {
        if self.caches.vmt_cache.front_cache_enabled() {
            VMT_FRONT_CACHE.with(|cache| {
                cache.borrow_mut().insert(
                    (base_method.clone(), this_type.clone(), generics.clone()),
                    method.clone(),
                    self.caches
                        .vmt_cache
                        .front_cache_capacity()
                        .expect("vmt front cache must have a configured capacity"),
                );
            });
        }

        self.caches
            .vmt_cache
            .insert((base_method, this_type, generics), method);
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

impl TypePropertyCacheAdapter for VmResolverCaches {
    fn get_hierarchy_cached(&self, child: &ConcreteType, parent: &ConcreteType) -> Option<bool> {
        if self.caches.hierarchy_cache.front_cache_enabled() {
            let key = (child.clone(), parent.clone());
            if let Some(front_cached) =
                HIERARCHY_FRONT_CACHE.with(|cache| cache.borrow_mut().get(&key))
            {
                self.caches
                    .hierarchy_cache
                    .record_front_cache(CacheEvent::Hit);
                self.caches.hierarchy_cache.record_hit();
                return Some(front_cached);
            }
            self.caches
                .hierarchy_cache
                .record_front_cache(CacheEvent::Miss);
        }

        self.caches.hierarchy_cache.record_key_clones(2);
        let key = (child.clone(), parent.clone());
        if let Some(cached_value) = self.caches.hierarchy_cache.get(&key) {
            if self.caches.hierarchy_cache.front_cache_enabled() {
                HIERARCHY_FRONT_CACHE.with(|cache| {
                    cache.borrow_mut().insert(
                        key,
                        cached_value,
                        self.caches
                            .hierarchy_cache
                            .front_cache_capacity()
                            .expect("hierarchy front cache must have a configured capacity"),
                    );
                });
            }
            Some(cached_value)
        } else {
            None
        }
    }

    fn set_hierarchy_cached(&self, child: ConcreteType, parent: ConcreteType, is_match: bool) {
        if self.caches.hierarchy_cache.front_cache_enabled() {
            HIERARCHY_FRONT_CACHE.with(|cache| {
                cache.borrow_mut().insert(
                    (child.clone(), parent.clone()),
                    is_match,
                    self.caches
                        .hierarchy_cache
                        .front_cache_capacity()
                        .expect("hierarchy front cache must have a configured capacity"),
                );
            });
        }

        self.caches
            .hierarchy_cache
            .insert((child, parent), is_match);
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

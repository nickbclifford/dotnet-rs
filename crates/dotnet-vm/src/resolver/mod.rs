//! VM-side adapter over `dotnet-runtime-resolver`, wiring the resolver's generic
//! cache and layout parameters to the concrete VM implementations.
use crate::{
    intrinsics,
    state::{GlobalCaches, SharedGlobalState},
    sync::{Arc, Weak},
};
use dotnet_assemblies::AssemblyLoader;
use dotnet_runtime_resolver::{ResolverCacheAdapter, ResolverLayoutAdapter};
use dotnet_types::{
    TypeDescription,
    generics::{ConcreteType, GenericLookup},
    members::{FieldDescription, MethodDescription},
};
use dotnet_value::layout::{FieldLayoutManager, LayoutManager};
use std::{collections::HashMap, ops::Deref};

#[derive(Clone)]
pub struct ResolverService {
    inner: dotnet_runtime_resolver::ResolverService<VmResolverCaches, VmResolverLayout>,
}

impl std::fmt::Debug for ResolverService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ResolverService").finish_non_exhaustive()
    }
}

impl Deref for ResolverService {
    type Target = dotnet_runtime_resolver::ResolverService<VmResolverCaches, VmResolverLayout>;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl ResolverService {
    pub fn new(shared: Arc<SharedGlobalState>) -> Self {
        let caches = Arc::new(VmResolverCaches::new(
            shared.caches.clone(),
            Some(Arc::downgrade(&shared)),
        ));
        let layout = VmResolverLayout::new(shared.caches.clone(), Some(Arc::downgrade(&shared)));
        let inner = dotnet_runtime_resolver::ResolverService::from_parts(
            shared.loader.clone(),
            caches,
            layout,
        );
        Self { inner }
    }

    pub fn from_parts(loader: Arc<AssemblyLoader>, caches: Arc<GlobalCaches>) -> Self {
        let adapter = Arc::new(VmResolverCaches::new(caches.clone(), None));
        let layout = VmResolverLayout::new(caches, None);
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
    shared: Option<Weak<SharedGlobalState>>,
}

impl VmResolverCaches {
    fn new(caches: Arc<GlobalCaches>, shared: Option<Weak<SharedGlobalState>>) -> Self {
        Self { caches, shared }
    }

    fn shared_state(&self) -> Option<Arc<SharedGlobalState>> {
        self.shared.as_ref().and_then(|s| s.upgrade())
    }
}

impl ResolverCacheAdapter for VmResolverCaches {
    fn get_intrinsic_cached(&self, method: &MethodDescription) -> Option<bool> {
        if let Some(cached) = self.caches.intrinsic_cache.get(method) {
            if let Some(shared) = self.shared_state() {
                shared.metrics.record_intrinsic_cache_hit();
            }
            Some(*cached)
        } else {
            if let Some(shared) = self.shared_state() {
                shared.metrics.record_intrinsic_cache_miss();
            }
            None
        }
    }

    fn set_intrinsic_cached(&self, method: MethodDescription, is_intrinsic: bool) {
        self.caches.intrinsic_cache.insert(method, is_intrinsic);
    }

    fn compute_is_intrinsic(&self, method: MethodDescription, loader: &AssemblyLoader) -> bool {
        intrinsics::is_intrinsic(method, loader, &self.caches.intrinsic_registry)
    }

    fn get_intrinsic_field_cached(&self, field: &FieldDescription) -> Option<bool> {
        if let Some(cached) = self.caches.intrinsic_field_cache.get(field) {
            if let Some(shared) = self.shared_state() {
                shared.metrics.record_intrinsic_field_cache_hit();
            }
            Some(*cached)
        } else {
            if let Some(shared) = self.shared_state() {
                shared.metrics.record_intrinsic_field_cache_miss();
            }
            None
        }
    }

    fn set_intrinsic_field_cached(&self, field: FieldDescription, is_intrinsic: bool) {
        self.caches
            .intrinsic_field_cache
            .insert(field, is_intrinsic);
    }

    fn compute_is_intrinsic_field(&self, field: FieldDescription, loader: &AssemblyLoader) -> bool {
        intrinsics::is_intrinsic_field(field, loader, &self.caches.intrinsic_registry)
    }

    fn get_vmt_cached(
        &self,
        key: &(MethodDescription, TypeDescription, GenericLookup),
    ) -> Option<MethodDescription> {
        if let Some(cached) = self.caches.vmt_cache.get(key) {
            if let Some(shared) = self.shared_state() {
                shared.metrics.record_vmt_cache_hit();
            }
            Some(cached.clone())
        } else {
            if let Some(shared) = self.shared_state() {
                shared.metrics.record_vmt_cache_miss();
            }
            None
        }
    }

    fn set_vmt_cached(
        &self,
        key: (MethodDescription, TypeDescription, GenericLookup),
        method: MethodDescription,
    ) {
        self.caches.vmt_cache.insert(key, method);
    }

    fn get_overrides_cached(
        &self,
        key: &(TypeDescription, GenericLookup),
    ) -> Option<Arc<HashMap<MethodDescription, MethodDescription>>> {
        if let Some(cached) = self.caches.overrides_cache.get(key) {
            if let Some(shared) = self.shared_state() {
                shared.metrics.record_overrides_cache_hit();
            }
            Some(cached.clone())
        } else {
            if let Some(shared) = self.shared_state() {
                shared.metrics.record_overrides_cache_miss();
            }
            None
        }
    }

    fn set_overrides_cached(
        &self,
        key: (TypeDescription, GenericLookup),
        overrides: Arc<HashMap<MethodDescription, MethodDescription>>,
    ) {
        self.caches.overrides_cache.insert(key, overrides);
    }

    fn get_hierarchy_cached(&self, key: &(ConcreteType, ConcreteType)) -> Option<bool> {
        if let Some(cached) = self.caches.hierarchy_cache.get(key) {
            if let Some(shared) = self.shared_state() {
                shared.metrics.record_hierarchy_cache_hit();
            }
            Some(*cached)
        } else {
            if let Some(shared) = self.shared_state() {
                shared.metrics.record_hierarchy_cache_miss();
            }
            None
        }
    }

    fn set_hierarchy_cached(&self, key: (ConcreteType, ConcreteType), is_match: bool) {
        self.caches.hierarchy_cache.insert(key, is_match);
    }

    fn get_value_type_cached(&self, td: &TypeDescription) -> Option<bool> {
        if let Some(cached) = self.caches.value_type_cache.get(td) {
            if let Some(shared) = self.shared_state() {
                shared.metrics.record_value_type_cache_hit();
            }
            Some(*cached)
        } else {
            if let Some(shared) = self.shared_state() {
                shared.metrics.record_value_type_cache_miss();
            }
            None
        }
    }

    fn set_value_type_cached(&self, td: TypeDescription, is_value_type: bool) {
        self.caches.value_type_cache.insert(td, is_value_type);
    }

    fn get_has_finalizer_cached(&self, td: &TypeDescription) -> Option<bool> {
        if let Some(cached) = self.caches.has_finalizer_cache.get(td) {
            if let Some(shared) = self.shared_state() {
                shared.metrics.record_has_finalizer_cache_hit();
            }
            Some(*cached)
        } else {
            if let Some(shared) = self.shared_state() {
                shared.metrics.record_has_finalizer_cache_miss();
            }
            None
        }
    }

    fn set_has_finalizer_cached(&self, td: TypeDescription, has_finalizer: bool) {
        self.caches.has_finalizer_cache.insert(td, has_finalizer);
    }
}

#[derive(Clone)]
pub struct VmResolverLayout {
    caches: Arc<GlobalCaches>,
    shared: Option<Weak<SharedGlobalState>>,
}

impl VmResolverLayout {
    fn new(caches: Arc<GlobalCaches>, shared: Option<Weak<SharedGlobalState>>) -> Self {
        Self { caches, shared }
    }

    fn shared_state(&self) -> Option<Arc<SharedGlobalState>> {
        self.shared.as_ref().and_then(Weak::upgrade)
    }
}

impl ResolverLayoutAdapter for VmResolverLayout {
    fn get_layout_cached(&self, key: &ConcreteType) -> Option<Arc<LayoutManager>> {
        if let Some(cached) = self.caches.layout_cache.get(key) {
            if let Some(shared) = self.shared_state() {
                shared.metrics.record_layout_cache_hit();
            }
            Some(cached.clone())
        } else {
            if let Some(shared) = self.shared_state() {
                shared.metrics.record_layout_cache_miss();
            }
            None
        }
    }

    fn set_layout_cached(&self, key: ConcreteType, layout: Arc<LayoutManager>) {
        self.caches.layout_cache.insert(key, layout);
    }

    fn get_instance_field_layout_cached(
        &self,
        key: &(TypeDescription, GenericLookup),
    ) -> Option<Arc<FieldLayoutManager>> {
        if let Some(cached) = self.caches.instance_field_layout_cache.get(key) {
            if let Some(shared) = self.shared_state() {
                shared.metrics.record_instance_field_layout_cache_hit();
            }
            Some(cached.clone())
        } else {
            if let Some(shared) = self.shared_state() {
                shared.metrics.record_instance_field_layout_cache_miss();
            }
            None
        }
    }

    fn set_instance_field_layout_cached(
        &self,
        key: (TypeDescription, GenericLookup),
        layout: Arc<FieldLayoutManager>,
    ) {
        self.caches.instance_field_layout_cache.insert(key, layout);
    }
}

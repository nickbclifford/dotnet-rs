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
use std::{
    cell::RefCell,
    collections::{HashMap, VecDeque},
    ops::Deref,
};

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

#[derive(Default)]
struct ResolverFrontCache {
    vmt: VecDeque<(
        MethodDescription,
        TypeDescription,
        GenericLookup,
        MethodDescription,
    )>,
    hierarchy: VecDeque<(ConcreteType, ConcreteType, bool)>,
}

impl ResolverFrontCache {
    fn get_vmt(
        &mut self,
        base_method: &MethodDescription,
        this_type: &TypeDescription,
        generics: &GenericLookup,
    ) -> Option<MethodDescription> {
        let index = self
            .vmt
            .iter()
            .position(|(m, t, g, _)| m == base_method && t == this_type && g == generics)?;
        let entry = self.vmt.remove(index)?;
        let value = entry.3.clone();
        self.vmt.push_back(entry);
        Some(value)
    }

    fn insert_vmt(
        &mut self,
        base_method: MethodDescription,
        this_type: TypeDescription,
        generics: GenericLookup,
        resolved_method: MethodDescription,
        capacity: usize,
    ) {
        if capacity == 0 {
            return;
        }

        if let Some(index) = self
            .vmt
            .iter()
            .position(|(m, t, g, _)| m == &base_method && t == &this_type && g == &generics)
        {
            self.vmt.remove(index);
        }

        self.vmt
            .push_back((base_method, this_type, generics, resolved_method));
        while self.vmt.len() > capacity {
            self.vmt.pop_front();
        }
    }

    fn get_hierarchy(&mut self, child: &ConcreteType, parent: &ConcreteType) -> Option<bool> {
        let index = self
            .hierarchy
            .iter()
            .position(|(c, p, _)| c == child && p == parent)?;
        let entry = self.hierarchy.remove(index)?;
        let value = entry.2;
        self.hierarchy.push_back(entry);
        Some(value)
    }

    fn insert_hierarchy(
        &mut self,
        child: ConcreteType,
        parent: ConcreteType,
        is_match: bool,
        capacity: usize,
    ) {
        if capacity == 0 {
            return;
        }

        if let Some(index) = self
            .hierarchy
            .iter()
            .position(|(c, p, _)| c == &child && p == &parent)
        {
            self.hierarchy.remove(index);
        }

        self.hierarchy.push_back((child, parent, is_match));
        while self.hierarchy.len() > capacity {
            self.hierarchy.pop_front();
        }
    }
}

thread_local! {
    static RESOLVER_FRONT_CACHE: RefCell<ResolverFrontCache> =
        RefCell::new(ResolverFrontCache::default());
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
        base_method: &MethodDescription,
        this_type: &TypeDescription,
        generics: &GenericLookup,
    ) -> Option<MethodDescription> {
        if self.caches.front_cache_enabled() {
            if let Some(front_cached) = RESOLVER_FRONT_CACHE
                .with(|cache| cache.borrow_mut().get_vmt(base_method, this_type, generics))
            {
                if let Some(shared) = self.shared_state() {
                    shared.metrics.record_vmt_front_cache_hit();
                    shared.metrics.record_vmt_cache_hit();
                }
                return Some(front_cached);
            }
            if let Some(shared) = self.shared_state() {
                shared.metrics.record_vmt_front_cache_miss();
            }
        }

        if let Some(shared) = self.shared_state() {
            shared.metrics.record_vmt_key_clones(3);
        }
        let key = (base_method.clone(), this_type.clone(), generics.clone());
        if let Some(cached) = self.caches.vmt_cache.get(&key) {
            if let Some(shared) = self.shared_state() {
                shared.metrics.record_vmt_cache_hit();
            }
            let cached_method = cached.clone();
            drop(cached);
            if self.caches.front_cache_enabled() {
                let (method_key, type_key, generic_key) = key;
                RESOLVER_FRONT_CACHE.with(|cache| {
                    cache.borrow_mut().insert_vmt(
                        method_key,
                        type_key,
                        generic_key,
                        cached_method.clone(),
                        self.caches.front_cache_capacity(),
                    );
                });
            }
            Some(cached_method)
        } else {
            if let Some(shared) = self.shared_state() {
                shared.metrics.record_vmt_cache_miss();
            }
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
        self.caches
            .set_vmt((base_method, this_type, generics), method);
    }

    fn record_vmt_key_clones(&self, count: u64) {
        if let Some(shared) = self.shared_state() {
            shared.metrics.record_vmt_key_clones(count);
        }
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

    fn get_hierarchy_cached(&self, child: &ConcreteType, parent: &ConcreteType) -> Option<bool> {
        if self.caches.front_cache_enabled() {
            if let Some(front_cached) =
                RESOLVER_FRONT_CACHE.with(|cache| cache.borrow_mut().get_hierarchy(child, parent))
            {
                if let Some(shared) = self.shared_state() {
                    shared.metrics.record_hierarchy_front_cache_hit();
                    shared.metrics.record_hierarchy_cache_hit();
                }
                return Some(front_cached);
            }
            if let Some(shared) = self.shared_state() {
                shared.metrics.record_hierarchy_front_cache_miss();
            }
        }

        if let Some(shared) = self.shared_state() {
            shared.metrics.record_hierarchy_key_clones(2);
        }
        let key = (child.clone(), parent.clone());
        if let Some(cached) = self.caches.hierarchy_cache.get(&key) {
            if let Some(shared) = self.shared_state() {
                shared.metrics.record_hierarchy_cache_hit();
            }
            let cached_value = *cached;
            drop(cached);
            if self.caches.front_cache_enabled() {
                let (child_key, parent_key) = key;
                RESOLVER_FRONT_CACHE.with(|cache| {
                    cache.borrow_mut().insert_hierarchy(
                        child_key,
                        parent_key,
                        cached_value,
                        self.caches.front_cache_capacity(),
                    );
                });
            }
            Some(cached_value)
        } else {
            if let Some(shared) = self.shared_state() {
                shared.metrics.record_hierarchy_cache_miss();
            }
            None
        }
    }

    fn set_hierarchy_cached(&self, child: ConcreteType, parent: ConcreteType, is_match: bool) {
        self.caches.set_hierarchy((child, parent), is_match);
    }

    fn record_hierarchy_key_clones(&self, count: u64) {
        if let Some(shared) = self.shared_state() {
            shared.metrics.record_hierarchy_key_clones(count);
        }
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

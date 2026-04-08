use crate::{
    MethodInfo, dispatch::ring_buffer::InstructionRingBuffer, error::TypeResolutionError,
    gc::GCCoordinator, intrinsics::IntrinsicRegistry, sync::SyncBlockManager,
    threading::ThreadManager,
};
use dashmap::DashMap;
use dotnet_assemblies::AssemblyLoader;
use dotnet_metrics::{CacheSizes, CacheStats, RuntimeMetrics, RuntimeMetricsSnapshot};
use dotnet_pinvoke::NativeLibraries;
use dotnet_runtime_memory::HeapManager;
use dotnet_tracer::{TraceLevel, Tracer};
use dotnet_types::{
    TypeDescription,
    generics::{ConcreteType, GenericLookup},
    members::{FieldDescription, MethodDescription},
    resolution::ResolutionS,
    runtime::RuntimeType,
};
use dotnet_utils::sync::{Arc, AtomicBool, Mutex, Ordering};
use dotnet_value::{
    layout::{FieldLayoutManager, LayoutManager},
    object::ObjectRef,
};
use gc_arena::{Collect, collect::Trace};
use std::{
    cell::{Cell, Ref, RefCell, RefMut},
    collections::{HashMap, VecDeque},
    env,
    hash::Hash,
    mem::size_of,
    sync::OnceLock,
};

#[cfg(feature = "multithreading")]
use dotnet_utils::sync::AtomicUsize;

pub use crate::statics::StaticStorageManager;

#[derive(Clone, Copy, Debug)]
struct CachePolicy {
    method_info_capacity: Option<usize>,
    vmt_capacity: Option<usize>,
    hierarchy_capacity: Option<usize>,
    front_cache_enabled: bool,
    front_cache_capacity: usize,
}

impl Default for CachePolicy {
    fn default() -> Self {
        Self {
            method_info_capacity: parse_env_usize("DOTNET_CACHE_LIMIT_METHOD_INFO"),
            vmt_capacity: parse_env_usize("DOTNET_CACHE_LIMIT_VMT"),
            hierarchy_capacity: parse_env_usize("DOTNET_CACHE_LIMIT_HIERARCHY"),
            front_cache_enabled: parse_env_bool("DOTNET_FRONT_CACHE_ENABLED", false),
            front_cache_capacity: parse_env_usize("DOTNET_FRONT_CACHE_CAPACITY").unwrap_or(128),
        }
    }
}

fn parse_env_usize(key: &str) -> Option<usize> {
    env::var(key)
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .filter(|v| *v > 0)
}

fn parse_env_bool(key: &str, default: bool) -> bool {
    match env::var(key) {
        Ok(v) => match v.trim().to_ascii_lowercase().as_str() {
            "1" | "true" | "yes" | "on" => true,
            "0" | "false" | "no" | "off" => false,
            _ => default,
        },
        Err(_) => default,
    }
}

fn estimated_dashmap_bytes<K, V>(map: &DashMap<K, V>) -> u64
where
    K: Eq + Hash,
{
    (map.len() as u64).saturating_mul((size_of::<K>() + size_of::<V>()) as u64)
}

fn insert_with_optional_capacity<K, V>(
    map: &DashMap<K, V>,
    key: K,
    value: V,
    capacity: Option<usize>,
) where
    K: Eq + Hash + Clone,
{
    if let Some(max_entries) = capacity {
        if let Some(mut existing) = map.get_mut(&key) {
            *existing = value;
            return;
        }

        while map.len() >= max_entries {
            let victim = map.iter().next().map(|entry| entry.key().clone());
            if let Some(victim_key) = victim {
                map.remove(&victim_key);
            } else {
                break;
            }
        }
    }

    map.insert(key, value);
}

#[derive(Default)]
struct MethodInfoFrontCache {
    entries: VecDeque<(MethodDescription, GenericLookup, Arc<MethodInfo<'static>>)>,
}

impl MethodInfoFrontCache {
    fn get(
        &mut self,
        method: &MethodDescription,
        generics: &GenericLookup,
    ) -> Option<Arc<MethodInfo<'static>>> {
        let index = self
            .entries
            .iter()
            .position(|(m, g, _)| m == method && g == generics)?;
        let entry = self.entries.remove(index)?;
        let value = Arc::clone(&entry.2);
        self.entries.push_back(entry);
        Some(value)
    }

    fn insert(
        &mut self,
        method: MethodDescription,
        generics: GenericLookup,
        value: Arc<MethodInfo<'static>>,
        capacity: usize,
    ) {
        if capacity == 0 {
            return;
        }

        if let Some(index) = self
            .entries
            .iter()
            .position(|(m, g, _)| m == &method && g == &generics)
        {
            self.entries.remove(index);
        }

        self.entries.push_back((method, generics, value));
        while self.entries.len() > capacity {
            self.entries.pop_front();
        }
    }
}

thread_local! {
    static METHOD_INFO_FRONT_CACHE: RefCell<MethodInfoFrontCache> =
        RefCell::new(MethodInfoFrontCache::default());
}

/// Grouped caches for type resolution and layout computation.
/// This struct reduces the API surface area of ResolutionContext.
pub struct GlobalCaches {
    /// Cache for type layouts: ConcreteType -> Arc<[`LayoutManager`]>
    pub layout_cache: DashMap<ConcreteType, Arc<LayoutManager>>,
    /// Cache for instance field layouts: (TypeDescription, GenericLookup) -> FieldLayoutManager
    pub instance_field_layout_cache:
        DashMap<(TypeDescription, GenericLookup), Arc<FieldLayoutManager>>,
    /// Cache for virtual method resolution: (base_method, this_type, generics) -> resolved_method
    pub vmt_cache: DashMap<(MethodDescription, TypeDescription, GenericLookup), MethodDescription>,
    /// Cache for type hierarchy checks: (child, parent) -> is_a result
    pub hierarchy_cache: DashMap<(ConcreteType, ConcreteType), bool>,
    /// Cache for intrinsic checks: method -> is_intrinsic
    pub intrinsic_cache: DashMap<MethodDescription, bool>,
    /// Cache for intrinsic field checks: field -> is_intrinsic
    pub intrinsic_field_cache: DashMap<FieldDescription, bool>,
    /// Cache for static field layouts: (TypeDescription, GenericLookup) -> FieldLayoutManager
    pub static_field_layout_cache:
        DashMap<(TypeDescription, GenericLookup), Arc<FieldLayoutManager>>,
    /// Cache for value type checks: TypeDescription -> bool
    pub value_type_cache: DashMap<TypeDescription, bool>,
    /// Cache for finalizer checks: TypeDescription -> bool
    pub has_finalizer_cache: DashMap<TypeDescription, bool>,
    /// Cache for resolved overrides: (TypeDescription, GenericLookup) -> Map<DeclMethod, ImplMethod>
    pub overrides_cache: DashMap<
        (TypeDescription, GenericLookup),
        Arc<HashMap<MethodDescription, MethodDescription>>,
    >,
    /// Cache for method info: (Method, Lookup) -> MethodInfo
    pub method_info_cache: DashMap<(MethodDescription, GenericLookup), Arc<MethodInfo<'static>>>,
    /// Registry of intrinsic methods
    pub intrinsic_registry: IntrinsicRegistry,
    cache_policy: CachePolicy,
}

impl GlobalCaches {
    pub fn new(_loader: &AssemblyLoader, _tracer: &Tracer) -> Self {
        let intrinsic_registry = IntrinsicRegistry::initialize();
        let cache_policy = CachePolicy::default();
        Self {
            layout_cache: DashMap::new(),
            instance_field_layout_cache: DashMap::new(),
            vmt_cache: DashMap::new(),
            hierarchy_cache: DashMap::new(),
            intrinsic_cache: DashMap::new(),
            intrinsic_field_cache: DashMap::new(),
            static_field_layout_cache: DashMap::new(),
            value_type_cache: DashMap::new(),
            has_finalizer_cache: DashMap::new(),
            overrides_cache: DashMap::new(),
            method_info_cache: DashMap::new(),
            intrinsic_registry,
            cache_policy,
        }
    }

    pub fn front_cache_enabled(&self) -> bool {
        self.cache_policy.front_cache_enabled
    }

    pub fn front_cache_capacity(&self) -> usize {
        self.cache_policy.front_cache_capacity
    }

    pub fn set_vmt(
        &self,
        key: (MethodDescription, TypeDescription, GenericLookup),
        value: MethodDescription,
    ) {
        insert_with_optional_capacity(&self.vmt_cache, key, value, self.cache_policy.vmt_capacity);
    }

    pub fn set_hierarchy(&self, key: (ConcreteType, ConcreteType), is_match: bool) {
        insert_with_optional_capacity(
            &self.hierarchy_cache,
            key,
            is_match,
            self.cache_policy.hierarchy_capacity,
        );
    }
}

/// Thread-safe shared state that does not contain any GC-managed pointers.
/// This state is shared across all execution threads and arenas.
pub struct SharedGlobalState {
    pub loader: Arc<AssemblyLoader>,
    pub pinvoke: NativeLibraries,
    pub sync_blocks: SyncBlockManager,
    pub thread_manager: Arc<ThreadManager>,
    pub metrics: RuntimeMetrics,
    pub tracer: Tracer,
    pub tracer_enabled: Arc<AtomicBool>,
    pub empty_generics: GenericLookup,
    /// Grouped caches for type resolution and layout computation
    pub caches: Arc<GlobalCaches>,
    pub statics: Arc<StaticStorageManager>,
    pub last_instructions: Arc<Mutex<InstructionRingBuffer>>,
    pub abort_requested: Arc<AtomicBool>,
    pub gc_coordinator: Arc<GCCoordinator>,
    /// Cache for shared reflection objects: RuntimeType -> index
    #[cfg(feature = "multithreading")]
    pub shared_runtime_types: DashMap<RuntimeType, usize>,
    #[cfg(feature = "multithreading")]
    pub shared_runtime_types_rev: DashMap<usize, RuntimeType>,
    #[cfg(feature = "multithreading")]
    pub next_runtime_type_index: AtomicUsize,
    /// Cache for shared method reflection objects: (Method, Lookup) -> index
    #[cfg(feature = "multithreading")]
    pub shared_runtime_methods: DashMap<(MethodDescription, GenericLookup), usize>,
    #[cfg(feature = "multithreading")]
    pub shared_runtime_methods_rev: DashMap<usize, (MethodDescription, GenericLookup)>,
    #[cfg(feature = "multithreading")]
    pub next_runtime_method_index: AtomicUsize,
    /// Cache for shared field reflection objects: (Field, Lookup) -> index
    #[cfg(feature = "multithreading")]
    pub shared_runtime_fields: DashMap<(FieldDescription, GenericLookup), usize>,
    #[cfg(feature = "multithreading")]
    pub shared_runtime_fields_rev: DashMap<usize, (FieldDescription, GenericLookup)>,
    #[cfg(feature = "multithreading")]
    pub next_runtime_field_index: AtomicUsize,
    pub resolution_shared_cache: OnceLock<Arc<crate::context::ResolutionShared>>,
}

// SAFETY: Under `--no-default-features` the runtime is single-threaded; `SharedGlobalState` is
// never accessed concurrently.  The `!Sync`/`!Send` fields are compat `Mutex`/`RwLock` wrappers
// over `RefCell` and `NonNull`-bearing descriptors, all safe here because no concurrent access
// can occur in this build.  These impls are intentionally absent when `multithreading` is
// enabled so the real thread-safe types provide the `Send`/`Sync` guarantees instead.
#[cfg(not(feature = "multithreading"))]
unsafe impl Sync for SharedGlobalState {}
#[cfg(not(feature = "multithreading"))]
unsafe impl Send for SharedGlobalState {}

impl GlobalCaches {
    pub fn get_method_info(
        &self,
        method: MethodDescription,
        generics: &GenericLookup,
        shared: Arc<SharedGlobalState>,
    ) -> Result<MethodInfo<'static>, TypeResolutionError> {
        if self.front_cache_enabled() {
            if let Some(front_cached) =
                METHOD_INFO_FRONT_CACHE.with(|cache| cache.borrow_mut().get(&method, generics))
            {
                shared.metrics.record_method_info_front_cache_hit();
                shared.metrics.record_method_info_cache_hit();
                return Ok((*front_cached).clone());
            }
            shared.metrics.record_method_info_front_cache_miss();
        }

        shared.metrics.record_method_info_key_clones(2);
        let key = (method.clone(), generics.clone());
        if let Some(entry) = self.method_info_cache.get(&key) {
            shared.metrics.record_method_info_cache_hit();
            let cached = Arc::clone(&entry);
            drop(entry);
            if self.front_cache_enabled() {
                let (method_key, generic_key) = key;
                METHOD_INFO_FRONT_CACHE.with(|cache| {
                    cache.borrow_mut().insert(
                        method_key,
                        generic_key,
                        Arc::clone(&cached),
                        self.front_cache_capacity(),
                    );
                });
            }
            return Ok((*cached).clone());
        }
        shared.metrics.record_method_info_cache_miss();
        let built = crate::build_method_info(method, generics, shared.clone())?;
        let built_arc = Arc::new(built.clone());
        insert_with_optional_capacity(
            &self.method_info_cache,
            key.clone(),
            Arc::clone(&built_arc),
            self.cache_policy.method_info_capacity,
        );
        if self.front_cache_enabled() {
            shared.metrics.record_method_info_key_clones(2);
            METHOD_INFO_FRONT_CACHE.with(|cache| {
                cache.borrow_mut().insert(
                    key.0,
                    key.1,
                    Arc::clone(&built_arc),
                    self.front_cache_capacity(),
                );
            });
        }
        Ok(built)
    }
}

impl SharedGlobalState {
    #[allow(clippy::arc_with_non_send_sync)]
    pub fn new(loader: Arc<AssemblyLoader>) -> Self {
        let tracer = Tracer::new();
        let caches = Arc::new(GlobalCaches::new(&loader, &tracer));

        let tracer_enabled = Arc::new(AtomicBool::new(tracer.is_enabled()));

        let stw_in_progress = Arc::new(AtomicBool::new(false));

        let state = Self {
            pinvoke: {
                let p = NativeLibraries::new(loader.get_root());
                #[cfg(feature = "fuzzing")]
                let p = p.with_sandbox(Arc::new(dotnet_pinvoke::DenySandbox));
                p
            },
            loader,
            sync_blocks: SyncBlockManager::new(),
            thread_manager: ThreadManager::new(stw_in_progress.clone()),
            metrics: RuntimeMetrics::new(),
            tracer,
            tracer_enabled,
            empty_generics: GenericLookup::default(),
            caches,
            statics: Arc::new(StaticStorageManager::new()),
            last_instructions: Arc::new(Mutex::new(InstructionRingBuffer::new())),
            abort_requested: Arc::new(AtomicBool::new(false)),
            gc_coordinator: Arc::new(GCCoordinator::new(stw_in_progress.clone())),
            #[cfg(feature = "multithreading")]
            shared_runtime_types: DashMap::new(),
            #[cfg(feature = "multithreading")]
            shared_runtime_types_rev: DashMap::new(),
            #[cfg(feature = "multithreading")]
            next_runtime_type_index: AtomicUsize::new(0),
            #[cfg(feature = "multithreading")]
            shared_runtime_methods: DashMap::new(),
            #[cfg(feature = "multithreading")]
            shared_runtime_methods_rev: DashMap::new(),
            #[cfg(feature = "multithreading")]
            next_runtime_method_index: AtomicUsize::new(0),
            #[cfg(feature = "multithreading")]
            shared_runtime_fields: DashMap::new(),
            #[cfg(feature = "multithreading")]
            shared_runtime_fields_rev: DashMap::new(),
            #[cfg(feature = "multithreading")]
            next_runtime_field_index: AtomicUsize::new(0),
            resolution_shared_cache: OnceLock::new(),
        };

        state
            .thread_manager
            .set_coordinator(Arc::downgrade(&state.gc_coordinator));

        state
    }

    pub fn get_cache_stats(&self) -> CacheStats {
        self.metrics.cache_statistics(self.cache_sizes())
    }

    pub fn get_runtime_metrics_snapshot(&self) -> RuntimeMetricsSnapshot {
        let cache_sizes = self.cache_sizes();
        let cache_stats = self.metrics.cache_statistics(cache_sizes);
        self.metrics.snapshot(cache_stats, cache_sizes)
    }

    fn cache_sizes(&self) -> CacheSizes {
        CacheSizes {
            layout_size: self.caches.layout_cache.len(),
            layout_bytes: estimated_dashmap_bytes(&self.caches.layout_cache),
            vmt_size: self.caches.vmt_cache.len(),
            vmt_bytes: estimated_dashmap_bytes(&self.caches.vmt_cache),
            intrinsic_size: self.caches.intrinsic_cache.len(),
            intrinsic_bytes: estimated_dashmap_bytes(&self.caches.intrinsic_cache),
            intrinsic_field_size: self.caches.intrinsic_field_cache.len(),
            intrinsic_field_bytes: estimated_dashmap_bytes(&self.caches.intrinsic_field_cache),
            hierarchy_size: self.caches.hierarchy_cache.len(),
            hierarchy_bytes: estimated_dashmap_bytes(&self.caches.hierarchy_cache),
            static_field_layout_size: self.caches.static_field_layout_cache.len(),
            static_field_layout_bytes: estimated_dashmap_bytes(
                &self.caches.static_field_layout_cache,
            ),
            instance_field_layout_size: self.caches.instance_field_layout_cache.len(),
            instance_field_layout_bytes: estimated_dashmap_bytes(
                &self.caches.instance_field_layout_cache,
            ),
            value_type_size: self.caches.value_type_cache.len(),
            value_type_bytes: estimated_dashmap_bytes(&self.caches.value_type_cache),
            has_finalizer_size: self.caches.has_finalizer_cache.len(),
            has_finalizer_bytes: estimated_dashmap_bytes(&self.caches.has_finalizer_cache),
            overrides_size: self.caches.overrides_cache.len(),
            overrides_bytes: estimated_dashmap_bytes(&self.caches.overrides_cache),
            method_info_size: self.caches.method_info_cache.len(),
            method_info_bytes: estimated_dashmap_bytes(&self.caches.method_info_cache),
            assembly_type_info: (
                self.loader.type_cache_hits.load(Ordering::Relaxed),
                self.loader.type_cache_misses.load(Ordering::Relaxed),
                self.loader.type_cache_size(),
            ),
            assembly_method_info: (
                self.loader.method_cache_hits.load(Ordering::Relaxed),
                self.loader.method_cache_misses.load(Ordering::Relaxed),
                self.loader.method_cache_size(),
            ),
            #[cfg(feature = "multithreading")]
            shared_runtime_types_size: self.shared_runtime_types.len(),
            #[cfg(not(feature = "multithreading"))]
            shared_runtime_types_size: 0,
            #[cfg(feature = "multithreading")]
            shared_runtime_types_bytes: estimated_dashmap_bytes(&self.shared_runtime_types),
            #[cfg(not(feature = "multithreading"))]
            shared_runtime_types_bytes: 0,
            #[cfg(feature = "multithreading")]
            shared_runtime_methods_size: self.shared_runtime_methods.len(),
            #[cfg(not(feature = "multithreading"))]
            shared_runtime_methods_size: 0,
            #[cfg(feature = "multithreading")]
            shared_runtime_methods_bytes: estimated_dashmap_bytes(&self.shared_runtime_methods),
            #[cfg(not(feature = "multithreading"))]
            shared_runtime_methods_bytes: 0,
            #[cfg(feature = "multithreading")]
            shared_runtime_fields_size: self.shared_runtime_fields.len(),
            #[cfg(not(feature = "multithreading"))]
            shared_runtime_fields_size: 0,
            #[cfg(feature = "multithreading")]
            shared_runtime_fields_bytes: estimated_dashmap_bytes(&self.shared_runtime_fields),
            #[cfg(not(feature = "multithreading"))]
            shared_runtime_fields_bytes: 0,
        }
    }

    pub fn resolution_shared(self: &Arc<Self>) -> Arc<crate::context::ResolutionShared> {
        self.resolution_shared_cache
            .get_or_init(|| {
                Arc::new(crate::context::ResolutionShared::new(
                    self.loader.clone(),
                    self.caches.clone(),
                    Some(Arc::downgrade(self)),
                ))
            })
            .clone()
    }
}

impl dotnet_runtime_memory::MemoryOrderingHost for SharedGlobalState {
    fn tracer_enabled_relaxed(&self) -> bool {
        self.tracer_enabled.load(Ordering::Relaxed)
    }
}

impl dotnet_runtime_memory::MemorySharedStateHost for SharedGlobalState {
    fn trace_gc_resurrection(&self, indent: usize, obj_type_name: &str, addr: usize) {
        self.tracer
            .trace_gc_resurrection(indent, obj_type_name, addr);
    }
}

impl Drop for SharedGlobalState {
    fn drop(&mut self) {
        let stats = self.get_cache_stats();
        if self.tracer_enabled.load(Ordering::Relaxed) {
            self.tracer
                .msg(TraceLevel::Info, 0, format_args!("{}", stats));
        }
    }
}

/// GC-managed state local to a single thread's arena.
pub struct ArenaLocalState<'gc> {
    pub heap: HeapManager<'gc>,
    pub statics: Arc<StaticStorageManager>,
    pub runtime_asms: RefCell<HashMap<ResolutionS, ObjectRef<'gc>>>,
    pub runtime_types: RefCell<HashMap<RuntimeType, ObjectRef<'gc>>>,
    pub runtime_types_list: RefCell<Vec<RuntimeType>>,
    pub runtime_methods: RefCell<Vec<(MethodDescription, GenericLookup)>>,
    pub runtime_method_objs: RefCell<HashMap<(MethodDescription, GenericLookup), ObjectRef<'gc>>>,
    pub runtime_fields: RefCell<Vec<(FieldDescription, GenericLookup)>>,
    pub runtime_field_objs: RefCell<HashMap<(FieldDescription, GenericLookup), ObjectRef<'gc>>>,
    pub active_borrows: Cell<usize>,
}

// SAFETY: `ArenaLocalState` correctly traces all GC-managed fields in its `trace` implementation.
// This includes the `heap`, the global `statics`, and all `ObjectRef<'gc>` values stored in the
// various RefCell-wrapped collections.
unsafe impl<'gc> Collect<'gc> for ArenaLocalState<'gc> {
    fn trace<Tr: Trace<'gc>>(&self, cc: &mut Tr) {
        self.heap.trace(cc);
        self.statics.trace(cc);
        for o in self.runtime_asms.borrow().values() {
            o.trace(cc);
        }
        for o in self.runtime_types.borrow().values() {
            o.trace(cc);
        }
        for o in self.runtime_method_objs.borrow().values() {
            o.trace(cc);
        }
        for o in self.runtime_field_objs.borrow().values() {
            o.trace(cc);
        }
    }
}

impl<'gc> ArenaLocalState<'gc> {
    pub fn new(statics: Arc<StaticStorageManager>) -> Self {
        Self {
            heap: HeapManager::new(),
            statics,
            runtime_asms: RefCell::new(HashMap::new()),
            runtime_types: RefCell::new(HashMap::new()),
            runtime_types_list: RefCell::new(vec![]),
            runtime_methods: RefCell::new(vec![]),
            runtime_method_objs: RefCell::new(HashMap::new()),
            runtime_fields: RefCell::new(vec![]),
            runtime_field_objs: RefCell::new(HashMap::new()),
            active_borrows: Cell::new(0),
        }
    }
}

pub struct ReflectionRegistry<'a, 'gc> {
    local: &'a ArenaLocalState<'gc>,
}

impl<'a, 'gc> ReflectionRegistry<'a, 'gc> {
    pub fn new(local: &'a ArenaLocalState<'gc>) -> Self {
        Self { local }
    }

    pub fn asms_read(&self) -> Ref<'a, HashMap<ResolutionS, ObjectRef<'gc>>> {
        self.local.runtime_asms.borrow()
    }

    pub fn asms_write(&self) -> RefMut<'a, HashMap<ResolutionS, ObjectRef<'gc>>> {
        self.local.runtime_asms.borrow_mut()
    }

    pub fn types_read(&self) -> Ref<'a, HashMap<RuntimeType, ObjectRef<'gc>>> {
        self.local.runtime_types.borrow()
    }

    pub fn types_write(&self) -> RefMut<'a, HashMap<RuntimeType, ObjectRef<'gc>>> {
        self.local.runtime_types.borrow_mut()
    }

    pub fn types_list_read(&self) -> Ref<'a, Vec<RuntimeType>> {
        self.local.runtime_types_list.borrow()
    }

    pub fn types_list_write(&self) -> RefMut<'a, Vec<RuntimeType>> {
        self.local.runtime_types_list.borrow_mut()
    }

    pub fn methods_read(&self) -> Ref<'a, Vec<(MethodDescription, GenericLookup)>> {
        self.local.runtime_methods.borrow()
    }

    pub fn methods_write(&self) -> RefMut<'a, Vec<(MethodDescription, GenericLookup)>> {
        self.local.runtime_methods.borrow_mut()
    }

    pub fn method_objs_read(
        &self,
    ) -> Ref<'a, HashMap<(MethodDescription, GenericLookup), ObjectRef<'gc>>> {
        self.local.runtime_method_objs.borrow()
    }

    pub fn method_objs_write(
        &self,
    ) -> RefMut<'a, HashMap<(MethodDescription, GenericLookup), ObjectRef<'gc>>> {
        self.local.runtime_method_objs.borrow_mut()
    }

    pub fn fields_read(&self) -> Ref<'a, Vec<(FieldDescription, GenericLookup)>> {
        self.local.runtime_fields.borrow()
    }

    pub fn fields_write(&self) -> RefMut<'a, Vec<(FieldDescription, GenericLookup)>> {
        self.local.runtime_fields.borrow_mut()
    }

    pub fn field_objs_read(
        &self,
    ) -> Ref<'a, HashMap<(FieldDescription, GenericLookup), ObjectRef<'gc>>> {
        self.local.runtime_field_objs.borrow()
    }

    pub fn field_objs_write(
        &self,
    ) -> RefMut<'a, HashMap<(FieldDescription, GenericLookup), ObjectRef<'gc>>> {
        self.local.runtime_field_objs.borrow_mut()
    }
}

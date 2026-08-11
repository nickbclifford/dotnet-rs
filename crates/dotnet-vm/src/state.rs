use crate::{
    MethodInfo,
    cache::{DEFAULT_CAPACITY, FrontCache, FrontCachePolicy, LockedCache, ShardedCache},
    dispatch::ring_buffer::InstructionRingBuffer,
    error::TypeResolutionError,
    gc::GCCoordinator,
    intrinsics::IntrinsicRegistry,
    sync::SyncBlockManager,
    threading::ThreadManager,
};
use dashmap::DashMap;
use dotnet_assemblies::AssemblyLoader;
use dotnet_metrics::{
    CacheKind, CacheSize, CacheSizes, CacheStats, RuntimeMetrics, RuntimeMetricsSnapshot,
};
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
    string::parse_env_bool,
};
use gc_arena::{Collect, collect::Trace};
use std::{
    cell::{Cell, Ref, RefCell, RefMut},
    collections::HashMap,
    env,
    sync::OnceLock,
};
#[cfg(feature = "multithreading")]
use {dotnet_metrics::ArenaGcPressureSnapshot, std::mem::size_of};

#[cfg(feature = "multithreading")]
use dotnet_utils::sync::AtomicUsize;

pub use crate::statics::StaticStorageManager;

fn parse_env_usize(key: &str) -> Option<usize> {
    env::var(key)
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .filter(|v| *v > 0)
}

/// Expands a cache capacity configuration from the global-cache registry.
macro_rules! global_cache_capacity {
    (unbounded, $method_info:ident, $vmt:ident, $hierarchy:ident $(,)?) => {
        None
    };
    (method_info, $method_info:ident, $vmt:ident, $hierarchy:ident $(,)?) => {
        $method_info
    };
    (vmt, $method_info:ident, $vmt:ident, $hierarchy:ident $(,)?) => {
        $vmt
    };
    (hierarchy, $method_info:ident, $vmt:ident, $hierarchy:ident $(,)?) => {
        $hierarchy
    };
}

/// Expands a front-cache configuration from the global-cache registry.
macro_rules! global_cache_front_policy {
    ((none), $enabled:ident, $capacity:ident $(,)?) => {
        None
    };
    ((configured, $tls:ident, $key:ty, $value:ty), $enabled:ident, $capacity:ident $(,)?) => {
        Some(FrontCachePolicy::new($enabled, $capacity))
    };
}

/// Emits the typed TLS companion coupled to a configured front-cache policy.
macro_rules! global_cache_tls {
    ((none)) => {};
    ((configured, $tls:ident, $key:ty, $value:ty)) => {
        thread_local! {
            pub(crate) static $tls: RefCell<FrontCache<$key, $value>> =
                RefCell::new(FrontCache::default());
        }
    };
}

/// Defines the single registry of VM-global caches.
///
/// Each entry owns the cross-cutting declaration, construction, size-reporting, and optional
/// typed TLS companion work. `IntrinsicRegistry` remains an ordinary `GlobalCaches` field
/// outside this registry.
macro_rules! define_global_caches {
    ($(
        $(#[$meta:meta])*
        $field:ident: $cache:ty => {
            kind: $kind:expr,
            capacity: $capacity:ident,
            front: $front:tt
        };
    )+) => {
        /// Grouped caches for type resolution and layout computation.
        /// This struct reduces the API surface area of ResolutionContext.
        pub struct GlobalCaches {
            $(
                $(#[$meta])*
                pub(crate) $field: $cache,
            )+
            /// Registry of intrinsic methods.
            pub intrinsic_registry: IntrinsicRegistry,
        }

        impl GlobalCaches {
            pub fn new(
                _loader: &AssemblyLoader,
                _tracer: &Tracer,
                metrics: Arc<RuntimeMetrics>,
            ) -> Self {
                let intrinsic_registry = IntrinsicRegistry::initialize();
                let method_info_capacity = parse_env_usize("DOTNET_CACHE_LIMIT_METHOD_INFO");
                let vmt_capacity = parse_env_usize("DOTNET_CACHE_LIMIT_VMT");
                let hierarchy_capacity = parse_env_usize("DOTNET_CACHE_LIMIT_HIERARCHY");
                let front_cache_enabled = parse_env_bool("DOTNET_FRONT_CACHE_ENABLED", true);
                let front_cache_capacity = parse_env_usize("DOTNET_FRONT_CACHE_CAPACITY")
                    .unwrap_or(DEFAULT_CAPACITY);
                Self {
                    $(
                        $field: <$cache>::new(
                            $kind,
                            Arc::clone(&metrics),
                            global_cache_capacity!(
                                $capacity,
                                method_info_capacity,
                                vmt_capacity,
                                hierarchy_capacity,
                            ),
                            global_cache_front_policy!(
                                $front,
                                front_cache_enabled,
                                front_cache_capacity,
                            ),
                        ),
                    )+
                    intrinsic_registry,
                }
            }

            /// Iterates over the one report for each cache declared in this registry.
            fn cache_size_reports(
                &self,
            ) -> impl Iterator<Item = (CacheKind, CacheSize)> + '_ {
                [$(self.$field.size_report(),)+].into_iter()
            }
        }

        $(global_cache_tls!($front);)+
    };
}

define_global_caches! {
    /// Cache for type layouts: `ConcreteType` -> `Arc<LayoutManager>`.
    layout_cache: ShardedCache<ConcreteType, Arc<LayoutManager>> => {
        kind: CacheKind::Layout,
        capacity: unbounded,
        front: (none)
    };
    /// Cache for virtual dispatch: `(base_method, this_type, generics)` -> resolved method.
    vmt_cache: ShardedCache<
        (MethodDescription, TypeDescription, GenericLookup),
        MethodDescription,
    > => {
        kind: CacheKind::Vmt,
        capacity: vmt,
        front: (
            configured,
            VMT_FRONT_CACHE,
            (MethodDescription, TypeDescription, GenericLookup),
            MethodDescription
        )
    };
    /// Cache for intrinsic checks: method -> `is_intrinsic`.
    intrinsic_cache: LockedCache<MethodDescription, bool> => {
        kind: CacheKind::Intrinsic,
        capacity: unbounded,
        front: (none)
    };
    /// Cache for intrinsic field checks: field -> `is_intrinsic`.
    intrinsic_field_cache: LockedCache<FieldDescription, bool> => {
        kind: CacheKind::IntrinsicField,
        capacity: unbounded,
        front: (none)
    };
    /// Cache for type hierarchy checks: `(child, parent)` -> `is_a` result.
    hierarchy_cache: ShardedCache<(ConcreteType, ConcreteType), bool> => {
        kind: CacheKind::Hierarchy,
        capacity: hierarchy,
        front: (configured, HIERARCHY_FRONT_CACHE, (ConcreteType, ConcreteType), bool)
    };
    /// Cache for static field layouts: descriptor and generics -> `Arc<FieldLayoutManager>`.
    static_field_layout_cache: ShardedCache<
        (TypeDescription, GenericLookup),
        Arc<FieldLayoutManager>,
    > => {
        kind: CacheKind::StaticFieldLayout,
        capacity: unbounded,
        front: (none)
    };
    /// Cache for instance field layouts: descriptor and generics -> `Arc<FieldLayoutManager>`.
    instance_field_layout_cache: ShardedCache<
        (TypeDescription, GenericLookup),
        Arc<FieldLayoutManager>,
    > => {
        kind: CacheKind::InstanceFieldLayout,
        capacity: unbounded,
        front: (none)
    };
    /// Cache for value-type checks: `TypeDescription` -> `bool`.
    value_type_cache: LockedCache<TypeDescription, bool> => {
        kind: CacheKind::ValueType,
        capacity: unbounded,
        front: (none)
    };
    /// Cache for finalizer checks: `TypeDescription` -> `bool`.
    has_finalizer_cache: LockedCache<TypeDescription, bool> => {
        kind: CacheKind::HasFinalizer,
        capacity: unbounded,
        front: (none)
    };
    /// Cache for resolved overrides: descriptor and generics -> declaration/implementation map.
    overrides_cache: ShardedCache<
        (TypeDescription, GenericLookup),
        Arc<HashMap<MethodDescription, MethodDescription>>,
    > => {
        kind: CacheKind::Overrides,
        capacity: unbounded,
        front: (none)
    };
    /// Cache for method info: method and generics -> `Arc<MethodInfo<'static>>`.
    method_info_cache: ShardedCache<
        (MethodDescription, GenericLookup),
        Arc<MethodInfo<'static>>,
    > => {
        kind: CacheKind::MethodInfo,
        capacity: method_info,
        front: (
            configured,
            METHOD_INFO_FRONT_CACHE,
            (MethodDescription, GenericLookup),
            Arc<MethodInfo<'static>>
        )
    };
    /// Cache for bodyless resolved-method delegate dispatch classification.
    delegate_dispatch_cache: LockedCache<
        MethodDescription,
        dotnet_intrinsics_delegates::DelegateDispatchKind,
    > => {
        kind: CacheKind::DelegateDispatch,
        capacity: unbounded,
        front: (none)
    };
    /// Exact static constrained metadata: `(kind, constraint, base method, source lookup)`.
    /// It is intentionally separate from the VMT, which remains the sole virtual-target cache.
    static_constrained_cache: ShardedCache<
        dotnet_runtime_resolver::StaticConstrainedCacheKey,
        MethodDescription,
    > => {
        kind: CacheKind::StaticConstrained,
        capacity: unbounded,
        front: (none)
    };
}

#[cfg(feature = "multithreading")]
pub struct SharedReflectionRegistry {
    pub runtime_types: DashMap<RuntimeType, usize>,
    pub runtime_types_rev: DashMap<usize, RuntimeType>,
    pub next_type_index: AtomicUsize,
    pub runtime_methods: DashMap<(MethodDescription, GenericLookup), usize>,
    pub runtime_methods_rev: DashMap<usize, (MethodDescription, GenericLookup)>,
    pub next_method_index: AtomicUsize,
    pub runtime_fields: DashMap<(FieldDescription, GenericLookup), usize>,
    pub runtime_fields_rev: DashMap<usize, (FieldDescription, GenericLookup)>,
    pub next_field_index: AtomicUsize,
}

#[cfg(feature = "multithreading")]
impl SharedReflectionRegistry {
    fn new() -> Self {
        Self {
            runtime_types: DashMap::new(),
            runtime_types_rev: DashMap::new(),
            next_type_index: AtomicUsize::new(0),
            runtime_methods: DashMap::new(),
            runtime_methods_rev: DashMap::new(),
            next_method_index: AtomicUsize::new(0),
            runtime_fields: DashMap::new(),
            runtime_fields_rev: DashMap::new(),
            next_field_index: AtomicUsize::new(0),
        }
    }

    fn cache_size_reports(&self) -> [(CacheKind, CacheSize); 3] {
        let runtime_types_entries = self.runtime_types.len();
        let runtime_methods_entries = self.runtime_methods.len();
        let runtime_fields_entries = self.runtime_fields.len();
        [
            (
                CacheKind::SharedRuntimeTypes,
                CacheSize {
                    entries: runtime_types_entries,
                    pointer_bytes: (runtime_types_entries as u64)
                        .saturating_mul((size_of::<RuntimeType>() + size_of::<usize>()) as u64),
                },
            ),
            (
                CacheKind::SharedRuntimeMethods,
                CacheSize {
                    entries: runtime_methods_entries,
                    pointer_bytes: (runtime_methods_entries as u64).saturating_mul(
                        (size_of::<(MethodDescription, GenericLookup)>() + size_of::<usize>())
                            as u64,
                    ),
                },
            ),
            (
                CacheKind::SharedRuntimeFields,
                CacheSize {
                    entries: runtime_fields_entries,
                    pointer_bytes: (runtime_fields_entries as u64).saturating_mul(
                        (size_of::<(FieldDescription, GenericLookup)>() + size_of::<usize>())
                            as u64,
                    ),
                },
            ),
        ]
    }
}

/// A clonable handle used to request executor abortion without sharing VM state.
#[derive(Clone, Debug)]
pub struct AbortSignal(Arc<AtomicBool>);

impl AbortSignal {
    /// Requests that the associated executor stop at its next abort check.
    pub fn request_abort(&self) {
        self.0.store(true, Ordering::Relaxed);
    }
}

#[cfg(test)]
static_assertions::assert_impl_all!(AbortSignal: Send, Sync);

/// Global VM state that does not contain any GC-managed pointers.
///
/// Multithreaded builds share this state across execution threads and arenas. In no-MT builds it
/// is deliberately `!Send` and executor-confined; only an [`AbortSignal`] may cross threads.
pub struct SharedGlobalState {
    pub loader: Arc<AssemblyLoader>,
    pub pinvoke: NativeLibraries,
    pub sync_blocks: SyncBlockManager,
    pub thread_manager: Arc<ThreadManager>,
    pub metrics: Arc<RuntimeMetrics>,
    pub tracer: Tracer,
    pub tracer_enabled: Arc<AtomicBool>,
    pub empty_generics: GenericLookup,
    /// Grouped caches for type resolution and layout computation
    pub caches: Arc<GlobalCaches>,
    pub statics: Arc<StaticStorageManager>,
    pub last_instructions: Arc<Mutex<InstructionRingBuffer>>,
    abort_requested: Arc<AtomicBool>,
    pub gc_coordinator: Arc<GCCoordinator>,
    #[cfg(feature = "multithreading")]
    pub reflection_registry: SharedReflectionRegistry,
    pub resolution_shared_cache: OnceLock<Arc<crate::context::ResolutionShared>>,
    pub app_context_switches: DashMap<String, bool>,
}

#[cfg(all(test, not(feature = "multithreading")))]
static_assertions::assert_not_impl_all!(SharedGlobalState: Send);

impl GlobalCaches {
    pub fn get_method_info(
        &self,
        method: MethodDescription,
        generics: &GenericLookup,
        shared: Arc<SharedGlobalState>,
    ) -> Result<MethodInfo<'static>, TypeResolutionError> {
        self.method_info_cache.record_key_clones(2);
        let key = (method.clone(), generics.clone());
        if let Some(cached) = self
            .method_info_cache
            .try_get_with_front(&key, &METHOD_INFO_FRONT_CACHE)
        {
            return Ok((*cached).clone());
        }
        let built = crate::build_method_info(method, generics, shared.clone())?;
        let built_arc = Arc::new(built.clone());
        if self.method_info_cache.front_cache_enabled() {
            // Retaining the compound key for the L1 fill makes the L2 insertion clone both key
            // components; the disabled path moves the key directly into L2.
            self.method_info_cache.record_key_clones(2);
        }
        self.method_info_cache
            .insert_with_front(key, built_arc, &METHOD_INFO_FRONT_CACHE);
        Ok(built)
    }
}

impl SharedGlobalState {
    pub fn new(loader: Arc<AssemblyLoader>) -> Self {
        let tracer = Tracer::new();
        let metrics = Arc::new(RuntimeMetrics::new());
        #[allow(
            clippy::arc_with_non_send_sync,
            reason = "no-MT global caches are executor-confined; Arc preserves feature-neutral ownership"
        )]
        let caches = Arc::new(GlobalCaches::new(&loader, &tracer, Arc::clone(&metrics)));

        let tracer_enabled = Arc::new(AtomicBool::new(tracer.is_enabled()));

        let stw_in_progress = Arc::new(AtomicBool::new(false));

        #[allow(
            clippy::arc_with_non_send_sync,
            reason = "no-MT static storage is executor-confined; Arc preserves feature-neutral ownership"
        )]
        let statics = Arc::new(StaticStorageManager::new());
        #[allow(
            clippy::arc_with_non_send_sync,
            reason = "no-MT instruction history is executor-confined; Arc preserves feature-neutral ownership"
        )]
        let last_instructions = Arc::new(Mutex::new(InstructionRingBuffer::new()));

        let state = Self {
            pinvoke: {
                let p = NativeLibraries::new(loader.get_root())
                    .with_search_dirs(loader.native_search_dirs());
                #[cfg(feature = "fuzzing")]
                let p = p.with_sandbox(Arc::new(dotnet_pinvoke::DenySandbox));
                p
            },
            loader,
            sync_blocks: SyncBlockManager::new(),
            thread_manager: ThreadManager::new(stw_in_progress.clone()),
            metrics,
            tracer,
            tracer_enabled,
            empty_generics: GenericLookup::default(),
            caches,
            statics,
            last_instructions,
            abort_requested: Arc::new(AtomicBool::new(false)),
            gc_coordinator: Arc::new(GCCoordinator::new(stw_in_progress.clone())),
            #[cfg(feature = "multithreading")]
            reflection_registry: SharedReflectionRegistry::new(),
            resolution_shared_cache: OnceLock::new(),
            app_context_switches: {
                let switches = DashMap::new();
                switches.insert("System.Globalization.Invariant".to_string(), true);
                switches.insert(
                    "System.Globalization.PredefinedCulturesOnly".to_string(),
                    true,
                );
                switches
            },
        };

        state
            .thread_manager
            .set_coordinator(Arc::downgrade(&state.gc_coordinator));

        state
    }

    /// Returns the narrow thread-safe handle for requesting executor cancellation.
    pub fn abort_signal(&self) -> AbortSignal {
        AbortSignal(Arc::clone(&self.abort_requested))
    }

    /// Returns whether an abort has been requested for this executor.
    pub(crate) fn is_abort_requested(&self) -> bool {
        self.abort_requested.load(Ordering::Relaxed)
    }

    pub fn get_cache_stats(&self) -> CacheStats {
        self.metrics.cache_statistics(self.cache_sizes())
    }

    pub fn get_runtime_metrics_snapshot(&self) -> RuntimeMetricsSnapshot {
        #[cfg(feature = "multithreading")]
        let gc_pressure_by_arena = self
            .gc_coordinator
            .arena_gc_pressure_snapshot()
            .into_iter()
            .map(|(arena_id, metrics)| {
                (
                    arena_id.to_string(),
                    ArenaGcPressureSnapshot {
                        collection_trigger_count: metrics.collection_trigger_count,
                        peak_allocation_counter: metrics.peak_allocation_counter,
                    },
                )
            })
            .collect();
        #[cfg(not(feature = "multithreading"))]
        let gc_pressure_by_arena = std::collections::BTreeMap::new();

        self.metrics
            .update_arena_gc_pressure_metrics(gc_pressure_by_arena);

        let cache_sizes = self.cache_sizes();
        let cache_stats = self.metrics.cache_statistics(cache_sizes);
        self.metrics.snapshot(cache_stats, cache_sizes)
    }

    fn cache_sizes(&self) -> CacheSizes {
        let mut caches = [CacheSize::default(); CacheKind::COUNT];
        for (kind, size) in self.caches.cache_size_reports() {
            caches[kind.as_index()] = size;
        }

        #[cfg(feature = "multithreading")]
        for (kind, size) in self.reflection_registry.cache_size_reports() {
            caches[kind.as_index()] = size;
        }

        CacheSizes {
            caches,
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
        }
    }

    pub fn resolution_shared(self: &Arc<Self>) -> Arc<crate::context::ResolutionShared> {
        self.resolution_shared_cache
            .get_or_init(|| {
                #[allow(
                    clippy::arc_with_non_send_sync,
                    reason = "no-MT resolver state is executor-confined; Arc preserves feature-neutral ownership"
                )]
                let shared = Arc::new(crate::context::ResolutionShared::new(
                    self.loader.clone(),
                    self.caches.clone(),
                    Some(Arc::downgrade(self)),
                ));
                shared
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
        // Only pay cache-lock costs when the tracer is actually consuming the stats.
        if self.tracer_enabled.load(Ordering::Relaxed) {
            let stats = self.get_cache_stats();
            self.tracer
                .msg(TraceLevel::Info, 0, format_args!("{}", stats));
        }
    }
}

/// Thread-local reflection caches stored inside the GC arena root.
///
/// The `RefCell` fields here are intentional and load-bearing for the `gc_arena` model:
/// `Collect::trace` is called with `&self`, and reflection operations frequently reach this
/// state through shared borrows (for example trait methods that only take `&self`). We still
/// need to mutate these caches on that path, so interior mutability is required.
///
/// In other words, these `RefCell`s are not borrow-checker convenience; they are what allows
/// this type to be both `Collect`-traceable and writable from shared arena access patterns.
pub struct ReflectionLocalState<'gc> {
    pub runtime_asms: RefCell<HashMap<ResolutionS, ObjectRef<'gc>>>,
    pub runtime_asm_resolutions: RefCell<HashMap<ObjectRef<'gc>, ResolutionS>>,
    pub runtime_types: RefCell<HashMap<RuntimeType, ObjectRef<'gc>>>,
    pub runtime_types_list: RefCell<Vec<RuntimeType>>,
    pub runtime_methods: RefCell<Vec<(MethodDescription, GenericLookup)>>,
    pub runtime_method_objs: RefCell<HashMap<(MethodDescription, GenericLookup), ObjectRef<'gc>>>,
    pub runtime_fields: RefCell<Vec<(FieldDescription, GenericLookup)>>,
    pub runtime_field_objs: RefCell<HashMap<(FieldDescription, GenericLookup), ObjectRef<'gc>>>,
    pub runtime_property_objs: RefCell<HashMap<(MethodDescription, GenericLookup), ObjectRef<'gc>>>,
}

impl<'gc> ReflectionLocalState<'gc> {
    pub fn new() -> Self {
        Self {
            runtime_asms: RefCell::new(HashMap::new()),
            runtime_asm_resolutions: RefCell::new(HashMap::new()),
            runtime_types: RefCell::new(HashMap::new()),
            runtime_types_list: RefCell::new(vec![]),
            runtime_methods: RefCell::new(vec![]),
            runtime_method_objs: RefCell::new(HashMap::new()),
            runtime_fields: RefCell::new(vec![]),
            runtime_field_objs: RefCell::new(HashMap::new()),
            runtime_property_objs: RefCell::new(HashMap::new()),
        }
    }
}

impl<'gc> Default for ReflectionLocalState<'gc> {
    fn default() -> Self {
        Self::new()
    }
}

// SAFETY: F5.TracesEveryGcRef — `ReflectionLocalState` traces every `ObjectRef<'gc>` stored in its reflection cache
// maps. The companion vectors (`runtime_types_list`, `runtime_methods`, `runtime_fields`) contain
// no GC-managed references.
unsafe impl<'gc> Collect<'gc> for ReflectionLocalState<'gc> {
    fn trace<Tr: Trace<'gc>>(&self, cc: &mut Tr) {
        for o in self.runtime_asms.borrow().values() {
            o.trace(cc);
        }
        for o in self.runtime_asm_resolutions.borrow().keys() {
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
        for o in self.runtime_property_objs.borrow().values() {
            o.trace(cc);
        }
    }
}

/// GC-managed state local to a single thread's arena.
pub struct ArenaLocalState<'gc> {
    pub heap: HeapManager<'gc>,
    pub reflection: ReflectionLocalState<'gc>,
    pub active_borrows: Cell<usize>,
}

// SAFETY: F5.TracesEveryGcRef — `ArenaLocalState` correctly traces all GC-managed fields in its `trace` implementation.
// This includes the `heap` and the nested `reflection` cache state.
unsafe impl<'gc> Collect<'gc> for ArenaLocalState<'gc> {
    fn trace<Tr: Trace<'gc>>(&self, cc: &mut Tr) {
        self.heap.trace(cc);
        self.reflection.trace(cc);
    }
}

impl<'gc> ArenaLocalState<'gc> {
    pub fn new() -> Self {
        Self {
            heap: HeapManager::new(),
            reflection: ReflectionLocalState::new(),
            active_borrows: Cell::new(0),
        }
    }
}

impl<'gc> Default for ArenaLocalState<'gc> {
    fn default() -> Self {
        Self::new()
    }
}

pub struct ReflectionRegistry<'a, 'gc> {
    local: &'a ReflectionLocalState<'gc>,
}

impl<'a, 'gc> ReflectionRegistry<'a, 'gc> {
    pub fn new(local: &'a ReflectionLocalState<'gc>) -> Self {
        Self { local }
    }

    pub fn asms_read(&self) -> Ref<'a, HashMap<ResolutionS, ObjectRef<'gc>>> {
        self.local.runtime_asms.borrow()
    }

    pub fn asms_write(&self) -> RefMut<'a, HashMap<ResolutionS, ObjectRef<'gc>>> {
        self.local.runtime_asms.borrow_mut()
    }

    pub fn asm_resolutions_read(&self) -> Ref<'a, HashMap<ObjectRef<'gc>, ResolutionS>> {
        self.local.runtime_asm_resolutions.borrow()
    }

    pub fn asm_resolutions_write(&self) -> RefMut<'a, HashMap<ObjectRef<'gc>, ResolutionS>> {
        self.local.runtime_asm_resolutions.borrow_mut()
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

    pub fn property_objs_read(
        &self,
    ) -> Ref<'a, HashMap<(MethodDescription, GenericLookup), ObjectRef<'gc>>> {
        self.local.runtime_property_objs.borrow()
    }

    pub fn property_objs_write(
        &self,
    ) -> RefMut<'a, HashMap<(MethodDescription, GenericLookup), ObjectRef<'gc>>> {
        self.local.runtime_property_objs.borrow_mut()
    }
}

#[cfg(test)]
mod abort_signal_tests {
    use super::*;

    #[test]
    fn cloned_signal_requests_abort_on_shared_flag() {
        let requested = Arc::new(AtomicBool::new(false));
        let signal = AbortSignal(Arc::clone(&requested));

        signal.clone().request_abort();

        assert!(requested.load(Ordering::Relaxed));
    }
}

#[cfg(test)]
mod global_cache_registry_tests {
    use super::*;

    #[test]
    fn construction_reports_every_registry_cache_once() {
        let loader = AssemblyLoader::new_bare("global-cache-registry-test".to_owned())
            .expect("bare assembly loader must initialize");
        let caches = GlobalCaches::new(&loader, &Tracer::new(), Arc::new(RuntimeMetrics::new()));

        let reports = caches.cache_size_reports().collect::<Vec<_>>();
        assert_eq!(
            reports.iter().map(|(kind, _)| *kind).collect::<Vec<_>>(),
            CacheKind::GLOBAL
        );
        assert!(
            reports
                .iter()
                .all(|(_, size)| size.entries == 0 && size.pointer_bytes == 0)
        );

        assert_eq!(caches.layout_cache.front_cache_capacity(), None);
        assert_eq!(caches.intrinsic_cache.front_cache_capacity(), None);
        assert_eq!(caches.intrinsic_field_cache.front_cache_capacity(), None);
        assert!(caches.hierarchy_cache.front_cache_capacity().is_some());
        assert!(caches.vmt_cache.front_cache_capacity().is_some());
        assert_eq!(
            caches.static_field_layout_cache.front_cache_capacity(),
            None
        );
        assert_eq!(
            caches.instance_field_layout_cache.front_cache_capacity(),
            None
        );
        assert_eq!(caches.value_type_cache.front_cache_capacity(), None);
        assert_eq!(caches.has_finalizer_cache.front_cache_capacity(), None);
        assert_eq!(caches.overrides_cache.front_cache_capacity(), None);
        assert!(caches.method_info_cache.front_cache_capacity().is_some());
        assert_eq!(caches.delegate_dispatch_cache.front_cache_capacity(), None);
    }
}

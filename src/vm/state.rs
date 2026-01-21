use crate::{
    assemblies::AssemblyLoader,
    types::{
        generics::{ConcreteType, GenericLookup},
        members::{FieldDescription, MethodDescription},
        runtime::RuntimeType,
        TypeDescription,
    },
    utils::{sync::{Arc, AtomicBool, Mutex, Ordering, RwLock, AtomicUsize}, ResolutionS},
    value::{
        layout::{FieldLayoutManager, LayoutManager},
        object::ObjectRef,
    },
};
use dashmap::DashMap;
use gc_arena::{Collect, Collection};
use std::{
    cell::{Cell, RefCell},
    collections::{HashMap, HashSet},
};

use super::{
    intrinsics::{is_intrinsic, is_intrinsic_field, IntrinsicRegistry},
    metrics::{CacheSizes, CacheStats, RuntimeMetrics},
    pinvoke::NativeLibraries,
    statics::StaticStorageManager,
    sync::SyncBlockManager,
    threading::ThreadManager,
    tracer::Tracer,
    HeapManager,
};

#[cfg(feature = "multithreaded-gc")]
use super::gc::GCCoordinator;

/// Grouped caches for type resolution and layout computation.
/// This struct reduces the API surface area of ResolutionContext.
pub struct GlobalCaches {
    /// Cache for type layouts: ConcreteType -> Arc<LayoutManager>
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
    /// Registry of intrinsic methods
    pub intrinsic_registry: IntrinsicRegistry,
}

impl GlobalCaches {
    pub fn new(loader: &AssemblyLoader, tracer: &mut Tracer) -> Self {
        let intrinsic_registry = IntrinsicRegistry::initialize(loader, Some(tracer));
        Self {
            layout_cache: DashMap::new(),
            instance_field_layout_cache: DashMap::new(),
            vmt_cache: DashMap::new(),
            hierarchy_cache: DashMap::new(),
            intrinsic_cache: DashMap::new(),
            intrinsic_field_cache: DashMap::new(),
            static_field_layout_cache: DashMap::new(),
            intrinsic_registry,
        }
    }
}

/// Thread-safe shared state that does not contain any GC-managed pointers.
/// This state is shared across all execution threads and arenas.
pub struct SharedGlobalState<'m> {
    pub loader: &'m AssemblyLoader,
    pub pinvoke: RwLock<NativeLibraries>,
    pub sync_blocks: SyncBlockManager,
    pub thread_manager: Arc<ThreadManager>,
    pub metrics: RuntimeMetrics,
    pub tracer: Mutex<Tracer>,
    pub tracer_enabled: Arc<AtomicBool>,
    pub empty_generics: GenericLookup,
    /// Grouped caches for type resolution and layout computation
    pub caches: Arc<GlobalCaches>,
    pub statics: StaticStorageManager,
    #[cfg(feature = "multithreaded-gc")]
    pub gc_coordinator: Arc<GCCoordinator>,
    /// Cache for shared reflection objects: RuntimeType -> index
    #[cfg(feature = "multithreaded-gc")]
    pub shared_runtime_types: DashMap<RuntimeType, usize>,
    #[cfg(feature = "multithreaded-gc")]
    pub shared_runtime_types_rev: DashMap<usize, RuntimeType>,
    #[cfg(feature = "multithreaded-gc")]
    pub next_runtime_type_index: AtomicUsize,
    /// Cache for shared method reflection objects: (Method, Lookup) -> index
    #[cfg(feature = "multithreaded-gc")]
    pub shared_runtime_methods: DashMap<(MethodDescription, GenericLookup), usize>,
    #[cfg(feature = "multithreaded-gc")]
    pub shared_runtime_methods_rev: DashMap<usize, (MethodDescription, GenericLookup)>,
    #[cfg(feature = "multithreaded-gc")]
    pub next_runtime_method_index: AtomicUsize,
    /// Cache for shared field reflection objects: (Field, Lookup) -> index
    #[cfg(feature = "multithreaded-gc")]
    pub shared_runtime_fields: DashMap<(FieldDescription, GenericLookup), usize>,
    #[cfg(feature = "multithreaded-gc")]
    pub shared_runtime_fields_rev: DashMap<usize, (FieldDescription, GenericLookup)>,
    #[cfg(feature = "multithreaded-gc")]
    pub next_runtime_field_index: AtomicUsize,
}

impl<'m> SharedGlobalState<'m> {
    pub fn new(loader: &'m AssemblyLoader) -> Self {
        let mut tracer = Tracer::new();
        let caches = Arc::new(GlobalCaches::new(loader, &mut tracer));

        // Pre-populate intrinsic caches
        for assembly in loader.assemblies() {
            for type_def in &assembly.definition().type_definitions {
                let td = TypeDescription::new(assembly, type_def);
                for method in &type_def.methods {
                    let md = MethodDescription {
                        parent: td,
                        method_resolution: td.resolution,
                        method,
                    };
                    caches
                        .intrinsic_cache
                        .insert(md, is_intrinsic(md, loader, &caches.intrinsic_registry));
                }
                for field in &type_def.fields {
                    let fd = FieldDescription {
                        parent: td,
                        field_resolution: td.resolution,
                        field,
                    };
                    caches.intrinsic_field_cache.insert(
                        fd,
                        is_intrinsic_field(fd, loader, &caches.intrinsic_registry),
                    );
                }
            }
        }

        let tracer_enabled = Arc::new(AtomicBool::new(tracer.is_enabled()));
        let this = Self {
            loader,
            pinvoke: RwLock::new(NativeLibraries::new(loader.get_root())),
            sync_blocks: SyncBlockManager::new(),
            thread_manager: ThreadManager::new(),
            metrics: RuntimeMetrics::new(),
            tracer: Mutex::new(tracer),
            tracer_enabled,
            empty_generics: GenericLookup::default(),
            caches,
            statics: StaticStorageManager::new(),
            #[cfg(feature = "multithreaded-gc")]
            gc_coordinator: Arc::new(GCCoordinator::new()),
            #[cfg(feature = "multithreaded-gc")]
            shared_runtime_types: DashMap::new(),
            #[cfg(feature = "multithreaded-gc")]
            shared_runtime_types_rev: DashMap::new(),
            #[cfg(feature = "multithreaded-gc")]
            next_runtime_type_index: AtomicUsize::new(0),
            #[cfg(feature = "multithreaded-gc")]
            shared_runtime_methods: DashMap::new(),
            #[cfg(feature = "multithreaded-gc")]
            shared_runtime_methods_rev: DashMap::new(),
            #[cfg(feature = "multithreaded-gc")]
            next_runtime_method_index: AtomicUsize::new(0),
            #[cfg(feature = "multithreaded-gc")]
            shared_runtime_fields: DashMap::new(),
            #[cfg(feature = "multithreaded-gc")]
            shared_runtime_fields_rev: DashMap::new(),
            #[cfg(feature = "multithreaded-gc")]
            next_runtime_field_index: AtomicUsize::new(0),
        };

        this
    }

    pub fn get_cache_stats(&self) -> CacheStats {
        self.metrics.cache_statistics(CacheSizes {
            layout_size: self.caches.layout_cache.len(),
            vmt_size: self.caches.vmt_cache.len(),
            intrinsic_size: self.caches.intrinsic_cache.len(),
            intrinsic_field_size: self.caches.intrinsic_field_cache.len(),
            hierarchy_size: self.caches.hierarchy_cache.len(),
            static_field_layout_size: self.caches.static_field_layout_cache.len(),
            instance_field_layout_size: self.caches.instance_field_layout_cache.len(),
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
        })
    }
}

/// GC-managed state local to a single thread's arena.
pub struct ArenaLocalState<'gc> {
    pub heap: HeapManager<'gc>,
    pub runtime_asms: RefCell<HashMap<ResolutionS, ObjectRef<'gc>>>,
    pub runtime_types: RefCell<HashMap<RuntimeType, ObjectRef<'gc>>>,
    pub runtime_types_list: RefCell<Vec<RuntimeType>>,
    pub runtime_methods: RefCell<Vec<(MethodDescription, GenericLookup)>>,
    pub runtime_method_objs: RefCell<HashMap<(MethodDescription, GenericLookup), ObjectRef<'gc>>>,
    pub runtime_fields: RefCell<Vec<(FieldDescription, GenericLookup)>>,
    pub runtime_field_objs: RefCell<HashMap<(FieldDescription, GenericLookup), ObjectRef<'gc>>>,
}

unsafe impl<'gc> Collect for ArenaLocalState<'gc> {
    fn trace(&self, cc: &Collection) {
        self.heap.trace(cc);
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

impl<'gc> Default for ArenaLocalState<'gc> {
    fn default() -> Self {
        Self::new()
    }
}

impl<'gc> ArenaLocalState<'gc> {
    pub fn new() -> Self {
        Self {
            heap: HeapManager {
                _all_objs: RefCell::new(HashSet::new()),
                finalization_queue: RefCell::new(vec![]),
                pending_finalization: RefCell::new(vec![]),
                pinned_objects: RefCell::new(HashSet::new()),
                gchandles: RefCell::new(vec![]),
                processing_finalizer: Cell::new(false),
                needs_full_collect: Cell::new(false),
                #[cfg(feature = "multithreaded-gc")]
                cross_arena_roots: RefCell::new(HashSet::new()),
            },
            runtime_asms: RefCell::new(HashMap::new()),
            runtime_types: RefCell::new(HashMap::new()),
            runtime_types_list: RefCell::new(vec![]),
            runtime_methods: RefCell::new(vec![]),
            runtime_method_objs: RefCell::new(HashMap::new()),
            runtime_fields: RefCell::new(vec![]),
            runtime_field_objs: RefCell::new(HashMap::new()),
        }
    }
}

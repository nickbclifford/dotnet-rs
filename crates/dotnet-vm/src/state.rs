#[cfg(feature = "multithreaded-gc")]
use crate::gc::GCCoordinator;
#[cfg(feature = "multithreaded-gc")]
use dotnet_utils::sync::AtomicUsize;

use crate::{
    intrinsics::IntrinsicRegistry,
    memory::HeapManager,
    metrics::{CacheSizes, CacheStats, RuntimeMetrics},
    pinvoke::NativeLibraries,
    sync::SyncBlockManager,
    threading::ThreadManager,
    tracer::Tracer,
};
use dashmap::DashMap;
use dotnet_assemblies::AssemblyLoader;
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
use gc_arena::{Collect, Collection};
use std::{
    cell::{Cell, Ref, RefCell, RefMut},
    collections::{BTreeMap, HashMap, HashSet},
};

pub use crate::statics::StaticStorageManager;

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
    /// Cache for value type checks: TypeDescription -> bool
    pub value_type_cache: DashMap<TypeDescription, bool>,
    /// Cache for finalizer checks: TypeDescription -> bool
    pub has_finalizer_cache: DashMap<TypeDescription, bool>,
    /// Cache for resolved overrides: (TypeDescription, GenericLookup) -> Map<DeclMethod, ImplMethod>
    pub overrides_cache:
        DashMap<(TypeDescription, GenericLookup), Arc<HashMap<usize, MethodDescription>>>,
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
            value_type_cache: DashMap::new(),
            has_finalizer_cache: DashMap::new(),
            overrides_cache: DashMap::new(),
            intrinsic_registry,
        }
    }
}

/// Thread-safe shared state that does not contain any GC-managed pointers.
/// This state is shared across all execution threads and arenas.
pub struct SharedGlobalState<'m> {
    pub loader: &'m AssemblyLoader,
    pub pinvoke: NativeLibraries,
    pub sync_blocks: SyncBlockManager,
    pub thread_manager: Arc<ThreadManager>,
    pub metrics: RuntimeMetrics,
    pub tracer: Mutex<Tracer>,
    pub tracer_enabled: Arc<AtomicBool>,
    pub empty_generics: GenericLookup,
    /// Grouped caches for type resolution and layout computation
    pub caches: Arc<GlobalCaches>,
    pub statics: Arc<StaticStorageManager>,
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

        let tracer_enabled = Arc::new(AtomicBool::new(tracer.is_enabled()));

        // Reset the static registry to ensure a clean state for the new VM.
        dotnet_value::pointer::reset_static_registry();

        Self {
            loader,
            pinvoke: NativeLibraries::new(loader.get_root()),
            sync_blocks: SyncBlockManager::new(),
            thread_manager: ThreadManager::new(),
            metrics: RuntimeMetrics::new(),
            tracer: Mutex::new(tracer),
            tracer_enabled,
            empty_generics: GenericLookup::default(),
            caches,
            statics: Arc::new(StaticStorageManager::new()),
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
        }
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
            value_type_size: self.caches.value_type_cache.len(),
            has_finalizer_size: self.caches.has_finalizer_cache.len(),
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
    pub statics: Arc<StaticStorageManager>,
    pub runtime_asms: RefCell<HashMap<ResolutionS, ObjectRef<'gc>>>,
    pub runtime_types: RefCell<HashMap<RuntimeType, ObjectRef<'gc>>>,
    pub runtime_types_list: RefCell<Vec<RuntimeType>>,
    pub runtime_methods: RefCell<Vec<(MethodDescription, GenericLookup)>>,
    pub runtime_method_objs: RefCell<HashMap<(MethodDescription, GenericLookup), ObjectRef<'gc>>>,
    pub runtime_fields: RefCell<Vec<(FieldDescription, GenericLookup)>>,
    pub runtime_field_objs: RefCell<HashMap<(FieldDescription, GenericLookup), ObjectRef<'gc>>>,
}

// SAFETY: `ArenaLocalState` correctly traces all GC-managed fields in its `trace` implementation.
// This includes the `heap`, the global `statics`, and all `ObjectRef<'gc>` values stored in the
// various RefCell-wrapped collections.
unsafe impl<'gc> Collect for ArenaLocalState<'gc> {
    fn trace(&self, cc: &Collection) {
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
            heap: HeapManager {
                _all_objs: RefCell::new(BTreeMap::new()),
                finalization_queue: RefCell::new(vec![]),
                pending_finalization: RefCell::new(vec![]),
                pinned_objects: RefCell::new(HashSet::new()),
                gchandles: RefCell::new(vec![]),
                processing_finalizer: Cell::new(false),
                needs_full_collect: Cell::new(false),
                #[cfg(feature = "multithreaded-gc")]
                cross_arena_roots: RefCell::new(HashSet::new()),
            },
            statics,
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

use crate::{
    context::ResolutionContext, gc::coordinator::GCCoordinator, layout::static_fields,
    threading::ThreadManagerOps,
};
use dotnet_types::{
    TypeDescription, error::TypeResolutionError, generics::GenericLookup,
    members::MethodDescription,
};
use dotnet_utils::{
    DebugStr,
    sync::{Arc, AtomicU8, AtomicU64, Condvar, OrderedMutex, Ordering, RwLock, levels},
};
use dotnet_value::{
    layout::{FieldKey, FieldLayout, FieldLayoutManager, GcDesc, HasLayout, LayoutManager, Scalar},
    object::ObjectRef,
    storage::FieldStorage,
};
use gc_arena::{Collect, collect::Trace};
use std::{
    collections::HashMap,
    fmt::{Debug, Formatter},
};

/// Initialization states for type static constructors (.cctor).
/// This is an atomic state machine for thread-safe type initialization.
pub const INIT_STATE_UNINITIALIZED: u8 = 0;
pub const INIT_STATE_INITIALIZING: u8 = 1;
pub const INIT_STATE_INITIALIZED: u8 = 2;
pub const INIT_STATE_FAILED: u8 = 3;

pub struct StaticStorage {
    /// Atomic initialization state for thread-safe .cctor execution.
    /// States: 0=uninitialized, 1=initializing (in progress), 2=initialized (complete)
    pub(crate) init_state: AtomicU8,
    /// The ID of the thread currently initializing this type.
    /// Only valid if init_state is INITIALIZING.
    pub(crate) initializing_thread: AtomicU64,
    /// Storage for static fields. Access must use atomic operations
    /// for thread-safety.
    pub storage: FieldStorage,
    /// Traced exception retained when initialization fails.
    failure: RwLock<Option<FieldStorage>>,
    /// Condition variable to wait for initialization completion.
    pub(crate) init_cond: Arc<Condvar>,
    /// Mutex for use with init_cond.
    pub(crate) init_mutex: Arc<OrderedMutex<levels::StaticInitMutex, ()>>,
}

impl StaticStorage {
    pub fn layout(&self) -> Arc<FieldLayoutManager> {
        self.storage.layout().clone()
    }
}

impl Clone for StaticStorage {
    fn clone(&self) -> Self {
        Self {
            init_state: AtomicU8::new(self.init_state.load(Ordering::Acquire)),
            initializing_thread: AtomicU64::new(self.initializing_thread.load(Ordering::Acquire)),
            storage: self.storage.clone(),
            failure: RwLock::new(self.failure.read().clone()),
            init_cond: self.init_cond.clone(),
            init_mutex: self.init_mutex.clone(),
        }
    }
}

// SAFETY: F5.TracesEveryGcRef — both field storage and the retained exception carrier
// are traced; the atomics and synchronization primitives contain no GC handles.
// FieldStorage is traced only during a stop-the-world pause, when mutators cannot modify it.
unsafe impl<'gc> Collect<'gc> for StaticStorage {
    fn trace<Tr: Trace<'gc>>(&self, cc: &mut Tr) {
        self.storage.trace(cc);
        if let Some(failure) = self.failure.read().as_ref() {
            failure.trace(cc);
        }
    }
}

impl Debug for StaticStorage {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self.init_state.load(Ordering::Acquire) {
            INIT_STATE_INITIALIZED => Debug::fmt(&self.storage, f),
            INIT_STATE_INITIALIZING => write!(f, "initializing"),
            _ => write!(f, "uninitialized"),
        }
    }
}

type StaticMap = HashMap<(TypeDescription, GenericLookup), Arc<StaticStorage>>;

pub struct StaticStorageManager {
    /// Thread-safe map of types to their static storage.
    /// We use a striped lock table for better performance and to allow concurrent access to different types,
    /// and each StaticStorage has its own locks for even more granularity.
    shards: [RwLock<StaticMap>; 16],
    /// Active initialization dependencies, including independently parked segments on one thread.
    wait_graph: OrderedMutex<levels::StaticWaitGraph, WaitGraph>,
}

const NUM_SHARDS: usize = 16;

#[derive(Default)]
struct WaitGraph {
    #[cfg(feature = "multithreading")]
    next: u64,
    edges: HashMap<u64, HashMap<u64, u64>>,
}

impl WaitGraph {
    #[cfg(feature = "multithreading")]
    fn insert(&mut self, waiter: u64, target: u64) -> u64 {
        self.next = self
            .next
            .checked_add(1)
            .expect("static wait identity exhausted");
        self.edges
            .entry(waiter)
            .or_default()
            .insert(self.next, target);
        self.next
    }

    fn remove(&mut self, waiter: u64, id: u64) {
        if let Some(edges) = self.edges.get_mut(&waiter) {
            edges.remove(&id);
            if edges.is_empty() {
                self.edges.remove(&waiter);
            }
        }
    }

    #[cfg(feature = "multithreading")]
    fn causes_cycle(&self, start: u64, target: u64) -> bool {
        let mut pending = vec![target];
        let mut visited = std::collections::HashSet::new();
        while let Some(current) = pending.pop() {
            if current == start {
                return true;
            }
            if visited.insert(current)
                && let Some(edges) = self.edges.get(&current)
            {
                pending.extend(edges.values().copied());
            }
        }
        false
    }
}

/// Cleans up an inserted edge until ownership transfers to a registration.
#[cfg(feature = "multithreading")]
struct WaitGraphEdgeGuard<'a> {
    wait_graph: &'a OrderedMutex<levels::StaticWaitGraph, WaitGraph>,
    waiter: u64,
    id: u64,
    armed: bool,
}

#[cfg(feature = "multithreading")]
impl<'a> WaitGraphEdgeGuard<'a> {
    fn new(
        wait_graph: &'a OrderedMutex<levels::StaticWaitGraph, WaitGraph>,
        waiter: u64,
        id: u64,
    ) -> Self {
        Self {
            wait_graph,
            waiter,
            id,
            armed: true,
        }
    }
    fn disarm(&mut self) {
        self.armed = false;
    }
}

#[cfg(feature = "multithreading")]
impl Drop for WaitGraphEdgeGuard<'_> {
    fn drop(&mut self) {
        if self.armed {
            self.wait_graph.lock().remove(self.waiter, self.id);
        }
    }
}

/// Owns one dependency without borrowing the manager across a suspension.
pub struct StaticWaitRegistration {
    manager: Arc<StaticStorageManager>,
    waiter: u64,
    id: u64,
}

impl std::fmt::Debug for StaticWaitRegistration {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StaticWaitRegistration")
            .field("waiter", &self.waiter)
            .field("id", &self.id)
            .finish()
    }
}

impl StaticWaitRegistration {
    #[cfg(feature = "multithreading")]
    fn new(manager: Arc<StaticStorageManager>, waiter: u64, id: u64) -> Self {
        let mut guard = WaitGraphEdgeGuard::new(&manager.wait_graph, waiter, id);
        let registration = Self {
            manager: manager.clone(),
            waiter,
            id,
        };
        guard.disarm();
        registration
    }
}

impl Drop for StaticWaitRegistration {
    fn drop(&mut self) {
        self.manager.wait_graph.lock().remove(self.waiter, self.id);
    }
}

// SAFETY: F5.TracesEveryGcRef — `StaticStorageManager::trace` correctly traces all `StaticStorage` values in its map.
// The map keys are non-GC metadata and do not need tracing. Each `StaticStorage` value is traced,
// which in turn traces all GC-managed references in static fields.
// Release builds read each shard without its lock, relying on the stop-the-world pause.
// The `wait_graph` contains only primitive IDs and does not need tracing.
unsafe impl<'gc> Collect<'gc> for StaticStorageManager {
    fn trace<Tr: Trace<'gc>>(&self, cc: &mut Tr) {
        for shard in &self.shards {
            #[cfg(debug_assertions)]
            {
                #[cfg(feature = "multithreading")]
                if let Some(id) = dotnet_utils::gc::get_currently_tracing() {
                    // Robust check: we only assert if the arena is still registered.
                    // This avoids false positives in parallel tests where reset_arena_registry()
                    // might have cleared the global registry.
                    if dotnet_utils::gc::is_valid_cross_arena_ref(id) {
                        debug_assert!(
                            dotnet_utils::gc::is_stw_in_progress(id),
                            "StaticStorageManager::trace: STW flag not set during tracing for arena {:?}",
                            id
                        );
                    }
                }

                // INVARIANT: F1.StwParked — tracing runs during a stop-the-world pause, so no mutator
                // holds a shard write lock and `try_read` succeeds; failure means STW was violated.
                let map = shard.try_read().expect(
                    "StaticStorageManager::trace: failed to acquire lock; STW failed? \
                     (another thread is holding a write lock)",
                );
                for v in map.values() {
                    v.trace(cc);
                }
            }
            #[cfg(not(debug_assertions))]
            {
                #[allow(
                    clippy::mutable_key_type,
                    reason = "the key's Hash/Eq never inspect interior-mutable metadata state; see the same allowance in dotnet-runtime-resolver's method caches"
                )]
                #[cfg_attr(
                    not(feature = "multithreading"),
                    expect(
                        clippy::multiple_unsafe_ops_per_block,
                        reason = "the single-threaded lock's `data_ptr` is itself unsafe; both operations share the stop-the-world premise"
                    )
                )]
                // SAFETY: F1.StwParked — tracing runs during a stop-the-world pause, so no mutator
                // holds or can acquire the shard lock while this shared reference is live. Debug
                // builds take `try_read` instead to check that premise.
                let map = unsafe { &*shard.data_ptr() };
                for v in map.values() {
                    v.trace(cc);
                }
            }
        }
    }
}

impl Debug for StaticStorageManager {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        let mut map = f.debug_map();
        for shard in &self.shards {
            let shard = shard.read();
            for (k, v) in shard.iter() {
                map.entry(&DebugStr(k.0.type_name()), v);
            }
        }
        map.finish()
    }
}

#[derive(Debug)]
pub enum StaticInitResult {
    /// This thread must execute the static constructor.
    Execute(MethodDescription),
    /// The type is already fully initialized.
    Initialized,
    /// This is a recursive call on the same thread; proceed as if initialized.
    Recursive,
    /// Type initialization failed previously.
    Failed,
    /// Another thread is currently initializing this type.
    Waiting(StaticWaitRegistration),
}

impl StaticStorageManager {
    pub fn new() -> Self {
        Self {
            shards: std::array::from_fn(|_| RwLock::new(HashMap::new())),
            wait_graph: OrderedMutex::new(WaitGraph::default()),
        }
    }

    fn get_shard_idx(&self, key: &(TypeDescription, GenericLookup)) -> usize {
        use std::hash::{Hash, Hasher};
        let mut s = std::collections::hash_map::DefaultHasher::new();
        key.hash(&mut s);
        (s.finish() as usize) % NUM_SHARDS
    }

    pub fn get_static_field_layout(
        &self,
        description: TypeDescription,
        context: &ResolutionContext,
    ) -> Result<Arc<FieldLayoutManager>, TypeResolutionError> {
        let key = (description.clone(), context.generics.clone());

        if let Some(cached) = context.caches().static_field_layout_cache.get(&key) {
            return Ok(cached);
        }

        let result = Arc::new(static_fields(description.clone(), context)?);
        context
            .caches()
            .static_field_layout_cache
            .insert(key, Arc::clone(&result));
        Ok(result)
    }
}

impl Default for StaticStorageManager {
    fn default() -> Self {
        Self::new()
    }
}

impl StaticStorageManager {
    fn lookup_in_shard(
        &self,
        description: TypeDescription,
        generics: &GenericLookup,
    ) -> Option<Arc<StaticStorage>> {
        let key = (description, generics.clone());
        let shard_idx = self.get_shard_idx(&key);
        let shard = self.shards[shard_idx].read();
        shard.get(&key).cloned()
    }

    pub fn get(
        &self,
        description: TypeDescription,
        generics: &GenericLookup,
    ) -> Arc<StaticStorage> {
        self.lookup_in_shard(description.clone(), generics)
            .expect("Static storage should have been initialized via init()")
    }

    /// Get the current initialization state of a type.
    pub fn get_init_state(&self, description: TypeDescription, generics: &GenericLookup) -> u8 {
        self.lookup_in_shard(description.clone(), generics)
            .map(|s| s.init_state.load(Ordering::Acquire))
            .unwrap_or(INIT_STATE_UNINITIALIZED)
    }

    /// Mark a type as failed after its .cctor throws an exception.
    pub fn mark_failed(&self, description: TypeDescription, generics: &GenericLookup) {
        if let Some(storage) = self.lookup_in_shard(description.clone(), generics) {
            let _lock = storage.init_mutex.lock();
            storage
                .init_state
                .store(INIT_STATE_FAILED, Ordering::Release);
            storage.initializing_thread.store(0, Ordering::Release);
            // Notify any threads waiting for initialization
            storage.init_cond.notify_all();
        }
    }

    pub(crate) fn failed_exception<'gc>(
        &self,
        description: TypeDescription,
        generics: &GenericLookup,
        _gc: dotnet_utils::gc::GCHandle<'gc>,
    ) -> Option<ObjectRef<'gc>> {
        let storage = self.get(description, generics);
        // INVARIANT: F6.InitializerFailureRoot — the traced carrier and its owner remain live
        // while this mutation decodes the reference into its GC-handle brand.
        let failure = storage.failure.read();
        failure.as_ref().map(|carrier| {
            carrier
                .field_at_offset::<ObjectRef<'gc>>(0)
                .expect("exception carrier layout")
                .read()
        })
    }

    /// Publish a failed initializer and retain its exception through coordinated collections.
    pub fn mark_failed_with_exception(
        &self,
        description: TypeDescription,
        generics: &GenericLookup,
        exception: ObjectRef<'_>,
    ) {
        let storage = self.get(description, generics);
        let mut fields = hashbrown::HashMap::new();
        fields.insert(
            FieldKey {
                owner: TypeDescription::NULL,
                name: "exception".into(),
            },
            FieldLayout {
                position: dotnet_utils::ByteOffset::new(0),
                layout: Arc::new(LayoutManager::Scalar(Scalar::ObjectRef)),
            },
        );
        let mut gc_desc = GcDesc::default();
        gc_desc.set(0);
        let carrier = FieldStorage::new(
            Arc::new(FieldLayoutManager {
                fields,
                total_size: ObjectRef::SIZE,
                alignment: 8,
                gc_desc,
                has_ref_fields: true,
            }),
            vec![0; ObjectRef::SIZE],
        );
        // INVARIANT: F6.InitializerFailureRoot — the current exception state or the
        // native activation retains this object until the traced carrier is published.
        carrier
            .field_at_offset::<ObjectRef>(0)
            .expect("exception carrier layout")
            .write(exception);
        let _lock = storage.init_mutex.lock();
        *storage.failure.write() = Some(carrier);
        storage
            .init_state
            .store(INIT_STATE_FAILED, Ordering::Release);
        storage.initializing_thread.store(0, Ordering::Release);
        storage.init_cond.notify_all();
    }

    /// Initialize static storage for a type and determine if a .cctor needs to run.
    /// This implementation ensures that a type's static constructor is only assigned
    /// to exactly one thread for execution, even if multiple threads race to initialize
    /// the same type simultaneously.
    ///
    /// Returns a `StaticInitResult` indicating what the calling thread should do.
    pub fn init(
        self: &Arc<Self>,
        description: TypeDescription,
        context: &ResolutionContext,
        thread_id: dotnet_utils::ArenaId,
    ) -> Result<StaticInitResult, TypeResolutionError> {
        let key = (description.clone(), context.generics.clone());
        let shard_idx = self.get_shard_idx(&key);

        // Ensure the storage exists.
        let storage_exists = {
            let shard = self.shards[shard_idx].read();
            shard.contains_key(&key)
        };

        if !storage_exists {
            // Use the cached layout if available, or create and cache it.
            // DO NOT hold the shard lock while doing layout computation,
            // as it might be expensive or trigger recursive resolutions.
            let layout = self.get_static_field_layout(description.clone(), context)?;
            let size = layout.size();
            let mut data = vec![0; size.as_usize()];

            // Seed static RVA-backed fields (for example, UTF-8 literal backing data)
            // into the static storage image so `ldsflda` sees the correct bytes.
            for field in &description.definition().fields {
                let Some(initial_data) = &field.initial_value else {
                    continue;
                };
                let Some(field_layout) = layout.get_field(description.clone(), field.name.as_ref())
                else {
                    continue;
                };

                let dst_start = field_layout.position.as_usize();
                let field_size = field_layout.layout.size().as_usize();
                let copy_len = std::cmp::min(field_size, initial_data.len());
                if copy_len == 0 {
                    continue;
                }

                let dst_end = dst_start + copy_len;
                data[dst_start..dst_end].copy_from_slice(&initial_data[..copy_len]);
            }

            let mut shard = self.shards[shard_idx].write();
            shard.entry(key.clone()).or_insert_with(|| {
                #[allow(
                    clippy::arc_with_non_send_sync,
                    reason = "no-MT initialization state is executor-confined; Arc preserves feature-neutral ownership"
                )]
                let init_mutex = Arc::new(OrderedMutex::new(()));
                #[allow(
                    clippy::arc_with_non_send_sync,
                    reason = "no-MT static storage is executor-confined; Arc preserves feature-neutral ownership"
                )]
                let storage = Arc::new(StaticStorage {
                    init_state: AtomicU8::new(INIT_STATE_UNINITIALIZED),
                    initializing_thread: AtomicU64::new(0),
                    storage: FieldStorage::new(layout, data),
                    failure: RwLock::new(None),
                    init_cond: Arc::new(Condvar::new()),
                    init_mutex,
                });
                storage
            });
        }

        // Get the storage and check its state.
        let storage = self.get(description.clone(), context.generics);
        let state = storage.init_state.load(Ordering::Acquire);

        if state == INIT_STATE_INITIALIZED {
            return Ok(StaticInitResult::Initialized);
        }

        if state == INIT_STATE_FAILED {
            return Ok(StaticInitResult::Failed);
        }

        if state == INIT_STATE_INITIALIZING
            && storage.initializing_thread.load(Ordering::Acquire) == thread_id.as_u64()
        {
            return Ok(StaticInitResult::Recursive);
        }

        // Check if there's a static initializer
        let cctor = description.clone().static_initializer();

        if cctor.is_none() {
            // No .cctor, so mark as initialized if it was uninitialized
            storage
                .init_state
                .compare_exchange(
                    INIT_STATE_UNINITIALIZED,
                    INIT_STATE_INITIALIZED,
                    Ordering::AcqRel,
                    Ordering::Acquire,
                )
                .ok();
            return Ok(StaticInitResult::Initialized);
        }

        let cctor = cctor.expect("checked is_none() above and returned early");

        let _init_lock = storage.init_mutex.lock();
        match storage.init_state.load(Ordering::Acquire) {
            INIT_STATE_UNINITIALIZED => {
                storage
                    .init_state
                    .store(INIT_STATE_INITIALIZING, Ordering::Release);
                storage
                    .initializing_thread
                    .store(thread_id.as_u64(), Ordering::Release);
                Ok(StaticInitResult::Execute(cctor))
            }
            INIT_STATE_INITIALIZED => Ok(StaticInitResult::Initialized),
            INIT_STATE_FAILED => Ok(StaticInitResult::Failed),
            INIT_STATE_INITIALIZING => {
                let target = storage.initializing_thread.load(Ordering::Acquire);
                assert_ne!(target, 0, "initializing type must have an owner");
                if target == thread_id.as_u64() {
                    return Ok(StaticInitResult::Recursive);
                }
                #[cfg(not(feature = "multithreading"))]
                unreachable!(
                    "single-threaded static initialization cannot wait for another thread"
                );
                #[cfg(feature = "multithreading")]
                {
                    let id = {
                        let mut graph = self.wait_graph.lock_after(_init_lock.held_level());
                        if graph.causes_cycle(thread_id.as_u64(), target) {
                            return Ok(StaticInitResult::Recursive);
                        }
                        graph.insert(thread_id.as_u64(), target)
                    };
                    Ok(StaticInitResult::Waiting(StaticWaitRegistration::new(
                        self.clone(),
                        thread_id.as_u64(),
                        id,
                    )))
                }
            }
            state => unreachable!("Invalid initialization state: {}", state),
        }
    }

    /// Mark a type as fully initialized after its .cctor completes.
    /// This must be called after the .cctor returned by `init()` has finished executing.
    pub fn mark_initialized(&self, description: TypeDescription, generics: &GenericLookup) {
        if let Some(storage) = self.lookup_in_shard(description, generics) {
            let _lock = storage.init_mutex.lock();
            storage
                .init_state
                .store(INIT_STATE_INITIALIZED, Ordering::Release);
            storage.initializing_thread.store(0, Ordering::Release);
            // Notify any threads waiting for initialization
            storage.init_cond.notify_all();
        }
    }

    /// Wait for a type's initialization to complete if another thread is currently initializing it.
    ///
    /// Returns `true` when the caller should yield to a GC safe point and retry later,
    /// or `false` when initialization is no longer in progress.
    ///
    /// The registration's edge is removed before every return, including a GC-yield request.
    ///
    /// This method is only valid after [`StaticInitResult::Waiting`] is returned. A
    /// single-threaded build cannot produce that result, so it has no valid caller
    /// there; reaching the wait arm in that configuration panics.
    pub fn wait_for_init(
        &self,
        description: TypeDescription,
        generics: &GenericLookup,
        thread_manager: &impl ThreadManagerOps,
        registration: StaticWaitRegistration,
        _gc_coordinator: &GCCoordinator,
    ) -> bool {
        let storage = self.get(description, generics);
        let mut should_yield = false;

        #[allow(
            clippy::never_loop,
            reason = "single-threaded builds can reach this loop only through an invariant violation"
        )]
        loop {
            // Check state before acquiring lock
            let state = storage.init_state.load(Ordering::Acquire);
            if state != INIT_STATE_INITIALIZING {
                break;
            }

            // Check for GC safe point before waiting
            if thread_manager.is_gc_stop_requested() {
                should_yield = true;
                break;
            }

            // Try to wait with a timeout
            #[cfg(feature = "multithreading")]
            {
                let mut lock = storage.init_mutex.lock();
                if storage.init_state.load(Ordering::Acquire) != INIT_STATE_INITIALIZING {
                    break;
                }

                // Wait for a short duration to allow periodic safe point checks
                use std::time::Duration;
                let _ = storage
                    .init_cond
                    .wait_for(lock.raw_mut(), Duration::from_millis(10));
            }
            #[cfg(not(feature = "multithreading"))]
            {
                unreachable!(
                    "wait_for_init is unreachable in single-threaded builds: StaticInitResult::Waiting cannot be produced when only one thread exists"
                );
            }
        }

        drop(registration);
        should_yield
    }
}

#[cfg(all(test, feature = "multithreading"))]
mod wait_graph_guard_tests {
    use super::*;
    use std::panic::{AssertUnwindSafe, catch_unwind};

    #[test]
    fn wait_graph_edge_guard_removes_edge_on_scope_exit() {
        let graph = OrderedMutex::<levels::StaticWaitGraph, WaitGraph>::new(WaitGraph::default());
        let id = graph.lock().insert(10, 20);
        {
            let _guard = WaitGraphEdgeGuard::new(&graph, 10, id);
            assert_eq!(graph.lock().edges[&10][&id], 20);
        }
        assert!(graph.lock().edges.is_empty());
    }

    #[test]
    fn wait_graph_edge_guard_removes_edge_during_unwind() {
        let graph = OrderedMutex::<levels::StaticWaitGraph, WaitGraph>::new(WaitGraph::default());
        let id = graph.lock().insert(30, 40);
        let _ = catch_unwind(AssertUnwindSafe(|| {
            let _guard = WaitGraphEdgeGuard::new(&graph, 30, id);
            panic!("forced unwind to verify wait-graph edge cleanup");
        }));
        assert!(graph.lock().edges.is_empty());
    }

    #[test]
    fn owned_wait_registration_removes_transferred_edge_on_unwind() {
        let manager = Arc::new(StaticStorageManager::new());
        let id = manager.wait_graph.lock().insert(50, 60);
        let registration = StaticWaitRegistration::new(manager.clone(), 50, id);
        assert_eq!(manager.wait_graph.lock().edges[&50][&id], 60);
        let _ = catch_unwind(AssertUnwindSafe(move || {
            let _registration = registration;
            panic!("forced unwind to verify owned wait cleanup");
        }));
        assert!(!manager.wait_graph.lock().edges.contains_key(&50));
    }

    #[test]
    fn wait_graph_preserves_nested_edges_and_detects_cycles() {
        let manager = Arc::new(StaticStorageManager::new());
        let (first, second) = {
            let mut graph = manager.wait_graph.lock();
            (graph.insert(1, 2), graph.insert(1, 3))
        };
        let first = StaticWaitRegistration::new(manager.clone(), 1, first);
        let second = StaticWaitRegistration::new(manager.clone(), 1, second);
        assert!(manager.wait_graph.lock().causes_cycle(2, 1));
        assert!(manager.wait_graph.lock().causes_cycle(3, 1));
        drop(second);
        assert!(manager.wait_graph.lock().causes_cycle(2, 1));
        assert!(!manager.wait_graph.lock().causes_cycle(3, 1));
        drop(first);
        assert!(manager.wait_graph.lock().edges.is_empty());
    }
}

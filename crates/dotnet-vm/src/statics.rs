use crate::{
    context::ResolutionContext, gc::coordinator::GCCoordinator, layout::LayoutFactory,
    threading::ThreadManagerOps,
};
use dotnet_metrics::RuntimeMetrics;
use dotnet_types::{
    TypeDescription, error::TypeResolutionError, generics::GenericLookup,
    members::MethodDescription,
};
use dotnet_utils::{
    DebugStr,
    sync::{Arc, AtomicU8, AtomicU64, Condvar, OrderedMutex, Ordering, RwLock, levels},
};
use dotnet_value::{
    layout::{FieldLayoutManager, HasLayout},
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
            init_cond: self.init_cond.clone(),
            init_mutex: self.init_mutex.clone(),
        }
    }
}

// SAFETY: `StaticStorage::trace` correctly traces its `storage` field, which contains all
// GC-managed references. The atomic fields (`init_state`, `initializing_thread`) and
// synchronization primitives (`init_cond`, `init_mutex`) are not GC-managed and do not need
// tracing. FieldStorage::trace uses atomic reads for ObjectRefs, ensuring thread-safe tracing
// during stop-the-world GC pauses.
unsafe impl<'gc> Collect<'gc> for StaticStorage {
    fn trace<Tr: Trace<'gc>>(&self, cc: &mut Tr) {
        // Tracing is safe because FieldStorage::trace uses atomic reads for ObjectRefs
        // and we are during a stop-the-world pause or at least in a state where
        // the GC has control.
        self.storage.trace(cc);
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
    /// Tracks which thread is waiting for which other thread to complete type initialization.
    /// This is used to detect and avoid deadlocks in multi-threaded circular initialization.
    /// wait_graph[T1] = T2 means T1 is waiting for T2.
    ///
    /// Lifecycle contract:
    /// - `init()` inserts `wait_graph[waiter] = owner` before returning `StaticInitResult::Waiting`.
    /// - `wait_for_init()` may refresh that edge while polling.
    /// - `wait_for_init()` must remove `wait_graph[waiter]` on every return path.
    ///
    /// The remove-on-all-returns rule is required because stale edges feed
    /// `causes_cycle()`, which can falsely report recursion/cycles for later unrelated init calls.
    wait_graph: OrderedMutex<levels::StaticWaitGraph, HashMap<u64, u64>>,
}

const NUM_SHARDS: usize = 16;

// SAFETY: `StaticStorageManager::trace` correctly traces all `StaticStorage` values in its map.
// The map keys are non-GC metadata and do not need tracing. Each `StaticStorage` value is traced,
// which in turn traces all GC-managed references in static fields.
// During tracing, we use data_ptr() to bypass locks, which is safe during stop-the-world GC pauses.
// The `wait_graph` contains only primitive IDs and does not need tracing.
unsafe impl<'gc> Collect<'gc> for StaticStorageManager {
    fn trace<Tr: Trace<'gc>>(&self, cc: &mut Tr) {
        for shard in &self.shards {
            // SAFETY: Tracing happens during a stop-the-world pause, so no other
            // threads are running. We can safely access the inner value without
            // acquiring the lock.
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

#[derive(Debug, PartialEq, Eq)]
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
    Waiting,
}

impl StaticStorageManager {
    pub fn new() -> Self {
        Self {
            shards: std::array::from_fn(|_| RwLock::new(HashMap::new())),
            wait_graph: OrderedMutex::new(HashMap::new()),
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
        metrics: Option<&RuntimeMetrics>,
    ) -> Result<Arc<FieldLayoutManager>, TypeResolutionError> {
        let key = (description.clone(), context.generics.clone());

        if let Some(cached) = context.caches().static_field_layout_cache.get(&key) {
            if let Some(m) = metrics {
                m.record_static_field_layout_cache_hit();
            }
            return Ok(Arc::clone(&cached));
        }

        if let Some(m) = metrics {
            m.record_static_field_layout_cache_miss();
        }
        let result = Arc::new(LayoutFactory::static_fields(description.clone(), context)?);
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

    /// Initialize static storage for a type and determine if a .cctor needs to run.
    /// This implementation ensures that a type's static constructor is only assigned
    /// to exactly one thread for execution, even if multiple threads race to initialize
    /// the same type simultaneously.
    ///
    /// Returns a `StaticInitResult` indicating what the calling thread should do.
    pub fn init(
        &self,
        description: TypeDescription,
        context: &ResolutionContext,
        thread_id: dotnet_utils::ArenaId,
        metrics: Option<&RuntimeMetrics>,
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
            let layout = self.get_static_field_layout(description.clone(), context, metrics)?;
            let size = layout.size();
            let mut shard = self.shards[shard_idx].write();
            #[allow(clippy::arc_with_non_send_sync)]
            shard.entry(key.clone()).or_insert_with(|| {
                Arc::new(StaticStorage {
                    init_state: AtomicU8::new(INIT_STATE_UNINITIALIZED),
                    initializing_thread: AtomicU64::new(0),
                    storage: FieldStorage::new(layout, vec![0; size.as_usize()]),
                    init_cond: Arc::new(Condvar::new()),
                    init_mutex: Arc::new(OrderedMutex::new(())),
                })
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

        let cctor = cctor.unwrap();

        let init_lock = storage.init_mutex.lock();
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
                // Another thread is currently initializing
                let target_thread = storage.initializing_thread.load(Ordering::Acquire);
                if target_thread != 0 {
                    let mut wait_graph = self.wait_graph.lock_after(init_lock.held_level());
                    if self.causes_cycle(&wait_graph, thread_id.as_u64(), target_thread) {
                        return Ok(StaticInitResult::Recursive);
                    }
                    // Atomic with cycle check: add the edge before returning Waiting
                    wait_graph.insert(thread_id.as_u64(), target_thread);
                }
                Ok(StaticInitResult::Waiting)
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
    /// Wait-graph invariant: the caller's `thread_id` edge must be removed before any return
    /// from this method, including early return paths (such as a GC-yield request).
    pub fn wait_for_init(
        &self,
        description: TypeDescription,
        generics: &GenericLookup,
        thread_manager: &impl ThreadManagerOps,
        thread_id: dotnet_utils::ArenaId,
        _gc_coordinator: &GCCoordinator,
    ) -> bool {
        let storage = self.get(description, generics);
        let mut should_yield = false;

        loop {
            // Check state before acquiring lock
            let state = storage.init_state.load(Ordering::Acquire);
            if state != INIT_STATE_INITIALIZING {
                break;
            }

            // Ensure our wait edge is up-to-date
            let target_thread = storage.initializing_thread.load(Ordering::Acquire);
            if target_thread != 0 && target_thread != thread_id.as_u64() {
                let mut wait_graph = self.wait_graph.lock();
                wait_graph.insert(thread_id.as_u64(), target_thread);
            }

            // Check for GC safe point before waiting
            if thread_manager.is_gc_stop_requested() {
                should_yield = true;
                break;
            }

            // Try to wait with a timeout
            {
                let mut lock = storage.init_mutex.lock();
                if storage.init_state.load(Ordering::Acquire) != INIT_STATE_INITIALIZING {
                    break;
                }

                // Wait for a short duration to allow periodic safe point checks
                #[cfg(feature = "multithreading")]
                {
                    use std::time::Duration;
                    let _ = storage
                        .init_cond
                        .wait_for(lock.raw_mut(), Duration::from_millis(10));
                }
                #[cfg(not(feature = "multithreading"))]
                {
                    // In single-threaded mode, just wait normally
                    storage.init_cond.wait(lock.raw_mut());
                }
            }
        }

        // Remove from wait graph before exit
        let mut wait_graph = self.wait_graph.lock();
        wait_graph.remove(&thread_id.as_u64());
        should_yield
    }

    /// Helper to detect if adding start -> target edge would cause a cycle.
    fn causes_cycle(&self, wait_graph: &HashMap<u64, u64>, start: u64, target: u64) -> bool {
        let mut current = target;
        while let Some(&next) = wait_graph.get(&current) {
            if next == start {
                return true;
            }
            current = next;
        }
        false
    }
}

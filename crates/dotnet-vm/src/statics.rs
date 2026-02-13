use crate::{
    context::ResolutionContext, gc::coordinator::GCCoordinator, layout::LayoutFactory,
    metrics::RuntimeMetrics, threading::ThreadManagerOps,
};
use dotnet_types::{TypeDescription, generics::GenericLookup, members::MethodDescription};
use dotnet_utils::{
    DebugStr,
    sync::{Arc, AtomicU8, AtomicU64, Condvar, Mutex, Ordering, RwLock},
};
use dotnet_value::{
    layout::{FieldLayoutManager, HasLayout},
    storage::FieldStorage,
};
use gc_arena::{Collect, Collection};
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
    pub(crate) init_mutex: Arc<Mutex<()>>,
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
unsafe impl Collect for StaticStorage {
    fn trace(&self, cc: &Collection) {
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

pub struct StaticStorageManager {
    /// Thread-safe map of types to their static storage.
    /// We use a RwLock for the map itself to allow concurrent access to different types,
    /// and each StaticStorage has its own locks for even more granularity.
    types: RwLock<HashMap<(TypeDescription, GenericLookup), Arc<StaticStorage>>>,
}

// SAFETY: `StaticStorageManager::trace` correctly traces all `StaticStorage` values in its map.
// The map keys are non-GC metadata and do not need tracing. Each `StaticStorage` value is traced,
// which in turn traces all GC-managed references in static fields. The RwLock is acquired for
// reading during tracing, which is safe during stop-the-world GC pauses.
unsafe impl Collect for StaticStorageManager {
    fn trace(&self, cc: &Collection) {
        let types = self.types.read();
        for v in types.values() {
            v.trace(cc);
        }
    }
}

impl Debug for StaticStorageManager {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        let types = self.types.read();
        f.debug_map()
            .entries(types.iter().map(|(k, v)| (DebugStr(k.0.type_name()), v)))
            .finish()
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
            types: RwLock::new(HashMap::new()),
        }
    }

    pub fn get_static_field_layout(
        &self,
        description: TypeDescription,
        context: &ResolutionContext,
        metrics: Option<&RuntimeMetrics>,
    ) -> Result<Arc<FieldLayoutManager>, dotnet_types::error::TypeResolutionError> {
        let key = (description, context.generics.clone());

        if let Some(cached) = context.caches.static_field_layout_cache.get(&key) {
            if let Some(m) = metrics {
                m.record_static_field_layout_cache_hit();
            }
            return Ok(Arc::clone(&cached));
        }

        if let Some(m) = metrics {
            m.record_static_field_layout_cache_miss();
        }
        let result = Arc::new(LayoutFactory::static_fields_with_metrics(
            description,
            context,
            metrics,
        )?);
        context
            .caches
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
    pub fn get(
        &self,
        description: TypeDescription,
        generics: &GenericLookup,
    ) -> Arc<StaticStorage> {
        let types = self.types.read();
        types
            .get(&(description, generics.clone()))
            .cloned()
            .expect("missing type in static storage")
    }

    /// Get the current initialization state of a type.
    pub fn get_init_state(&self, description: TypeDescription, generics: &GenericLookup) -> u8 {
        let types = self.types.read();
        types
            .get(&(description, generics.clone()))
            .map(|s| s.init_state.load(Ordering::Acquire))
            .unwrap_or(INIT_STATE_UNINITIALIZED)
    }

    /// Mark a type as failed after its .cctor throws an exception.
    pub fn mark_failed(&self, description: TypeDescription, generics: &GenericLookup) {
        let types = self.types.read();
        if let Some(storage) = types.get(&(description, generics.clone())) {
            storage
                .init_state
                .store(INIT_STATE_FAILED, Ordering::Release);
            // Notify any threads waiting for initialization
            let _lock = storage.init_mutex.lock();
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
    ) -> Result<StaticInitResult, dotnet_types::error::TypeResolutionError> {
        let key = (description, context.generics.clone());
        // Ensure the storage exists. We use a write lock on the types map for this.
        {
            let mut types = self.types.write();
            if !types.contains_key(&key) {
                // Use the cached layout if available, or create and cache it
                let layout = self.get_static_field_layout(description, context, metrics)?;
                let size = layout.size();
                #[allow(clippy::arc_with_non_send_sync)]
                types.insert(
                    key.clone(),
                    Arc::new(StaticStorage {
                        init_state: AtomicU8::new(INIT_STATE_UNINITIALIZED),
                        initializing_thread: AtomicU64::new(0),
                        storage: FieldStorage::new(layout, vec![0; size.as_usize()]),
                        init_cond: Arc::new(Condvar::new()),
                        init_mutex: Arc::new(Mutex::new(())),
                    }),
                );
            }
        }

        // Get the storage and check its state.
        let storage = self.get(description, context.generics);
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
        let cctor = description.static_initializer();

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

        match storage.init_state.compare_exchange(
            INIT_STATE_UNINITIALIZED,
            INIT_STATE_INITIALIZING,
            Ordering::AcqRel,
            Ordering::Acquire,
        ) {
            Ok(_) => {
                storage
                    .initializing_thread
                    .store(thread_id.as_u64(), Ordering::Release);
                Ok(StaticInitResult::Execute(cctor))
            }
            Err(INIT_STATE_INITIALIZED) => {
                // Already initialized by another thread
                Ok(StaticInitResult::Initialized)
            }
            Err(INIT_STATE_INITIALIZING) => {
                // Another thread is currently initializing
                Ok(StaticInitResult::Waiting)
            }
            Err(_) => unreachable!("Invalid initialization state"),
        }
    }

    /// Mark a type as fully initialized after its .cctor completes.
    /// This must be called after the .cctor returned by `init()` has finished executing.
    pub fn mark_initialized(&self, description: TypeDescription, generics: &GenericLookup) {
        let types = self.types.read();
        if let Some(storage) = types.get(&(description, generics.clone())) {
            storage
                .init_state
                .store(INIT_STATE_INITIALIZED, Ordering::Release);
            // Notify any threads waiting for initialization
            let _lock = storage.init_mutex.lock();
            storage.init_cond.notify_all();
        }
    }

    /// Wait for a type's initialization to complete if another thread is currently initializing it.
    /// This method checks for GC safe points while waiting to avoid deadlocks during stop-the-world pauses.
    pub fn wait_for_init(
        &self,
        description: TypeDescription,
        generics: &GenericLookup,
        thread_manager: &impl ThreadManagerOps,
        thread_id: dotnet_utils::ArenaId,
        gc_coordinator: &GCCoordinator,
    ) {
        let storage = self.get(description, generics);

        loop {
            // Check state before acquiring lock
            let state = storage.init_state.load(Ordering::Acquire);
            if state != INIT_STATE_INITIALIZING {
                break;
            }

            // Check for GC safe point before waiting
            if thread_manager.is_gc_stop_requested() {
                thread_manager.safe_point(thread_id, gc_coordinator);
            }

            // Try to wait with a timeout
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
                    .wait_for(&mut lock, Duration::from_millis(10));
            }
            #[cfg(not(feature = "multithreading"))]
            {
                // In single-threaded mode, just wait normally
                storage.init_cond.wait(&mut lock);
            }
        }
    }
}

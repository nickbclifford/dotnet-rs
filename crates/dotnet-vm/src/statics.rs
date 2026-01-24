use dotnet_utils::{
    gc::GCHandle,
    sync::{Arc, AtomicU64, AtomicU8, Condvar, Mutex, Ordering, RwLock},
    DebugStr,
};
use dotnet_value::{layout::FieldLayoutManager, storage::FieldStorage};
use crate::{
    context::ResolutionContext, gc::coordinator::GCCoordinator, layout::LayoutFactory,
    metrics::RuntimeMetrics, threading::ThreadManagerOps, CallStack, MethodInfo,
};
use dotnet_types::{generics::GenericLookup, members::MethodDescription, TypeDescription};
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
    ) -> Arc<FieldLayoutManager> {
        let key = (description, context.generics.clone());

        if let Some(cached) = context.caches.static_field_layout_cache.get(&key) {
            return Arc::clone(&cached);
        }

        if let Some(m) = metrics {
            m.record_static_field_layout_cache_miss();
        }
        let result = Arc::new(LayoutFactory::static_fields_with_metrics(
            description,
            context,
            metrics,
        ));
        context
            .caches
            .static_field_layout_cache
            .insert(key, Arc::clone(&result));
        result
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

    /// Initialize static storage for a type and determine if a .cctor needs to run.
    /// This implementation ensures that a type's static constructor is only assigned
    /// to exactly one thread for execution, even if multiple threads race to initialize
    /// the same type simultaneously.
    ///
    /// Returns a `StaticInitResult` indicating what the calling thread should do.
    #[must_use]
    pub fn init(
        &self,
        description: TypeDescription,
        context: &ResolutionContext,
        thread_id: u64,
        metrics: Option<&RuntimeMetrics>,
    ) -> StaticInitResult {
        let key = (description, context.generics.clone());
        // Ensure the storage exists. We use a write lock on the types map for this.
        {
            let mut types = self.types.write();
            types.entry(key.clone()).or_insert_with(|| {
                // Use the cached layout if available, or create and cache it
                let layout = self.get_static_field_layout(description, context, metrics);
                Arc::new(StaticStorage {
                    init_state: AtomicU8::new(INIT_STATE_UNINITIALIZED),
                    initializing_thread: AtomicU64::new(0),
                    storage: FieldStorage::new(layout),
                    init_cond: Arc::new(Condvar::new()),
                    init_mutex: Arc::new(Mutex::new(())),
                })
            });
        }

        // Get the storage and check its state.
        let storage = self.get(description, context.generics);
        let state = storage.init_state.load(Ordering::Acquire);

        if state == INIT_STATE_INITIALIZED {
            return StaticInitResult::Initialized;
        }

        if state == INIT_STATE_INITIALIZING
            && storage.initializing_thread.load(Ordering::Acquire) == thread_id
        {
            return StaticInitResult::Recursive;
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
            return StaticInitResult::Initialized;
        }

        let cctor = cctor.unwrap();

        match storage.init_state.compare_exchange(
            INIT_STATE_UNINITIALIZED,
            INIT_STATE_INITIALIZING,
            Ordering::AcqRel,
            Ordering::Acquire,
        ) {
            Ok(_) => {
                // We won the race - this thread should execute the .cctor
                storage
                    .initializing_thread
                    .store(thread_id, Ordering::Release);
                StaticInitResult::Execute(cctor)
            }
            Err(INIT_STATE_INITIALIZED) => {
                // Already initialized by another thread
                StaticInitResult::Initialized
            }
            Err(INIT_STATE_INITIALIZING) => {
                // Another thread is currently initializing
                StaticInitResult::Waiting
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
        thread_id: u64,
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

impl<'gc, 'm: 'gc> CallStack<'gc, 'm> {
    /// Initialize static storage for a type and invoke its .cctor if needed.
    /// This method is thread-safe.
    ///
    /// Returns `true` if a .cctor was invoked (caller should return early),
    /// or `false` if initialization is complete or not needed.
    pub fn initialize_static_storage(
        &mut self,
        gc: GCHandle<'gc>,
        description: TypeDescription,
        generics: GenericLookup,
    ) -> bool {
        // Check GC safe point before potentially running static constructors
        // which may allocate many objects
        self.check_gc_safe_point();

        let ctx = ResolutionContext {
            resolution: description.resolution,
            generics: &generics,
            loader: self.loader(),
            type_owner: Some(description),
            method_owner: None,
            caches: self.shared.caches.clone(),
        };

        loop {
            let tid = self.thread_id.get();
            let init_result = {
                // Thread-safe path: use StaticStorageManager's internal locking
                self.shared
                    .statics
                    .init(description, &ctx, tid, Some(&self.shared.metrics))
            };

            use StaticInitResult::*;
            match init_result {
                Execute(m) => {
                    vm_trace!(
                        self,
                        "-- calling static constructor (will return to ip {}) --",
                        self.current_frame().state.ip
                    );
                    self.call_frame(
                        gc,
                        MethodInfo::new(m, &generics, self.shared.clone()),
                        generics,
                    );
                    return true;
                }
                Initialized | Recursive => {
                    return false;
                }
                Waiting => {
                    // If INITIALIZING on another thread, we wait using the condition variable.
                    // wait_for_init now checks for GC safe points internally
                    #[cfg(feature = "multithreaded-gc")]
                    self.shared.statics.wait_for_init(
                        description,
                        &generics,
                        self.shared.thread_manager.as_ref(),
                        self.thread_id.get(),
                        &self.shared.gc_coordinator,
                    );
                    #[cfg(not(feature = "multithreaded-gc"))]
                    self.shared.statics.wait_for_init(
                        description,
                        &generics,
                        self.shared.thread_manager.as_ref(),
                        self.thread_id.get(),
                        &Default::default(),
                    );
                }
            }
        }
    }
}

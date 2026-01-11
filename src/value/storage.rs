use crate::{
    types::{generics::GenericLookup, members::MethodDescription, TypeDescription},
    utils::{is_ptr_aligned_to_field, DebugStr},
    value::{
        layout::{FieldLayoutManager, HasLayout, LayoutManager, Scalar},
        object::ObjectRef,
    },
    vm::{
        context::ResolutionContext,
        metrics::RuntimeMetrics,
        sync::{Arc, AtomicU16, AtomicU32, AtomicU64, AtomicU8, Condvar, Mutex, Ordering, RwLock},
    },
};
use gc_arena::{Collect, Collection};
use std::{
    collections::{HashMap, HashSet},
    fmt::{Debug, Formatter},
    ops::Range,
};

#[derive(Clone, PartialEq)]
pub struct FieldStorage {
    layout: Arc<FieldLayoutManager>,
    storage: Vec<u8>,
}

impl Debug for FieldStorage {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        let mut fs: Vec<(_, _)> = self
            .layout
            .fields
            .iter()
            .map(|(k, v)| {
                let data = &self.storage[v.as_range()];
                let data_rep = match &*v.layout {
                    LayoutManager::Scalar(Scalar::ObjectRef) => {
                        format!("{:?}", ObjectRef::read(data))
                    }
                    LayoutManager::Scalar(Scalar::ManagedPtr) => {
                        // Skip reading ManagedPtr to avoid transmute issues
                        let bytes: Vec<_> = data.iter().map(|b| format!("{:02x}", b)).collect();
                        format!("ptr({})", bytes.join(" "))
                    }
                    _ => {
                        let bytes: Vec<_> = data.iter().map(|b| format!("{:02x}", b)).collect();
                        bytes.join(" ")
                    }
                };

                (
                    v.position,
                    DebugStr(format!("{} {}: {}", v.layout.type_tag(), k, data_rep)),
                )
            })
            .collect();

        fs.sort_by_key(|(p, _)| *p);

        f.debug_list()
            .entries(fs.into_iter().map(|(_, r)| r))
            .finish()
    }
}

unsafe impl Collect for FieldStorage {
    #[inline]
    fn trace(&self, cc: &Collection) {
        self.layout.trace(&self.storage, cc);
    }
}

impl FieldStorage {
    pub fn new(layout: Arc<FieldLayoutManager>) -> Self {
        let size = layout.size();
        // Defensive check: ensure the total allocation size is reasonable.
        if size > 0x1000_0000 {
            // 256MB limit for a single object instance
            panic!("massive field storage allocation: {} bytes", size);
        }
        Self {
            storage: vec![0; size],
            layout,
        }
    }

    pub fn instance_fields(description: TypeDescription, context: &ResolutionContext) -> Self {
        Self::new(FieldLayoutManager::instance_field_layout_cached(
            description,
            context,
            None,
        ))
    }

    pub fn resurrect<'gc>(&self, fc: &gc_arena::Finalization<'gc>, visited: &mut HashSet<usize>) {
        self.layout.resurrect(&self.storage, fc, visited);
    }

    pub fn static_fields(description: TypeDescription, context: &ResolutionContext) -> Self {
        Self::new(Arc::new(FieldLayoutManager::static_fields(
            description,
            context,
        )))
    }

    pub fn get(&self) -> &[u8] {
        &self.storage
    }

    pub fn get_mut(&mut self) -> &mut [u8] {
        &mut self.storage
    }

    pub fn layout(&self) -> &Arc<FieldLayoutManager> {
        &self.layout
    }

    fn get_field_range(&self, owner: TypeDescription, field: &str) -> Range<usize> {
        match self.layout.get_field(owner, field) {
            None => panic!("field {}::{} not found", owner.type_name(), field),
            Some(l) => l.as_range(),
        }
    }

    pub fn get_field_local(&self, owner: TypeDescription, field: &str) -> &[u8] {
        &self.storage[self.get_field_range(owner, field)]
    }

    pub fn get_field_mut_local(&mut self, owner: TypeDescription, field: &str) -> &mut [u8] {
        let r = self.get_field_range(owner, field);
        &mut self.storage[r]
    }

    pub fn has_field(&self, owner: TypeDescription, field: &str) -> bool {
        self.layout.get_field(owner, field).is_some()
    }

    pub fn get_field_atomic(
        &self,
        owner: TypeDescription,
        field: &str,
        ordering: Ordering,
    ) -> Vec<u8> {
        let layout = self
            .layout
            .get_field(owner, field)
            .unwrap_or_else(|| panic!("field {}::{} not found", owner.type_name(), field));
        let size = layout.layout.size();
        let offset = layout.position;
        let ptr = unsafe { self.storage.as_ptr().add(offset) };

        // Check alignment before attempting atomic operations
        // Unaligned atomic operations are UB in Rust
        if !is_ptr_aligned_to_field(ptr, size) {
            // Fall back to non-atomic read if not aligned
            // This can happen with ExplicitLayout types
            return self.get_field_local(owner, field).to_vec();
        }

        match size {
            1 => vec![unsafe { (*(ptr as *const AtomicU8)).load(ordering) }],
            2 => unsafe { (*(ptr as *const AtomicU16)).load(ordering) }
                .to_ne_bytes()
                .to_vec(),
            4 => unsafe { (*(ptr as *const AtomicU32)).load(ordering) }
                .to_ne_bytes()
                .to_vec(),
            8 => unsafe { (*(ptr as *const AtomicU64)).load(ordering) }
                .to_ne_bytes()
                .to_vec(),
            _ => self.get_field_local(owner, field).to_vec(),
        }
    }

    pub fn set_field_atomic(
        &self,
        owner: TypeDescription,
        field: &str,
        value: &[u8],
        ordering: Ordering,
    ) {
        let layout = self
            .layout
            .get_field(owner, field)
            .unwrap_or_else(|| panic!("field {}::{} not found", owner.type_name(), field));
        let size = layout.layout.size();
        let offset = layout.position;
        let ptr = unsafe { self.storage.as_ptr().add(offset) as *mut u8 };

        if size != value.len() {
            panic!(
                "size mismatch for field {}::{} (expected {}, got {})",
                owner.type_name(),
                field,
                size,
                value.len()
            );
        }

        // Check alignment before attempting atomic operations
        // Unaligned atomic operations are UB in Rust
        if !is_ptr_aligned_to_field(ptr, size) {
            // Fall back to non-atomic write if not aligned
            // This can happen with ExplicitLayout types
            // NOTE: This violates ECMA-335 atomicity requirements, but it's
            // better than UB. A proper implementation would need to lock.
            unsafe {
                std::ptr::copy_nonoverlapping(value.as_ptr(), ptr, size.min(value.len()));
            }
            return;
        }

        match size {
            1 => unsafe { (*(ptr as *const AtomicU8)).store(value[0], ordering) },
            2 => unsafe {
                let val = u16::from_ne_bytes(value.try_into().unwrap());
                (*(ptr as *const AtomicU16)).store(val, ordering)
            },
            4 => unsafe {
                let val = u32::from_ne_bytes(value.try_into().unwrap());
                (*(ptr as *const AtomicU32)).store(val, ordering)
            },
            8 => unsafe {
                let val = u64::from_ne_bytes(value.try_into().unwrap());
                (*(ptr as *const AtomicU64)).store(val, ordering)
            },
            _ => unsafe {
                std::ptr::copy_nonoverlapping(value.as_ptr(), ptr, size);
            },
        }
    }
}

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
    types: RwLock<HashMap<TypeDescription, Arc<StaticStorage>>>,
    /// Cache for static field layouts to avoid recomputing them on every access.
    pub(crate) field_layout_cache:
        dashmap::DashMap<(TypeDescription, GenericLookup), Arc<FieldLayoutManager>>,
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
            .entries(types.iter().map(|(k, v)| (DebugStr(k.type_name()), v)))
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
            field_layout_cache: dashmap::DashMap::new(),
        }
    }

    pub fn get_static_field_layout(
        &self,
        description: TypeDescription,
        context: &ResolutionContext,
        metrics: Option<&RuntimeMetrics>,
    ) -> Arc<FieldLayoutManager> {
        let key = (description, context.generics.clone());

        if let Some(cached) = self.field_layout_cache.get(&key) {
            return Arc::clone(&cached);
        }

        if let Some(m) = metrics {
            m.record_static_field_layout_cache_miss();
        }
        let result = Arc::new(FieldLayoutManager::static_fields_with_metrics(
            description,
            context,
            metrics,
        ));
        self.field_layout_cache.insert(key, Arc::clone(&result));
        result
    }
}

impl Default for StaticStorageManager {
    fn default() -> Self {
        Self::new()
    }
}

impl StaticStorageManager {
    pub fn get(&self, description: TypeDescription) -> Arc<StaticStorage> {
        let types = self.types.read();
        types
            .get(&description)
            .cloned()
            .expect("missing type in static storage")
    }

    /// Get the current initialization state of a type.
    pub fn get_init_state(&self, description: TypeDescription) -> u8 {
        let types = self.types.read();
        types
            .get(&description)
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
        // Ensure the storage exists. We use a write lock on the types map for this.
        {
            let mut types = self.types.write();
            types.entry(description).or_insert_with(|| {
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
        let storage = self.get(description);
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
    pub fn mark_initialized(&self, description: TypeDescription) {
        let types = self.types.read();
        if let Some(storage) = types.get(&description) {
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
        thread_manager: &impl crate::vm::threading::ThreadManagerOps,
        thread_id: u64,
        gc_coordinator: &crate::vm::gc::coordinator::GCCoordinator,
    ) {
        let storage = self.get(description);

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

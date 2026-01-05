use crate::vm::sync::{Arc, AtomicU64, Mutex, Ordering};
use std::{
    cell::Cell,
    collections::HashMap,
    thread::ThreadId,
};

thread_local! {
    /// Cached managed thread ID for the current thread
    pub(super) static MANAGED_THREAD_ID: Cell<Option<u64>> = const { Cell::new(None) };
    /// Flag indicating if this thread is currently performing GC
    pub(super) static IS_PERFORMING_GC: Cell<bool> = const { Cell::new(false) };
}

/// Represents the state of a .NET thread.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThreadState {
    /// Thread is currently running
    Running,
    /// Thread is at a safe point and can be suspended for GC
    AtSafePoint,
    /// Thread is suspended (stopped for GC)
    Suspended,
    /// Thread has exited
    Exited,
}

/// Information about a managed .NET thread.
#[derive(Debug)]
pub(super) struct ManagedThread {
    /// Native OS thread ID
    pub(super) native_id: ThreadId,
    /// .NET managed thread ID (stable u64 representation)
    pub(super) managed_id: u64,
    /// Current state of the thread
    pub(super) state: AtomicU64, // Encoded ThreadState
}

impl ManagedThread {
    pub(super) fn new(native_id: ThreadId, managed_id: u64) -> Self {
        Self {
            native_id,
            managed_id,
            state: AtomicU64::new(ThreadState::Running as u64),
        }
    }

    #[allow(dead_code)]
    pub(super) fn get_state(&self) -> ThreadState {
        match self.state.load(Ordering::Acquire) {
            0 => ThreadState::Running,
            1 => ThreadState::AtSafePoint,
            2 => ThreadState::Suspended,
            3 => ThreadState::Exited,
            _ => ThreadState::Running, // Fallback
        }
    }

    pub(super) fn set_state(&self, state: ThreadState) {
        self.state.store(state as u64, Ordering::Release);
    }
}

pub struct ThreadManager {
    /// Map from managed thread ID to thread info
    pub(super) threads: Mutex<HashMap<u64, Arc<ManagedThread>>>,
    /// Counter for allocating managed thread IDs
    pub(super) next_thread_id: AtomicU64,
    
    #[cfg(feature = "multithreaded-gc")]
    /// Whether a GC stop-the-world is currently requested
    pub(super) gc_stop_requested: crate::vm::sync::AtomicBool,
    #[cfg(feature = "multithreaded-gc")]
    /// Number of threads that have reached safe point during GC
    pub(super) threads_at_safepoint: crate::vm::sync::AtomicUsize,
    #[cfg(feature = "multithreaded-gc")]
    /// Condvar for notifying when all threads reach safe point
    pub(super) all_threads_stopped: crate::vm::sync::Condvar,
    #[cfg(feature = "multithreaded-gc")]
    /// Mutex for GC coordination
    pub(super) gc_coordination: Mutex<()>,
}

/// Get the current thread's managed ID from thread-local storage.
pub fn get_current_thread_id() -> u64 {
    MANAGED_THREAD_ID.with(|id| id.get().unwrap_or(0))
}

impl ThreadManager {
    pub fn new() -> Self {
        Self {
            threads: Mutex::new(HashMap::new()),
            next_thread_id: AtomicU64::new(1), // Thread ID 0 is reserved
            
            #[cfg(feature = "multithreaded-gc")]
            gc_stop_requested: crate::vm::sync::AtomicBool::new(false),
            #[cfg(feature = "multithreaded-gc")]
            threads_at_safepoint: crate::vm::sync::AtomicUsize::new(0),
            #[cfg(feature = "multithreaded-gc")]
            all_threads_stopped: crate::vm::sync::Condvar::new(),
            #[cfg(feature = "multithreaded-gc")]
            gc_coordination: Mutex::new(()),
        }
    }

    /// Register a new thread with the thread manager.
    /// Returns the managed thread ID assigned to this thread.
    pub fn register_thread(&self) -> u64 {
        let native_id = std::thread::current().id();
        let managed_id = self.next_thread_id.fetch_add(1, Ordering::SeqCst);

        let thread_info = Arc::new(ManagedThread::new(native_id, managed_id));

        {
            let mut threads = self.threads.lock();
            threads.insert(managed_id, thread_info);
        }

        // Cache the managed ID in thread-local storage
        MANAGED_THREAD_ID.set(Some(managed_id));

        managed_id
    }

    /// Register a new thread with tracing support
    pub fn register_thread_traced(&self, tracer: &crate::vm::gc::tracer::Tracer, name: &str) -> u64 {
        let managed_id = self.register_thread();
        tracer.trace_thread_create(0, managed_id, name);
        managed_id
    }

    /// Unregister a thread (called when thread exits).
    pub fn unregister_thread(&self, managed_id: u64) {
        let mut threads = self.threads.lock();
        if let Some(thread) = threads.get(&managed_id) {
            thread.set_state(ThreadState::Exited);
        }
        threads.remove(&managed_id);

        // Clear thread-local cache
        MANAGED_THREAD_ID.set(None);

        #[cfg(feature = "multithreaded-gc")]
        {
            // If we were at a safe point, decrement the counter
            if self.threads_at_safepoint.load(Ordering::Acquire) > 0 {
                // We don't decrement here because the thread is gone,
                // but we MUST notify the coordinator because thread_count decreased.
                self.all_threads_stopped.notify_all();
            }
        }
    }

    /// Unregister a thread with tracing support
    pub fn unregister_thread_traced(&self, managed_id: u64, tracer: &crate::vm::gc::tracer::Tracer) {
        tracer.trace_thread_exit(0, managed_id);
        self.unregister_thread(managed_id);
    }

    /// Get the current thread's managed ID.
    /// Returns None if the thread is not registered.
    pub fn current_thread_id(&self) -> Option<u64> {
        // Try thread-local cache first
        let cached_id = MANAGED_THREAD_ID.get();
        if cached_id.is_some() {
            return cached_id;
        }

        // Fallback to linear search
        let native_id = std::thread::current().id();
        let threads = self.threads.lock();
        let managed_id = threads
            .values()
            .find(|t| t.native_id == native_id)
            .map(|t| t.managed_id);

        // Update cache if found
        if managed_id.is_some() {
            MANAGED_THREAD_ID.set(managed_id);
        }

        managed_id
    }

    /// Get the number of currently active threads.
    pub fn thread_count(&self) -> usize {
        self.threads.lock().len()
    }
}

impl Default for ThreadManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(not(feature = "multithreaded-gc"))]
impl ThreadManager {
    pub fn is_gc_stop_requested(&self) -> bool {
        false
    }

    pub fn safe_point(&self, _managed_id: u64, _coordinator: &crate::vm::gc::coordinator::GCCoordinator) {}

    pub fn execute_gc_command(
        &self,
        _command: crate::vm::gc::coordinator::GCCommand,
        _coordinator: &crate::vm::gc::coordinator::GCCoordinator,
    ) {
    }

    pub fn safe_point_traced(
        &self,
        _managed_id: u64,
        _coordinator: &crate::vm::gc::coordinator::GCCoordinator,
        _tracer: &crate::vm::gc::tracer::Tracer,
        _location: &str,
    ) {
    }

    pub fn request_stop_the_world(&self) -> crate::vm::threading::gc_coord::StopTheWorldGuard<'static> {
        panic!("request_stop_the_world called without multithreaded-gc feature")
    }

    pub fn request_stop_the_world_traced(
        &self,
        _tracer: &crate::vm::gc::tracer::Tracer,
    ) -> crate::vm::threading::gc_coord::StopTheWorldGuard<'static> {
        panic!("request_stop_the_world_traced called without multithreaded-gc feature")
    }
}

#[cfg(not(feature = "multithreaded-gc"))]
pub mod gc_coord {
    pub struct StopTheWorldGuard<'a> {
        _marker: std::marker::PhantomData<&'a ()>,
    }
    impl StopTheWorldGuard<'_> {
        pub fn elapsed_micros(&self) -> u64 {
            0
        }
    }
}

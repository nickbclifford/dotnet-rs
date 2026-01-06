use crate::vm::{
    gc::{
        coordinator::{GCCommand, GCCoordinator},
        tracer::Tracer,
    },
    sync::{Arc, AtomicBool, AtomicU64, AtomicUsize, Condvar, Mutex, MutexGuard, Ordering},
    threading::{STWGuardOps, ThreadManagerOps, ThreadState},
};
use std::{
    cell::Cell,
    collections::HashMap,
    mem, sync,
    thread::{self, ThreadId},
    time::{Duration, Instant},
};

thread_local! {
    /// Cached managed thread ID for the current thread
    pub(super) static MANAGED_THREAD_ID: Cell<Option<u64>> = const { Cell::new(None) };
    /// Flag indicating if this thread is currently performing GC
    pub(super) static IS_PERFORMING_GC: Cell<bool> = const { Cell::new(false) };
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
    /// Weak reference to self for creating guards
    pub(super) self_weak: sync::OnceLock<sync::Weak<ThreadManager>>,

    #[cfg(feature = "multithreaded-gc")]
    /// Whether a GC stop-the-world is currently requested
    pub(super) gc_stop_requested: AtomicBool,
    #[cfg(feature = "multithreaded-gc")]
    /// Number of threads that have reached safe point during GC
    pub(super) threads_at_safepoint: AtomicUsize,
    #[cfg(feature = "multithreaded-gc")]
    /// Condvar for notifying when all threads reach safe point
    pub(super) all_threads_stopped: Condvar,
    #[cfg(feature = "multithreaded-gc")]
    /// Mutex for GC coordination
    pub(super) gc_coordination: Mutex<()>,
}

/// Get the current thread's managed ID from thread-local storage.
pub fn get_current_thread_id() -> u64 {
    MANAGED_THREAD_ID.with(|id| id.get().unwrap_or(0))
}

impl ThreadManager {
    pub fn new() -> Arc<Self> {
        let manager = Arc::new(Self {
            threads: Mutex::new(HashMap::new()),
            next_thread_id: AtomicU64::new(1), // Thread ID 0 is reserved
            self_weak: sync::OnceLock::new(),

            #[cfg(feature = "multithreaded-gc")]
            gc_stop_requested: AtomicBool::new(false),
            #[cfg(feature = "multithreaded-gc")]
            threads_at_safepoint: AtomicUsize::new(0),
            #[cfg(feature = "multithreaded-gc")]
            all_threads_stopped: Condvar::new(),
            #[cfg(feature = "multithreaded-gc")]
            gc_coordination: Mutex::new(()),
        });
        let _ = manager.self_weak.set(Arc::downgrade(&manager));
        manager
    }

    #[cfg(feature = "multithreaded-gc")]
    pub(super) fn resume_threads(&self) {
        self.gc_stop_requested.store(false, Ordering::Release);
        self.all_threads_stopped.notify_all();
    }
}

impl ThreadManagerOps for ThreadManager {
    type Guard = StopTheWorldGuard;

    /// Register a new thread with the thread manager.
    /// Returns the managed thread ID assigned to this thread.
    fn register_thread(&self) -> u64 {
        let native_id = thread::current().id();
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
    fn register_thread_traced(&self, tracer: &Tracer, name: &str) -> u64 {
        let managed_id = self.register_thread();
        tracer.trace_thread_create(0, managed_id, name);
        managed_id
    }

    /// Unregister a thread (called when thread exits).
    fn unregister_thread(&self, managed_id: u64) {
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
    fn unregister_thread_traced(&self, managed_id: u64, tracer: &Tracer) {
        tracer.trace_thread_exit(0, managed_id);
        self.unregister_thread(managed_id);
    }

    /// Get the current thread's managed ID.
    /// Returns None if the thread is not registered.
    fn current_thread_id(&self) -> Option<u64> {
        // Try thread-local cache first
        let cached_id = MANAGED_THREAD_ID.get();
        if cached_id.is_some() {
            return cached_id;
        }

        // Fallback to linear search
        let native_id = thread::current().id();
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
    fn thread_count(&self) -> usize {
        self.threads.lock().len()
    }

    fn is_gc_stop_requested(&self) -> bool {
        #[cfg(feature = "multithreaded-gc")]
        {
            self.gc_stop_requested.load(Ordering::Acquire)
        }
        #[cfg(not(feature = "multithreaded-gc"))]
        {
            false
        }
    }

    fn safe_point(&self, managed_id: u64, coordinator: &GCCoordinator) {
        #[cfg(feature = "multithreaded-gc")]
        {
            if !self.is_gc_stop_requested() {
                return;
            }

            if IS_PERFORMING_GC.get() {
                return;
            }

            let thread_info = {
                let threads = self.threads.lock();
                threads.get(&managed_id).cloned()
            };

            if let Some(thread) = thread_info {
                thread.set_state(ThreadState::AtSafePoint);
                self.threads_at_safepoint.fetch_add(1, Ordering::AcqRel);
                self.all_threads_stopped.notify_all();

                {
                    let mut guard = self.gc_coordination.lock();
                    while self.gc_stop_requested.load(Ordering::Acquire) {
                        if coordinator.has_command(managed_id) {
                            if let Some(command) = coordinator.get_command(managed_id) {
                                self.execute_gc_command(command, coordinator);
                                coordinator.command_finished(managed_id);
                            }
                        }
                        self.all_threads_stopped.wait(&mut guard);
                    }
                }

                self.threads_at_safepoint.fetch_sub(1, Ordering::Release);
                thread.set_state(ThreadState::Running);
            }
        }
        #[cfg(not(feature = "multithreaded-gc"))]
        {
            let _ = (managed_id, coordinator);
        }
    }

    fn execute_gc_command(&self, command: GCCommand, coordinator: &GCCoordinator) {
        #[cfg(feature = "multithreaded-gc")]
        {
            execute_gc_command_for_current_thread(command, coordinator);
        }
        #[cfg(not(feature = "multithreaded-gc"))]
        {
            let _ = (command, coordinator);
        }
    }

    fn safe_point_traced(
        &self,
        managed_id: u64,
        coordinator: &GCCoordinator,
        tracer: &Tracer,
        location: &str,
    ) {
        #[cfg(feature = "multithreaded-gc")]
        {
            if tracer.is_enabled() && self.is_gc_stop_requested() {
                tracer.trace_thread_safepoint(0, managed_id, location);
            }
            self.safe_point(managed_id, coordinator);
        }
        #[cfg(not(feature = "multithreaded-gc"))]
        {
            let _ = (managed_id, coordinator, tracer, location);
        }
    }

    fn request_stop_the_world(&self) -> Self::Guard {
        #[cfg(feature = "multithreaded-gc")]
        {
            let mut guard = self.gc_coordination.lock();
            let start_time = Instant::now();
            const WARN_TIMEOUT: Duration = Duration::from_secs(1);
            let mut warned = false;

            self.gc_stop_requested.store(true, Ordering::Release);

            loop {
                let thread_count = self.thread_count();
                let current_id = self.current_thread_id();
                let target_stopped = if current_id.is_some() {
                    thread_count.saturating_sub(1)
                } else {
                    thread_count
                };

                if self.threads_at_safepoint.load(Ordering::Acquire) >= target_stopped {
                    break;
                }

                if !warned && start_time.elapsed() > WARN_TIMEOUT {
                    eprintln!("[GC WARNING] Stop-the-world pause taking longer than expected:");
                    eprintln!("  Total threads: {}", thread_count);
                    eprintln!(
                        "  Threads at safe point: {}",
                        self.threads_at_safepoint.load(Ordering::Acquire)
                    );

                    let threads = self.threads.lock();
                    eprintln!("  Threads not at safe point:");
                    for (tid, thread) in threads.iter() {
                        if thread.get_state() != ThreadState::AtSafePoint {
                            eprintln!(
                                "    - Thread ID {}: {:?} (native: {:?})",
                                tid,
                                thread.get_state(),
                                thread.native_id
                            );
                        }
                    }
                    drop(threads);
                    warned = true;
                }

                self.all_threads_stopped.wait(&mut guard);
            }

            if warned {
                eprintln!(
                    "[GC] Stop-the-world completed after {} ms",
                    start_time.elapsed().as_millis()
                );
            }

            // Upgrade the weak reference to get an Arc
            let manager_arc = self
                .self_weak
                .get()
                .expect("ThreadManager::self_weak not initialized")
                .upgrade()
                .expect("ThreadManager dropped while request_stop_the_world was called");

            // SAFETY: We transmute the guard lifetime to 'static. This is safe because:
            // 1. We're storing an Arc<ThreadManager> which keeps the ThreadManager alive
            // 2. The guard borrows from the ThreadManager's gc_coordination mutex
            // 3. The StopTheWorldGuard holds both the Arc and the guard, ensuring the mutex
            //    outlives the guard
            let guard_static: MutexGuard<'static, ()> = unsafe { mem::transmute(guard) };
            StopTheWorldGuard::new(manager_arc, guard_static, start_time)
        }
        #[cfg(not(feature = "multithreaded-gc"))]
        {
            StopTheWorldGuard {
                _marker: PhantomData,
            }
        }
    }

    fn request_stop_the_world_traced(&self, tracer: &Tracer) -> Self::Guard {
        #[cfg(feature = "multithreaded-gc")]
        {
            let thread_count = self.thread_count();
            if tracer.is_enabled() {
                tracer.trace_stw_start(0, thread_count);
            }
            self.request_stop_the_world()
        }
        #[cfg(not(feature = "multithreaded-gc"))]
        {
            let _ = tracer;
            self.request_stop_the_world()
        }
    }
}

pub struct StopTheWorldGuard {
    #[cfg(feature = "multithreaded-gc")]
    manager: Arc<ThreadManager>,
    #[cfg(feature = "multithreaded-gc")]
    _lock: MutexGuard<'static, ()>,
    #[cfg(feature = "multithreaded-gc")]
    start_time: Instant,
}

impl StopTheWorldGuard {
    #[cfg(feature = "multithreaded-gc")]
    pub(super) fn new(
        manager: Arc<ThreadManager>,
        lock: MutexGuard<'static, ()>,
        start_time: Instant,
    ) -> Self {
        IS_PERFORMING_GC.set(true);
        Self {
            manager,
            _lock: lock,
            start_time,
        }
    }
}

impl STWGuardOps for StopTheWorldGuard {
    fn elapsed_micros(&self) -> u64 {
        #[cfg(feature = "multithreaded-gc")]
        {
            self.start_time.elapsed().as_micros() as u64
        }
        #[cfg(not(feature = "multithreaded-gc"))]
        {
            0
        }
    }
}

impl Drop for StopTheWorldGuard {
    fn drop(&mut self) {
        #[cfg(feature = "multithreaded-gc")]
        {
            self.manager.resume_threads();
            IS_PERFORMING_GC.set(false);
        }
    }
}

pub fn execute_gc_command_for_current_thread(command: GCCommand, coordinator: &GCCoordinator) {
    #[cfg(feature = "multithreaded-gc")]
    {
        use crate::vm::gc::arena::THREAD_ARENA;
        match command {
            GCCommand::CollectAll => {
                THREAD_ARENA.with(|cell| {
                    if let Ok(mut arena_opt) = cell.try_borrow_mut() {
                        if let Some(arena) = arena_opt.as_mut() {
                            let thread_id = get_current_thread_id();
                            crate::vm::gc::coordinator::set_currently_tracing(Some(thread_id));

                            arena.mutate(|_, c| {
                                c.heap().cross_arena_roots.borrow_mut().clear();
                            });

                            let mut marked = None;
                            while marked.is_none() {
                                marked = arena.mark_all();
                            }
                            if let Some(marked) = marked {
                                marked.finalize(|fc, c| c.finalize_check(fc));
                            }
                            arena.collect_all();
                            crate::vm::gc::coordinator::set_currently_tracing(None);

                            for (target_id, ptr) in
                                crate::vm::gc::coordinator::take_found_cross_arena_refs()
                            {
                                coordinator.record_cross_arena_ref(target_id, ptr);
                            }
                        }
                    }
                });
            }
            GCCommand::MarkObjects(ptrs) => {
                THREAD_ARENA.with(|cell| {
                    if let Ok(mut arena_opt) = cell.try_borrow_mut() {
                        if let Some(arena) = arena_opt.as_mut() {
                            let thread_id = get_current_thread_id();
                            crate::vm::gc::coordinator::set_currently_tracing(Some(thread_id));

                            arena.mutate(|_, c| {
                                let mut roots = c.heap().cross_arena_roots.borrow_mut();
                                for ptr in ptrs {
                                    roots.insert(ptr);
                                }
                            });

                            let mut marked = None;
                            while marked.is_none() {
                                marked = arena.mark_all();
                            }
                            if let Some(marked) = marked {
                                marked.finalize(|fc, c| c.finalize_check(fc));
                            }
                            arena.collect_all();

                            crate::vm::gc::coordinator::set_currently_tracing(None);
                            for (target_id, ptr) in
                                crate::vm::gc::coordinator::take_found_cross_arena_refs()
                            {
                                coordinator.record_cross_arena_ref(target_id, ptr);
                            }
                        }
                    }
                });
            }
        }
    }
    #[cfg(not(feature = "multithreaded-gc"))]
    {
        let _ = (command, coordinator);
    }
}

use crate::{
    gc::coordinator::{GCCommand, GCCoordinator},
    threading::{STWGuardOps, ThreadManagerOps, ThreadState},
    tracer::Tracer,
};
use dotnet_utils::{
    ArenaId,
    gc::{register_arena, set_currently_tracing, take_found_cross_arena_refs, unregister_arena},
    sync::{
        Arc, AtomicBool, AtomicU64, AtomicUsize, Condvar, MANAGED_THREAD_ID, Mutex, MutexGuard,
        Ordering, get_current_thread_id,
    },
};
use dotnet_value::object::ObjectPtr;
use std::{
    cell::Cell,
    collections::HashMap,
    mem, sync,
    thread::{self, ThreadId},
    time::{Duration, Instant},
};
use tracing::warn;

thread_local! {
    /// Flag indicating if this thread is currently performing GC
    pub(crate) static IS_PERFORMING_GC: Cell<bool> = const { Cell::new(false) };
}

/// Global counter for allocating managed thread IDs across all thread managers.
/// This prevents ArenaId collisions when parallel tests each create their own ThreadManager.
static NEXT_GLOBAL_THREAD_ID: AtomicU64 = AtomicU64::new(1); // Thread ID 0 is reserved

/// Information about a managed .NET thread.
#[derive(Debug)]
pub(super) struct ManagedThread {
    /// Native OS thread ID
    pub(super) native_id: ThreadId,
    /// .NET managed thread ID (stable representation)
    pub(super) managed_id: ArenaId,
    /// Current state of the thread
    pub(super) state: AtomicU64, // Encoded ThreadState
}

impl ManagedThread {
    pub(super) fn new(native_id: ThreadId, managed_id: ArenaId) -> Self {
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
    pub(super) threads: Mutex<HashMap<ArenaId, Arc<ManagedThread>>>,
    /// Weak reference to self for creating guards
    pub(super) self_weak: sync::OnceLock<sync::Weak<ThreadManager>>,

    gc_stop_requested: AtomicBool,
    /// Number of threads that have reached safe point during GC
    threads_at_safepoint: AtomicUsize,
    /// Condvar for notifying when all threads reach safe point
    all_threads_stopped: Condvar,
    /// Mutex for GC coordination
    gc_coordination: Mutex<()>,
    /// Flag indicating if a stop-the-world pause is in progress
    stw_in_progress: Arc<AtomicBool>,
    /// Reference to the GC coordinator for resume signaling
    coordinator: Mutex<Option<sync::Weak<GCCoordinator>>>,
}

impl ThreadManager {
    pub fn new(stw_in_progress: Arc<AtomicBool>) -> Arc<Self> {
        let manager = Arc::new(Self {
            threads: Mutex::new(HashMap::new()),
            self_weak: sync::OnceLock::new(),

            gc_stop_requested: AtomicBool::new(false),
            threads_at_safepoint: AtomicUsize::new(0),
            all_threads_stopped: Condvar::new(),
            gc_coordination: Mutex::new(()),
            stw_in_progress,
            coordinator: Mutex::new(None),
        });
        let _ = manager.self_weak.set(Arc::downgrade(&manager));
        manager
    }

    pub(super) fn resume_threads(&self) {
        self.gc_stop_requested.store(false, Ordering::Release);
        self.all_threads_stopped.notify_all();

        // Notify all arenas to check their resume condition
        let coordinator_opt = { self.coordinator.lock().clone() };
        if let Some(weak) = coordinator_opt
            && let Some(coordinator) = weak.upgrade()
        {
            coordinator.notify_resume();
        }
    }

    pub fn set_coordinator(&self, coordinator: sync::Weak<GCCoordinator>) {
        let mut guard = self.coordinator.lock();
        *guard = Some(coordinator);
    }

    fn get_coordinator(&self) -> Option<sync::Arc<GCCoordinator>> {
        let guard = self.coordinator.lock();
        guard.as_ref()?.upgrade()
    }
}

impl ThreadManagerOps for ThreadManager {
    type Guard = StopTheWorldGuard;

    /// Register a new thread with the thread manager.
    /// Returns the managed thread ID assigned to this thread.
    fn register_thread(&self) -> ArenaId {
        let native_id = thread::current().id();
        let managed_id_u64 = NEXT_GLOBAL_THREAD_ID.fetch_add(1, Ordering::Relaxed);
        let managed_id = ArenaId::new(managed_id_u64);

        let thread_info = Arc::new(ManagedThread::new(native_id, managed_id));

        {
            let mut threads = self.threads.lock();
            threads.insert(managed_id, thread_info);
        }

        // Cache the managed ID in thread-local storage
        MANAGED_THREAD_ID.set(Some(managed_id));

        // Defensive: clear any leaked state from a previous test on this OS thread
        IS_PERFORMING_GC.set(false);

        #[cfg(feature = "multithreading")]
        register_arena(managed_id, self.stw_in_progress.clone());

        managed_id
    }

    /// Register a new thread with tracing support
    fn register_thread_traced(&self, tracer: &mut Tracer, name: &str) -> ArenaId {
        let managed_id = self.register_thread();
        tracer.trace_thread_create(0, managed_id, name);
        managed_id
    }

    /// Unregister a thread (called when thread exits).
    fn unregister_thread(&self, managed_id: ArenaId) {
        let mut threads = self.threads.lock();
        if let Some(thread) = threads.get(&managed_id) {
            thread.set_state(ThreadState::Exited);
        }
        threads.remove(&managed_id);

        // Clear thread-local cache
        MANAGED_THREAD_ID.set(None);

        #[cfg(feature = "multithreading")]
        unregister_arena(managed_id);

        #[cfg(feature = "multithreading")]
        {
            // ALWAYS notify the coordinator when a thread exits, regardless of
            // safepoint count. The GC initiator in request_stop_the_world waits
            // for threads_at_safepoint >= thread_count - 1. When a thread exits,
            // thread_count decreases, potentially satisfying the condition. If we
            // don't notify, the initiator stays blocked on the condvar forever.
            self.all_threads_stopped.notify_all();
        }
    }

    /// Unregister a thread with tracing support
    fn unregister_thread_traced(&self, managed_id: ArenaId, tracer: &mut Tracer) {
        tracer.trace_thread_exit(0, managed_id);
        self.unregister_thread(managed_id);
    }

    /// Get the current thread's managed ID.
    /// Returns None if the thread is not registered.
    fn current_thread_id(&self) -> Option<ArenaId> {
        // Try thread-local cache first
        let cached_id = MANAGED_THREAD_ID.get();
        if let Some(id) = cached_id {
            return Some(id);
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
        self.gc_stop_requested.load(Ordering::Acquire)
    }

    fn safe_point(&self, managed_id: ArenaId, coordinator: &GCCoordinator) {
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

            while let Some(command) =
                coordinator.wait_for_command_or_resume(managed_id, &self.gc_stop_requested)
            {
                self.execute_gc_command(command, coordinator);
                coordinator.command_finished(managed_id);
            }

            self.threads_at_safepoint.fetch_sub(1, Ordering::Release);
            thread.set_state(ThreadState::Running);
        }
    }

    fn execute_gc_command(&self, command: GCCommand, coordinator: &GCCoordinator) {
        execute_gc_command_for_current_thread(command, coordinator);
    }

    fn safe_point_traced(
        &self,
        managed_id: ArenaId,
        coordinator: &GCCoordinator,
        tracer: &mut Tracer,
        location: &str,
    ) {
        if tracer.is_enabled() && self.is_gc_stop_requested() {
            tracer.trace_thread_safepoint(0, managed_id, location);
        }
        self.safe_point(managed_id, coordinator);
    }

    fn request_stop_the_world(&self) -> Self::Guard {
        let start_time = Instant::now();
        const WARN_TIMEOUT: Duration = Duration::from_secs(1);
        let mut warned = false;

        // Signal all threads to stop.
        // Multiple threads might do this simultaneously, which is fine.
        self.gc_stop_requested.store(true, Ordering::Release);

        let mut guard = loop {
            if let Some(g) = self.gc_coordination.try_lock() {
                break g;
            }

            // If we're not the one holding the coordination lock, we might be
            // requested to stop by the thread that DOES hold it.
            if let Some(id) = self.current_thread_id()
                && let Some(coordinator) = self.get_coordinator()
            {
                self.safe_point(id, &coordinator);
            }
            thread::yield_now();
        };

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
                warn!("[GC WARNING] Stop-the-world pause taking longer than expected:");
                warn!("  Total threads: {}", thread_count);
                warn!(
                    "  Threads at safe point: {}",
                    self.threads_at_safepoint.load(Ordering::Acquire)
                );

                let threads = self.threads.lock();
                warn!("  Threads not at safe point:");
                for (tid, thread) in threads.iter() {
                    if thread.get_state() != ThreadState::AtSafePoint {
                        warn!(
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
            warn!(
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

    fn request_stop_the_world_traced(&self, tracer: &mut Tracer) -> Self::Guard {
        let thread_count = self.thread_count();
        if tracer.is_enabled() {
            tracer.trace_stw_start(0, thread_count);
        }
        self.request_stop_the_world()
    }
}

pub struct StopTheWorldGuard {
    manager: Arc<ThreadManager>,
    _lock: MutexGuard<'static, ()>,
    start_time: Instant,
}

impl StopTheWorldGuard {
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
        self.start_time.elapsed().as_micros() as u64
    }
}

impl Drop for StopTheWorldGuard {
    fn drop(&mut self) {
        self.manager.resume_threads();
        IS_PERFORMING_GC.set(false);
    }
}

fn record_found_cross_arena_refs(coordinator: &GCCoordinator) {
    for (target_id, ptr_usize) in take_found_cross_arena_refs() {
        let ptr = unsafe { ObjectPtr::from_raw(ptr_usize as *const _) }.unwrap();
        coordinator.record_cross_arena_ref(target_id, ptr);
    }
}

pub fn execute_gc_command_for_current_thread(command: GCCommand, coordinator: &GCCoordinator) {
    use crate::gc::arena::THREAD_ARENA;
    match command {
        GCCommand::MarkAll => {
            THREAD_ARENA.with(|cell| {
                let mut arena_opt = cell.borrow_mut();
                if let Some(arena) = arena_opt.as_mut() {
                    let thread_id = get_current_thread_id();
                    set_currently_tracing(Some(thread_id));

                    arena.mutate(|_, c| {
                        c.stack.local.heap.cross_arena_roots.borrow_mut().clear();
                    });

                    let mut marked = None;
                    while marked.is_none() {
                        marked = arena.mark_all();
                    }
                    // Do not finalize or sweep yet.

                    set_currently_tracing(None);

                    record_found_cross_arena_refs(coordinator);
                }
            });
        }
        GCCommand::MarkObjects(ptrs) => {
            THREAD_ARENA.with(|cell| {
                let mut arena_opt = cell.borrow_mut();
                if let Some(arena) = arena_opt.as_mut() {
                    let thread_id = get_current_thread_id();
                    set_currently_tracing(Some(thread_id));

                    arena.mutate(|_, c| {
                        let mut roots = c.stack.local.heap.cross_arena_roots.borrow_mut();
                        for ptr_usize in ptrs {
                            let ptr =
                                unsafe { ObjectPtr::from_raw(ptr_usize as *const _) }.unwrap();
                            roots.insert(ptr);
                        }
                    });

                    let mut marked = None;
                    while marked.is_none() {
                        marked = arena.mark_all();
                    }
                    // Do not finalize or sweep yet.

                    set_currently_tracing(None);

                    record_found_cross_arena_refs(coordinator);
                }
            });
        }
        GCCommand::Finalize => {
            THREAD_ARENA.with(|cell| {
                let mut arena_opt = cell.borrow_mut();
                if let Some(arena) = arena_opt.as_mut() {
                    // Ensure we are in Marked phase
                    let mut marked = None;
                    while marked.is_none() {
                        marked = arena.mark_all();
                    }

                    if let Some(marked) = marked {
                        crate::gc::finalize_arena(marked);
                    }
                }
            });
        }
        GCCommand::Sweep => {
            THREAD_ARENA.with(|cell| {
                let mut arena_opt = cell.borrow_mut();
                if let Some(arena) = arena_opt.as_mut() {
                    // Finish the collection (finalize and sweep)
                    arena.collect_all();
                }
            });
        }
    }
}

#[cfg(debug_assertions)]
use crate::gc::coordinator::debug_assert_collection_lock_held;
use crate::{
    gc::coordinator::{
        CommandCompletionGuard, GCCommand, GCCoordinator, MarkPhaseCommand, SweepPhaseCommand,
    },
    threading::{IS_PERFORMING_GC, STWGuardOps, ThreadState},
};
use dotnet_tracer::Tracer;
use dotnet_utils::{
    ArenaId,
    gc::{
        register_arena, set_currently_tracing, take_found_cross_arena_refs_with_generation,
        try_acquire_lease,
    },
    sync::{
        Arc, AtomicBool, AtomicU64, AtomicUsize, Condvar, MANAGED_THREAD_ID, Mutex, OrderedMutex,
        OrderedMutexGuard, Ordering, Weak, get_current_thread_id, levels,
    },
};
use dotnet_value::object::ObjectPtr;
use std::{
    collections::HashMap,
    thread::{self, ThreadId},
    time::{Duration, Instant},
};
use tracing::warn;

#[cfg(feature = "deadlock-diagnostics")]
use parking_lot::deadlock;

#[cfg(debug_assertions)]
macro_rules! debug_assert_top_level_gc_lock_order {
    ($manager:expr, $context:expr) => {
        if $manager.stw_in_progress.load(Ordering::Acquire) {
            debug_assert_collection_lock_held($context);
        }
    };
}

#[cfg(not(debug_assertions))]
macro_rules! debug_assert_top_level_gc_lock_order {
    ($manager:expr, $context:expr) => {
        let _ = (&$manager, &$context);
    };
}

#[cfg(feature = "deadlock-diagnostics")]
fn deadlock_report_interval() -> Duration {
    const ENV_NAME: &str = "DOTNET_DEADLOCK_DIAGNOSTICS_INTERVAL_MS";
    const DEFAULT_MS: u64 = 10_000;

    match std::env::var(ENV_NAME) {
        Ok(raw) => match raw.parse::<u64>() {
            Ok(0) => {
                warn!("{}=0 is invalid, using default {} ms", ENV_NAME, DEFAULT_MS);
                Duration::from_millis(DEFAULT_MS)
            }
            Ok(ms) => Duration::from_millis(ms),
            Err(error) => {
                warn!(
                    "Invalid {} value {:?}: {}. Using default {} ms",
                    ENV_NAME, raw, error, DEFAULT_MS
                );
                Duration::from_millis(DEFAULT_MS)
            }
        },
        Err(_) => Duration::from_millis(DEFAULT_MS),
    }
}

#[cfg(feature = "deadlock-diagnostics")]
struct DeadlockReporter {
    shutdown: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}

#[cfg(feature = "deadlock-diagnostics")]
impl DeadlockReporter {
    fn start() -> Self {
        let shutdown = Arc::new(AtomicBool::new(false));
        let shutdown_for_thread = Arc::clone(&shutdown);
        let interval = deadlock_report_interval();

        let thread = thread::Builder::new()
            .name("dotnet-deadlock-reporter".to_string())
            .spawn(move || {
                while !shutdown_for_thread.load(Ordering::Acquire) {
                    thread::park_timeout(interval);
                    if shutdown_for_thread.load(Ordering::Acquire) {
                        break;
                    }

                    let deadlocks = deadlock::check_deadlock();
                    if deadlocks.is_empty() {
                        continue;
                    }

                    warn!(
                        "[deadlock-diagnostics] detected {} deadlock cycle(s)",
                        deadlocks.len()
                    );
                    for (cycle_index, cycle) in deadlocks.iter().enumerate() {
                        warn!(
                            "[deadlock-diagnostics] cycle {} has {} thread(s)",
                            cycle_index + 1,
                            cycle.len()
                        );
                        for thread in cycle {
                            warn!(
                                "[deadlock-diagnostics] cycle {} thread {:?}\n{:?}",
                                cycle_index + 1,
                                thread.thread_id(),
                                thread.backtrace()
                            );
                        }
                    }
                }
            })
            .map_err(|error| {
                warn!(
                    "[deadlock-diagnostics] failed to start reporter thread: {}",
                    error
                );
            })
            .ok();

        Self { shutdown, thread }
    }

    fn stop(&mut self) {
        self.shutdown.store(true, Ordering::Release);
        if let Some(handle) = self.thread.take() {
            handle.thread().unpark();
            if handle.join().is_err() {
                warn!("[deadlock-diagnostics] reporter thread panicked during shutdown");
            }
        }
    }
}

#[cfg(feature = "deadlock-diagnostics")]
impl Drop for DeadlockReporter {
    fn drop(&mut self) {
        self.stop();
    }
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

    pub(super) fn get_state(&self) -> Result<ThreadState, u64> {
        ThreadState::try_from(self.state.load(Ordering::Acquire))
    }

    pub(super) fn set_state(&self, state: ThreadState) {
        self.state.store(state as u64, Ordering::Release);
    }
}

pub struct ThreadManager {
    /// Map from managed thread ID to thread info
    pub(super) threads: OrderedMutex<levels::ThreadRegistry, HashMap<ArenaId, Arc<ManagedThread>>>,

    gc_stop_requested: AtomicBool,
    /// Number of threads that have reached safe point during GC
    threads_at_safepoint: AtomicUsize,
    /// Condvar for notifying when all threads reach safe point
    all_threads_stopped: Condvar,
    /// Mutex for GC coordination
    gc_coordination: OrderedMutex<levels::GcCoordination, ()>,
    /// Flag indicating if a stop-the-world pause is in progress
    stw_in_progress: Arc<AtomicBool>,
    /// Reference to the GC coordinator for resume signaling
    coordinator: Mutex<Option<Weak<GCCoordinator>>>,
    #[cfg(feature = "deadlock-diagnostics")]
    _deadlock_reporter: DeadlockReporter,
}

impl ThreadManager {
    pub fn new(stw_in_progress: Arc<AtomicBool>) -> Arc<Self> {
        Arc::new(Self {
            threads: OrderedMutex::new(HashMap::new()),

            gc_stop_requested: AtomicBool::new(false),
            threads_at_safepoint: AtomicUsize::new(0),
            all_threads_stopped: Condvar::new(),
            gc_coordination: OrderedMutex::new(()),
            stw_in_progress,
            coordinator: Mutex::new(None),
            #[cfg(feature = "deadlock-diagnostics")]
            _deadlock_reporter: DeadlockReporter::start(),
        })
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

    pub fn set_coordinator(&self, coordinator: Weak<GCCoordinator>) {
        let mut guard = self.coordinator.lock();
        *guard = Some(coordinator);
    }

    fn get_coordinator(&self) -> Option<Arc<GCCoordinator>> {
        let guard = self.coordinator.lock();
        guard.as_ref()?.upgrade()
    }
}

impl super::ThreadManagerBackend for ThreadManager {
    type Guard<'a> = StopTheWorldGuard<'a>;

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

        // NOTE: We do NOT call unregister_arena here anymore.
        // Arena unregistration happens when the arena memory is actually freed:
        // - In Executor::drop when THREAD_ARENA is cleared (normal case)
        // - In ArenaGuard::drop when the extracted arena is dropped (test case)
        // This ensures the arena remains valid for cross-arena references until
        // the memory is actually destroyed.

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
                // Arm the guard before executing so that a panic inside
                // `execute_gc_command` still delivers the finish signal to the
                // GC initiator, preventing it from blocking in
                // `wait_on_other_arenas` forever.
                let completion_guard = CommandCompletionGuard::new(managed_id, coordinator);
                self.execute_gc_command(command, coordinator);
                let _disarmed_guard = completion_guard.disarm();
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

    fn request_stop_the_world(&self) -> Self::Guard<'_> {
        let start_time = Instant::now();
        const WARN_TIMEOUT: Duration = Duration::from_secs(1);
        #[cfg(feature = "multithreading")]
        const WAIT_RECHECK_INTERVAL: Duration = Duration::from_millis(10);
        let mut warned = false;

        // Signal all threads to stop.
        // Multiple threads might do this simultaneously, which is fine.
        self.gc_stop_requested.store(true, Ordering::Release);

        // Panic guard: if anything below panics before we hand ownership to
        // `StopTheWorldGuard`, resume threads so they are never left hanging.
        // Disarmed only on the successful return path via `panic_guard.disarm()`.
        let mut panic_guard = ResumeOnPanic::new(self);

        let mut guard = loop {
            if let Some(g) = self.gc_coordination.try_lock() {
                debug_assert_top_level_gc_lock_order!(
                    self,
                    "ThreadManager::request_stop_the_world"
                );
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
            let (thread_count, current_id) = {
                let native_id = thread::current().id();
                let threads = self.threads.lock_after(guard.held_level());
                let current_id = MANAGED_THREAD_ID.get().or_else(|| {
                    threads
                        .values()
                        .find(|t| t.native_id == native_id)
                        .map(|t| t.managed_id)
                });
                if current_id.is_some() {
                    MANAGED_THREAD_ID.set(current_id);
                }
                (threads.len(), current_id)
            };
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

                let threads = self.threads.lock_after(guard.held_level());
                warn!("  Threads not at safe point:");
                for (tid, thread) in threads.iter() {
                    if thread.get_state() != Ok(ThreadState::AtSafePoint) {
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

            // Re-check periodically to avoid a lost-notify deadlock window:
            // thread_count / threads_at_safepoint are updated outside
            // gc_coordination, so a notify can race with this wait call.
            #[cfg(feature = "multithreading")]
            {
                self.all_threads_stopped
                    .wait_for(guard.raw_mut(), WAIT_RECHECK_INTERVAL);
            }
            #[cfg(not(feature = "multithreading"))]
            {
                self.all_threads_stopped.wait(guard.raw_mut());
            }
        }

        if warned {
            warn!(
                "[GC] Stop-the-world completed after {} ms",
                start_time.elapsed().as_millis()
            );
        }

        // Disarm: StopTheWorldGuard takes over responsibility for resume.
        panic_guard.disarm();
        StopTheWorldGuard::new(self, guard, start_time)
    }

    fn request_stop_the_world_traced(&self, tracer: &mut Tracer) -> Self::Guard<'_> {
        let thread_count = self.thread_count();
        if tracer.is_enabled() {
            tracer.trace_stw_start(0, thread_count);
        }
        self.request_stop_the_world()
    }
}

pub struct StopTheWorldGuard<'a> {
    manager: &'a ThreadManager,
    _lock: OrderedMutexGuard<'a, levels::GcCoordination, ()>,
    start_time: Instant,
}

impl<'a> StopTheWorldGuard<'a> {
    pub(super) fn new(
        manager: &'a ThreadManager,
        lock: OrderedMutexGuard<'a, levels::GcCoordination, ()>,
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

impl STWGuardOps for StopTheWorldGuard<'_> {
    fn elapsed_micros(&self) -> u64 {
        self.start_time.elapsed().as_micros() as u64
    }
}

impl Drop for StopTheWorldGuard<'_> {
    fn drop(&mut self) {
        // Ensure IS_PERFORMING_GC is always cleared even if resume_threads
        // panics, so the thread-local flag is never left in a stale "true" state.
        // GcFlagGuard is declared before the resume call so it is dropped last
        // (reverse declaration order), guaranteeing the flag is cleared on both
        // the normal path and any panic unwind from resume_threads.
        struct GcFlagGuard;
        impl Drop for GcFlagGuard {
            fn drop(&mut self) {
                IS_PERFORMING_GC.set(false);
            }
        }
        let _flag_guard = GcFlagGuard;
        self.manager.resume_threads();
        // _flag_guard drops here (or on unwind), clearing IS_PERFORMING_GC.
    }
}

/// RAII panic guard used inside [`ThreadManager::request_stop_the_world`].
///
/// Calls [`ThreadManager::resume_threads`] on drop unless explicitly
/// [`disarm`](Self::disarm)ed, ensuring threads are never left suspended if the
/// caller unwinds before handing responsibility to [`StopTheWorldGuard`].
///
/// # Composition
/// This guard intentionally mirrors the `disarm()`-before-handover discipline
/// used by other panic-safe GC drop guards in the codebase.
/// `Executor::perform_full_gc` composes this with
/// `gc::coordinator::GcCycleGuard`, which guarantees `CollectionSession`
/// cleanup runs before `StopTheWorldGuard` drop on unwind.
pub(super) struct ResumeOnPanic<'m> {
    manager: &'m ThreadManager,
    /// `true` once [`Self::disarm`] has been called; `Drop` is a no-op when set.
    disarmed: bool,
}

impl<'m> ResumeOnPanic<'m> {
    /// Create a new armed guard for `manager`.
    pub(super) fn new(manager: &'m ThreadManager) -> Self {
        Self {
            manager,
            disarmed: false,
        }
    }

    /// Disarm the guard so that `Drop` does **not** call `resume_threads`.
    ///
    /// Call this on the normal completion path once [`StopTheWorldGuard`] has
    /// taken over responsibility for resuming threads.
    pub(super) fn disarm(&mut self) {
        self.disarmed = true;
    }
}

impl Drop for ResumeOnPanic<'_> {
    fn drop(&mut self) {
        if !self.disarmed {
            // Scope exited without a `disarm()` call — either the caller
            // panicked or forgot to transfer ownership to StopTheWorldGuard.
            // Resume threads so none are left hanging at a safe point forever.
            self.manager.resume_threads();
        }
    }
}

fn record_found_cross_arena_refs(coordinator: &GCCoordinator) {
    for (target_id, ptr_usize, recorded_gen) in take_found_cross_arena_refs_with_generation() {
        if let Some(lease) = try_acquire_lease(target_id) {
            if lease.generation() == recorded_gen {
                // SAFETY: The pointer was recorded from a live cross-arena reference.
                // We hold a matching generation lease for `target_id`, so teardown
                // cannot complete while reconstructing `ObjectPtr`.
                let ptr = unsafe { ObjectPtr::from_raw(ptr_usize as *const _) }.unwrap();
                coordinator.record_cross_arena_ref(target_id, ptr);
            } else {
                warn!(
                    "Ignoring found cross-arena reference to re-registered arena {:?} (gen mismatch: expected {}, got {})",
                    target_id,
                    recorded_gen,
                    lease.generation()
                );
            }
        } else {
            warn!(
                "Ignoring found cross-arena reference to exited arena {:?}",
                target_id
            );
        }
    }
}

pub fn execute_gc_command_for_current_thread(command: GCCommand, coordinator: &GCCoordinator) {
    use crate::gc::arena::THREAD_ARENA;
    match command {
        GCCommand::Mark(mark_command) => match mark_command {
            MarkPhaseCommand::All => {
                THREAD_ARENA.with(|cell| {
                    let mut arena_opt = cell.try_borrow_mut().expect("Nested arena borrow detected during GC command execution! This is a violation of safe-point invariants.");
                    if let Some(arena) = arena_opt.as_mut() {
                        let thread_id = get_current_thread_id();
                        set_currently_tracing(Some(thread_id));

                        #[cfg(feature = "bench-instrumentation")]
                        let clear_cross_arena_roots_start = Instant::now();
                        arena.mutate(|_, c| {
                            c.stack.local.heap.cross_arena_roots.borrow_mut().clear();
                        });
                        #[cfg(feature = "bench-instrumentation")]
                        dotnet_metrics::record_active_gc_trace_root_timing(
                            "mark_all.clear_cross_arena_roots",
                            clear_cross_arena_roots_start.elapsed(),
                        );

                        #[cfg(feature = "bench-instrumentation")]
                        let finish_marking_start = Instant::now();
                        let _ = arena.finish_marking();
                        // Do not finalize or sweep yet.
                        #[cfg(feature = "bench-instrumentation")]
                        dotnet_metrics::record_active_gc_trace_root_timing(
                            "mark_all.finish_marking",
                            finish_marking_start.elapsed(),
                        );

                        set_currently_tracing(None);

                        #[cfg(feature = "bench-instrumentation")]
                        let harvest_cross_arena_refs_start = Instant::now();
                        record_found_cross_arena_refs(coordinator);
                        #[cfg(feature = "bench-instrumentation")]
                        dotnet_metrics::record_active_gc_trace_root_timing(
                            "mark_all.harvest_cross_arena_refs",
                            harvest_cross_arena_refs_start.elapsed(),
                        );
                    }
                });
            }
            MarkPhaseCommand::Objects(ptrs) => {
                THREAD_ARENA.with(|cell| {
                    let mut arena_opt = cell.try_borrow_mut().expect("Nested arena borrow detected during GC command execution! This is a violation of safe-point invariants.");
                    if let Some(arena) = arena_opt.as_mut() {
                        let thread_id = get_current_thread_id();
                        set_currently_tracing(Some(thread_id));

                        #[cfg(feature = "bench-instrumentation")]
                        let install_roots_start = Instant::now();
                        arena.mutate(|_, c| {
                            let mut roots = c.stack.local.heap.cross_arena_roots.borrow_mut();
                            for ptr in ptrs {
                                // SAFETY: `ptrs` originates from coordinator-owned object
                                // pointers captured during marking. They are only consumed
                                // during the same collection cycle while arenas are stopped.
                                let ptr =
                                    unsafe { ObjectPtr::from_raw(ptr.as_ptr() as *const _) }
                                        .unwrap();
                                roots.insert(ptr);
                            }
                        });
                        #[cfg(feature = "bench-instrumentation")]
                        dotnet_metrics::record_active_gc_trace_root_timing(
                            "mark_objects.install_cross_arena_roots",
                            install_roots_start.elapsed(),
                        );

                        #[cfg(feature = "bench-instrumentation")]
                        let finish_marking_start = Instant::now();
                        let _ = arena.finish_marking();
                        // Do not finalize or sweep yet.
                        #[cfg(feature = "bench-instrumentation")]
                        dotnet_metrics::record_active_gc_trace_root_timing(
                            "mark_objects.finish_marking",
                            finish_marking_start.elapsed(),
                        );

                        set_currently_tracing(None);

                        #[cfg(feature = "bench-instrumentation")]
                        let harvest_cross_arena_refs_start = Instant::now();
                        record_found_cross_arena_refs(coordinator);
                        #[cfg(feature = "bench-instrumentation")]
                        dotnet_metrics::record_active_gc_trace_root_timing(
                            "mark_objects.harvest_cross_arena_refs",
                            harvest_cross_arena_refs_start.elapsed(),
                        );
                    }
                });
            }
        },
        GCCommand::Sweep(sweep_command) => match sweep_command {
            SweepPhaseCommand::Finalize => {
                THREAD_ARENA.with(|cell| {
                    let mut arena_opt = cell.try_borrow_mut().expect("Nested arena borrow detected during GC command execution! This is a violation of safe-point invariants.");
                    if let Some(arena) = arena_opt.as_mut() {
                        // Ensure we are in Marked phase
                        let marked = arena.finish_marking();

                        if let Some(marked) = marked {
                            crate::gc::finalize_arena(marked);
                        }
                    }
                });
            }
            SweepPhaseCommand::Sweep => {
                THREAD_ARENA.with(|cell| {
                    let mut arena_opt = cell.try_borrow_mut().expect("Nested arena borrow detected during GC command execution! This is a violation of safe-point invariants.");
                    if let Some(arena) = arena_opt.as_mut() {
                        // Finish the collection (finalize and sweep)
                        arena.finish_cycle();
                    }
                });
            }
        },
    }
}

#[cfg(all(test, feature = "multithreading"))]
mod tests {
    use super::*;
    use crate::threading::ThreadManagerOps;
    use std::{sync::mpsc, thread, time::Duration};

    // ---------------------------------------------------------------------------
    // Helpers — `ResumeOnPanic` is defined at module level (above) and is
    // available here via `use super::*`.
    // ---------------------------------------------------------------------------

    #[test]
    fn stw_guard_drop_resets_gc_flags() {
        let manager = ThreadManager::new(Arc::new(AtomicBool::new(false)));

        assert!(!manager.is_gc_stop_requested());
        assert!(!IS_PERFORMING_GC.get());

        {
            let _guard = manager.request_stop_the_world();
            assert!(manager.is_gc_stop_requested());
            assert!(IS_PERFORMING_GC.get());
        }

        assert!(!manager.is_gc_stop_requested());
        assert!(!IS_PERFORMING_GC.get());
    }

    #[test]
    fn stw_guard_holds_gc_coordination_lock_until_drop() {
        let manager = ThreadManager::new(Arc::new(AtomicBool::new(false)));
        let first_guard = manager.request_stop_the_world();

        let manager_clone = Arc::clone(&manager);
        let (entered_tx, entered_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();

        let join = thread::spawn(move || {
            let _second_guard = manager_clone.request_stop_the_world();
            entered_tx.send(()).unwrap();
            release_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        });

        assert!(
            entered_rx.recv_timeout(Duration::from_millis(100)).is_err(),
            "second request_stop_the_world unexpectedly acquired coordination lock early"
        );

        drop(first_guard);

        entered_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("second request_stop_the_world did not acquire coordination lock after drop");
        release_tx.send(()).unwrap();
        join.join().unwrap();
    }

    #[cfg(debug_assertions)]
    #[test]
    fn lock_order_request_stop_the_world_rejects_inverted_top_level_order() {
        use std::panic;

        // Simulate a caller requesting STW while the coordinator reports an
        // active collection but the current thread does not hold
        // GCCoordinator::collection_lock.
        let manager = ThreadManager::new(Arc::new(AtomicBool::new(true)));

        let result = panic::catch_unwind(panic::AssertUnwindSafe(|| {
            let _guard = manager.request_stop_the_world();
        }));

        let panic_payload =
            result.expect_err("inverted top-level lock order must panic in debug builds");
        let message = if let Some(msg) = panic_payload.downcast_ref::<&str>() {
            (*msg).to_string()
        } else if let Some(msg) = panic_payload.downcast_ref::<String>() {
            msg.clone()
        } else {
            "<non-string panic payload>".to_string()
        };

        assert!(
            message.contains("top-level lock order violation"),
            "unexpected panic payload: {message}"
        );
        assert!(
            !manager.is_gc_stop_requested(),
            "panic guard must clear gc_stop_requested on unwind"
        );
    }

    /// `resume_threads` is idempotent: calling it multiple times must not panic
    /// and must leave `gc_stop_requested` cleared.
    #[test]
    fn resume_threads_is_idempotent() {
        let manager = ThreadManager::new(Arc::new(AtomicBool::new(false)));

        // Prime the flag as if STW was in progress.
        manager.gc_stop_requested.store(true, Ordering::Release);
        assert!(manager.is_gc_stop_requested());

        manager.resume_threads();
        assert!(!manager.is_gc_stop_requested());

        // Second call: must be a no-op, not a panic or corruption.
        manager.resume_threads();
        assert!(!manager.is_gc_stop_requested());
    }

    /// When the `ResumeOnPanic` guard is armed and then dropped (simulating an
    /// unwind before `StopTheWorldGuard` is constructed), it calls
    /// `resume_threads` so that no other thread hangs waiting for resume.
    #[test]
    fn stw_panic_guard_armed_resumes_threads_on_drop() {
        let manager = ThreadManager::new(Arc::new(AtomicBool::new(false)));

        // Simulate: gc_stop_requested was set by request_stop_the_world, then
        // a panic fires before the StopTheWorldGuard is returned.
        manager.gc_stop_requested.store(true, Ordering::Release);
        assert!(manager.is_gc_stop_requested());

        {
            let _guard = ResumeOnPanic::new(&manager);
            // Drop without disarming → resume_threads() is called.
        }

        assert!(
            !manager.is_gc_stop_requested(),
            "armed ResumeOnPanic must clear gc_stop_requested on drop"
        );
    }

    /// When the `ResumeOnPanic` guard is disarmed (normal STW success path),
    /// dropping it must NOT call `resume_threads` again.
    #[test]
    fn stw_panic_guard_disarmed_does_not_resume_on_drop() {
        let manager = ThreadManager::new(Arc::new(AtomicBool::new(false)));

        manager.gc_stop_requested.store(true, Ordering::Release);

        {
            let mut guard = ResumeOnPanic::new(&manager);
            guard.disarm(); // disarm — StopTheWorldGuard takes ownership
            // Drop: disarmed=true, so resume_threads is NOT called.
        }

        // gc_stop_requested is still true: the guard was correctly disarmed and
        // did not interfere with the subsequent StopTheWorldGuard.
        assert!(
            manager.is_gc_stop_requested(),
            "disarmed ResumeOnPanic must not call resume_threads"
        );

        // Clean up.
        manager.resume_threads();
    }

    /// Dropping `StopTheWorldGuard` clears `IS_PERFORMING_GC` — the
    /// `GcFlagGuard` inside `Drop` ensures the flag is always reset.
    #[test]
    fn stw_guard_gc_flag_cleared_on_drop() {
        let manager = ThreadManager::new(Arc::new(AtomicBool::new(false)));

        IS_PERFORMING_GC.set(false);
        let guard = manager.request_stop_the_world();
        assert!(
            IS_PERFORMING_GC.get(),
            "IS_PERFORMING_GC must be true while guard is live"
        );

        drop(guard);
        assert!(
            !IS_PERFORMING_GC.get(),
            "IS_PERFORMING_GC must be false after StopTheWorldGuard drop"
        );
    }

    #[test]
    fn test_managed_thread_get_state_valid_and_invalid() {
        let native_id = thread::current().id();
        let managed_id = ArenaId::new(1);
        let thread = ManagedThread::new(native_id, managed_id);

        // Initial state should be Running (0)
        assert_eq!(thread.get_state(), Ok(ThreadState::Running));

        // Set to AtSafePoint (1)
        thread.set_state(ThreadState::AtSafePoint);
        assert_eq!(thread.get_state(), Ok(ThreadState::AtSafePoint));

        // Set to Exited (2)
        thread.set_state(ThreadState::Exited);
        assert_eq!(thread.get_state(), Ok(ThreadState::Exited));

        // Manually set to an invalid value (e.g., 3, which was Exited before refactor)
        thread.state.store(3, Ordering::Release);
        assert_eq!(thread.get_state(), Err(3));

        // Manually set to another invalid value
        thread.state.store(255, Ordering::Release);
        assert_eq!(thread.get_state(), Err(255));
    }

    // ------------------------------------------------------------------
    // Stress: register/unregister interleaved with repeated STW cycles
    //
    // N worker threads register, spin calling safe_point, then unregister.
    // An unregistered GC driver thread fires request_stop_the_world in a loop.
    // All threads must quiesce at each safepoint and resume cleanly; no
    // deadlocks, panics, or leaked gc_stop_requested state are permitted.
    // ------------------------------------------------------------------
    #[test]
    fn stress_register_unregister_interleaved_with_stw() {
        use std::sync::Arc as StdArc;
        use std::sync::Barrier;

        const WORKER_THREADS: usize = 3;
        const STW_CYCLES: usize = 5;

        let stw_flag = Arc::new(AtomicBool::new(false));
        let manager = ThreadManager::new(stw_flag.clone());
        let coordinator = Arc::new(GCCoordinator::new(stw_flag));
        manager.set_coordinator(Arc::downgrade(&coordinator));

        let barrier = StdArc::new(Barrier::new(WORKER_THREADS + 1));
        let done = Arc::new(AtomicBool::new(false));

        let mut handles = vec![];

        for _ in 0..WORKER_THREADS {
            let manager = Arc::clone(&manager);
            let coordinator = Arc::clone(&coordinator);
            let barrier = barrier.clone();
            let done = done.clone();
            handles.push(thread::spawn(move || {
                let managed_id = manager.register_thread();
                barrier.wait(); // synchronise: all workers registered before GC starts

                // Spin at safepoints until the GC driver signals completion.
                while !done.load(Ordering::Acquire) {
                    manager.safe_point(managed_id, &coordinator);
                    thread::yield_now();
                }

                // One final pass to drain any in-flight STW.
                manager.safe_point(managed_id, &coordinator);
                manager.unregister_thread(managed_id);
            }));
        }

        // GC driver is not a registered managed thread.
        barrier.wait();

        for _ in 0..STW_CYCLES {
            // Guard drop calls resume_threads(), clearing gc_stop_requested.
            let _guard = manager.request_stop_the_world();
        }

        // Tell workers to exit, then wake any still blocked in safe_point.
        done.store(true, Ordering::Release);
        manager.resume_threads();

        for h in handles {
            h.join().unwrap();
        }

        assert!(
            !manager.is_gc_stop_requested(),
            "gc_stop_requested must be clear after stress"
        );
        assert!(
            !IS_PERFORMING_GC.get(),
            "IS_PERFORMING_GC must be clear after stress"
        );
    }

    // ------------------------------------------------------------------
    // Stress: unregister during STW wait must not deadlock
    //
    // The GC driver waits for threads_at_safepoint >= thread_count.  When a
    // thread unregisters instead of hitting a safepoint, thread_count
    // decreases and all_threads_stopped is notified.  The driver must detect
    // the reduced thread count and proceed without hanging.
    // ------------------------------------------------------------------
    #[test]
    fn stress_unregister_during_stw_wait_does_not_deadlock() {
        use std::sync::Arc as StdArc;
        use std::sync::Barrier;

        const WORKER_THREADS: usize = 3;
        const ROUNDS: usize = 4;

        let stw_flag = Arc::new(AtomicBool::new(false));
        let manager = ThreadManager::new(stw_flag.clone());
        let coordinator = Arc::new(GCCoordinator::new(stw_flag));
        manager.set_coordinator(Arc::downgrade(&coordinator));

        for _ in 0..ROUNDS {
            let barrier = StdArc::new(Barrier::new(WORKER_THREADS + 1));

            let mut handles = vec![];

            for _ in 0..WORKER_THREADS {
                let manager = Arc::clone(&manager);
                let coordinator = Arc::clone(&coordinator);
                let barrier = barrier.clone();
                handles.push(thread::spawn(move || {
                    let managed_id = manager.register_thread();
                    barrier.wait(); // let GC driver start STW before we react

                    // Each worker either hits a safe_point OR simply unregisters
                    // without going through a safepoint, interleaved by thread
                    // scheduling.  Both paths must allow the GC driver to make
                    // forward progress (no hang).
                    manager.safe_point(managed_id, &coordinator);
                    manager.unregister_thread(managed_id);
                }));
            }

            // GC driver fires STW right after all workers are registered.
            barrier.wait();
            {
                let _guard = manager.request_stop_the_world();
                // Guard drops here: resume_threads() fires.
            }

            for h in handles {
                h.join().unwrap();
            }
        }

        assert!(!manager.is_gc_stop_requested());
        assert!(!IS_PERFORMING_GC.get());
    }
}

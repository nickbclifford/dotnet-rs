#[cfg(feature = "multithreading")]
use crate::{
    sync::{HeldLockLevel, OrderedMutex, OrderedMutexGuard, Ordering, levels},
    threading::execute_gc_command_for_current_thread,
};
#[cfg(feature = "multithreading")]
use dotnet_utils::sync::{Arc, AtomicBool};
#[cfg(feature = "multithreading")]
use dotnet_value::object::ObjectPtr;
#[cfg(all(feature = "multithreading", debug_assertions))]
use std::cell::Cell;
#[cfg(feature = "multithreading")]
use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    marker::PhantomData,
    mem::ManuallyDrop,
};

#[cfg(feature = "multithreading")]
pub use dotnet_utils::gc::{
    ALLOCATION_THRESHOLD, ArenaHandle, GCCommand, MarkObjectPointers, MarkPhaseCommand,
    OpaqueObjectPtr, SweepPhaseCommand, clear_tracing_state, get_currently_tracing,
    set_currently_tracing,
};

#[cfg(feature = "multithreading")]
/// Coordinates stop-the-world collections across multiple thread-local arenas.
pub struct GCCoordinator {
    /// thread_id -> arena metadata
    arenas: OrderedMutex<levels::CoordinatorArenas, HashMap<dotnet_utils::ArenaId, ArenaHandle>>,
    /// Global lock held during collection
    collection_lock: OrderedMutex<levels::CollectionLock, ()>,
    /// Flag indicating if a stop-the-world pause is in progress (shared with arenas)
    stw_in_progress: Arc<AtomicBool>,
    /// Cross-arena references found during marking
    cross_arena_refs:
        OrderedMutex<levels::CrossArenaRefs, HashMap<dotnet_utils::ArenaId, HashSet<ObjectPtr>>>,
}

#[cfg(all(feature = "multithreading", debug_assertions))]
thread_local! {
    static COLLECTION_LOCK_DEBUG_DEPTH: Cell<u32> = const { Cell::new(0) };
}

#[cfg(all(feature = "multithreading", debug_assertions))]
fn debug_mark_collection_lock_acquired() {
    COLLECTION_LOCK_DEBUG_DEPTH.with(|depth| depth.set(depth.get() + 1));
}

#[cfg(all(feature = "multithreading", debug_assertions))]
fn debug_mark_collection_lock_released() {
    COLLECTION_LOCK_DEBUG_DEPTH.with(|depth| {
        let current = depth.get();
        assert!(
            current > 0,
            "debug_mark_collection_lock_released: lock-depth underflow"
        );
        depth.set(current - 1);
    });
}

#[cfg(all(feature = "multithreading", debug_assertions))]
pub(crate) fn debug_assert_collection_lock_held(context: &str) {
    COLLECTION_LOCK_DEBUG_DEPTH.with(|depth| {
        debug_assert!(
            depth.get() > 0,
            "{context}: top-level lock order violation: expected GCCoordinator::collection_lock before ThreadManager::gc_coordination",
        );
    });
}

#[cfg(all(feature = "multithreading", debug_assertions))]
struct CollectionLockDebugGuard;

#[cfg(all(feature = "multithreading", debug_assertions))]
impl CollectionLockDebugGuard {
    fn new() -> Self {
        debug_mark_collection_lock_acquired();
        Self
    }
}

#[cfg(all(feature = "multithreading", debug_assertions))]
impl Drop for CollectionLockDebugGuard {
    fn drop(&mut self) {
        debug_mark_collection_lock_released();
    }
}

#[cfg(all(feature = "multithreading", not(debug_assertions)))]
struct CollectionLockDebugGuard;

#[cfg(all(feature = "multithreading", not(debug_assertions)))]
impl CollectionLockDebugGuard {
    fn new() -> Self {
        Self
    }
}

#[cfg(feature = "multithreading")]
impl GCCoordinator {
    pub fn new(stw_in_progress: Arc<AtomicBool>) -> Self {
        Self {
            arenas: OrderedMutex::new(HashMap::new()),
            collection_lock: OrderedMutex::new(()),
            stw_in_progress,
            cross_arena_refs: OrderedMutex::new(HashMap::new()),
        }
    }

    fn assert_no_pending_commands(
        &self,
        held: HeldLockLevel<levels::CollectionLock>,
        context: &str,
    ) {
        let arenas = self.arenas.lock_after(held);
        for handle in arenas.values() {
            let cmd = handle.current_command().lock_after(arenas.held_level());
            assert!(
                cmd.is_none(),
                "{context}: expected no pending GC command for arena {:?}, found {:?}",
                handle.thread_id(),
                *cmd
            );
        }
    }

    /// Register a thread-local arena with the coordinator.
    pub fn register_arena(&self, handle: ArenaHandle) {
        let mut arenas = self.arenas.lock();
        arenas.insert(handle.thread_id(), handle);
    }

    /// Unregister a thread-local arena.
    pub fn unregister_arena(&self, thread_id: dotnet_utils::ArenaId) {
        let mut arenas = self.arenas.lock();
        if let Some(handle) = arenas.remove(&thread_id) {
            // Signal any threads waiting for this arena to finish a GC command.
            // This is critical if the thread is exiting during a stop-the-world pause.
            let mut cmd = handle.current_command().lock_after(arenas.held_level());
            *cmd = None;
            handle.finish_signal().notify_all();
        }
    }

    /// Check if any arena requires collection due to allocation pressure.
    pub fn should_collect(&self) -> bool {
        let arenas = self.arenas.lock();
        for handle in arenas.values() {
            if handle.needs_collection().load(Ordering::Acquire) {
                return true;
            }
        }
        false
    }

    /// Enter a collection session by acquiring the coordinator lock and
    /// setting `stw_in_progress`.
    pub fn begin_collection(&self) -> Option<CollectionSession<'_>> {
        let lock = self.collection_lock.try_lock()?;
        self.enter_collecting_state(lock.held_level(), "begin_collection");
        Some(CollectionSession::new(self, lock))
    }

    fn enter_collecting_state(&self, held: HeldLockLevel<levels::CollectionLock>, context: &str) {
        let stw_in_progress = self.stw_in_progress.load(Ordering::Acquire);
        assert!(
            !stw_in_progress,
            "{context}: expected idle coordinator before beginning collection, got stw_in_progress={stw_in_progress}",
        );
        self.assert_no_pending_commands(held, &format!("{context} precondition"));

        self.stw_in_progress.store(true, Ordering::Release);
    }

    fn finish_collection_inner(&self, held: HeldLockLevel<levels::CollectionLock>, context: &str) {
        let stw_in_progress = self.stw_in_progress.load(Ordering::Acquire);
        assert!(
            stw_in_progress,
            "{context}: expected active collection before finishing, got stw_in_progress={stw_in_progress}",
        );
        self.assert_no_pending_commands(held, &format!("{context} precondition"));

        self.stw_in_progress.store(false, Ordering::Release);

        // Reset all collection flags
        {
            let arenas = self.arenas.lock_after(held);
            for handle in arenas.values() {
                handle.needs_collection().store(false, Ordering::Release);
                // We don't reset allocation_counter here as it might be useful for stats,
                // or we might want to reset it. For now, let's just clear the flag.
            }
        }

        self.assert_no_pending_commands(held, &format!("{context} postcondition"));
        let stw_in_progress = self.stw_in_progress.load(Ordering::Acquire);
        assert!(
            !stw_in_progress,
            "{context}: expected Idle after finishing, got stw_in_progress={stw_in_progress}",
        );
    }

    /// Get the total allocated size across all arenas.
    pub fn total_allocated(&self) -> usize {
        let arenas = self.arenas.lock();
        arenas
            .values()
            .map(|h| h.allocation_counter().load(Ordering::Acquire))
            .sum()
    }

    /// Get the total bytes managed by GC-arena across all threads.
    pub fn total_gc_allocation(&self) -> usize {
        let arenas = self.arenas.lock();
        arenas
            .values()
            .map(|h| h.gc_allocated_bytes().load(Ordering::Acquire))
            .sum()
    }

    /// Get the total external bytes tracked by GC-arena across all threads.
    pub fn total_external_allocation(&self) -> usize {
        let arenas = self.arenas.lock();
        arenas
            .values()
            .map(|h| h.external_allocated_bytes().load(Ordering::Acquire))
            .sum()
    }

    fn get_arena(&self, thread_id: dotnet_utils::ArenaId) -> Option<ArenaHandle> {
        self.arenas.lock().get(&thread_id).cloned()
    }

    fn get_arena_after_collection_lock(
        &self,
        held: HeldLockLevel<levels::CollectionLock>,
        thread_id: dotnet_utils::ArenaId,
    ) -> Option<ArenaHandle> {
        self.arenas.lock_after(held).get(&thread_id).cloned()
    }

    /// Check if a thread has a pending GC command.
    pub(crate) fn has_command(&self, thread_id: dotnet_utils::ArenaId) -> bool {
        if let Some(handle) = self.get_arena(thread_id) {
            handle.current_command().lock().is_some()
        } else {
            false
        }
    }

    /// Mark a command as finished for a thread.
    pub(crate) fn command_finished(&self, thread_id: dotnet_utils::ArenaId) {
        if let Some(handle) = self.get_arena(thread_id) {
            let mut cmd = handle.current_command().lock();
            assert!(
                cmd.is_some(),
                "command_finished: arena {:?} has no pending command to finish",
                thread_id
            );
            *cmd = None;
            handle.finish_signal().notify_all();
        }
    }

    /// Wait for a command or for the GC to finish.
    /// Returns Some(command) if a command was received, or None if the GC finished.
    pub(crate) fn wait_for_command_or_resume(
        &self,
        thread_id: dotnet_utils::ArenaId,
        stop_requested: &AtomicBool,
    ) -> Option<GCCommand> {
        if let Some(handle) = self.get_arena(thread_id) {
            let mut cmd_guard = handle.current_command().lock();
            while stop_requested.load(Ordering::Acquire) {
                if let Some(cmd) = cmd_guard.clone() {
                    return Some(cmd);
                }
                handle.command_signal().wait(cmd_guard.raw_mut());
            }
        }
        None
    }

    /// Notify all arenas that they should check their resume condition.
    pub(crate) fn notify_resume(&self) {
        let arenas = self.arenas.lock();
        for handle in arenas.values() {
            handle.command_signal().notify_all();
        }
    }

    /// Record a cross-arena reference found during marking.
    pub(crate) fn record_cross_arena_ref(
        &self,
        target_thread_id: dotnet_utils::ArenaId,
        ptr: ObjectPtr,
    ) {
        let stw_in_progress = self.stw_in_progress.load(Ordering::Acquire);
        assert!(
            stw_in_progress,
            "record_cross_arena_ref: requires active collection, got stw_in_progress={stw_in_progress}",
        );
        let mut refs = self.cross_arena_refs.lock();
        refs.entry(target_thread_id).or_default().insert(ptr);
    }
}

#[cfg(feature = "multithreading")]
impl Default for GCCoordinator {
    fn default() -> Self {
        Self::new(Arc::new(AtomicBool::new(false)))
    }
}

/// RAII collection session.
///
/// Constructed by [`GCCoordinator::begin_collection`]. While this value is
/// alive, the coordinator lock is held and `stw_in_progress` is set. Dropping
/// this value restores idle coordinator state.
#[cfg(feature = "multithreading")]
pub struct CollectionSession<'coord> {
    coordinator: &'coord GCCoordinator,
    /// Holds the coordinator's `collection_lock` for the duration of the session.
    lock: Option<OrderedMutexGuard<'coord, levels::CollectionLock, ()>>,
    arena_snapshot_scratch: RefCell<Vec<ArenaHandle>>,
    _debug_lock_guard: CollectionLockDebugGuard,
}

#[cfg(feature = "multithreading")]
impl<'coord> CollectionSession<'coord> {
    fn new(
        coordinator: &'coord GCCoordinator,
        lock: OrderedMutexGuard<'coord, levels::CollectionLock, ()>,
    ) -> Self {
        Self {
            coordinator,
            lock: Some(lock),
            arena_snapshot_scratch: RefCell::new(Vec::new()),
            _debug_lock_guard: CollectionLockDebugGuard::new(),
        }
    }

    fn finish_inner(&mut self, context: &str) {
        let Some(lock) = self.lock.take() else {
            return;
        };
        self.coordinator
            .finish_collection_inner(lock.held_level(), context);
        drop(lock);
    }
}

#[cfg(feature = "multithreading")]
impl CollectionSession<'_> {
    fn collection_lock_level(&self) -> HeldLockLevel<levels::CollectionLock> {
        self.lock
            .as_ref()
            .expect("CollectionSession requires an active collection lock")
            .held_level()
    }

    fn snapshot_other_arenas(
        &self,
        initiating_thread_id: dotnet_utils::ArenaId,
    ) -> std::cell::RefMut<'_, Vec<ArenaHandle>> {
        let mut scratch = self.arena_snapshot_scratch.borrow_mut();
        scratch.clear();

        let arenas = self
            .coordinator
            .arenas
            .lock_after(self.collection_lock_level());
        let additional = arenas
            .len()
            .saturating_sub(1)
            .saturating_sub(scratch.capacity());
        if additional > 0 {
            scratch.reserve(additional);
        }
        scratch.extend(
            arenas
                .values()
                .filter(|handle| handle.thread_id() != initiating_thread_id)
                .cloned(),
        );
        drop(arenas);
        scratch
    }

    fn wait_on_other_arenas(&self, initiating_thread_id: dotnet_utils::ArenaId) {
        let handles = self.snapshot_other_arenas(initiating_thread_id);
        for handle in handles.iter() {
            let mut cmd = handle.current_command().lock();
            while cmd.is_some() {
                handle.finish_signal().wait(cmd.raw_mut());
            }
        }
    }

    fn enqueue_command_for_arena(&self, handle: &ArenaHandle, command: GCCommand, context: &str) {
        let mut cmd = handle.current_command().lock();
        assert!(
            cmd.is_none(),
            "{context}: arena {:?} already has pending command {:?} before dispatching {:?}",
            handle.thread_id(),
            *cmd,
            command
        );
        *cmd = Some(command);
        handle.command_signal().notify_all();
    }

    fn send_command_to_other_arenas_collecting(
        &self,
        initiating_thread_id: dotnet_utils::ArenaId,
        command: GCCommand,
    ) {
        let handles = self.snapshot_other_arenas(initiating_thread_id);
        for handle in handles.iter() {
            self.enqueue_command_for_arena(handle, command.clone(), "send_command_to_other_arenas");
        }
    }

    fn send_command_to_all_and_wait(
        &self,
        initiating_thread_id: dotnet_utils::ArenaId,
        command: GCCommand,
    ) {
        self.send_command_to_other_arenas_collecting(initiating_thread_id, command.clone());
        execute_gc_command_for_current_thread(command, self.coordinator);
        self.wait_on_other_arenas(initiating_thread_id);
    }

    /// Perform a coordinated collection across all registered arenas.
    pub fn collect_all_arenas(&self, initiating_thread_id: dotnet_utils::ArenaId) {
        assert!(
            self.coordinator
                .get_arena_after_collection_lock(self.collection_lock_level(), initiating_thread_id)
                .is_some(),
            "collect_all_arenas precondition: initiating thread {:?} must be registered",
            initiating_thread_id
        );

        // Initial marking: each arena marks its local roots.
        {
            // Clear any stale cross-arena references from previous collections
            let mut refs = self
                .coordinator
                .cross_arena_refs
                .lock_after(self.collection_lock_level());
            refs.clear();
        }

        // Send MarkAll command to all arenas
        self.send_command_to_all_and_wait(
            initiating_thread_id,
            GCCommand::Mark(MarkPhaseCommand::All),
        );

        // Fixed-point iteration for cross-arena resurrection.
        // Keep iterating until no new cross-arena references are found
        #[cfg(feature = "bench-instrumentation")]
        let mut fixed_point_iterations = 0u64;
        loop {
            let cross_refs = {
                let mut refs = self
                    .coordinator
                    .cross_arena_refs
                    .lock_after(self.collection_lock_level());
                if refs.is_empty() {
                    // No cross-arena references found, we're done
                    break;
                }
                // Move the current references out without cloning so capacity can be reused.
                std::mem::take(&mut *refs)
            };
            #[cfg(feature = "bench-instrumentation")]
            {
                fixed_point_iterations += 1;
                let cross_arena_object_volume = cross_refs
                    .values()
                    .map(std::collections::HashSet::len)
                    .sum::<usize>() as u64;
                dotnet_metrics::record_active_gc_fixed_point_iteration(
                    fixed_point_iterations,
                    cross_arena_object_volume,
                );
            }
            // For each target arena, send MarkObjects command with the objects to resurrect
            let mut initiator_mark_objs = None;
            for (target_thread_id, ptrs) in cross_refs {
                let mut ptrs_for_mark = MarkObjectPointers::with_capacity(ptrs.len());
                for ptr in ptrs {
                    ptrs_for_mark.insert(OpaqueObjectPtr::from_raw(ptr.as_ptr() as *const ()));
                }
                if target_thread_id == initiating_thread_id {
                    // Save for direct execution by initiating thread
                    initiator_mark_objs = Some(ptrs_for_mark);
                } else if let Some(handle) = self
                    .coordinator
                    .get_arena_after_collection_lock(self.collection_lock_level(), target_thread_id)
                {
                    self.enqueue_command_for_arena(
                        &handle,
                        GCCommand::Mark(MarkPhaseCommand::Objects(ptrs_for_mark)),
                        "collect_all_arenas fixed-point dispatch",
                    );
                }
            }

            // Execute MarkObjects for the initiating thread directly
            if let Some(ptrs) = initiator_mark_objs {
                assert!(
                    !self.coordinator.has_command(initiating_thread_id),
                    "collect_all_arenas fixed-point dispatch: initiating arena {:?} unexpectedly has a pending command",
                    initiating_thread_id
                );
                execute_gc_command_for_current_thread(
                    GCCommand::Mark(MarkPhaseCommand::Objects(ptrs)),
                    self.coordinator,
                );
            }

            // Wait for all MarkObjects commands to complete (excluding initiating thread)
            self.wait_on_other_arenas(initiating_thread_id);

            // Check if any new cross-arena references were discovered
            let has_new_refs = {
                let refs = self
                    .coordinator
                    .cross_arena_refs
                    .lock_after(self.collection_lock_level());
                !refs.is_empty()
            };

            if !has_new_refs {
                // Fixed point reached - no new cross-arena references found
                break;
            }
        }
        #[cfg(feature = "bench-instrumentation")]
        dotnet_metrics::record_active_gc_fixed_point_cycle(fixed_point_iterations);

        // Run finalizers.
        self.send_command_to_all_and_wait(
            initiating_thread_id,
            GCCommand::Sweep(SweepPhaseCommand::Finalize),
        );

        // Sweep unmarked objects.
        self.send_command_to_all_and_wait(
            initiating_thread_id,
            GCCommand::Sweep(SweepPhaseCommand::Sweep),
        );
    }

    #[cfg(test)]
    pub(crate) fn send_command_to_other_arenas(
        &self,
        initiating_thread_id: dotnet_utils::ArenaId,
        command: GCCommand,
    ) {
        self.send_command_to_other_arenas_collecting(initiating_thread_id, command);
    }

    /// Finish the collection session early. `Drop` is a no-op afterwards.
    pub fn finish(mut self) {
        self.finish_inner("CollectionSession::finish");
    }
}

#[cfg(feature = "multithreading")]
impl Drop for CollectionSession<'_> {
    fn drop(&mut self) {
        self.finish_inner("CollectionSession::drop");
    }
}

/// Unified guard for a full GC cycle.
///
/// Owns the coordinator collection session and the STW guard in one RAII type
/// with a fixed drop order:
/// 1. [`CollectionSession`] cleanup (`stw_in_progress=false`, lock release)
/// 2. STW guard drop (thread resume)
///
/// This ordering is enforced in [`Drop`] even during panic unwind.
#[cfg(feature = "multithreading")]
pub struct GcCycleGuard<'coord, StwGuard> {
    session: Option<CollectionSession<'coord>>,
    stw_guard: Option<StwGuard>,
}

#[cfg(feature = "multithreading")]
impl<'coord, StwGuard> GcCycleGuard<'coord, StwGuard> {
    pub fn new(session: CollectionSession<'coord>, stw_guard: StwGuard) -> Self {
        Self {
            session: Some(session),
            stw_guard: Some(stw_guard),
        }
    }

    pub fn collect_all_arenas(&self, initiating_thread_id: dotnet_utils::ArenaId) {
        self.session
            .as_ref()
            .expect("GcCycleGuard::collect_all_arenas requires an active collection session")
            .collect_all_arenas(initiating_thread_id);
    }

    pub fn finish_collection(&mut self) {
        if let Some(session) = self.session.take() {
            session.finish();
        }
    }
}

#[cfg(feature = "multithreading")]
impl<StwGuard> Drop for GcCycleGuard<'_, StwGuard> {
    fn drop(&mut self) {
        // Coordinator cleanup must happen before threads resume.
        self.finish_collection();
        drop(self.stw_guard.take());
    }
}

/// RAII guard that ensures [`GCCoordinator::command_finished`] is called when the
/// guard is dropped, unless the guard has been explicitly disarmed.
///
/// Used in the safe-point loop to guarantee the completion signal is always sent
/// to the coordinator even when the command handler unwinds due to a panic, so
/// the GC initiator is never left waiting forever for a finish notification.
#[cfg(feature = "multithreading")]
pub(crate) struct Armed;

#[cfg(feature = "multithreading")]
pub(crate) struct Disarmed;

#[cfg(feature = "multithreading")]
pub(crate) trait CompletionGuardState {
    const COMPLETE_ON_DROP: bool;
}

#[cfg(feature = "multithreading")]
impl CompletionGuardState for Armed {
    const COMPLETE_ON_DROP: bool = true;
}

#[cfg(feature = "multithreading")]
impl CompletionGuardState for Disarmed {
    const COMPLETE_ON_DROP: bool = false;
}

#[cfg(feature = "multithreading")]
pub(crate) struct CommandCompletionGuard<'coord, State: CompletionGuardState> {
    coordinator: &'coord GCCoordinator,
    thread_id: dotnet_utils::ArenaId,
    _state: PhantomData<State>,
}

#[cfg(feature = "multithreading")]
impl<'coord> CommandCompletionGuard<'coord, Armed> {
    /// Create a new armed guard for `thread_id` on `coordinator`.
    ///
    /// Call this immediately before executing a GC command so that any
    /// panic inside the command handler still delivers the finish signal.
    pub(crate) fn new(
        thread_id: dotnet_utils::ArenaId,
        coordinator: &'coord GCCoordinator,
    ) -> Self {
        Self {
            coordinator,
            thread_id,
            _state: PhantomData,
        }
    }

    /// Disarm the guard, transitioning from `Armed` to `Disarmed`.
    ///
    /// Call this on the normal completion path after `command_finished` has
    /// already been called manually.
    pub(crate) fn disarm(self) -> CommandCompletionGuard<'coord, Disarmed> {
        let this = ManuallyDrop::new(self);
        CommandCompletionGuard {
            coordinator: this.coordinator,
            thread_id: this.thread_id,
            _state: PhantomData,
        }
    }
}

#[cfg(feature = "multithreading")]
impl<State: CompletionGuardState> Drop for CommandCompletionGuard<'_, State> {
    fn drop(&mut self) {
        if State::COMPLETE_ON_DROP {
            // The scope exited while still `Armed` — either the command handler
            // panicked or a future refactor forgot to disarm. Call
            // `command_finished` so the GC initiator is never left blocked in
            // `wait_on_other_arenas`.
            self.coordinator.command_finished(self.thread_id);
        }
    }
}

#[cfg(not(feature = "multithreading"))]
mod stubs {
    #[derive(Debug, Clone, Copy)]
    pub struct GCCommand;

    pub struct GCCoordinator;

    impl GCCoordinator {
        pub fn new(
            _stw_in_progress: dotnet_utils::sync::Arc<dotnet_utils::sync::AtomicBool>,
        ) -> Self {
            Self
        }
    }

    pub(crate) fn clear_tracing_state() {}

    impl Default for GCCoordinator {
        fn default() -> Self {
            Self::new(dotnet_utils::sync::Arc::new(
                dotnet_utils::sync::AtomicBool::new(false),
            ))
        }
    }
}

#[cfg(not(feature = "multithreading"))]
pub(crate) use stubs::clear_tracing_state;
#[cfg(not(feature = "multithreading"))]
pub use stubs::{GCCommand, GCCoordinator};

#[cfg(all(test, feature = "multithreading"))]
mod tests {
    use super::*;
    use crate::sync::{Arc, AtomicBool, Ordering};
    use crate::threading::{ThreadManager, ThreadManagerOps};

    #[test]
    fn test_coordinator_registration() {
        let stw_flag = Arc::new(AtomicBool::new(false));
        let coordinator = GCCoordinator::new(stw_flag);
        let handle = ArenaHandle::new(dotnet_utils::ArenaId(1));

        handle.allocation_counter().store(100, Ordering::Release);

        coordinator.register_arena(handle.clone());
        assert_eq!(coordinator.total_allocated(), 100);

        handle.needs_collection().store(true, Ordering::Release);
        assert!(coordinator.should_collect());

        let session = coordinator.begin_collection().unwrap();
        session.finish();

        assert!(!coordinator.should_collect());
        assert!(!handle.needs_collection().load(Ordering::Acquire));

        coordinator.unregister_arena(dotnet_utils::ArenaId(1));
        assert_eq!(coordinator.total_allocated(), 0);
    }

    #[test]
    fn test_allocation_pressure_trigger() {
        let stw_flag = Arc::new(AtomicBool::new(false));
        let coordinator = GCCoordinator::new(stw_flag);
        let handle = ArenaHandle::new(dotnet_utils::ArenaId(1));

        coordinator.register_arena(handle.clone());

        // Allocate just below threshold using the handle directly
        handle.record_allocation(ALLOCATION_THRESHOLD - 100);
        assert!(!handle.needs_collection().load(Ordering::Acquire));

        // Allocate to cross threshold
        handle.record_allocation(200);
        assert!(handle.needs_collection().load(Ordering::Acquire));

        // Reset via coordinator
        let session = coordinator.begin_collection().unwrap();
        session.finish();
        assert!(!handle.needs_collection().load(Ordering::Acquire));
    }

    #[test]
    fn test_collect_all_arenas_no_deadlock() {
        use std::thread;
        use std::time::Duration;

        let stw_flag = Arc::new(AtomicBool::new(false));
        let coordinator = Arc::new(GCCoordinator::new(stw_flag));

        let handle1 = ArenaHandle::new(dotnet_utils::ArenaId(1));
        let handle2 = ArenaHandle::new(dotnet_utils::ArenaId(2));

        coordinator.register_arena(handle1);
        coordinator.register_arena(handle2.clone());

        let done = Arc::new(AtomicBool::new(false));
        let done_clone = done.clone();
        let coordinator_clone = coordinator.clone();

        let t = thread::spawn(move || {
            // Wait until we actually have a command
            while !done_clone.load(Ordering::Relaxed) {
                let has_cmd = {
                    let cmd = handle2.current_command().lock();
                    cmd.is_some()
                };

                if has_cmd {
                    // Command received! Now call command_finished.
                    coordinator_clone.command_finished(dotnet_utils::ArenaId(2));
                } else {
                    thread::sleep(Duration::from_millis(10));
                }
            }
        });

        // Initiator (Thread 1) calls collect_all_arenas.
        // This used to deadlock because wait_on_other_arenas held the arenas lock
        // while waiting for command_finished, which also needed the arenas lock.
        let session = coordinator.begin_collection().unwrap();
        session.collect_all_arenas(dotnet_utils::ArenaId(1));
        session.finish();

        done.store(true, Ordering::Relaxed);
        t.join().unwrap();
    }

    #[test]
    fn test_collect_all_arenas_unregister_race() {
        use std::thread;
        use std::time::Duration;

        let stw_flag = Arc::new(AtomicBool::new(false));
        let coordinator = Arc::new(GCCoordinator::new(stw_flag));

        let handle1 = ArenaHandle::new(dotnet_utils::ArenaId(1));
        let handle2 = ArenaHandle::new(dotnet_utils::ArenaId(2));

        coordinator.register_arena(handle1);
        coordinator.register_arena(handle2.clone());

        let coordinator_clone = coordinator.clone();

        let t = thread::spawn(move || {
            // Wait until a command is sent to arena 2
            loop {
                let cmd = handle2.current_command().lock();
                if cmd.is_some() {
                    break;
                }
                drop(cmd);
                thread::sleep(Duration::from_millis(10));
            }

            // Instead of finishing the command, unregister the arena!
            coordinator_clone.unregister_arena(dotnet_utils::ArenaId(2));
        });

        // Initiator (Thread 1) calls collect_all_arenas.
        // It will send a command to Thread 2 and wait for it.
        // If unregister_arena doesn't notify, this will hang.
        let session = coordinator.begin_collection().unwrap();
        session.collect_all_arenas(dotnet_utils::ArenaId(1));
        session.finish();

        t.join().unwrap();
    }

    #[test]
    #[should_panic(expected = "collect_all_arenas precondition")]
    fn test_collect_all_arenas_requires_registered_initiator() {
        let stw_flag = Arc::new(AtomicBool::new(false));
        let coordinator = GCCoordinator::new(stw_flag);
        let handle1 = ArenaHandle::new(dotnet_utils::ArenaId(1));
        coordinator.register_arena(handle1);

        let session = coordinator
            .begin_collection()
            .expect("collection should start");
        session.collect_all_arenas(dotnet_utils::ArenaId(2));
    }

    #[test]
    #[should_panic(expected = "command_finished")]
    fn test_command_finished_requires_pending_command() {
        let stw_flag = Arc::new(AtomicBool::new(false));
        let coordinator = GCCoordinator::new(stw_flag);
        let handle1 = ArenaHandle::new(dotnet_utils::ArenaId(1));
        coordinator.register_arena(handle1);
        let _session = coordinator.begin_collection().unwrap();

        coordinator.command_finished(dotnet_utils::ArenaId(1));
    }

    // ------------------------------------------------------------------
    // Stress: concurrent register/unregister racing with collection cycles
    //
    // Invariant: start_collection/finish_collection must never deadlock or
    // panic regardless of how many arenas are concurrently registered or
    // unregistered around the collection window.
    // ------------------------------------------------------------------
    #[test]
    fn stress_concurrent_register_unregister_with_collection_cycles() {
        use std::sync::Barrier;
        use std::thread;

        const WORKER_THREADS: usize = 4;
        const ITERATIONS: usize = 60;
        const GC_CYCLES: usize = 20;
        // Use arena IDs in a range that won't collide with other tests.
        const BASE_ID: u64 = 900_000;

        let stw_flag = Arc::new(AtomicBool::new(false));
        let coordinator = Arc::new(GCCoordinator::new(stw_flag));
        let barrier = Arc::new(Barrier::new(WORKER_THREADS + 1));

        let mut handles = vec![];

        // Worker threads: each cyclically registers and unregisters an arena,
        // recording allocations to ensure `should_collect` can fire.
        for t in 0..WORKER_THREADS {
            let coordinator = coordinator.clone();
            let barrier = barrier.clone();
            handles.push(thread::spawn(move || {
                barrier.wait();
                for i in 0..ITERATIONS {
                    let id = dotnet_utils::ArenaId(BASE_ID + (t * ITERATIONS + i) as u64);
                    let handle = ArenaHandle::new(id);
                    coordinator.register_arena(handle.clone());
                    // Simulate some allocation pressure so the coordinator
                    // exercises the should_collect path in some iterations.
                    if i % 5 == 0 {
                        handle.record_allocation(ALLOCATION_THRESHOLD / 2);
                    }
                    coordinator.unregister_arena(id);
                }
            }));
        }

        // GC driver thread (this thread): loops calling start/finish_collection
        // while worker threads hammer register/unregister concurrently.
        barrier.wait();
        for _ in 0..GC_CYCLES {
            // start_collection returns None when another collection is already
            // in progress; we simply skip and yield in that case.
            if let Some(session) = coordinator.begin_collection() {
                session.finish();
            }
            thread::yield_now();
        }

        for h in handles {
            h.join().unwrap();
        }
    }

    // ------------------------------------------------------------------
    // Stress: register/unregister races with safepoint command dispatch
    //
    // Fires collect_all_arenas while arenas register and unregister
    // concurrently, exercising the "unregister beats command" path where
    // unregister_arena notifies all_threads_stopped.
    // ------------------------------------------------------------------
    #[test]
    fn stress_collect_all_arenas_with_concurrent_unregisters() {
        use std::sync::Barrier;
        use std::thread;
        use std::time::Duration;

        const WORKER_THREADS: usize = 3;
        const ROUNDS: usize = 10;
        const BASE_ID: u64 = 950_000;

        let stw_flag = Arc::new(AtomicBool::new(false));
        let coordinator = Arc::new(GCCoordinator::new(stw_flag));
        let barrier = Arc::new(Barrier::new(WORKER_THREADS + 1));

        let mut handles = vec![];

        for t in 0..WORKER_THREADS {
            let coordinator = coordinator.clone();
            let barrier = barrier.clone();
            handles.push(thread::spawn(move || {
                barrier.wait();
                for round in 0..ROUNDS {
                    let id = dotnet_utils::ArenaId(BASE_ID + (t * ROUNDS + round) as u64);
                    let handle = ArenaHandle::new(id);

                    coordinator.register_arena(handle.clone());

                    // Brief pause so the initiator can race the command send.
                    thread::sleep(Duration::from_micros(50));

                    // Unregistering mid-collection: coordinator must not hang
                    // waiting for this arena's command_finished, because
                    // unregister_arena removes it from the pending set.
                    coordinator.unregister_arena(id);
                }
            }));
        }

        barrier.wait();
        // GC initiator: register a stable arena representing "this thread" so
        // collect_all_arenas can be invoked (it requires an initiating_thread_id).
        let initiator_id = dotnet_utils::ArenaId(BASE_ID - 1);
        coordinator.register_arena(ArenaHandle::new(initiator_id));

        for _ in 0..ROUNDS {
            if let Some(session) = coordinator.begin_collection() {
                session.collect_all_arenas(initiator_id);
                session.finish();
            }
            thread::yield_now();
        }

        coordinator.unregister_arena(initiator_id);

        for h in handles {
            h.join().unwrap();
        }
    }

    // ------------------------------------------------------------------
    // Lock-order stress harness: repeatedly exercise the canonical top-level
    // acquisition order `collection_lock -> gc_coordination` while worker
    // threads are live and polling safe points.
    // ------------------------------------------------------------------
    #[test]
    fn stress_lock_order_gc_cycle_guard_with_live_safepoints() {
        use std::sync::Barrier;
        use std::thread;

        const WORKER_THREADS: usize = 2;
        const CYCLES: usize = 10;

        let stw_flag = Arc::new(AtomicBool::new(false));
        let coordinator = Arc::new(GCCoordinator::new(Arc::clone(&stw_flag)));
        let manager = ThreadManager::new(stw_flag);
        manager.set_coordinator(Arc::downgrade(&coordinator));

        let initiator_id = manager.register_thread();
        coordinator.register_arena(ArenaHandle::new(initiator_id));

        let done = Arc::new(AtomicBool::new(false));
        let barrier = Arc::new(Barrier::new(WORKER_THREADS + 1));
        let mut workers = vec![];

        for _ in 0..WORKER_THREADS {
            let manager = Arc::clone(&manager);
            let coordinator = Arc::clone(&coordinator);
            let done = Arc::clone(&done);
            let barrier = Arc::clone(&barrier);
            workers.push(thread::spawn(move || {
                let managed_id = manager.register_thread();
                coordinator.register_arena(ArenaHandle::new(managed_id));
                barrier.wait();

                while !done.load(Ordering::Acquire) {
                    manager.safe_point(managed_id, &coordinator);
                    thread::yield_now();
                }

                manager.safe_point(managed_id, &coordinator);
                coordinator.unregister_arena(managed_id);
                manager.unregister_thread(managed_id);
            }));
        }

        barrier.wait();

        for _ in 0..CYCLES {
            let session = coordinator
                .begin_collection()
                .expect("collection session must start");
            let stw_guard = manager.request_stop_the_world();
            let gc_cycle = GcCycleGuard::new(session, stw_guard);
            gc_cycle.collect_all_arenas(initiator_id);
            drop(gc_cycle);

            assert!(
                !manager.is_gc_stop_requested(),
                "gc_stop_requested must be cleared after each GC cycle"
            );
            assert!(
                !coordinator.stw_in_progress.load(Ordering::Acquire),
                "coordinator must be idle after each GC cycle"
            );
        }

        done.store(true, Ordering::Release);

        for worker in workers {
            worker.join().unwrap();
        }

        coordinator.unregister_arena(initiator_id);
        manager.unregister_thread(initiator_id);
        assert!(
            !manager.is_gc_stop_requested(),
            "gc_stop_requested must be clear after lock-order stress harness"
        );
    }

    // ------------------------------------------------------------------
    // Panic-injection: CollectionSession must restore Idle state
    // when the caller unwinds mid-collection.
    //
    // Without the guard the coordinator would be stuck in an active collection
    // forever after a mid-collection panic, making all subsequent GC
    // attempts silently skip (`begin_collection` returns `None`).
    // ------------------------------------------------------------------
    #[test]
    fn test_collection_session_cleanup_on_panic() {
        use std::panic;

        let stw_flag = Arc::new(AtomicBool::new(false));
        let coordinator = Arc::new(GCCoordinator::new(stw_flag));

        // Baseline: a clean start/finish cycle works.
        {
            let session = coordinator
                .begin_collection()
                .expect("initial session must start");
            session.finish();
        }

        // Inject a panic mid-collection. Session drop must drive the
        // coordinator back to Idle even though the caller never reaches
        // the explicit `finish()` call.
        let coordinator_clone = Arc::clone(&coordinator);
        let result = panic::catch_unwind(panic::AssertUnwindSafe(|| {
            let _session = coordinator_clone
                .begin_collection()
                .expect("session must start inside catch_unwind");
            // Simulate work that panics before finish() is called.
            panic!("simulated mid-collection panic");
        }));

        assert!(result.is_err(), "the closure should have panicked");

        // After unwind: session Drop called finish_collection_inner, which must
        // have cleared stw_in_progress. Check this BEFORE starting a new session
        // (which would re-set stw_in_progress).
        assert!(
            !coordinator.stw_in_progress.load(Ordering::Acquire),
            "stw_in_progress must be false after guard-driven finish_collection"
        );

        // The coordinator must be back in Idle — a fresh session must succeed.
        let session = coordinator
            .begin_collection()
            .expect("coordinator must be Idle after CollectionSession cleanup on unwind");
        session.finish();
    }

    // ------------------------------------------------------------------
    // Panic-injection: CommandCompletionGuard must deliver command_finished
    // when the command handler unwinds.
    //
    // Without the guard, a panic inside `execute_gc_command` would leave the
    // arena's command slot occupied and the GC initiator blocked forever in
    // `wait_on_other_arenas`.
    // ------------------------------------------------------------------
    #[test]
    fn test_command_completion_guard_sends_signal_on_panic() {
        use std::panic;

        // Use arena IDs unlikely to collide with other tests.
        let initiator_id = dotnet_utils::ArenaId(800_001);
        let worker_id = dotnet_utils::ArenaId(800_002);

        let stw_flag = Arc::new(AtomicBool::new(false));
        let coordinator = Arc::new(GCCoordinator::new(stw_flag));

        coordinator.register_arena(ArenaHandle::new(initiator_id));
        coordinator.register_arena(ArenaHandle::new(worker_id));

        // Start a collection session so command dispatch is valid.
        let session = coordinator
            .begin_collection()
            .expect("collection must start");

        // Dispatch a command to the worker arena (but not the initiator).
        session.send_command_to_other_arenas(initiator_id, GCCommand::Mark(MarkPhaseCommand::All));

        assert!(
            coordinator.has_command(worker_id),
            "worker must have a pending command after send_command_to_other_arenas"
        );

        // Simulate a panic inside execute_gc_command while the guard is live.
        // The guard's Drop must call command_finished even though the command
        // handler never returned normally.
        let coordinator_clone = Arc::clone(&coordinator);
        let result = panic::catch_unwind(panic::AssertUnwindSafe(|| {
            let _guard = CommandCompletionGuard::new(worker_id, &coordinator_clone);
            // Simulate the command handler panicking mid-execution.
            panic!("simulated panic during GC command execution");
        }));

        assert!(result.is_err(), "the closure should have panicked");

        // The guard's Drop must have called command_finished, clearing the
        // command slot so the GC initiator would not be left hanging.
        assert!(
            !coordinator.has_command(worker_id),
            "command slot must be cleared after CommandCompletionGuard drop on unwind"
        );

        // Verify the disarmed path: a normally-completing guard must NOT call
        // command_finished a second time (which would panic due to no pending cmd).
        session.send_command_to_other_arenas(initiator_id, GCCommand::Mark(MarkPhaseCommand::All));
        assert!(coordinator.has_command(worker_id));
        {
            let guard = CommandCompletionGuard::new(worker_id, &coordinator);
            // Normal completion: disarm before manual call.
            let _disarmed_guard = guard.disarm();
            coordinator.command_finished(worker_id);
            // guard drops here — must be a no-op since it was disarmed.
        }
        assert!(
            !coordinator.has_command(worker_id),
            "command slot must be cleared by the manual command_finished call"
        );

        session.finish();
        coordinator.unregister_arena(initiator_id);
        coordinator.unregister_arena(worker_id);
    }

    // ------------------------------------------------------------------
    // Panic-injection: when `GcCycleGuard` owns both collection session and
    // STW guard, coordinator cleanup must complete before thread resume.
    // ------------------------------------------------------------------
    #[test]
    fn test_session_cleanup_precedes_stw_resume_on_unwind() {
        use std::panic;

        struct ProbeStwGuard<'a, InnerGuard> {
            inner: Option<InnerGuard>,
            coordinator: &'a GCCoordinator,
            saw_collecting_before_resume: &'a AtomicBool,
        }

        impl<InnerGuard> Drop for ProbeStwGuard<'_, InnerGuard> {
            fn drop(&mut self) {
                // This observes coordinator state immediately before the wrapped
                // STW guard resumes threads.
                let collecting = self.coordinator.stw_in_progress.load(Ordering::Acquire);
                self.saw_collecting_before_resume
                    .store(collecting, Ordering::Release);
                drop(self.inner.take());
            }
        }

        let stw_flag = Arc::new(AtomicBool::new(false));
        let coordinator = Arc::new(GCCoordinator::new(Arc::clone(&stw_flag)));
        let manager = ThreadManager::new(stw_flag);
        manager.set_coordinator(Arc::downgrade(&coordinator));

        let saw_collecting_before_resume = AtomicBool::new(false);
        let coordinator_for_unwind = Arc::clone(&coordinator);
        let manager_for_unwind = Arc::clone(&manager);

        let result = panic::catch_unwind(panic::AssertUnwindSafe(|| {
            let session = coordinator_for_unwind
                .begin_collection()
                .expect("collection session must start");
            let stw_guard = manager_for_unwind.request_stop_the_world();
            let probe_stw_guard = ProbeStwGuard {
                inner: Some(stw_guard),
                coordinator: coordinator_for_unwind.as_ref(),
                saw_collecting_before_resume: &saw_collecting_before_resume,
            };
            let _gc_cycle = GcCycleGuard::new(session, probe_stw_guard);
            panic!("simulated panic mid-GC");
        }));

        assert!(result.is_err(), "closure should panic");
        assert!(
            !saw_collecting_before_resume.load(Ordering::Acquire),
            "collection session cleanup must complete before STW guard drop resumes threads"
        );
        assert!(
            !manager.is_gc_stop_requested(),
            "STW resume must clear gc_stop_requested on unwind"
        );
        assert!(
            !coordinator.stw_in_progress.load(Ordering::Acquire),
            "coordinator must be Idle after unwind cleanup"
        );
    }
}

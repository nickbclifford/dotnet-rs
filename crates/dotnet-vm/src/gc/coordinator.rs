#[cfg(feature = "multithreaded-gc")]
use crate::threading::execute_gc_command_for_current_thread;
#[cfg(feature = "multithreaded-gc")]
use dotnet_utils::sync::{AtomicBool, Mutex, Ordering};
#[cfg(feature = "multithreaded-gc")]
use dotnet_value::object::ObjectPtr;
#[cfg(feature = "multithreaded-gc")]
use std::collections::{HashMap, HashSet};

#[cfg(feature = "multithreaded-gc")]
pub use dotnet_utils::gc::{
    ALLOCATION_THRESHOLD, ArenaHandle, GCCommand, clear_tracing_state, get_currently_tracing,
    set_currently_tracing, take_found_cross_arena_refs,
};

#[cfg(feature = "multithreaded-gc")]
/// Coordinates stop-the-world collections across multiple thread-local arenas.
pub struct GCCoordinator {
    /// thread_id -> arena metadata
    arenas: Mutex<HashMap<dotnet_utils::ArenaId, ArenaHandle>>,
    /// Global lock held during collection
    collection_lock: Mutex<()>,
    /// Flag indicating if a collection is currently in progress
    is_collecting: AtomicBool,
    /// Cross-arena references found during marking
    cross_arena_refs: Mutex<HashMap<dotnet_utils::ArenaId, HashSet<ObjectPtr>>>,
}

#[cfg(feature = "multithreaded-gc")]
impl GCCoordinator {
    pub fn new() -> Self {
        Self {
            arenas: Mutex::new(HashMap::new()),
            collection_lock: Mutex::new(()),
            is_collecting: AtomicBool::new(false),
            cross_arena_refs: Mutex::new(HashMap::new()),
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
        arenas.remove(&thread_id);
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

    /// Mark that a collection has started.
    pub fn start_collection(&self) -> Option<MutexGuard<'_, ()>> {
        let guard = self.collection_lock.try_lock()?;
        self.is_collecting.store(true, Ordering::Release);
        dotnet_utils::gc::set_stw_in_progress(true);
        Some(guard)
    }

    /// Mark that a collection has finished.
    pub fn finish_collection(&self) {
        self.is_collecting.store(false, Ordering::Release);
        dotnet_utils::gc::set_stw_in_progress(false);

        // Reset all collection flags
        let arenas = self.arenas.lock();
        for handle in arenas.values() {
            handle.needs_collection().store(false, Ordering::Release);
            // We don't reset allocation_counter here as it might be useful for stats,
            // or we might want to reset it. For now, let's just clear the flag.
        }
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

    fn get_all_arenas(&self) -> Vec<ArenaHandle> {
        let arenas = self.arenas.lock();
        arenas.values().cloned().collect()
    }

    fn wait_on_other_arenas(&self, initiating_thread_id: dotnet_utils::ArenaId) {
        for handle in self.get_all_arenas() {
            if handle.thread_id() != initiating_thread_id {
                let mut cmd = handle.current_command().lock();
                while cmd.is_some() {
                    handle.finish_signal().wait(&mut cmd);
                }
            }
        }
    }

    fn send_command_to_other_arenas(&self, initiating_thread_id: dotnet_utils::ArenaId, command: GCCommand) {
        for handle in self.get_all_arenas() {
            if handle.thread_id() != initiating_thread_id {
                let mut cmd = handle.current_command().lock();
                *cmd = Some(command.clone());
                handle.command_signal().notify_all();
            }
        }
    }

    fn send_command_to_all_and_wait(&self, initiating_thread_id: dotnet_utils::ArenaId, command: GCCommand) {
        self.send_command_to_other_arenas(initiating_thread_id, command.clone());
        execute_gc_command_for_current_thread(command, self);
        self.wait_on_other_arenas(initiating_thread_id);
    }

    /// Perform a coordinated collection across all registered arenas.
    pub fn collect_all_arenas(&self, initiating_thread_id: dotnet_utils::ArenaId) {
        // This is called by the thread that triggered the GC, after STW is established.

        // Phase 1: Initial marking - each arena marks its local roots
        {
            // Clear any stale cross-arena references from previous collections
            let mut refs = self.cross_arena_refs.lock();
            refs.clear();
        }

        // Send MarkAll command to all arenas
        self.send_command_to_all_and_wait(initiating_thread_id, GCCommand::MarkAll);

        // Phase 2: Fixed-point iteration for cross-arena resurrection
        // Keep iterating until no new cross-arena references are found
        loop {
            let cross_refs = {
                let refs = self.cross_arena_refs.lock();
                if refs.is_empty() {
                    // No cross-arena references found, we're done
                    break;
                }
                // Take a snapshot of current cross-arena refs
                refs.clone()
            };

            // Clear the global table for the next iteration
            {
                let mut refs = self.cross_arena_refs.lock();
                refs.clear();
            }

            // For each target arena, send MarkObjects command with the objects to resurrect
            let mut initiator_mark_objs = None;
            for (target_thread_id, ptrs) in cross_refs {
                let ptrs_usize: HashSet<usize> = ptrs.iter().map(|p| p.as_ptr() as usize).collect();
                if target_thread_id == initiating_thread_id {
                    // Save for direct execution by initiating thread
                    initiator_mark_objs = Some(ptrs_usize);
                } else if let Some(handle) = self.get_arena(target_thread_id) {
                    let mut cmd = handle.current_command().lock();
                    *cmd = Some(GCCommand::MarkObjects(ptrs_usize));
                    handle.command_signal().notify_all();
                }
            }

            // Execute MarkObjects for the initiating thread directly
            if let Some(ptrs) = initiator_mark_objs {
                execute_gc_command_for_current_thread(GCCommand::MarkObjects(ptrs), self);
            }

            // Wait for all MarkObjects commands to complete (excluding initiating thread)
            self.wait_on_other_arenas(initiating_thread_id);

            // Check if any new cross-arena references were discovered
            let has_new_refs = {
                let refs = self.cross_arena_refs.lock();
                !refs.is_empty()
            };

            if !has_new_refs {
                // Fixed point reached - no new cross-arena references found
                break;
            }
        }

        // Phase 3: Finalize
        self.send_command_to_all_and_wait(initiating_thread_id, GCCommand::Finalize);

        // Phase 4: Sweep
        self.send_command_to_all_and_wait(initiating_thread_id, GCCommand::Sweep);
    }

    /// Check if a thread has a pending GC command.
    pub fn has_command(&self, thread_id: dotnet_utils::ArenaId) -> bool {
        if let Some(handle) = self.get_arena(thread_id) {
            handle.current_command().lock().is_some()
        } else {
            false
        }
    }

    /// Get the pending command for a thread.
    pub fn get_command(&self, thread_id: dotnet_utils::ArenaId) -> Option<GCCommand> {
        if let Some(handle) = self.get_arena(thread_id) {
            handle.current_command().lock().clone()
        } else {
            None
        }
    }

    /// Mark a command as finished for a thread.
    pub fn command_finished(&self, thread_id: dotnet_utils::ArenaId) {
        if let Some(handle) = self.get_arena(thread_id) {
            let mut cmd = handle.current_command().lock();
            *cmd = None;
            handle.finish_signal().notify_all();
        }
    }

    /// Record a cross-arena reference found during marking.
    pub fn record_cross_arena_ref(&self, target_thread_id: dotnet_utils::ArenaId, ptr: ObjectPtr) {
        let mut refs = self.cross_arena_refs.lock();
        refs.entry(target_thread_id).or_default().insert(ptr);
    }
}

#[cfg(feature = "multithreaded-gc")]
impl Default for GCCoordinator {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(feature = "multithreaded-gc")]
pub type MutexGuard<'a, T> = crate::sync::MutexGuard<'a, T>;

#[cfg(not(feature = "multithreaded-gc"))]
pub mod stubs {
    use crate::sync::MutexGuard;
    use dotnet_value::object::ObjectPtr;
    use std::collections::HashSet;

    #[derive(Debug, Clone)]
    pub enum GCCommand {
        MarkAll,
        MarkObjects(HashSet<ObjectPtr>),
        Sweep,
    }

    #[derive(Debug, Clone)]
    pub struct ArenaHandle {
        pub thread_id: dotnet_utils::ArenaId,
    }

    impl ArenaHandle {
        pub fn new(thread_id: dotnet_utils::ArenaId) -> Self {
            Self { thread_id }
        }
        pub fn record_allocation(&self, _size: usize) {}
    }

    pub struct GCCoordinator;

    impl GCCoordinator {
        pub fn new() -> Self {
            Self
        }
        pub fn register_arena(&self, _handle: ArenaHandle) {}
        pub fn unregister_arena(&self, _thread_id: dotnet_utils::ArenaId) {}
        pub fn should_collect(&self) -> bool {
            false
        }
        pub fn finish_collection(&self) {}
        pub fn record_cross_arena_ref(&self, _target_thread_id: dotnet_utils::ArenaId, _ptr: ObjectPtr) {}
        pub fn start_collection(&self) -> Option<MutexGuard<'_, ()>> {
            None
        }
        pub fn total_allocated(&self) -> usize {
            0
        }
        pub fn total_gc_allocation(&self) -> usize {
            0
        }
        pub fn total_external_allocation(&self) -> usize {
            0
        }
    }

    pub fn set_currently_tracing(_thread_id: Option<dotnet_utils::ArenaId>) {}
    pub fn get_currently_tracing() -> Option<dotnet_utils::ArenaId> {
        None
    }
    pub fn take_found_cross_arena_refs() -> Vec<(dotnet_utils::ArenaId, ObjectPtr)> {
        Vec::new()
    }
    pub fn record_cross_arena_ref(_target_thread_id: dotnet_utils::ArenaId, _ptr: ObjectPtr) {}

    impl Default for GCCoordinator {
        fn default() -> Self {
            Self::new()
        }
    }
}

#[cfg(not(feature = "multithreaded-gc"))]
pub use stubs::*;

#[cfg(all(test, feature = "multithreaded-gc"))]
mod tests {
    use super::*;
    use crate::sync::{Arc, AtomicBool, Ordering};

    #[test]
    fn test_coordinator_registration() {
        let coordinator = GCCoordinator::new();
        let handle = ArenaHandle::new(dotnet_utils::ArenaId(1));

        handle.allocation_counter().store(100, Ordering::Release);

        coordinator.register_arena(handle.clone());
        assert_eq!(coordinator.total_allocated(), 100);

        handle.needs_collection().store(true, Ordering::Release);
        assert!(coordinator.should_collect());

        let _guard = coordinator.start_collection().unwrap();
        coordinator.finish_collection();

        assert!(!coordinator.should_collect());
        assert!(!handle.needs_collection().load(Ordering::Acquire));

        coordinator.unregister_arena(dotnet_utils::ArenaId(1));
        assert_eq!(coordinator.total_allocated(), 0);
    }

    #[test]
    fn test_allocation_pressure_trigger() {
        let coordinator = GCCoordinator::new();
        let handle = ArenaHandle::new(dotnet_utils::ArenaId(1));

        coordinator.register_arena(handle.clone());

        // Allocate just below threshold using the handle directly
        handle.record_allocation(ALLOCATION_THRESHOLD - 100);
        assert!(!handle.needs_collection().load(Ordering::Acquire));

        // Allocate to cross threshold
        handle.record_allocation(200);
        assert!(handle.needs_collection().load(Ordering::Acquire));

        // Reset via coordinator
        coordinator.finish_collection();
        assert!(!handle.needs_collection().load(Ordering::Acquire));
    }

    #[test]
    fn test_collect_all_arenas_no_deadlock() {
        use std::thread;
        use std::time::Duration;

        let coordinator = Arc::new(GCCoordinator::new());

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
        coordinator.collect_all_arenas(dotnet_utils::ArenaId(1));

        done.store(true, Ordering::Relaxed);
        t.join().unwrap();
    }
}

use crate::value::object::ObjectPtr;
use parking_lot::{Condvar, Mutex};
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;

thread_local! {
    /// Found cross-arena references during the current marking phase.
    static FOUND_CROSS_ARENA_REFS: RefCell<Vec<(u64, ObjectPtr)>> = const { RefCell::new(Vec::new()) };
    /// The thread ID of the arena currently being traced.
    static CURRENTLY_TRACING_THREAD_ID: Cell<Option<u64>> = const { Cell::new(None) };
    /// The ArenaHandle for the current thread.
    static CURRENT_ARENA_HANDLE: RefCell<Option<ArenaHandle>> = const { RefCell::new(None) };
}

/// Set the ArenaHandle for the current thread.
pub fn set_current_arena_handle(handle: ArenaHandle) {
    CURRENT_ARENA_HANDLE.with(|h| {
        *h.borrow_mut() = Some(handle);
    });
}

/// Record an allocation of the given size in the current thread's arena.
/// This tracks allocation pressure and may trigger a GC when the threshold is exceeded.
///
/// Called automatically by `ObjectRef::new()` for all heap allocations.
pub fn record_allocation(size: usize) {
    CURRENT_ARENA_HANDLE.with(|h| {
        if let Some(handle) = h.borrow().as_ref() {
            handle.record_allocation(size);
        }
    });
}

/// Check if the current thread's arena has requested a collection.
pub fn is_current_arena_collection_requested() -> bool {
    CURRENT_ARENA_HANDLE.with(|h| {
        if let Some(handle) = h.borrow().as_ref() {
            handle.needs_collection.load(Ordering::Acquire)
        } else {
            false
        }
    })
}

/// Reset the collection request flag for the current thread's arena.
pub fn reset_current_arena_collection_requested() {
    CURRENT_ARENA_HANDLE.with(|h| {
        if let Some(handle) = h.borrow().as_ref() {
            handle.needs_collection.store(false, Ordering::Release);
            handle.allocation_counter.store(0, Ordering::Release);
        }
    });
}

/// Allocation threshold in bytes that triggers a GC request for a thread-local arena.
/// When a thread allocates more than this amount since the last GC, it will request
/// a coordinated collection at the next safe point.
const ALLOCATION_THRESHOLD: usize = 1024 * 1024; // 1MB per-thread trigger

impl ArenaHandle {
    /// Record an allocation and check if we've exceeded the threshold.
    pub fn record_allocation(&self, size: usize) {
        let current = self.allocation_counter.fetch_add(size, Ordering::Relaxed);
        if current + size > ALLOCATION_THRESHOLD {
            self.needs_collection.store(true, Ordering::Release);
        }
    }
}

use std::cell::{Cell, RefCell};

/// Set the thread ID of the arena currently being traced.
pub fn set_currently_tracing(thread_id: Option<u64>) {
    CURRENTLY_TRACING_THREAD_ID.with(|id| {
        id.set(thread_id);
    });
}

/// Get the thread ID of the arena currently being traced.
pub fn get_currently_tracing() -> Option<u64> {
    CURRENTLY_TRACING_THREAD_ID.with(|id| id.get())
}

/// Take all found cross-arena references and clear the local list.
pub fn take_found_cross_arena_refs() -> Vec<(u64, ObjectPtr)> {
    FOUND_CROSS_ARENA_REFS.with(|refs| {
        let mut r = refs.borrow_mut();
        std::mem::take(&mut *r)
    })
}

/// Record a cross-arena reference found during marking.
///
/// When an object in arena A references an object in arena B, this function
/// records the reference so the coordinator can ensure object B is kept alive
/// during the fixed-point iteration phase of GC.
///
/// This is called automatically by `ObjectRef::trace()` when it detects
/// that the owner_id of a referenced object differs from the currently tracing thread.
pub fn record_cross_arena_ref(target_thread_id: u64, ptr: ObjectPtr) {
    FOUND_CROSS_ARENA_REFS.with(|refs| {
        refs.borrow_mut().push((target_thread_id, ptr));
    });
}

/// GC commands sent from the coordinator to worker threads.
#[derive(Debug, Clone)]
pub enum GCCommand {
    /// Perform a full collection of the local arena.
    CollectAll,
    /// Mark specific objects in the local arena (for cross-arena resurrection).
    MarkObjects(HashSet<ObjectPtr>),
}

/// Metadata about each thread's arena and its communication channel.
#[derive(Debug, Clone)]
pub struct ArenaHandle {
    pub thread_id: u64,
    pub allocation_counter: Arc<AtomicUsize>,
    pub needs_collection: Arc<AtomicBool>,
    /// Command currently being processed by this thread.
    pub current_command: Arc<Mutex<Option<GCCommand>>>,
    /// Signal to wake up the thread when a command is available.
    pub command_signal: Arc<Condvar>,
    /// Signal to the coordinator that the command is finished.
    pub finish_signal: Arc<Condvar>,
}

/// Coordinates stop-the-world collections across multiple thread-local arenas.
pub struct GCCoordinator {
    /// thread_id -> arena metadata
    arenas: Mutex<HashMap<u64, ArenaHandle>>,
    /// Global lock held during collection
    collection_lock: Mutex<()>,
    /// Flag indicating if a collection is currently in progress
    is_collecting: AtomicBool,
    /// Cross-arena references found during marking
    cross_arena_refs: Mutex<HashMap<u64, HashSet<ObjectPtr>>>,
}

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
        arenas.insert(handle.thread_id, handle);
    }

    /// Unregister a thread-local arena.
    pub fn unregister_arena(&self, thread_id: u64) {
        let mut arenas = self.arenas.lock();
        arenas.remove(&thread_id);
    }

    /// Check if any arena requires collection due to allocation pressure.
    pub fn should_collect(&self) -> bool {
        let arenas = self.arenas.lock();
        for handle in arenas.values() {
            if handle.needs_collection.load(Ordering::Acquire) {
                return true;
            }
        }
        false
    }

    /// Mark that a collection has started.
    pub fn start_collection(&self) -> Option<MutexGuard<'_, ()>> {
        let guard = self.collection_lock.try_lock()?;
        self.is_collecting.store(true, Ordering::Release);
        Some(guard)
    }

    /// Mark that a collection has finished.
    pub fn finish_collection(&self) {
        self.is_collecting.store(false, Ordering::Release);

        // Reset all collection flags
        let arenas = self.arenas.lock();
        for handle in arenas.values() {
            handle.needs_collection.store(false, Ordering::Release);
            // We don't reset allocation_counter here as it might be useful for stats,
            // or we might want to reset it. For now, let's just clear the flag.
        }
    }

    /// Get the total allocated size across all arenas.
    pub fn total_allocated(&self) -> usize {
        let arenas = self.arenas.lock();
        arenas
            .values()
            .map(|h| h.allocation_counter.load(Ordering::Acquire))
            .sum()
    }

    /// Perform a coordinated collection across all registered arenas.
    ///
    /// This implements the cross-arena resurrection algorithm with fixed-point iteration:
    ///
    /// # Algorithm
    ///
    /// 1. **Phase 1 - Initial Marking**: Each arena marks from its local roots (stack, GC handles, etc.)
    ///    - Cross-arena references are detected by comparing `owner_id` with the tracing thread ID
    ///    - Cross-arena refs are recorded but NOT traced in the source arena
    ///
    /// 2. **Phase 2 - Fixed-Point Iteration**: Repeatedly resurrect cross-arena referenced objects
    ///    - For each cross-arena ref, send `MarkObjects` command to the target arena
    ///    - Target arenas re-mark from these new roots
    ///    - Continue until no new cross-arena refs are discovered (fixed point)
    ///
    /// 3. **Phase 3 - Sweep**: Each arena independently sweeps unmarked objects
    ///
    /// # Threading Model
    ///
    /// - The `initiating_thread_id` is the thread that triggered GC and calls this function
    /// - It executes GC commands directly (to avoid self-deadlock)
    /// - Other threads receive commands via their command channels and execute at safe points
    ///
    /// # Safety
    ///
    /// This function MUST be called while holding a `StopTheWorldGuard` to ensure all threads
    /// are at safe points and cannot mutate objects during marking.
    pub fn collect_all_arenas(&self, initiating_thread_id: u64) {
        // This is called by the thread that triggered the GC, after STW is established.

        let handles: Vec<ArenaHandle> = {
            let arenas = self.arenas.lock();
            arenas.values().cloned().collect()
        };

        // Phase 1: Initial marking - each arena marks its local roots
        {
            // Clear any stale cross-arena references from previous collections
            let mut refs = self.cross_arena_refs.lock();
            refs.clear();
        }

        // Send CollectAll command to all OTHER arenas (not the initiating thread)
        for handle in &handles {
            if handle.thread_id != initiating_thread_id {
                let mut cmd = handle.current_command.lock();
                *cmd = Some(GCCommand::CollectAll);
                handle.command_signal.notify_all();
            }
        }

        // The initiating thread performs its own collection directly
        crate::vm::threading::execute_gc_command_for_current_thread(GCCommand::CollectAll, self);

        // Wait for all OTHER arenas to finish initial marking
        for handle in &handles {
            if handle.thread_id != initiating_thread_id {
                let mut cmd = handle.current_command.lock();
                while cmd.is_some() {
                    handle.finish_signal.wait(&mut cmd);
                }
            }
        }

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
                if target_thread_id == initiating_thread_id {
                    // Save for direct execution by initiating thread
                    initiator_mark_objs = Some(ptrs);
                } else if let Some(handle) =
                    handles.iter().find(|h| h.thread_id == target_thread_id)
                {
                    let mut cmd = handle.current_command.lock();
                    *cmd = Some(GCCommand::MarkObjects(ptrs));
                    handle.command_signal.notify_all();
                }
            }

            // Execute MarkObjects for the initiating thread directly
            if let Some(ptrs) = initiator_mark_objs {
                crate::vm::threading::execute_gc_command_for_current_thread(
                    GCCommand::MarkObjects(ptrs),
                    self,
                );
            }

            // Wait for all MarkObjects commands to complete (excluding initiating thread)
            for handle in &handles {
                if handle.thread_id != initiating_thread_id {
                    let mut cmd = handle.current_command.lock();
                    while cmd.is_some() {
                        handle.finish_signal.wait(&mut cmd);
                    }
                }
            }

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

        // Phase 3: Sweep completed (already done by collect_all in each arena)
    }

    /// Check if a thread has a pending GC command.
    pub fn has_command(&self, thread_id: u64) -> bool {
        let arenas = self.arenas.lock();
        if let Some(handle) = arenas.get(&thread_id) {
            handle.current_command.lock().is_some()
        } else {
            false
        }
    }

    /// Get the pending command for a thread.
    pub fn get_command(&self, thread_id: u64) -> Option<GCCommand> {
        let arenas = self.arenas.lock();
        if let Some(handle) = arenas.get(&thread_id) {
            handle.current_command.lock().clone()
        } else {
            None
        }
    }

    /// Mark a command as finished for a thread.
    pub fn command_finished(&self, thread_id: u64) {
        let arenas = self.arenas.lock();
        if let Some(handle) = arenas.get(&thread_id) {
            let mut cmd = handle.current_command.lock();
            *cmd = None;
            handle.finish_signal.notify_all();
        }
    }

    /// Record a cross-arena reference found during marking.
    pub fn record_cross_arena_ref(&self, target_thread_id: u64, ptr: ObjectPtr) {
        let mut refs = self.cross_arena_refs.lock();
        refs.entry(target_thread_id).or_default().insert(ptr);
    }
}

impl Default for GCCoordinator {
    fn default() -> Self {
        Self::new()
    }
}

pub type MutexGuard<'a, T> = parking_lot::MutexGuard<'a, T>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_coordinator_registration() {
        let coordinator = GCCoordinator::new();
        let counter = Arc::new(AtomicUsize::new(100));
        let flag = Arc::new(AtomicBool::new(false));

        let handle = ArenaHandle {
            thread_id: 1,
            allocation_counter: counter.clone(),
            needs_collection: flag.clone(),
            current_command: Arc::new(Mutex::new(None)),
            command_signal: Arc::new(Condvar::new()),
            finish_signal: Arc::new(Condvar::new()),
        };

        coordinator.register_arena(handle);
        assert_eq!(coordinator.total_allocated(), 100);

        flag.store(true, Ordering::Release);
        assert!(coordinator.should_collect());

        let _guard = coordinator.start_collection().unwrap();
        coordinator.finish_collection();

        assert!(!coordinator.should_collect());
        assert!(!flag.load(Ordering::Acquire));

        coordinator.unregister_arena(1);
        assert_eq!(coordinator.total_allocated(), 0);
    }

    #[test]
    fn test_allocation_pressure_trigger() {
        let coordinator = GCCoordinator::new();
        let handle = ArenaHandle {
            thread_id: 1,
            allocation_counter: Arc::new(AtomicUsize::new(0)),
            needs_collection: Arc::new(AtomicBool::new(false)),
            current_command: Arc::new(Mutex::new(None)),
            command_signal: Arc::new(Condvar::new()),
            finish_signal: Arc::new(Condvar::new()),
        };

        coordinator.register_arena(handle.clone());
        set_current_arena_handle(handle.clone());

        // Allocate just below threshold
        record_allocation(ALLOCATION_THRESHOLD - 100);
        assert!(!is_current_arena_collection_requested());

        // Allocate to cross threshold
        record_allocation(200);
        assert!(is_current_arena_collection_requested());

        reset_current_arena_collection_requested();
        assert!(!is_current_arena_collection_requested());
        assert_eq!(handle.allocation_counter.load(Ordering::Acquire), 0);
    }
}

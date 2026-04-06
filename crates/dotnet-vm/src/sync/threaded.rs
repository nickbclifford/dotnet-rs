use crate::{
    gc::coordinator::GCCoordinator,
    sync::{Arc, Condvar, LockResult, Mutex},
    threading::ThreadManagerOps,
};
use dotnet_metrics::RuntimeMetrics;
use dotnet_utils::ArenaId;
use std::{collections::HashMap, time::Instant};

#[derive(Debug)]
struct SyncBlockState {
    /// Thread ID of the current lock owner (ArenaId::INVALID means unlocked)
    owner_thread_id: ArenaId,
    /// Lock recursion count (for nested locks by the same thread)
    recursion_count: usize,
}

/// Represents a synchronization block for Monitor operations on .NET objects.
#[derive(Debug)]
pub struct SyncBlock {
    state: Mutex<SyncBlockState>,
    condvar: Condvar,
}

impl SyncBlock {
    pub(super) fn new() -> Self {
        Self {
            state: Mutex::new(SyncBlockState {
                owner_thread_id: ArenaId::INVALID,
                recursion_count: 0,
            }),
            condvar: Condvar::new(),
        }
    }
}

impl super::SyncBlockBackend for SyncBlock {
    fn try_enter(&self, thread_id: ArenaId) -> bool {
        let mut state = self.state.lock();
        if state.owner_thread_id == ArenaId::INVALID {
            state.owner_thread_id = thread_id;
            state.recursion_count = 1;
            true
        } else if state.owner_thread_id == thread_id {
            state.recursion_count += 1;
            true
        } else {
            false
        }
    }

    fn enter(&self, thread_id: ArenaId, metrics: &RuntimeMetrics) {
        use std::time::Instant;
        let mut state = self.state.lock();

        if state.owner_thread_id == thread_id {
            state.recursion_count += 1;
            return;
        }

        if state.owner_thread_id != ArenaId::INVALID {
            let start_wait = Instant::now();
            while state.owner_thread_id != ArenaId::INVALID {
                self.condvar.wait(&mut state);
            }
            metrics.record_lock_contention(start_wait.elapsed());
        }

        state.owner_thread_id = thread_id;
        state.recursion_count = 1;
    }

    fn enter_with_timeout(
        &self,
        thread_id: ArenaId,
        timeout_ms: u64,
        metrics: &RuntimeMetrics,
    ) -> bool {
        use std::time::{Duration, Instant};

        if timeout_ms == 0 {
            return super::SyncBlockBackend::try_enter(self, thread_id);
        }

        let mut state = self.state.lock();

        if state.owner_thread_id == thread_id {
            state.recursion_count += 1;
            return true;
        }

        let deadline = Instant::now() + Duration::from_millis(timeout_ms);

        if state.owner_thread_id != ArenaId::INVALID {
            let start_wait = Instant::now();
            while state.owner_thread_id != ArenaId::INVALID {
                let now = Instant::now();
                if now >= deadline {
                    return false;
                }

                let remaining = deadline - now;
                let timed_out = self.condvar.wait_for(&mut state, remaining).timed_out();

                if timed_out && state.owner_thread_id != ArenaId::INVALID {
                    return false;
                }
            }
            metrics.record_lock_contention(start_wait.elapsed());
        }

        state.owner_thread_id = thread_id;
        state.recursion_count = 1;
        true
    }

    fn enter_safe(
        &self,
        thread_id: ArenaId,
        metrics: &RuntimeMetrics,
        thread_manager: &impl ThreadManagerOps,
        _gc_coordinator: &GCCoordinator,
    ) -> LockResult {
        use std::time::Instant;

        loop {
            let mut state = self.state.lock();

            if state.owner_thread_id == thread_id {
                state.recursion_count += 1;
                return LockResult::Success;
            }

            if state.owner_thread_id == ArenaId::INVALID {
                state.owner_thread_id = thread_id;
                state.recursion_count = 1;
                return LockResult::Success;
            }

            // Owner is someone else, we need to wait
            let start_wait = Instant::now();

            // Wait with timeout to allow periodic safe point checks
            let timeout = super::get_safe_point_yield_duration();
            let _ = self.condvar.wait_for(&mut state, timeout);

            if state.owner_thread_id == ArenaId::INVALID {
                state.owner_thread_id = thread_id;
                state.recursion_count = 1;
                metrics.record_lock_contention(start_wait.elapsed());
                return LockResult::Success;
            }

            // Drop the lock before checking safe point
            drop(state);

            // Check for GC safe point
            if thread_manager.is_gc_stop_requested() {
                return LockResult::Yield;
            }
        }
    }

    fn enter_with_timeout_safe(
        &self,
        thread_id: ArenaId,
        deadline: Instant,
        metrics: &RuntimeMetrics,
        thread_manager: &impl ThreadManagerOps,
        _gc_coordinator: &GCCoordinator,
    ) -> LockResult {
        loop {
            let now = Instant::now();

            let mut state = self.state.lock();

            if state.owner_thread_id == thread_id {
                state.recursion_count += 1;
                return LockResult::Success;
            }

            if state.owner_thread_id == ArenaId::INVALID {
                state.owner_thread_id = thread_id;
                state.recursion_count = 1;
                return LockResult::Success;
            }

            if now >= deadline {
                return LockResult::Timeout;
            }

            // Calculate remaining time, with a maximum wait defined by the safe point yield interval
            let remaining = deadline - now;
            let wait_duration = remaining.min(super::get_safe_point_yield_duration());

            let start_wait = Instant::now();
            let _ = self.condvar.wait_for(&mut state, wait_duration);

            if state.owner_thread_id == ArenaId::INVALID {
                state.owner_thread_id = thread_id;
                state.recursion_count = 1;
                metrics.record_lock_contention(start_wait.elapsed());
                return LockResult::Success;
            }

            // Drop the lock before checking safe point
            drop(state);

            // Check for GC safe point
            if thread_manager.is_gc_stop_requested() {
                return LockResult::Yield;
            }
        }
    }

    fn exit(&self, thread_id: ArenaId) -> bool {
        let mut state = self.state.lock();

        if state.owner_thread_id != thread_id || thread_id == ArenaId::INVALID {
            return false;
        }

        if state.recursion_count == 0 {
            return false;
        }

        state.recursion_count -= 1;

        if state.recursion_count == 0 {
            state.owner_thread_id = ArenaId::INVALID;
            self.condvar.notify_one();
        }

        true
    }

    fn wait(&self, _thread_id: ArenaId, _timeout_ms: Option<u64>) -> Result<(), &'static str> {
        Err("Monitor.Wait() is not yet implemented")
    }

    fn pulse(&self, _thread_id: ArenaId) -> Result<(), &'static str> {
        Err("Monitor.Pulse() is not yet implemented")
    }

    fn pulse_all(&self, _thread_id: ArenaId) -> Result<(), &'static str> {
        Err("Monitor.PulseAll() is not yet implemented")
    }
}

pub struct SyncBlockManager {
    blocks: Mutex<HashMap<usize, Arc<SyncBlock>>>,
    next_index: Mutex<usize>,
}

impl SyncBlockManager {
    pub fn new() -> Self {
        Self {
            blocks: Mutex::new(HashMap::new()),
            next_index: Mutex::new(1),
        }
    }
}

impl super::SyncManagerBackend for SyncBlockManager {
    type Block = SyncBlock;

    fn get_or_create_sync_block(
        &self,
        get_index: impl FnOnce() -> Option<usize>,
        set_index: impl FnOnce(usize),
    ) -> (usize, Arc<Self::Block>) {
        let mut blocks = self.blocks.lock();

        if let Some(index) = get_index()
            && let Some(block) = blocks.get(&index)
        {
            return (index, block.clone());
        }

        let index = {
            let mut next = self.next_index.lock();
            let idx = *next;
            *next += 1;
            idx
        };

        let block = Arc::new(SyncBlock::new());
        blocks.insert(index, block.clone());
        set_index(index);

        (index, block)
    }

    fn get_sync_block(&self, index: usize) -> Option<Arc<SyncBlock>> {
        self.blocks.lock().get(&index).cloned()
    }

    fn try_enter_block(
        &self,
        block: Arc<SyncBlock>,
        thread_id: ArenaId,
        _metrics: &RuntimeMetrics,
    ) -> bool {
        super::SyncBlockBackend::try_enter(block.as_ref(), thread_id)
    }
}

#[cfg(all(test, feature = "multithreading"))]
mod tests {
    use super::*;
    use crate::{
        gc::coordinator::{GCCommand, GCCoordinator},
        sync::{LockResult, SyncBlockOps},
        threading::{STWGuardOps, ThreadManagerOps},
    };
    use dotnet_metrics::RuntimeMetrics;
    use dotnet_tracer::Tracer;
    use dotnet_utils::ArenaId;
    use std::{
        sync::{
            Arc as StdArc,
            atomic::{AtomicBool, Ordering},
            mpsc,
        },
        thread,
        time::{Duration, Instant},
    };

    // ArenaId constants for test threads
    const THREAD_A: ArenaId = ArenaId(1);
    const THREAD_B: ArenaId = ArenaId(2);

    // --- Minimal mock for ThreadManagerOps ---

    struct MockSTWGuard;
    impl STWGuardOps for MockSTWGuard {
        fn elapsed_micros(&self) -> u64 {
            0
        }
    }

    struct MockThreadManager {
        gc_stop: AtomicBool,
    }

    impl MockThreadManager {
        fn new() -> Self {
            Self {
                gc_stop: AtomicBool::new(false),
            }
        }

        fn set_gc_stop(&self, v: bool) {
            self.gc_stop.store(v, Ordering::SeqCst);
        }
    }

    impl ThreadManagerOps for MockThreadManager {
        type Guard<'a> = MockSTWGuard;

        fn register_thread(&self) -> ArenaId {
            THREAD_A
        }
        fn register_thread_traced(&self, _tracer: &mut Tracer, _name: &str) -> ArenaId {
            THREAD_A
        }
        fn unregister_thread(&self, _managed_id: ArenaId) {}
        fn unregister_thread_traced(&self, _managed_id: ArenaId, _tracer: &mut Tracer) {}
        fn current_thread_id(&self) -> Option<ArenaId> {
            None
        }
        fn thread_count(&self) -> usize {
            1
        }
        fn is_gc_stop_requested(&self) -> bool {
            self.gc_stop.load(Ordering::SeqCst)
        }
        fn safe_point(&self, _managed_id: ArenaId, _coordinator: &GCCoordinator) {}
        fn execute_gc_command(&self, _command: GCCommand, _coordinator: &GCCoordinator) {}
        fn safe_point_traced(
            &self,
            _managed_id: ArenaId,
            _coordinator: &GCCoordinator,
            _tracer: &mut Tracer,
            _location: &str,
        ) {
        }
        fn request_stop_the_world(&self) -> Self::Guard<'_> {
            MockSTWGuard
        }
        fn request_stop_the_world_traced(&self, _tracer: &mut Tracer) -> Self::Guard<'_> {
            MockSTWGuard
        }
    }

    fn make_gc() -> GCCoordinator {
        GCCoordinator::new(Arc::new(AtomicBool::new(false)))
    }

    fn make_metrics() -> RuntimeMetrics {
        RuntimeMetrics::new()
    }

    // ===== enter_with_timeout tests =====

    #[test]
    fn enter_with_timeout_zero_on_free_lock_succeeds() {
        // timeout_ms=0 delegates to try_enter; free lock → true
        let block = SyncBlock::new();
        assert!(block.enter_with_timeout(THREAD_A, 0, &make_metrics()));
    }

    #[test]
    fn enter_with_timeout_zero_on_held_lock_fails() {
        // timeout_ms=0 delegates to try_enter; lock held by another → false
        let block = SyncBlock::new();
        block.enter_with_timeout(THREAD_A, 1000, &make_metrics());
        assert!(!block.enter_with_timeout(THREAD_B, 0, &make_metrics()));
    }

    #[test]
    fn enter_with_timeout_nonzero_on_free_lock_succeeds() {
        // Non-zero timeout; lock is free → immediate acquire → true
        let block = SyncBlock::new();
        assert!(block.enter_with_timeout(THREAD_A, 500, &make_metrics()));
    }

    #[test]
    fn enter_with_timeout_recursive_reentry_by_same_thread_succeeds() {
        // Same thread enters twice; both calls return true and recursion count is tracked
        let block = SyncBlock::new();
        let metrics = make_metrics();
        assert!(block.enter_with_timeout(THREAD_A, 500, &metrics));
        assert!(block.enter_with_timeout(THREAD_A, 500, &metrics));
        // Two exits needed to fully release
        assert!(block.exit(THREAD_A));
        assert!(block.exit(THREAD_A));
    }

    #[test]
    fn enter_with_timeout_returns_false_when_held_past_deadline() {
        // Lock held by THREAD_A; THREAD_B uses a tiny timeout and must time out
        let block = SyncBlock::new();
        block.enter_with_timeout(THREAD_A, 5000, &make_metrics());
        let result = block.enter_with_timeout(THREAD_B, 1, &make_metrics());
        assert!(!result, "expected timeout (false) while lock is held");
    }

    #[test]
    fn enter_with_timeout_succeeds_after_lock_released_by_other_thread() {
        // THREAD_A holds the lock briefly, then releases; THREAD_B acquires successfully
        let block = StdArc::new(SyncBlock::new());
        let block_a = StdArc::clone(&block);
        let (tx, rx) = mpsc::channel::<()>();

        let handle = thread::spawn(move || {
            block_a.enter_with_timeout(THREAD_A, 5000, &make_metrics());
            let _ = tx.send(()); // signal: lock is held
            thread::sleep(Duration::from_millis(20));
            block_a.exit(THREAD_A);
        });

        rx.recv().unwrap(); // wait until thread A holds the lock

        let result = block.enter_with_timeout(THREAD_B, 500, &make_metrics());
        assert!(
            result,
            "expected to acquire lock after thread A released it"
        );

        handle.join().unwrap();
    }

    // ===== enter_with_timeout_safe tests =====

    #[test]
    fn enter_with_timeout_safe_free_lock_returns_success() {
        // Free lock with generous deadline → Success immediately
        let block = SyncBlock::new();
        let mgr = MockThreadManager::new();
        let gc = make_gc();
        let deadline = Instant::now() + Duration::from_secs(5);
        let result = block.enter_with_timeout_safe(THREAD_A, deadline, &make_metrics(), &mgr, &gc);
        assert_eq!(result, LockResult::Success);
    }

    #[test]
    fn enter_with_timeout_safe_recursive_reentry_by_same_thread_returns_success() {
        // Same thread calls twice; second call must return Success and recursion is tracked
        let block = SyncBlock::new();
        let mgr = MockThreadManager::new();
        let gc = make_gc();
        let d = || Instant::now() + Duration::from_secs(5);
        block.enter_with_timeout_safe(THREAD_A, d(), &make_metrics(), &mgr, &gc);
        let result = block.enter_with_timeout_safe(THREAD_A, d(), &make_metrics(), &mgr, &gc);
        assert_eq!(result, LockResult::Success);
        assert!(block.exit(THREAD_A));
        assert!(block.exit(THREAD_A));
    }

    #[test]
    fn enter_with_timeout_safe_expired_deadline_on_held_lock_returns_timeout() {
        // THREAD_A holds lock; THREAD_B uses already-expired deadline → Timeout
        let block = SyncBlock::new();
        block.enter_with_timeout(THREAD_A, 5000, &make_metrics());
        let mgr = MockThreadManager::new();
        let gc = make_gc();
        let expired = Instant::now() - Duration::from_millis(1);
        let result = block.enter_with_timeout_safe(THREAD_B, expired, &make_metrics(), &mgr, &gc);
        assert_eq!(result, LockResult::Timeout);
    }

    #[test]
    fn enter_with_timeout_safe_gc_stop_requested_returns_yield() {
        // THREAD_A holds lock; GC stop is flagged → THREAD_B gets Yield after safe-point interval
        let block = SyncBlock::new();
        block.enter_with_timeout(THREAD_A, 5000, &make_metrics());
        let mgr = MockThreadManager::new();
        mgr.set_gc_stop(true);
        let gc = make_gc();
        let deadline = Instant::now() + Duration::from_secs(10);
        let result = block.enter_with_timeout_safe(THREAD_B, deadline, &make_metrics(), &mgr, &gc);
        assert_eq!(result, LockResult::Yield);
    }

    #[test]
    fn enter_with_timeout_safe_succeeds_after_lock_released_by_other_thread() {
        // THREAD_A holds lock briefly, releases; THREAD_B acquires → Success
        let block = StdArc::new(SyncBlock::new());
        let block_a = StdArc::clone(&block);
        let (tx, rx) = mpsc::channel::<()>();

        let handle = thread::spawn(move || {
            block_a.enter_with_timeout(THREAD_A, 5000, &make_metrics());
            let _ = tx.send(());
            thread::sleep(Duration::from_millis(20));
            block_a.exit(THREAD_A);
        });

        rx.recv().unwrap();

        let mgr = MockThreadManager::new();
        let gc = make_gc();
        let deadline = Instant::now() + Duration::from_secs(5);
        let result = block.enter_with_timeout_safe(THREAD_B, deadline, &make_metrics(), &mgr, &gc);
        assert_eq!(result, LockResult::Success);

        handle.join().unwrap();
    }
}

impl Default for SyncBlockManager {
    fn default() -> Self {
        Self::new()
    }
}

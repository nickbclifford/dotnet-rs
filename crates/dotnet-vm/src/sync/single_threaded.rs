use crate::{
    gc::coordinator::GCCoordinator,
    metrics::RuntimeMetrics,
    sync::{Arc, LockResult, Mutex},
    threading::ThreadManagerOps,
};
use dotnet_utils::ArenaId;
use std::{
    collections::HashMap,
    sync::atomic::{AtomicUsize, Ordering},
};

#[derive(Debug)]
pub struct SyncBlock {
    recursion_count: AtomicUsize,
}

impl SyncBlock {
    pub(super) fn new() -> Self {
        Self {
            recursion_count: AtomicUsize::new(0),
        }
    }
}

impl super::SyncBlockBackend for SyncBlock {
    fn try_enter(&self, _thread_id: ArenaId) -> bool {
        self.recursion_count.fetch_add(1, Ordering::Relaxed);
        true
    }

    fn enter(&self, thread_id: ArenaId, _metrics: &RuntimeMetrics) {
        self.try_enter(thread_id);
    }

    fn enter_with_timeout(
        &self,
        thread_id: ArenaId,
        _timeout_ms: u64,
        _metrics: &RuntimeMetrics,
    ) -> bool {
        self.enter(thread_id, _metrics);
        true
    }

    fn enter_safe(
        &self,
        thread_id: ArenaId,
        metrics: &RuntimeMetrics,
        _thread_manager: &impl ThreadManagerOps,
        _gc_coordinator: &GCCoordinator,
    ) -> LockResult {
        self.enter(thread_id, metrics);
        LockResult::Success
    }

    fn enter_with_timeout_safe(
        &self,
        thread_id: ArenaId,
        deadline: std::time::Instant,
        metrics: &RuntimeMetrics,
        _thread_manager: &impl ThreadManagerOps,
        _gc_coordinator: &GCCoordinator,
    ) -> LockResult {
        let now = std::time::Instant::now();
        let timeout_ms = if deadline > now {
            (deadline - now).as_millis() as u64
        } else {
            0
        };
        if self.enter_with_timeout(thread_id, timeout_ms, metrics) {
            LockResult::Success
        } else {
            LockResult::Timeout
        }
    }

    fn exit(&self, _thread_id: ArenaId) -> bool {
        let count = self.recursion_count.load(Ordering::Relaxed);
        if count > 0 {
            self.recursion_count.fetch_sub(1, Ordering::Relaxed);
            true
        } else {
            false
        }
    }

    fn wait(&self, _thread_id: ArenaId, _timeout_ms: Option<u64>) -> Result<(), &'static str> {
        Err("Monitor.Wait() is not supported in single-threaded mode")
    }

    fn pulse(&self, _thread_id: ArenaId) -> Result<(), &'static str> {
        Err("Monitor.Pulse() is not supported in single-threaded mode")
    }

    fn pulse_all(&self, _thread_id: ArenaId) -> Result<(), &'static str> {
        Err("Monitor.PulseAll() is not supported in single-threaded mode")
    }
}

pub struct SyncBlockManager {
    blocks: Mutex<HashMap<usize, Arc<SyncBlock>>>,
    next_index: AtomicUsize,
}

impl SyncBlockManager {
    pub fn new() -> Self {
        Self {
            blocks: Mutex::new(HashMap::new()),
            next_index: AtomicUsize::new(1),
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
        if let Some(index) = get_index() {
            let blocks = self.blocks.lock();
            if let Some(block) = blocks.get(&index) {
                return (index, block.clone());
            }
        }

        let index = self.next_index.fetch_add(1, Ordering::Relaxed);

        let block = Arc::new(SyncBlock::new());
        let mut blocks = self.blocks.lock();
        blocks.insert(index, block.clone());

        set_index(index);
        (index, block)
    }

    fn get_sync_block(&self, index: usize) -> Option<Arc<SyncBlock>> {
        let blocks = self.blocks.lock();
        blocks.get(&index).cloned()
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

impl Default for SyncBlockManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gc::coordinator::GCCoordinator;
    use crate::metrics::RuntimeMetrics;
    use crate::sync::{LockResult, SyncBlockOps};
    use crate::threading::ThreadManagerOps;
    use dotnet_utils::ArenaId;
    use std::sync::Arc as StdArc;
    use std::sync::atomic::AtomicBool;

    const THREAD_A: ArenaId = ArenaId(1);

    struct MockSTWGuard;
    impl crate::threading::STWGuardOps for MockSTWGuard {
        fn elapsed_micros(&self) -> u64 {
            0
        }
    }

    struct MockThreadManager;
    impl ThreadManagerOps for MockThreadManager {
        type Guard<'a> = MockSTWGuard;
        fn register_thread(&self) -> ArenaId {
            THREAD_A
        }
        fn register_thread_traced(&self, _: &mut crate::tracer::Tracer, _: &str) -> ArenaId {
            THREAD_A
        }
        fn unregister_thread(&self, _: ArenaId) {}
        fn unregister_thread_traced(&self, _: ArenaId, _: &mut crate::tracer::Tracer) {}
        fn current_thread_id(&self) -> Option<ArenaId> {
            Some(THREAD_A)
        }
        fn thread_count(&self) -> usize {
            1
        }
        fn is_gc_stop_requested(&self) -> bool {
            false
        }
        fn safe_point(&self, _: ArenaId, _: &GCCoordinator) {}
        fn execute_gc_command(&self, _: crate::gc::coordinator::GCCommand, _: &GCCoordinator) {}
        fn safe_point_traced(
            &self,
            _: ArenaId,
            _: &GCCoordinator,
            _: &mut crate::tracer::Tracer,
            _: &str,
        ) {
        }
        fn request_stop_the_world(&self) -> Self::Guard<'_> {
            MockSTWGuard
        }
        fn request_stop_the_world_traced(&self, _: &mut crate::tracer::Tracer) -> Self::Guard<'_> {
            MockSTWGuard
        }
    }

    #[test]
    fn test_single_threaded_monitor_recursion() {
        let block = SyncBlock::new();

        // Successive entries should succeed (recursion)
        assert!(block.try_enter(THREAD_A));
        assert!(block.try_enter(THREAD_A));
        assert_eq!(block.recursion_count.load(Ordering::Relaxed), 2);

        // Successive exits should succeed
        assert!(block.exit(THREAD_A));
        assert!(block.exit(THREAD_A));
        assert_eq!(block.recursion_count.load(Ordering::Relaxed), 0);

        // Extra exit should fail
        assert!(!block.exit(THREAD_A));
    }

    #[test]
    fn test_single_threaded_enter_safe() {
        let block = SyncBlock::new();
        let metrics = RuntimeMetrics::new();
        let tm = MockThreadManager;
        let gc = GCCoordinator::new(StdArc::new(AtomicBool::new(false)));

        assert_eq!(
            block.enter_safe(THREAD_A, &metrics, &tm, &gc),
            LockResult::Success
        );
        assert_eq!(block.recursion_count.load(Ordering::Relaxed), 1);
        assert!(block.exit(THREAD_A));
    }
}

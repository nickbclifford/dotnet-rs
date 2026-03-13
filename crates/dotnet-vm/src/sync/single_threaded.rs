use crate::{
    gc::coordinator::GCCoordinator,
    metrics::RuntimeMetrics,
    sync::{Arc, LockResult, Mutex, SyncBlockOps, SyncManagerOps},
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

impl SyncBlockOps for SyncBlock {
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

impl SyncManagerOps for SyncBlockManager {
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
        block.try_enter(thread_id)
    }
}

impl Default for SyncBlockManager {
    fn default() -> Self {
        Self::new()
    }
}

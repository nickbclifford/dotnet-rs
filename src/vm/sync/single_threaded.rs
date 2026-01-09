use crate::{
    value::object::ObjectRef,
    vm::{
        gc::coordinator::GCCoordinator,
        metrics::RuntimeMetrics,
        sync::{Mutex, SyncBlockOps, SyncManagerOps},
        threading::ThreadManagerOps,
    },
};
use std::{collections::HashMap, sync::{Arc, atomic::{AtomicUsize, Ordering}}};

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
    fn try_enter(&self, _thread_id: u64) -> bool {
        self.recursion_count.fetch_add(1, Ordering::SeqCst);
        true
    }

    fn enter(&self, thread_id: u64, _metrics: &RuntimeMetrics) {
        self.try_enter(thread_id);
    }

    fn enter_with_timeout(
        &self,
        thread_id: u64,
        _timeout_ms: u64,
        _metrics: &RuntimeMetrics,
    ) -> bool {
        self.enter(thread_id, _metrics);
        true
    }

    fn enter_safe(
        &self,
        thread_id: u64,
        metrics: &RuntimeMetrics,
        _thread_manager: &impl ThreadManagerOps,
        _gc_coordinator: &GCCoordinator,
    ) {
        self.enter(thread_id, metrics);
    }

    fn enter_with_timeout_safe(
        &self,
        thread_id: u64,
        timeout_ms: u64,
        metrics: &RuntimeMetrics,
        _thread_manager: &impl ThreadManagerOps,
        _gc_coordinator: &GCCoordinator,
    ) -> bool {
        self.enter_with_timeout(thread_id, timeout_ms, metrics)
    }

    fn exit(&self, _thread_id: u64) -> bool {
        let count = self.recursion_count.load(Ordering::SeqCst);
        if count > 0 {
            self.recursion_count.fetch_sub(1, Ordering::SeqCst);
            true
        } else {
            false
        }
    }

    fn wait(&self, _thread_id: u64, _timeout_ms: Option<u64>) -> Result<(), &'static str> {
        Err("Monitor.Wait() is not supported in single-threaded mode")
    }

    fn pulse(&self, _thread_id: u64) -> Result<(), &'static str> {
        Err("Monitor.Pulse() is not supported in single-threaded mode")
    }

    fn pulse_all(&self, _thread_id: u64) -> Result<(), &'static str> {
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
        _object: &ObjectRef<'_>,
        get_index: impl FnOnce() -> Option<usize>,
        set_index: impl FnOnce(usize),
    ) -> (usize, Arc<SyncBlock>) {
        if let Some(index) = get_index() {
            let blocks = self.blocks.lock();
            if let Some(block) = blocks.get(&index) {
                return (index, block.clone());
            }
        }

        let index = self.next_index.fetch_add(1, Ordering::SeqCst);

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
        thread_id: u64,
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

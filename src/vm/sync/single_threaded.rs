use crate::{
    value::object::ObjectRef,
    vm::{metrics::RuntimeMetrics, sync::{SyncBlockOps, SyncManagerOps}},
};
use std::{cell::Cell, collections::HashMap, sync::Arc};

#[derive(Debug)]
pub struct SyncBlock {
    recursion_count: Cell<usize>,
}

impl SyncBlock {
    pub(super) fn new() -> Self {
        Self {
            recursion_count: Cell::new(0),
        }
    }
}

impl SyncBlockOps for SyncBlock {
    fn try_enter(&self, _thread_id: u64) -> bool {
        self.recursion_count.set(self.recursion_count.get() + 1);
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

    fn exit(&self, _thread_id: u64) -> bool {
        let count = self.recursion_count.get();
        if count > 0 {
            self.recursion_count.set(count - 1);
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
    blocks: Cell<Option<HashMap<usize, Arc<SyncBlock>>>>,
    next_index: Cell<usize>,
}

impl SyncBlockManager {
    pub fn new() -> Self {
        Self {
            blocks: Cell::new(Some(HashMap::new())),
            next_index: Cell::new(1),
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
            let blocks = self.blocks.take().expect("Sync blocks were already taken");
            if let Some(block) = blocks.get(&index) {
                let res = (index, block.clone());
                self.blocks.set(Some(blocks));
                return res;
            }
            self.blocks.set(Some(blocks));
        }

        let index = self.next_index.get();
        self.next_index.set(index + 1);

        let block = Arc::new(SyncBlock::new());
        let mut blocks = self.blocks.take().expect("Sync blocks were already taken");
        blocks.insert(index, block.clone());
        self.blocks.set(Some(blocks));

        set_index(index);
        (index, block)
    }

    fn get_sync_block(&self, index: usize) -> Option<Arc<SyncBlock>> {
        let blocks = self.blocks.take().expect("Sync blocks were already taken");
        let res = blocks.get(&index).cloned();
        self.blocks.set(Some(blocks));
        res
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

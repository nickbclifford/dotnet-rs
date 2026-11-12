use crate::sync::{Arc, Mutex, SyncBlockOps};
use dotnet_metrics::RuntimeMetrics;
use dotnet_utils::ArenaId;
use std::{
    collections::HashMap,
    num::NonZeroUsize,
    sync::atomic::{AtomicUsize, Ordering},
};

#[cfg(test)]
thread_local! {
    pub(crate) static MONITOR_TEST_CONTENDED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

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

    pub(crate) fn release_epoch(&self) -> u64 {
        0
    }

    pub(crate) fn wait_for_release(&self, _epoch: u64, duration: std::time::Duration) {
        std::thread::sleep(duration);
    }
}

impl super::SyncBlockOps for SyncBlock {
    fn try_enter(&self, _thread_id: ArenaId) -> bool {
        #[cfg(test)]
        if MONITOR_TEST_CONTENDED.with(std::cell::Cell::get) {
            return false;
        }
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

    fn exit(&self, _thread_id: ArenaId) -> bool {
        let count = self.recursion_count.load(Ordering::Relaxed);
        if count > 0 {
            self.recursion_count.fetch_sub(1, Ordering::Relaxed);
            true
        } else {
            false
        }
    }

    fn is_owned(&self, _thread_id: ArenaId) -> bool {
        self.recursion_count.load(Ordering::Relaxed) > 0
    }
    fn begin_wait(
        &self,
        thread_id: ArenaId,
        _deadline: Option<std::time::Instant>,
    ) -> Option<(u64, usize)> {
        if !self.is_owned(thread_id) {
            return None;
        }
        Some((0, self.recursion_count.swap(0, Ordering::Relaxed)))
    }
    fn poll_wait(&self, _id: u64, deadline: Option<std::time::Instant>) -> Option<bool> {
        deadline
            .filter(|d| std::time::Instant::now() >= *d)
            .map(|_| false)
    }
    fn cancel_wait(&self, _id: u64) {}
    fn restore_wait(&self, _thread_id: ArenaId, recursion: usize) -> bool {
        self.recursion_count.store(recursion, Ordering::Relaxed);
        true
    }
    fn pulse(&self, thread_id: ArenaId, _all: bool) -> bool {
        self.is_owned(thread_id)
    }
}

pub struct SyncBlockManager {
    blocks: Mutex<HashMap<NonZeroUsize, Arc<SyncBlock>>>,
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

impl super::SyncManagerOps for SyncBlockManager {
    type Block = SyncBlock;

    fn create_sync_block(&self) -> (NonZeroUsize, Arc<Self::Block>) {
        let index = NonZeroUsize::new(self.next_index.fetch_add(1, Ordering::Relaxed))
            .expect("SyncBlockManager index overflowed into zero");

        let block = Arc::new(SyncBlock::new());
        let mut blocks = self.blocks.lock();
        blocks.insert(index, block.clone());

        (index, block)
    }

    fn get_sync_block(&self, index: NonZeroUsize) -> Option<Arc<SyncBlock>> {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sync::{SyncBlockOps, SyncManagerOps};
    use dotnet_utils::ArenaId;

    const THREAD_A: ArenaId = ArenaId::new(1);

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
    fn test_single_threaded_manager_allocates_non_zero_indices() {
        let manager = SyncBlockManager::new();
        let (index, _block) = manager.create_sync_block();
        assert_ne!(index.get(), 0);
    }

    #[test]
    fn test_single_threaded_manager_resolves_created_sync_block() {
        let manager = SyncBlockManager::new();
        let (index, first) = manager.create_sync_block();
        let (other_index, _other) = manager.create_sync_block();
        let second = manager
            .get_sync_block(index)
            .expect("created block resolves");

        assert_ne!(other_index, index);
        assert!(Arc::ptr_eq(&first, &second));
    }
}

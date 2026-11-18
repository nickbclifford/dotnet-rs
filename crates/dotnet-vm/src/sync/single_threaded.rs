use crate::sync::{Arc, Mutex, SyncBlockOps};
use dotnet_metrics::RuntimeMetrics;
use dotnet_utils::ArenaId;
use std::{
    collections::{HashMap, VecDeque},
    num::NonZeroUsize,
    sync::atomic::{AtomicUsize, Ordering},
};

#[cfg(test)]
thread_local! {
    pub(crate) static MONITOR_TEST_CONTENDED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

#[derive(Debug)]
pub struct SyncBlock {
    state: Mutex<MonitorState>,
}

#[derive(Debug)]
struct MonitorState {
    owner: ArenaId,
    recursion: usize,
    next_waiter: u64,
    waiters: VecDeque<(u64, Option<std::time::Instant>, bool)>,
}

impl SyncBlock {
    pub(super) fn new() -> Self {
        Self {
            state: Mutex::new(MonitorState {
                owner: ArenaId::INVALID,
                recursion: 0,
                next_waiter: 0,
                waiters: VecDeque::new(),
            }),
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
    fn try_enter(&self, thread_id: ArenaId) -> bool {
        #[cfg(test)]
        if MONITOR_TEST_CONTENDED.with(std::cell::Cell::get) {
            return false;
        }
        let mut state = self.state.lock();
        if thread_id == ArenaId::INVALID
            || (state.owner != ArenaId::INVALID && state.owner != thread_id)
        {
            return false;
        }
        state.owner = thread_id;
        state.recursion += 1;
        true
    }

    fn enter(&self, thread_id: ArenaId, _metrics: &RuntimeMetrics) {
        assert!(
            self.try_enter(thread_id),
            "monitor entry would deadlock in single-threaded mode"
        );
    }

    fn enter_with_timeout(
        &self,
        thread_id: ArenaId,
        _timeout_ms: u64,
        _metrics: &RuntimeMetrics,
    ) -> bool {
        self.try_enter(thread_id)
    }

    fn exit(&self, thread_id: ArenaId) -> bool {
        let mut state = self.state.lock();
        if thread_id == ArenaId::INVALID || state.owner != thread_id {
            return false;
        }
        state.recursion -= 1;
        if state.recursion == 0 {
            state.owner = ArenaId::INVALID;
        }
        true
    }

    fn is_owned(&self, thread_id: ArenaId) -> bool {
        thread_id != ArenaId::INVALID && self.state.lock().owner == thread_id
    }
    fn begin_wait(
        &self,
        thread_id: ArenaId,
        deadline: Option<std::time::Instant>,
    ) -> Option<(u64, usize)> {
        let mut state = self.state.lock();
        if thread_id == ArenaId::INVALID || state.owner != thread_id {
            return None;
        }
        state.next_waiter = state
            .next_waiter
            .checked_add(1)
            .expect("monitor waiter id exhausted");
        let id = state.next_waiter;
        state.waiters.push_back((id, deadline, false));
        let recursion = state.recursion;
        state.owner = ArenaId::INVALID;
        state.recursion = 0;
        Some((id, recursion))
    }
    fn poll_wait(&self, id: u64, deadline: Option<std::time::Instant>) -> Option<bool> {
        let mut state = self.state.lock();
        let pos = state
            .waiters
            .iter()
            .position(|w| w.0 == id)
            .expect("monitor waiter missing");
        let pulsed = state.waiters[pos].2;
        if pulsed || deadline.is_some_and(|d| std::time::Instant::now() >= d) {
            state.waiters.remove(pos);
            Some(pulsed)
        } else {
            None
        }
    }
    fn cancel_wait(&self, id: u64) {
        self.state.lock().waiters.retain(|w| w.0 != id);
    }
    fn restore_wait(&self, thread_id: ArenaId, recursion: usize) -> bool {
        let mut state = self.state.lock();
        if thread_id == ArenaId::INVALID || state.owner != ArenaId::INVALID || recursion == 0 {
            return false;
        }
        state.owner = thread_id;
        state.recursion = recursion;
        true
    }
    fn pulse(&self, thread_id: ArenaId, all: bool) -> bool {
        let mut state = self.state.lock();
        if thread_id == ArenaId::INVALID || state.owner != thread_id {
            return false;
        }
        let now = std::time::Instant::now();
        for waiter in &mut state.waiters {
            if !waiter.2 && waiter.1.is_none_or(|d| now < d) {
                waiter.2 = true;
                if !all {
                    break;
                }
            }
        }
        true
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

    pub(crate) fn has_owned_monitor(&self, owner: ArenaId) -> bool {
        self.blocks
            .lock()
            .values()
            .any(|block| block.is_owned(owner))
    }
}

impl super::SyncManagerOps for SyncBlockManager {
    type Block = SyncBlock;

    fn create_sync_block(&self) -> (NonZeroUsize, Arc<Self::Block>) {
        let index = NonZeroUsize::new(self.next_index.fetch_add(1, Ordering::Relaxed))
            .expect("SyncBlockManager index overflowed into zero");

        #[allow(
            clippy::arc_with_non_send_sync,
            reason = "single-threaded monitor state is executor-confined; Arc keeps the feature-neutral interface"
        )]
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
        assert_eq!(block.state.lock().recursion, 2);

        // Successive exits should succeed
        assert!(block.exit(THREAD_A));
        assert!(block.exit(THREAD_A));
        assert_eq!(block.state.lock().recursion, 0);

        // Extra exit should fail
        assert!(!block.exit(THREAD_A));
    }

    #[test]
    fn owners_wait_pulse_and_restore_recursion() {
        let block = SyncBlock::new();
        let other = ArenaId::new(2);
        assert!(block.try_enter(THREAD_A));
        assert!(block.try_enter(THREAD_A));
        assert!(!block.try_enter(other));
        assert!(!block.exit(other));
        assert!(!block.pulse(other, false));
        assert!(block.begin_wait(other, None).is_none());
        let (id, recursion) = block.begin_wait(THREAD_A, None).unwrap();
        assert_eq!(recursion, 2);
        assert!(block.try_enter(other));
        assert!(!block.restore_wait(THREAD_A, recursion));
        assert!(block.pulse(other, false));
        assert_eq!(block.poll_wait(id, None), Some(true));
        assert!(block.exit(other));
        assert!(block.restore_wait(THREAD_A, recursion));
        assert!(block.exit(THREAD_A));
        assert!(block.is_owned(THREAD_A));
        assert!(block.exit(THREAD_A));
        assert!(!block.is_owned(THREAD_A));
    }

    #[test]
    fn timed_wait_and_pulse_all_preserve_ownership() {
        use std::time::{Duration, Instant};
        let block = SyncBlock::new();
        let other = ArenaId::new(2);
        assert!(block.try_enter(THREAD_A));
        let expired = Instant::now() - Duration::from_millis(1);
        let (expired_id, depth) = block.begin_wait(THREAD_A, Some(expired)).unwrap();
        assert!(block.try_enter(other));
        assert!(block.pulse(other, true));
        assert_eq!(block.poll_wait(expired_id, Some(expired)), Some(false));
        assert!(!block.restore_wait(THREAD_A, depth));
        assert!(block.exit(other));
        assert!(block.restore_wait(THREAD_A, depth));
        assert!(block.exit(THREAD_A));
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

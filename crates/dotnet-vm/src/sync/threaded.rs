use crate::sync::{Arc, Condvar, OrderedMutex, OrderedMutexGuard, SyncBlockOps, levels};
use dotnet_metrics::RuntimeMetrics;
use dotnet_utils::ArenaId;
use std::{collections::HashMap, num::NonZeroUsize, time::Instant};

#[derive(Debug)]
struct SyncBlockState {
    /// Thread ID of the current lock owner (ArenaId::INVALID means unlocked)
    owner_thread_id: ArenaId,
    /// Lock recursion count (for nested locks by the same thread)
    recursion_count: usize,
    release_epoch: u64,
    next_waiter: u64,
    waiters: std::collections::VecDeque<Waiter>,
}

#[derive(Debug)]
struct Waiter {
    id: u64,
    deadline: Option<Instant>,
    pulsed: bool,
}

/// Represents a synchronization block for Monitor operations on .NET objects.
#[derive(Debug)]
pub struct SyncBlock {
    state: OrderedMutex<levels::SyncState, SyncBlockState>,
    condvar: Condvar,
}

#[cfg(test)]
thread_local! {
    pub(crate) static WAIT_TEST_HOOK: std::cell::RefCell<Option<Box<dyn FnMut()>>> = std::cell::RefCell::new(None);
}

enum EnterState<'a> {
    Acquired,
    Contended(OrderedMutexGuard<'a, levels::SyncState, SyncBlockState>),
}

impl SyncBlock {
    pub(super) fn new() -> Self {
        Self {
            state: OrderedMutex::new(SyncBlockState {
                owner_thread_id: ArenaId::INVALID,
                recursion_count: 0,
                release_epoch: 0,
                next_waiter: 0,
                waiters: std::collections::VecDeque::new(),
            }),
            condvar: Condvar::new(),
        }
    }

    pub(crate) fn release_epoch(&self) -> u64 {
        self.state.lock().release_epoch
    }

    pub(crate) fn wait_for_release(&self, epoch: u64, duration: std::time::Duration) {
        let mut state = self.state.lock();
        if state.release_epoch == epoch {
            #[cfg(test)]
            WAIT_TEST_HOOK.with(|hook| {
                if let Some(callback) = hook.borrow_mut().as_mut() {
                    callback();
                }
            });
            self.condvar.wait_for(state.raw_mut(), duration);
        }
    }

    #[cfg(test)]
    pub(crate) fn notify_spurious(&self) {
        let _state = self.state.lock();
        self.condvar.notify_all();
    }

    fn mark_entered(
        state: &mut OrderedMutexGuard<'_, levels::SyncState, SyncBlockState>,
        thread_id: ArenaId,
    ) {
        state.owner_thread_id = thread_id;
        state.recursion_count = 1;
    }

    fn lock_and_try_enter(&self, thread_id: ArenaId) -> EnterState<'_> {
        let mut state = self.state.lock();

        if state.owner_thread_id == thread_id {
            state.recursion_count += 1;
            return EnterState::Acquired;
        }

        if state.owner_thread_id == ArenaId::INVALID {
            Self::mark_entered(&mut state, thread_id);
            return EnterState::Acquired;
        }

        EnterState::Contended(state)
    }
}

impl super::SyncBlockOps for SyncBlock {
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

        let mut state = match self.lock_and_try_enter(thread_id) {
            EnterState::Acquired => return,
            EnterState::Contended(state) => state,
        };

        let start_wait = Instant::now();
        while state.owner_thread_id != ArenaId::INVALID {
            self.condvar.wait(state.raw_mut());
        }
        metrics.record_lock_contention(start_wait.elapsed());

        Self::mark_entered(&mut state, thread_id);
    }

    fn enter_with_timeout(
        &self,
        thread_id: ArenaId,
        timeout_ms: u64,
        metrics: &RuntimeMetrics,
    ) -> bool {
        use std::time::{Duration, Instant};

        if timeout_ms == 0 {
            return super::SyncBlockOps::try_enter(self, thread_id);
        }

        let mut state = match self.lock_and_try_enter(thread_id) {
            EnterState::Acquired => return true,
            EnterState::Contended(state) => state,
        };

        let deadline = Instant::now() + Duration::from_millis(timeout_ms);
        let start_wait = Instant::now();
        while state.owner_thread_id != ArenaId::INVALID {
            let now = Instant::now();
            if now >= deadline {
                return false;
            }

            let remaining = deadline - now;
            let timed_out = self
                .condvar
                .wait_for(state.raw_mut(), remaining)
                .timed_out();

            if timed_out && state.owner_thread_id != ArenaId::INVALID {
                return false;
            }
        }
        metrics.record_lock_contention(start_wait.elapsed());

        Self::mark_entered(&mut state, thread_id);
        true
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
            state.release_epoch = state
                .release_epoch
                .checked_add(1)
                .expect("monitor release epoch exhausted");
            self.condvar.notify_all();
        }

        true
    }

    fn is_owned(&self, thread_id: ArenaId) -> bool {
        thread_id != ArenaId::INVALID && self.state.lock().owner_thread_id == thread_id
    }

    fn begin_wait(&self, thread_id: ArenaId, deadline: Option<Instant>) -> Option<(u64, usize)> {
        let mut state = self.state.lock();
        if thread_id == ArenaId::INVALID || state.owner_thread_id != thread_id {
            return None;
        }
        let recursion = state.recursion_count;
        state.next_waiter = state
            .next_waiter
            .checked_add(1)
            .expect("monitor waiter id exhausted");
        let id = state.next_waiter;
        state.waiters.push_back(Waiter {
            id,
            deadline,
            pulsed: false,
        });
        state.owner_thread_id = ArenaId::INVALID;
        state.recursion_count = 0;
        state.release_epoch = state
            .release_epoch
            .checked_add(1)
            .expect("monitor release epoch exhausted");
        self.condvar.notify_all();
        Some((id, recursion))
    }

    fn poll_wait(&self, id: u64, deadline: Option<Instant>) -> Option<bool> {
        let mut state = self.state.lock();
        let pos = state
            .waiters
            .iter()
            .position(|waiter| waiter.id == id)
            .expect("monitor waiter missing");
        let pulsed = state.waiters[pos].pulsed;
        if pulsed || deadline.is_some_and(|d| Instant::now() >= d) {
            state.waiters.remove(pos);
            Some(pulsed)
        } else {
            None
        }
    }

    fn cancel_wait(&self, id: u64) {
        self.state.lock().waiters.retain(|waiter| waiter.id != id);
    }

    fn restore_wait(&self, thread_id: ArenaId, recursion: usize) -> bool {
        let mut state = self.state.lock();
        if state.owner_thread_id != ArenaId::INVALID {
            return false;
        }
        state.owner_thread_id = thread_id;
        state.recursion_count = recursion;
        true
    }

    fn pulse(&self, thread_id: ArenaId, all: bool) -> bool {
        let mut state = self.state.lock();
        if thread_id == ArenaId::INVALID || state.owner_thread_id != thread_id {
            return false;
        }
        let now = Instant::now();
        let mut changed = false;
        for waiter in &mut state.waiters {
            if !waiter.pulsed && waiter.deadline.is_none_or(|d| now < d) {
                waiter.pulsed = true;
                changed = true;
                if !all {
                    break;
                }
            }
        }
        if changed {
            state.release_epoch = state
                .release_epoch
                .checked_add(1)
                .expect("monitor release epoch exhausted");
            self.condvar.notify_all();
        }
        true
    }
}

pub struct SyncBlockManager {
    blocks: OrderedMutex<levels::SyncBlocks, HashMap<NonZeroUsize, Arc<SyncBlock>>>,
    next_index: OrderedMutex<levels::SyncNextIndex, usize>,
}

impl SyncBlockManager {
    pub fn new() -> Self {
        Self {
            blocks: OrderedMutex::new(HashMap::new()),
            next_index: OrderedMutex::new(1),
        }
    }
}

impl super::SyncManagerOps for SyncBlockManager {
    type Block = SyncBlock;

    fn create_sync_block(&self) -> (NonZeroUsize, Arc<Self::Block>) {
        let mut blocks = self.blocks.lock();

        let index = {
            let mut next = self.next_index.lock_after(blocks.held_level());
            let idx = *next;
            *next = next
                .checked_add(1)
                .expect("SyncBlockManager index overflowed");
            NonZeroUsize::new(idx).expect("SyncBlockManager index must never be zero")
        };

        let block = Arc::new(SyncBlock::new());
        blocks.insert(index, block.clone());

        (index, block)
    }

    fn get_sync_block(&self, index: NonZeroUsize) -> Option<Arc<SyncBlock>> {
        self.blocks.lock().get(&index).cloned()
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

#[cfg(all(test, feature = "multithreading"))]
mod tests {
    use super::*;
    use crate::sync::{SyncBlockOps, SyncManagerOps};
    use dotnet_metrics::RuntimeMetrics;
    use dotnet_utils::ArenaId;
    use std::{
        sync::{Arc as StdArc, mpsc},
        thread,
        time::Duration,
    };

    // ArenaId constants for test threads
    const THREAD_A: ArenaId = ArenaId::new(1);
    const THREAD_B: ArenaId = ArenaId::new(2);

    fn make_metrics() -> RuntimeMetrics {
        RuntimeMetrics::new()
    }

    fn spawn_lock_holder(
        block: &StdArc<SyncBlock>,
    ) -> (mpsc::Receiver<()>, thread::JoinHandle<()>) {
        let block = StdArc::clone(block);
        let (tx, rx) = mpsc::channel::<()>();
        let handle = thread::spawn(move || {
            block.enter_with_timeout(THREAD_A, 5000, &make_metrics());
            let _ = tx.send(()); // signal: lock is held
            thread::sleep(Duration::from_millis(20));
            block.exit(THREAD_A);
        });
        (rx, handle)
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
        let (rx, handle) = spawn_lock_holder(&block);

        rx.recv().unwrap(); // wait until thread A holds the lock

        let result = block.enter_with_timeout(THREAD_B, 500, &make_metrics());
        assert!(
            result,
            "expected to acquire lock after thread A released it"
        );

        handle.join().unwrap();
    }

    #[test]
    fn sync_block_manager_allocates_non_zero_indices() {
        let manager = SyncBlockManager::new();
        let (index, _block) = manager.create_sync_block();
        assert_ne!(index.get(), 0);
    }

    #[test]
    fn sync_block_manager_resolves_created_sync_block() {
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

impl Default for SyncBlockManager {
    fn default() -> Self {
        Self::new()
    }
}

use crate::{
    gc::coordinator::GCCoordinator,
    metrics::RuntimeMetrics,
    sync::{SyncBlockOps, SyncManagerOps},
    threading::ThreadManagerOps,
};
use dotnet_utils::ArenaId;
use parking_lot::{Condvar, Mutex};
use std::{collections::HashMap, sync::Arc};

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

impl SyncBlockOps for SyncBlock {
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
            return self.try_enter(thread_id);
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
        gc_coordinator: &GCCoordinator,
    ) {
        use std::time::{Duration, Instant};

        loop {
            let mut state = self.state.lock();

            if state.owner_thread_id == thread_id {
                state.recursion_count += 1;
                return;
            }

            if state.owner_thread_id == ArenaId::INVALID {
                state.owner_thread_id = thread_id;
                state.recursion_count = 1;
                return;
            }

            // Owner is someone else, we need to wait
            let start_wait = Instant::now();

            // Wait with timeout to allow periodic safe point checks
            let timeout = Duration::from_millis(10);
            let _ = self.condvar.wait_for(&mut state, timeout);

            if state.owner_thread_id == ArenaId::INVALID {
                state.owner_thread_id = thread_id;
                state.recursion_count = 1;
                metrics.record_lock_contention(start_wait.elapsed());
                return;
            }

            // Drop the lock before checking safe point
            drop(state);

            // Check for GC safe point
            if thread_manager.is_gc_stop_requested() {
                thread_manager.safe_point(thread_id, gc_coordinator);
            }
        }
    }

    fn enter_with_timeout_safe(
        &self,
        thread_id: ArenaId,
        timeout_ms: u64,
        metrics: &RuntimeMetrics,
        thread_manager: &impl ThreadManagerOps,
        gc_coordinator: &GCCoordinator,
    ) -> bool {
        use std::time::{Duration, Instant};

        if timeout_ms == 0 {
            return self.try_enter(thread_id);
        }

        let deadline = Instant::now() + Duration::from_millis(timeout_ms);

        loop {
            let now = Instant::now();
            if now >= deadline {
                return false;
            }

            let mut state = self.state.lock();

            if state.owner_thread_id == thread_id {
                state.recursion_count += 1;
                return true;
            }

            if state.owner_thread_id == ArenaId::INVALID {
                state.owner_thread_id = thread_id;
                state.recursion_count = 1;
                return true;
            }

            // Calculate remaining time, with a maximum wait of 10ms
            let remaining = deadline - now;
            let wait_duration = remaining.min(Duration::from_millis(10));

            let start_wait = Instant::now();
            let _ = self.condvar.wait_for(&mut state, wait_duration);

            if state.owner_thread_id == ArenaId::INVALID {
                state.owner_thread_id = thread_id;
                state.recursion_count = 1;
                metrics.record_lock_contention(start_wait.elapsed());
                return true;
            }

            // Drop the lock before checking safe point
            drop(state);

            // Check for GC safe point
            if thread_manager.is_gc_stop_requested() {
                thread_manager.safe_point(thread_id, gc_coordinator);
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

impl SyncManagerOps for SyncBlockManager {
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
        block.try_enter(thread_id)
    }
}

impl Default for SyncBlockManager {
    fn default() -> Self {
        Self::new()
    }
}

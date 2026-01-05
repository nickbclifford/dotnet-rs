use crate::value::object::ObjectRef;
use crate::vm::metrics::RuntimeMetrics;
use crate::vm::sync::{SyncBlockOps, SyncManagerOps};
use parking_lot::{Condvar, Mutex};
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Debug)]
struct SyncBlockState {
    /// Thread ID of the current lock owner (0 means unlocked)
    owner_thread_id: u64,
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
                owner_thread_id: 0,
                recursion_count: 0,
            }),
            condvar: Condvar::new(),
        }
    }
}

impl SyncBlockOps for SyncBlock {
    fn try_enter(&self, thread_id: u64) -> bool {
        let mut state = self.state.lock();
        if state.owner_thread_id == 0 {
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

    fn enter(&self, thread_id: u64, metrics: &RuntimeMetrics) {
        use std::time::Instant;
        let mut state = self.state.lock();

        if state.owner_thread_id == thread_id {
            state.recursion_count += 1;
            return;
        }

        if state.owner_thread_id != 0 {
            let start_wait = Instant::now();
            while state.owner_thread_id != 0 {
                self.condvar.wait(&mut state);
            }
            metrics.record_lock_contention(start_wait.elapsed());
        }

        state.owner_thread_id = thread_id;
        state.recursion_count = 1;
    }

    fn enter_with_timeout(
        &self,
        thread_id: u64,
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

        if state.owner_thread_id != 0 {
            let start_wait = Instant::now();
            while state.owner_thread_id != 0 {
                let now = Instant::now();
                if now >= deadline {
                    return false;
                }

                let remaining = deadline - now;
                let timed_out = self.condvar.wait_for(&mut state, remaining).timed_out();

                if timed_out && state.owner_thread_id != 0 {
                    return false;
                }
            }
            metrics.record_lock_contention(start_wait.elapsed());
        }

        state.owner_thread_id = thread_id;
        state.recursion_count = 1;
        true
    }

    fn exit(&self, thread_id: u64) -> bool {
        let mut state = self.state.lock();

        if state.owner_thread_id != thread_id || thread_id == 0 {
            return false;
        }

        if state.recursion_count == 0 {
            return false;
        }

        state.recursion_count -= 1;

        if state.recursion_count == 0 {
            state.owner_thread_id = 0;
            self.condvar.notify_one();
        }

        true
    }

    fn wait(&self, _thread_id: u64, _timeout_ms: Option<u64>) -> Result<(), &'static str> {
        Err("Monitor.Wait() is not yet implemented")
    }

    fn pulse(&self, _thread_id: u64) -> Result<(), &'static str> {
        Err("Monitor.Pulse() is not yet implemented")
    }

    fn pulse_all(&self, _thread_id: u64) -> Result<(), &'static str> {
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
        _object: &ObjectRef<'_>,
        get_index: impl FnOnce() -> Option<usize>,
        set_index: impl FnOnce(usize),
    ) -> (usize, Arc<SyncBlock>) {
        let mut blocks = self.blocks.lock();

        if let Some(index) = get_index() {
            if let Some(block) = blocks.get(&index) {
                return (index, block.clone());
            }
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

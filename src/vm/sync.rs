use crate::value::object::ObjectRef;
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
/// Each sync block contains a mutex and condition variable for thread synchronization.
///
/// # Implementation Details
///
/// This implements .NET's `Monitor` class, which provides recursive locking semantics.
/// The same thread can acquire the lock multiple times, and must release it the same
/// number of times before another thread can acquire it.
///
/// # Thread Safety
///
/// The SyncBlock uses `parking_lot::Mutex` and `Condvar` for efficient synchronization.
/// These primitives are fair and prevent priority inversion.
///
/// # Future Extensions
///
/// The `Condvar` will be used to implement `Monitor.Wait()` and `Monitor.Pulse()` methods.
#[derive(Debug)]
pub struct SyncBlock {
    state: Mutex<SyncBlockState>,
    condvar: Condvar,
}

impl SyncBlock {
    fn new() -> Self {
        Self {
            state: Mutex::new(SyncBlockState {
                owner_thread_id: 0,
                recursion_count: 0,
            }),
            condvar: Condvar::new(),
        }
    }

    /// Try to enter the monitor (non-blocking).
    /// Returns true if lock was acquired, false otherwise.
    pub fn try_enter(&self, thread_id: u64) -> bool {
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

    /// Enter the monitor (blocking).
    pub fn enter(&self, thread_id: u64, metrics: &crate::vm::metrics::RuntimeMetrics) {
        use std::time::Instant;
        let mut state = self.state.lock();

        // Recursive case
        if state.owner_thread_id == thread_id {
            state.recursion_count += 1;
            return;
        }

        // Block until we can acquire the lock
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

    /// Try to enter the monitor with a timeout.
    /// Returns true if the lock was acquired, false if timeout expired.
    ///
    /// # Arguments
    /// * `thread_id` - The ID of the calling thread
    /// * `timeout_ms` - Timeout in milliseconds. If zero, behaves like try_enter.
    /// * `metrics` - Runtime metrics to record contention
    ///
    /// # Returns
    /// * `true` - Lock was successfully acquired
    /// * `false` - Timeout expired without acquiring the lock
    pub fn enter_with_timeout(
        &self,
        thread_id: u64,
        timeout_ms: u64,
        metrics: &crate::vm::metrics::RuntimeMetrics,
    ) -> bool {
        use std::time::{Duration, Instant};

        // Zero timeout = try_enter behavior
        if timeout_ms == 0 {
            return self.try_enter(thread_id);
        }

        let mut state = self.state.lock();

        // Recursive case
        if state.owner_thread_id == thread_id {
            state.recursion_count += 1;
            return true;
        }

        let deadline = Instant::now() + Duration::from_millis(timeout_ms);

        // Block until we can acquire the lock or timeout
        if state.owner_thread_id != 0 {
            let start_wait = Instant::now();
            while state.owner_thread_id != 0 {
                let now = Instant::now();
                if now >= deadline {
                    // Timeout expired
                    return false;
                }

                let remaining = deadline - now;
                let timed_out = self.condvar.wait_for(&mut state, remaining).timed_out();

                if timed_out && state.owner_thread_id != 0 {
                    // Timeout expired and lock still not available
                    return false;
                }
            }
            metrics.record_lock_contention(start_wait.elapsed());
        }

        // Lock is now available
        state.owner_thread_id = thread_id;
        state.recursion_count = 1;
        true
    }

    /// Exit the monitor.
    /// Returns true if successfully exited, false if not owned by this thread.
    pub fn exit(&self, thread_id: u64) -> bool {
        let mut state = self.state.lock();

        // Check if we own the lock
        if state.owner_thread_id != thread_id || thread_id == 0 {
            return false;
        }

        if state.recursion_count == 0 {
            return false; // Should never happen
        }

        state.recursion_count -= 1;

        if state.recursion_count == 0 {
            // Release the lock
            state.owner_thread_id = 0;
            self.condvar.notify_one();
        }

        true
    }

    /// Wait for a signal on this monitor's condition variable.
    /// TODO: Implement Monitor.Wait() - Currently returns NotImplemented error
    ///
    /// # Arguments
    /// * `thread_id` - The ID of the calling thread (must own the lock)
    /// * `timeout_ms` - Optional timeout in milliseconds
    ///
    /// # Returns
    /// Result indicating success or error (NotImplemented for now)
    pub fn wait(&self, _thread_id: u64, _timeout_ms: Option<u64>) -> Result<(), &'static str> {
        Err("Monitor.Wait() is not yet implemented")
    }

    /// Signal one waiting thread on this monitor's condition variable.
    /// TODO: Implement Monitor.Pulse()
    ///
    /// # Arguments
    /// * `thread_id` - The ID of the calling thread (must own the lock)
    ///
    /// # Returns
    /// Result indicating success or error (NotImplemented for now)
    pub fn pulse(&self, _thread_id: u64) -> Result<(), &'static str> {
        Err("Monitor.Pulse() is not yet implemented")
    }

    /// Signal all waiting threads on this monitor's condition variable.
    /// TODO: Implement Monitor.PulseAll()
    ///
    /// # Arguments
    /// * `thread_id` - The ID of the calling thread (must own the lock)
    ///
    /// # Returns
    /// Result indicating success or error (NotImplemented for now)
    pub fn pulse_all(&self, _thread_id: u64) -> Result<(), &'static str> {
        Err("Monitor.PulseAll() is not yet implemented")
    }
}

/// Manages synchronization blocks for .NET objects.
/// This provides the infrastructure for System.Threading.Monitor operations.
pub struct SyncBlockManager {
    /// Map from sync block index to sync block
    blocks: Mutex<HashMap<usize, Arc<SyncBlock>>>,
    /// Counter for allocating new sync block indices
    next_index: Mutex<usize>,
}

impl SyncBlockManager {
    pub fn new() -> Self {
        Self {
            blocks: Mutex::new(HashMap::new()),
            next_index: Mutex::new(1), // Start at 1, 0 means no sync block
        }
    }

    /// Get or create a sync block for an object.
    /// Returns the sync block index and a reference to the sync block.
    pub fn get_or_create_sync_block(
        &self,
        _object: &ObjectRef<'_>,
        get_index: impl FnOnce() -> Option<usize>,
        set_index: impl FnOnce(usize),
    ) -> (usize, Arc<SyncBlock>) {
        // Check if object already has a sync block
        if let Some(index) = get_index() {
            let blocks = self.blocks.lock();
            if let Some(block) = blocks.get(&index) {
                return (index, block.clone());
            }
        }

        // Allocate new sync block
        let index = {
            let mut next = self.next_index.lock();
            let idx = *next;
            *next += 1;
            idx
        };

        let block = Arc::new(SyncBlock::new());

        {
            let mut blocks = self.blocks.lock();
            blocks.insert(index, block.clone());
        }

        set_index(index);
        (index, block)
    }

    /// Get an existing sync block by index.
    pub fn get_sync_block(&self, index: usize) -> Option<Arc<SyncBlock>> {
        self.blocks.lock().get(&index).cloned()
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
    use std::thread;
    use std::time::Duration;

    #[test]
    fn test_sync_block_recursion() {
        let block = SyncBlock::new();
        let tid = 1;
        let metrics = crate::vm::metrics::RuntimeMetrics::new();

        assert!(block.try_enter(tid));
        block.enter(tid, &metrics); // Recursive
        assert_eq!(block.state.lock().recursion_count, 2);

        assert!(block.exit(tid));
        assert_eq!(block.state.lock().recursion_count, 1);
        assert!(block.exit(tid));
        assert_eq!(block.state.lock().recursion_count, 0);
        assert_eq!(block.state.lock().owner_thread_id, 0);
    }

    #[test]
    fn test_sync_block_contention() {
        let block = Arc::new(SyncBlock::new());
        let block_clone = block.clone();
        let metrics = Arc::new(crate::vm::metrics::RuntimeMetrics::new());
        let metrics_clone = metrics.clone();

        block.enter(1, &metrics);

        let handle = thread::spawn(move || {
            // This should block until the other thread exits
            block_clone.enter(2, &metrics_clone);
            let recursion = block_clone.state.lock().recursion_count;
            block_clone.exit(2);
            recursion
        });

        thread::sleep(Duration::from_millis(50));
        block.exit(1);

        let recursion = handle.join().unwrap();
        assert_eq!(recursion, 1);
    }

    #[test]
    fn test_sync_block_try_enter() {
        let block = SyncBlock::new();
        assert!(block.try_enter(1));
        assert!(!block.try_enter(2));
        block.exit(1);
        assert!(block.try_enter(2));
    }

    #[test]
    fn test_stress_heavy_contention() {
        // Stress test with many threads competing for the same lock
        let block = Arc::new(SyncBlock::new());
        let mut handles = vec![];
        let counter = Arc::new(Mutex::new(0u64));
        let metrics = Arc::new(crate::vm::metrics::RuntimeMetrics::new());

        for tid in 1..=10 {
            let block_clone = block.clone();
            let counter_clone = counter.clone();
            let metrics_clone = metrics.clone();
            let handle = thread::spawn(move || {
                for _ in 0..100 {
                    block_clone.enter(tid, &metrics_clone);
                    // Critical section
                    let mut val = counter_clone.lock();
                    *val += 1;
                    drop(val);
                    thread::sleep(Duration::from_micros(10));
                    block_clone.exit(tid);
                }
            });
            handles.push(handle);
        }

        for handle in handles {
            handle.join().unwrap();
        }

        assert_eq!(*counter.lock(), 1000);
    }

    #[test]
    fn test_wait_not_implemented() {
        let block = SyncBlock::new();
        let metrics = crate::vm::metrics::RuntimeMetrics::new();
        block.enter(1, &metrics);
        let result = block.wait(1, None);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), "Monitor.Wait() is not yet implemented");
        block.exit(1);
    }

    #[test]
    fn test_pulse_not_implemented() {
        let block = SyncBlock::new();
        let metrics = crate::vm::metrics::RuntimeMetrics::new();
        block.enter(1, &metrics);
        let result = block.pulse(1);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), "Monitor.Pulse() is not yet implemented");
        block.exit(1);
    }

    #[test]
    fn test_pulse_all_not_implemented() {
        let block = SyncBlock::new();
        let metrics = crate::vm::metrics::RuntimeMetrics::new();
        block.enter(1, &metrics);
        let result = block.pulse_all(1);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), "Monitor.PulseAll() is not yet implemented");
        block.exit(1);
    }

    #[test]
    fn test_enter_with_timeout_immediate() {
        let block = SyncBlock::new();
        let metrics = crate::vm::metrics::RuntimeMetrics::new();

        // Lock is free, should succeed immediately
        assert!(block.enter_with_timeout(1, 1000, &metrics));
        assert_eq!(block.state.lock().owner_thread_id, 1);
        block.exit(1);
    }

    #[test]
    fn test_enter_with_timeout_zero() {
        let block = SyncBlock::new();
        let metrics = crate::vm::metrics::RuntimeMetrics::new();
        block.enter(1, &metrics);

        // Zero timeout behaves like try_enter
        assert!(!block.enter_with_timeout(2, 0, &metrics));

        block.exit(1);
        assert!(block.enter_with_timeout(2, 0, &metrics));
        block.exit(2);
    }

    #[test]
    fn test_enter_with_timeout_recursive() {
        let block = SyncBlock::new();
        let metrics = crate::vm::metrics::RuntimeMetrics::new();
        block.enter(1, &metrics);

        // Recursive lock should succeed immediately
        assert!(block.enter_with_timeout(1, 100, &metrics));
        assert_eq!(block.state.lock().recursion_count, 2);

        block.exit(1);
        block.exit(1);
    }

    #[test]
    fn test_enter_with_timeout_expires() {
        let block = Arc::new(SyncBlock::new());
        let metrics = Arc::new(crate::vm::metrics::RuntimeMetrics::new());
        let metrics_clone = metrics.clone();
        block.enter(1, &metrics);

        let block_clone = block.clone();
        let handle = thread::spawn(move || {
            // Try to enter with 100ms timeout, should fail
            let start = std::time::Instant::now();
            let result = block_clone.enter_with_timeout(2, 100, &metrics_clone);
            let elapsed = start.elapsed();
            (result, elapsed)
        });

        // Wait longer than timeout to ensure it expires
        thread::sleep(Duration::from_millis(150));
        block.exit(1);

        let (result, elapsed) = handle.join().unwrap();
        assert!(!result); // Should have timed out
        assert!(elapsed.as_millis() >= 90 && elapsed.as_millis() <= 150);
    }

    #[test]
    fn test_enter_with_timeout_succeeds() {
        let block = Arc::new(SyncBlock::new());
        let metrics = Arc::new(crate::vm::metrics::RuntimeMetrics::new());
        let metrics_clone = metrics.clone();
        block.enter(1, &metrics);

        let block_clone = block.clone();
        let handle = thread::spawn(move || {
            // Try to enter with 1000ms timeout, should succeed
            let start = std::time::Instant::now();
            let result = block_clone.enter_with_timeout(2, 1000, &metrics_clone);
            let elapsed = start.elapsed();
            (result, elapsed)
        });

        // Release lock after 50ms
        thread::sleep(Duration::from_millis(50));
        block.exit(1);

        let (result, elapsed) = handle.join().unwrap();
        assert!(result); // Should have succeeded
        assert!(elapsed.as_millis() < 1000); // Should have taken less than timeout

        // Clean up
        let block_clone2 = Arc::try_unwrap(block).unwrap();
        block_clone2.enter(2, &metrics);
        block_clone2.exit(2);
    }
}

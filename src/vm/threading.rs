use parking_lot::{Condvar, Mutex, MutexGuard};
use std::{
    cell::Cell,
    collections::HashMap,
    sync::{
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
        Arc,
    },
    thread::ThreadId,
};

thread_local! {
    /// Cached managed thread ID for the current thread
    static MANAGED_THREAD_ID: Cell<Option<u64>> = const { Cell::new(None) };
    /// Flag indicating if this thread is currently performing GC
    static IS_PERFORMING_GC: Cell<bool> = const { Cell::new(false) };
}

/// Represents the state of a .NET thread.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThreadState {
    /// Thread is currently running
    Running,
    /// Thread is at a safe point and can be suspended for GC
    AtSafePoint,
    /// Thread is suspended (stopped for GC)
    Suspended,
    /// Thread has exited
    Exited,
}

/// Information about a managed .NET thread.
#[derive(Debug)]
struct ManagedThread {
    /// Native OS thread ID
    native_id: ThreadId,
    /// .NET managed thread ID (stable u64 representation)
    managed_id: u64,
    /// Current state of the thread
    state: AtomicU64, // Encoded ThreadState
}

impl ManagedThread {
    fn new(native_id: ThreadId, managed_id: u64) -> Self {
        Self {
            native_id,
            managed_id,
            state: AtomicU64::new(ThreadState::Running as u64),
        }
    }

    #[allow(dead_code)]
    fn get_state(&self) -> ThreadState {
        match self.state.load(Ordering::Acquire) {
            0 => ThreadState::Running,
            1 => ThreadState::AtSafePoint,
            2 => ThreadState::Suspended,
            3 => ThreadState::Exited,
            _ => ThreadState::Running, // Fallback
        }
    }

    fn set_state(&self, state: ThreadState) {
        self.state.store(state as u64, Ordering::Release);
    }
}

/// Manages the lifecycle of .NET threads and coordinates stop-the-world GC pauses.
///
/// This manager provides:
/// - Thread registration and tracking
/// - Safe point coordination for GC
/// - Stop-the-world GC suspension/resumption
/// - Thread ID management
///
/// # Thread Safety
///
/// The ThreadManager is designed to be shared via `Arc` across threads. All internal state
/// uses atomic operations or `parking_lot` synchronization primitives for thread safety.
///
/// # GC Coordination
///
/// The stop-the-world protocol works as follows:
/// 1. GC coordinator calls `request_stop_the_world()`, which sets `gc_stop_requested` flag
/// 2. All running threads check this flag at their next safe point
/// 3. Threads arriving at safe points call `safe_point()`, which increments the counter and blocks
/// 4. Once all threads are at safe points, the coordinator wakes up and performs GC
/// 5. When GC finishes, the `StopTheWorldGuard` is dropped, clearing the flag and waking all threads
///
/// # Safety Invariants
///
/// - Threads must call `safe_point()` regularly to avoid stalling GC
/// - Threads must be registered before calling `safe_point()`
/// - Threads must be unregistered before exiting to avoid deadlocking future GC cycles
pub struct ThreadManager {
    /// Map from managed thread ID to thread info
    threads: Mutex<HashMap<u64, Arc<ManagedThread>>>,
    /// Counter for allocating managed thread IDs
    next_thread_id: AtomicU64,
    /// Whether a GC stop-the-world is currently requested
    gc_stop_requested: AtomicBool,
    /// Number of threads that have reached safe point during GC
    threads_at_safepoint: AtomicUsize,
    /// Condvar for notifying when all threads reach safe point
    all_threads_stopped: Condvar,
    /// Mutex for GC coordination
    gc_coordination: Mutex<()>,
}

impl ThreadManager {
    pub fn new() -> Self {
        Self {
            threads: Mutex::new(HashMap::new()),
            next_thread_id: AtomicU64::new(1), // Thread ID 0 is reserved
            gc_stop_requested: AtomicBool::new(false),
            threads_at_safepoint: AtomicUsize::new(0),
            all_threads_stopped: Condvar::new(),
            gc_coordination: Mutex::new(()),
        }
    }

    /// Register a new thread with the thread manager.
    /// Returns the managed thread ID assigned to this thread.
    pub fn register_thread(&self) -> u64 {
        let native_id = std::thread::current().id();
        let managed_id = self.next_thread_id.fetch_add(1, Ordering::SeqCst);

        let thread_info = Arc::new(ManagedThread::new(native_id, managed_id));

        {
            let mut threads = self.threads.lock();
            threads.insert(managed_id, thread_info);
        }

        // Cache the managed ID in thread-local storage
        MANAGED_THREAD_ID.set(Some(managed_id));

        managed_id
    }

    /// Unregister a thread (called when thread exits).
    pub fn unregister_thread(&self, managed_id: u64) {
        let mut threads = self.threads.lock();
        if let Some(thread) = threads.get(&managed_id) {
            thread.set_state(ThreadState::Exited);
        }
        threads.remove(&managed_id);

        // Clear thread-local cache
        MANAGED_THREAD_ID.set(None);

        // If we were at a safe point, decrement the counter
        if self.threads_at_safepoint.load(Ordering::Acquire) > 0 {
            // We don't decrement here because the thread is gone,
            // but we MUST notify the coordinator because thread_count decreased.
            self.all_threads_stopped.notify_all();
        }
    }

    /// Get the current thread's managed ID.
    /// Returns None if the thread is not registered.
    pub fn current_thread_id(&self) -> Option<u64> {
        // Try thread-local cache first
        let cached_id = MANAGED_THREAD_ID.get();
        if cached_id.is_some() {
            return cached_id;
        }

        // Fallback to linear search
        let native_id = std::thread::current().id();
        let threads = self.threads.lock();
        let managed_id = threads
            .values()
            .find(|t| t.native_id == native_id)
            .map(|t| t.managed_id);

        // Update cache if found
        if managed_id.is_some() {
            MANAGED_THREAD_ID.set(managed_id);
        }

        managed_id
    }

    /// Get the number of currently active threads.
    pub fn thread_count(&self) -> usize {
        self.threads.lock().len()
    }

    /// Check if a GC stop-the-world is currently requested.
    /// Threads should call `safe_point()` when they detect this.
    #[inline]
    pub fn is_gc_stop_requested(&self) -> bool {
        self.gc_stop_requested.load(Ordering::Acquire)
    }

    /// Mark that the current thread has reached a safe point.
    /// If a GC is requested, this will block until the GC completes.
    pub fn safe_point(&self, managed_id: u64) {
        // Fast path: no GC requested
        if !self.is_gc_stop_requested() {
            return;
        }

        // Critical fix: If this thread is currently performing GC, do not try to reach
        // a safe point as it would deadlock trying to re-acquire gc_coordination lock
        let is_gc_thread = IS_PERFORMING_GC.get();
        if is_gc_thread {
            return;
        }

        // Get thread info
        let thread_info = {
            let threads = self.threads.lock();
            threads.get(&managed_id).cloned()
        };

        if let Some(thread) = thread_info {
            // Mark ourselves at safe point
            thread.set_state(ThreadState::AtSafePoint);

            // CRITICAL: Increment safe point counter BEFORE acquiring gc_coordination lock.
            // This prevents a deadlock where:
            // 1. GC coordinator holds gc_coordination lock and waits for threads_at_safepoint to reach target
            // 2. Worker threads try to acquire gc_coordination lock but are blocked
            // 3. Deadlock: coordinator waits for counter, workers wait for lock
            // By incrementing before the lock, the coordinator can see us immediately.
            self.threads_at_safepoint.fetch_add(1, Ordering::AcqRel);

            // Notify coordinator that we've reached a safe point
            self.all_threads_stopped.notify_all();

            // Wait for GC to complete.
            // We lock the coordination mutex which the coordinator holds during GC.
            {
                let mut guard = self.gc_coordination.lock();

                // Wait while GC is in progress
                while self.gc_stop_requested.load(Ordering::Acquire) {
                    self.all_threads_stopped.wait(&mut guard);
                }
            }

            // Decrement safe point counter after we're done waiting
            self.threads_at_safepoint.fetch_sub(1, Ordering::Release);

            // Mark ourselves running again
            thread.set_state(ThreadState::Running);
        }
    }

    /// Request a stop-the-world pause and wait for all threads to reach safe points.
    /// Returns a GC guard that will resume threads when dropped.
    ///
    /// # Timeout Detection
    /// If threads take too long to reach safe points, warnings are logged with thread IDs.
    pub fn request_stop_the_world(&self) -> StopTheWorldGuard<'_> {
        let mut guard = self.gc_coordination.lock();
        let start_time = std::time::Instant::now();
        const WARN_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(1);
        let mut warned = false;

        // Set the stop flag
        self.gc_stop_requested.store(true, Ordering::Release);

        // Wait for all threads to reach safe points
        loop {
            let thread_count = self.thread_count();
            let current_id = self.current_thread_id();
            let target_stopped = if current_id.is_some() {
                thread_count.saturating_sub(1)
            } else {
                thread_count
            };

            if self.threads_at_safepoint.load(Ordering::Acquire) >= target_stopped {
                break;
            }

            // Check for timeout and warn with thread context
            if !warned && start_time.elapsed() > WARN_TIMEOUT {
                eprintln!("[GC WARNING] Stop-the-world pause taking longer than expected:");
                eprintln!("  Total threads: {}", thread_count);
                eprintln!("  Threads at safe point: {}", self.threads_at_safepoint.load(Ordering::Acquire));

                // List threads that haven't reached safe point
                let threads = self.threads.lock();
                eprintln!("  Threads not at safe point:");
                for (tid, thread) in threads.iter() {
                    if thread.get_state() != ThreadState::AtSafePoint {
                        eprintln!("    - Thread ID {}: {:?} (native: {:?})", tid, thread.get_state(), thread.native_id);
                    }
                }
                drop(threads);
                warned = true;
            }

            // Wait for a thread to reach safe point or unregister
            self.all_threads_stopped.wait(&mut guard);
        }

        if warned {
            eprintln!("[GC] Stop-the-world completed after {} ms", start_time.elapsed().as_millis());
        }

        StopTheWorldGuard::new(self, guard)
    }

    /// Resume all threads after a stop-the-world pause.
    /// This is called automatically when the StopTheWorldGuard is dropped.
    fn resume_threads(&self) {
        self.gc_stop_requested.store(false, Ordering::Release);
        self.all_threads_stopped.notify_all();
    }
}

impl Default for ThreadManager {
    fn default() -> Self {
        Self::new()
    }
}

/// RAII guard for stop-the-world GC pauses.
/// When dropped, automatically resumes all threads.
pub struct StopTheWorldGuard<'a> {
    manager: &'a ThreadManager,
    _lock: MutexGuard<'a, ()>,
}

impl<'a> StopTheWorldGuard<'a> {
    /// Create a new StopTheWorldGuard and mark this thread as performing GC
    fn new(manager: &'a ThreadManager, lock: MutexGuard<'a, ()>) -> Self {
        IS_PERFORMING_GC.set(true);
        Self {
            manager,
            _lock: lock,
        }
    }
}

impl<'a> Drop for StopTheWorldGuard<'a> {
    fn drop(&mut self) {
        self.manager.resume_threads();
        IS_PERFORMING_GC.set(false);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::thread;
    use std::time::Duration;

    #[test]
    fn test_thread_registration() {
        let manager = ThreadManager::new();
        let id = manager.register_thread();
        assert_eq!(id, 1);
        assert_eq!(manager.thread_count(), 1);

        manager.unregister_thread(id);
        assert_eq!(manager.thread_count(), 0);
    }

    #[test]
    fn test_safe_point_no_gc() {
        let manager = ThreadManager::new();
        let id = manager.register_thread();

        // Safe point should return immediately when no GC requested
        manager.safe_point(id);

        manager.unregister_thread(id);
    }

    #[test]
    fn test_stop_the_world() {
        let manager = Arc::new(ThreadManager::new());
        let manager_clone = manager.clone();

        // Spawn a worker thread
        let handle = thread::spawn(move || {
            let id = manager_clone.register_thread();

            // Simulate work with safe points
            for _ in 0..10 {
                thread::sleep(Duration::from_millis(10));
                manager_clone.safe_point(id);
            }

            manager_clone.unregister_thread(id);
        });

        // Give the thread time to start
        thread::sleep(Duration::from_millis(20));

        // Request stop-the-world
        {
            let _guard = manager.request_stop_the_world();
            // All threads should be stopped here
            // Simulate GC work
            thread::sleep(Duration::from_millis(50));
        }
        // Threads resume when guard is dropped

        handle.join().unwrap();
    }

    #[test]
    fn test_late_arriving_thread_at_safepoint() {
        // Test for the race condition where a thread arrives at a safe point
        // after GC coordinator has started waiting but before it completes
        let manager = Arc::new(ThreadManager::new());

        // Register two threads
        let id1 = manager.register_thread();
        let id2 = manager.register_thread();

        let manager_clone1 = manager.clone();
        let manager_clone2 = manager.clone();

        // Thread 1: Quickly reach safe point
        let handle1 = thread::spawn(move || {
            thread::sleep(Duration::from_millis(5));
            manager_clone1.safe_point(id1);
            manager_clone1.unregister_thread(id1);
        });

        // Thread 2: Arrive late to safe point
        let handle2 = thread::spawn(move || {
            thread::sleep(Duration::from_millis(50)); // Delay arrival
            manager_clone2.safe_point(id2);
            manager_clone2.unregister_thread(id2);
        });

        // Start GC coordination thread
        let manager_gc = manager.clone();
        let handle_gc = thread::spawn(move || {
            thread::sleep(Duration::from_millis(10));
            let _guard = manager_gc.request_stop_the_world();
            thread::sleep(Duration::from_millis(100)); // Hold GC lock
        });

        // All threads should complete without deadlock
        handle1.join().unwrap();
        handle2.join().unwrap();
        handle_gc.join().unwrap();

        assert_eq!(manager.thread_count(), 0);
    }

    #[test]
    fn test_stress_many_threads() {
        // Stress test with many threads registering/unregistering concurrently
        let manager = Arc::new(ThreadManager::new());
        let mut handles = vec![];

        for i in 0..20 {
            let manager_clone = manager.clone();
            let handle = thread::spawn(move || {
                let id = manager_clone.register_thread();
                // Simulate work
                for _ in 0..10 {
                    thread::sleep(Duration::from_micros(100 * (i % 5) as u64));
                    manager_clone.safe_point(id);
                }
                manager_clone.unregister_thread(id);
            });
            handles.push(handle);
        }

        // Periodically trigger GC while threads are running
        let manager_gc = manager.clone();
        let gc_handle = thread::spawn(move || {
            for _ in 0..5 {
                thread::sleep(Duration::from_millis(10));
                let _guard = manager_gc.request_stop_the_world();
                thread::sleep(Duration::from_millis(5));
            }
        });

        for handle in handles {
            handle.join().unwrap();
        }
        gc_handle.join().unwrap();

        assert_eq!(manager.thread_count(), 0);
    }

    #[test]
    fn test_stress_thread_creation_during_gc() {
        // Test thread creation/destruction while GC is active
        let manager = Arc::new(ThreadManager::new());
        let manager_worker = manager.clone();
        let manager_gc = manager.clone();

        // Start GC pause in background
        let gc_handle = thread::spawn(move || {
            thread::sleep(Duration::from_millis(10));
            let _guard = manager_gc.request_stop_the_world();
            thread::sleep(Duration::from_millis(100));
        });

        // Create and destroy threads while GC is paused
        let worker_handle = thread::spawn(move || {
            for _ in 0..10 {
                let id = manager_worker.register_thread();
                thread::sleep(Duration::from_millis(5));
                manager_worker.safe_point(id);
                manager_worker.unregister_thread(id);
                thread::sleep(Duration::from_millis(5));
            }
        });

        worker_handle.join().unwrap();
        gc_handle.join().unwrap();

        assert_eq!(manager.thread_count(), 0);
    }
}

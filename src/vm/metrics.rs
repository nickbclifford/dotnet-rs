use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

#[derive(Debug, Default)]
pub struct RuntimeMetrics {
    /// Total time spent in GC stop-the-world pauses (in microseconds)
    pub gc_pause_total_us: AtomicU64,
    /// Number of full GC cycles performed
    pub gc_pause_count: AtomicU64,
    /// Number of times a thread had to block waiting for a lock
    pub lock_contention_count: AtomicU64,
    /// Total time spent waiting for locks (in microseconds)
    pub lock_contention_total_us: AtomicU64,
}

impl RuntimeMetrics {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn record_gc_pause(&self, duration: Duration) {
        self.gc_pause_total_us
            .fetch_add(duration.as_micros() as u64, Ordering::Relaxed);
        self.gc_pause_count.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_lock_contention(&self, duration: Duration) {
        self.lock_contention_count.fetch_add(1, Ordering::Relaxed);
        self.lock_contention_total_us
            .fetch_add(duration.as_micros() as u64, Ordering::Relaxed);
    }
}

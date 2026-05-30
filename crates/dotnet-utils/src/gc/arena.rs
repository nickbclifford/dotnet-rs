use crate::{
    gc::GCCommand,
    sync::{Arc, AtomicBool, AtomicU64, AtomicUsize, Condvar, OrderedMutex, Ordering, levels},
};
use std::sync::{LazyLock, OnceLock};

/// Metadata about each thread's arena and its communication channel.
#[derive(Debug, Clone)]
pub struct ArenaHandle {
    inner: Arc<ArenaHandleInner>,
}

#[derive(Debug)]
pub struct ArenaHandleInner {
    pub thread_id: crate::ArenaId,
    pub allocation_counter: AtomicUsize,
    /// Number of times this arena crossed its allocation threshold and requested collection.
    pub collection_trigger_count: AtomicU64,
    /// Lifetime high-water mark for `allocation_counter` before per-cycle resets.
    pub peak_allocation_counter: AtomicU64,
    #[cfg(feature = "memory-validation")]
    pub allocation_call_count: AtomicUsize,
    pub gc_allocated_bytes: AtomicUsize,
    pub external_allocated_bytes: AtomicUsize,
    pub needs_collection: AtomicBool,
    pub needs_any_collection: OnceLock<Arc<AtomicBool>>,
    /// Command currently being processed by this thread.
    pub current_command: OrderedMutex<levels::ArenaCurrentCommand, Option<GCCommand>>,
    /// Signal to wake up the thread when a command is available.
    pub command_signal: Condvar,
    /// Signal to the coordinator that the command is finished.
    pub finish_signal: Condvar,
}

impl ArenaHandleInner {
    /// Record an allocation and check if we've exceeded the threshold.
    ///
    /// Returns `true` when this call flipped `needs_collection` from `false` to `true`.
    pub fn record_allocation(&self, size: usize) -> bool {
        #[cfg(feature = "memory-validation")]
        self.allocation_call_count.fetch_add(1, Ordering::Relaxed);

        let allocation_after = self.allocation_counter.fetch_add(size, Ordering::Relaxed) + size;
        update_atomic_max(&self.peak_allocation_counter, allocation_after as u64);

        if allocation_after > *ALLOCATION_THRESHOLD
            && self
                .needs_collection
                .compare_exchange(false, true, Ordering::Release, Ordering::Relaxed)
                .is_ok()
        {
            self.collection_trigger_count
                .fetch_add(1, Ordering::Relaxed);
            if let Some(needs_any_collection) = self.needs_any_collection.get() {
                needs_any_collection.store(true, Ordering::Release);
            }
            return true;
        }

        false
    }
}

impl ArenaHandle {
    pub fn new(thread_id: crate::ArenaId) -> Self {
        Self {
            inner: Arc::new(ArenaHandleInner {
                thread_id,
                allocation_counter: AtomicUsize::new(0),
                collection_trigger_count: AtomicU64::new(0),
                peak_allocation_counter: AtomicU64::new(0),
                #[cfg(feature = "memory-validation")]
                allocation_call_count: AtomicUsize::new(0),
                gc_allocated_bytes: AtomicUsize::new(0),
                external_allocated_bytes: AtomicUsize::new(0),
                needs_collection: AtomicBool::new(false),
                needs_any_collection: OnceLock::new(),
                current_command: OrderedMutex::new(None),
                command_signal: Condvar::new(),
                finish_signal: Condvar::new(),
            }),
        }
    }

    pub fn as_inner(&self) -> &ArenaHandleInner {
        &self.inner
    }

    pub fn thread_id(&self) -> crate::ArenaId {
        self.inner.thread_id
    }

    pub fn allocation_counter(&self) -> &AtomicUsize {
        &self.inner.allocation_counter
    }

    pub fn gc_allocated_bytes(&self) -> &AtomicUsize {
        &self.inner.gc_allocated_bytes
    }

    pub fn collection_trigger_count(&self) -> &AtomicU64 {
        &self.inner.collection_trigger_count
    }

    pub fn peak_allocation_counter(&self) -> &AtomicU64 {
        &self.inner.peak_allocation_counter
    }

    pub fn external_allocated_bytes(&self) -> &AtomicUsize {
        &self.inner.external_allocated_bytes
    }

    pub fn needs_collection(&self) -> &AtomicBool {
        &self.inner.needs_collection
    }

    pub fn current_command(&self) -> &OrderedMutex<levels::ArenaCurrentCommand, Option<GCCommand>> {
        &self.inner.current_command
    }

    pub fn command_signal(&self) -> &Condvar {
        &self.inner.command_signal
    }

    pub fn finish_signal(&self) -> &Condvar {
        &self.inner.finish_signal
    }

    pub fn bind_needs_any_collection_flag(&self, flag: Arc<AtomicBool>) {
        if let Err(rejected) = self.inner.needs_any_collection.set(flag) {
            let existing = self
                .inner
                .needs_any_collection
                .get()
                .expect("needs_any_collection should be initialized after set failure");
            assert!(
                Arc::ptr_eq(existing, &rejected),
                "ArenaHandle {:?} already bound to a different needs_any_collection flag",
                self.thread_id(),
            );
        }
    }

    /// Record an allocation and check if we've exceeded the threshold.
    ///
    /// Returns `true` when this call flipped `needs_collection` from `false` to `true`.
    pub fn record_allocation(&self, size: usize) -> bool {
        self.inner.record_allocation(size)
    }
}

fn update_atomic_max(target: &AtomicU64, candidate: u64) {
    let mut current = target.load(Ordering::Relaxed);
    while candidate > current {
        match target.compare_exchange_weak(current, candidate, Ordering::Relaxed, Ordering::Relaxed)
        {
            Ok(_) => return,
            Err(observed) => current = observed,
        }
    }
}

/// Allocation threshold in bytes that triggers a GC request for a thread-local arena.
///
/// This is initialized from the `DOTNET_GC_THRESHOLD_BYTES` environment variable,
/// defaulting to 1MB (1,048,576 bytes).
///
/// ### Calibration Intent / Design Goal
///
/// The threshold represents the number of bytes allocated in an arena before a garbage collection is requested.
/// - **Trigger Condition**: It is a budget of *bytes allocated since the last collection*, rather than an estimate of
///   *bytes currently alive*. This is because `allocation_counter` is reset to 0 in `finish_collection_inner` after
///   each collection cycle completes.
/// - **Workload & Threading Dynamics**: In single-threaded execution, 1MB is generally sufficient for minor collection
///   frequency. However, in heavily multi-threaded workloads, each thread possesses its own arena handle. Since the
///   threshold is evaluated per thread/arena, a multi-threaded workload might experience either:
///   1. Frequent global GC pauses if many threads trigger independently at small thresholds, or
///   2. Uneven trigger rates because each thread sees only its fraction of total system allocations.
///
///   Allowing runtime calibration via `DOTNET_GC_THRESHOLD_BYTES` enables fine-tuning: setting a larger threshold (e.g. 4MB
///   or 8MB) reduces pause frequency in high-throughput workloads, while a smaller threshold keeps the memory footprint tight.
pub static ALLOCATION_THRESHOLD: LazyLock<usize> = LazyLock::new(|| {
    std::env::var("DOTNET_GC_THRESHOLD_BYTES")
        .ok()
        .and_then(|val| val.parse::<usize>().ok())
        .filter(|threshold| *threshold > 0)
        .unwrap_or(1024 * 1024)
});

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn record_allocation_tracks_peak_and_collection_trigger_count() {
        let handle = ArenaHandle::new(crate::ArenaId(1));

        handle.record_allocation(*ALLOCATION_THRESHOLD - 16);
        assert_eq!(
            handle.peak_allocation_counter().load(Ordering::Relaxed),
            (*ALLOCATION_THRESHOLD - 16) as u64
        );
        assert_eq!(
            handle.collection_trigger_count().load(Ordering::Relaxed),
            0,
            "threshold not crossed yet"
        );

        handle.record_allocation(32);
        assert_eq!(
            handle.collection_trigger_count().load(Ordering::Relaxed),
            1,
            "crossing threshold should increment trigger counter once"
        );
        assert!(
            handle.peak_allocation_counter().load(Ordering::Relaxed)
                >= (*ALLOCATION_THRESHOLD + 16) as u64
        );

        handle.record_allocation(64);
        assert_eq!(
            handle.collection_trigger_count().load(Ordering::Relaxed),
            1,
            "further allocations while needs_collection=true must not double-count"
        );
    }
}

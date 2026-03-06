use crate::{
    gc::GCCommand,
    sync::{Arc, AtomicBool, AtomicUsize, Condvar, Mutex, Ordering},
};

/// Metadata about each thread's arena and its communication channel.
#[derive(Debug, Clone)]
pub struct ArenaHandle {
    inner: Arc<ArenaHandleInner>,
}

#[derive(Debug)]
pub struct ArenaHandleInner {
    pub thread_id: crate::ArenaId,
    pub allocation_counter: AtomicUsize,
    #[cfg(feature = "memory-validation")]
    pub allocation_call_count: AtomicUsize,
    pub gc_allocated_bytes: AtomicUsize,
    pub external_allocated_bytes: AtomicUsize,
    pub needs_collection: AtomicBool,
    /// Command currently being processed by this thread.
    pub current_command: Mutex<Option<GCCommand>>,
    /// Signal to wake up the thread when a command is available.
    pub command_signal: Condvar,
    /// Signal to the coordinator that the command is finished.
    pub finish_signal: Condvar,
}

impl ArenaHandleInner {
    /// Record an allocation and check if we've exceeded the threshold.
    pub fn record_allocation(&self, size: usize) {
        #[cfg(feature = "memory-validation")]
        self.allocation_call_count.fetch_add(1, Ordering::Relaxed);

        let current = self.allocation_counter.fetch_add(size, Ordering::Relaxed);
        if current + size > ALLOCATION_THRESHOLD {
            self.needs_collection.store(true, Ordering::Release);
        }
    }
}

impl ArenaHandle {
    pub fn new(thread_id: crate::ArenaId) -> Self {
        Self {
            inner: Arc::new(ArenaHandleInner {
                thread_id,
                allocation_counter: AtomicUsize::new(0),
                #[cfg(feature = "memory-validation")]
                allocation_call_count: AtomicUsize::new(0),
                gc_allocated_bytes: AtomicUsize::new(0),
                external_allocated_bytes: AtomicUsize::new(0),
                needs_collection: AtomicBool::new(false),
                current_command: Mutex::new(None),
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

    pub fn external_allocated_bytes(&self) -> &AtomicUsize {
        &self.inner.external_allocated_bytes
    }

    pub fn needs_collection(&self) -> &AtomicBool {
        &self.inner.needs_collection
    }

    pub fn current_command(&self) -> &Mutex<Option<GCCommand>> {
        &self.inner.current_command
    }

    pub fn command_signal(&self) -> &Condvar {
        &self.inner.command_signal
    }

    pub fn finish_signal(&self) -> &Condvar {
        &self.inner.finish_signal
    }

    /// Record an allocation and check if we've exceeded the threshold.
    pub fn record_allocation(&self, size: usize) {
        self.inner.record_allocation(size);
    }
}

/// Allocation threshold in bytes that triggers a GC request for a thread-local arena.
pub const ALLOCATION_THRESHOLD: usize = 1024 * 1024; // 1MB per-thread trigger

use dotnet_metrics::RuntimeMetrics;
use dotnet_utils::ArenaId;
use std::{num::NonZeroUsize, sync::OnceLock, time::Duration};

pub const DEFAULT_SAFE_POINT_YIELD_MS: u64 = 10;

/// Returns the duration to wait before checking for a GC safe point during lock acquisition.
/// This value is configurable via the `DOTNET_SAFE_POINT_YIELD_MS` environment variable.
pub fn get_safe_point_yield_duration() -> Duration {
    static DURATION: OnceLock<Duration> = OnceLock::new();
    *DURATION.get_or_init(|| {
        let ms = std::env::var("DOTNET_SAFE_POINT_YIELD_MS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(DEFAULT_SAFE_POINT_YIELD_MS);
        Duration::from_millis(ms)
    })
}

#[cfg(not(feature = "multithreading"))]
mod single_threaded;
#[cfg(feature = "multithreading")]
mod threaded;

#[cfg(all(test, not(feature = "multithreading")))]
pub(crate) use single_threaded::MONITOR_TEST_CONTENDED;
#[cfg(not(feature = "multithreading"))]
pub use single_threaded::{SyncBlock, SyncBlockManager};

#[cfg(all(test, feature = "multithreading"))]
pub(crate) use threaded::WAIT_TEST_HOOK;
#[cfg(feature = "multithreading")]
pub use threaded::{SyncBlock, SyncBlockManager};

pub trait SyncBlockOps {
    fn try_enter(&self, thread_id: ArenaId) -> bool;
    fn enter(&self, thread_id: ArenaId, metrics: &RuntimeMetrics);
    fn enter_with_timeout(
        &self,
        thread_id: ArenaId,
        timeout_ms: u64,
        metrics: &RuntimeMetrics,
    ) -> bool;
    fn exit(&self, thread_id: ArenaId) -> bool;
    fn is_owned(&self, thread_id: ArenaId) -> bool;
    fn begin_wait(
        &self,
        thread_id: ArenaId,
        deadline: Option<std::time::Instant>,
    ) -> Option<(u64, usize)>;
    fn poll_wait(&self, id: u64, deadline: Option<std::time::Instant>) -> Option<bool>;
    fn cancel_wait(&self, id: u64);
    fn restore_wait(&self, thread_id: ArenaId, recursion: usize) -> bool;
    fn pulse(&self, thread_id: ArenaId, all: bool) -> bool;
}

/// Owns the queue entry until a pulse, timeout, or cancellation. No GC references.
pub(crate) struct MonitorWait {
    pub(crate) block: Arc<SyncBlock>,
    id: Option<u64>,
    recursion: usize,
    pub(crate) deadline: Option<std::time::Instant>,
    result: Option<bool>,
}

impl MonitorWait {
    pub(crate) fn begin(
        block: Arc<SyncBlock>,
        thread: ArenaId,
        deadline: Option<std::time::Instant>,
    ) -> Option<Self> {
        let (id, recursion) = block.begin_wait(thread, deadline)?;
        Some(Self {
            block,
            id: Some(id),
            recursion,
            deadline,
            result: None,
        })
    }
    pub(crate) fn poll(&mut self, thread: ArenaId) -> Option<bool> {
        if let Some(id) = self.id {
            self.result = self.block.poll_wait(id, self.deadline);
            self.result?;
            self.id = None;
        }
        if self.block.restore_wait(thread, self.recursion) {
            self.result
        } else {
            None
        }
    }
    #[cfg(all(test, feature = "multithreading"))]
    pub(crate) fn expire_for_test(&mut self) {
        use std::time::{Duration, Instant};

        self.deadline = Some(Instant::now() - Duration::from_secs(1));
    }
    pub(crate) fn sleep_deadline(&self) -> Option<std::time::Instant> {
        if self.id.is_some() {
            self.deadline
        } else {
            None
        }
    }
}
impl Drop for MonitorWait {
    fn drop(&mut self) {
        if let Some(id) = self.id.take() {
            self.block.cancel_wait(id);
        }
    }
}

pub trait SyncManagerOps {
    type Block: SyncBlockOps;

    /// Creates a sync block under a fresh index. Indices are never reused and blocks are never
    /// removed, so a published index always resolves through `get_sync_block`.
    ///
    /// Callers publish the index with `ObjectRef::get_or_assign_sync_block_index`, which assigns
    /// at most one index per object.
    fn create_sync_block(&self) -> (NonZeroUsize, Arc<Self::Block>);

    fn get_sync_block(&self, index: NonZeroUsize) -> Option<Arc<Self::Block>>;

    fn try_enter_block(
        &self,
        block: Arc<Self::Block>,
        thread_id: ArenaId,
        metrics: &RuntimeMetrics,
    ) -> bool;
}

// Re-export basic sync primitives from utils::sync.
// This allows existing code to continue using vm::sync::Arc, etc.
pub use dotnet_utils::sync::{
    AcquireAfter, Arc, AtomicBool, AtomicI8, AtomicI16, AtomicI32, AtomicI64, AtomicIsize,
    AtomicU8, AtomicU16, AtomicU32, AtomicU64, AtomicUsize, HeldLockLevel, LockLevel, OrderedMutex,
    OrderedMutexGuard, OrderedRwLock, OrderedRwLockReadGuard, OrderedRwLockWriteGuard, Ordering,
    Weak, get_current_thread_id, levels, unlocked,
};

#[cfg(feature = "multithreading")]
pub use dotnet_utils::sync::{
    Condvar, MappedRwLockReadGuard, MappedRwLockWriteGuard, Mutex, MutexGuard, RwLock,
    RwLockReadGuard, RwLockWriteGuard,
};

#[cfg(not(feature = "multithreading"))]
pub use dotnet_utils::sync::{
    Condvar, MappedRwLockReadGuard, MappedRwLockWriteGuard, Mutex, MutexGuard, RwLock,
    RwLockReadGuard, RwLockWriteGuard,
};

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn test_default_yield_duration() {
        // Since OnceLock is used, we can't easily test changing the env var
        // if it has already been initialized by another test or the runtime.
        // But we can check that it returns a reasonable value.
        let duration = get_safe_point_yield_duration();

        // If not set in the environment, it should be the default 10ms.
        // If it was set by the environment in this process, it will be that value.
        // In a clean test run, it should be 10ms.
        if std::env::var("DOTNET_SAFE_POINT_YIELD_MS").is_err() {
            assert_eq!(duration, Duration::from_millis(DEFAULT_SAFE_POINT_YIELD_MS));
        }
    }
}

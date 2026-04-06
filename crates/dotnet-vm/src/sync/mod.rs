use crate::{gc::coordinator::GCCoordinator, threading::ThreadManagerOps};
use dotnet_metrics::RuntimeMetrics;
use dotnet_utils::ArenaId;
use std::{sync::OnceLock, time::Duration};

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

#[cfg(not(feature = "multithreading"))]
pub use single_threaded::*;

#[cfg(feature = "multithreading")]
pub use threaded::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LockResult {
    Success,
    Timeout,
    Yield,
}

pub trait SyncBlockOps {
    fn try_enter(&self, thread_id: ArenaId) -> bool;
    fn enter(&self, thread_id: ArenaId, metrics: &RuntimeMetrics);
    fn enter_with_timeout(
        &self,
        thread_id: ArenaId,
        timeout_ms: u64,
        metrics: &RuntimeMetrics,
    ) -> bool;
    fn enter_safe(
        &self,
        thread_id: ArenaId,
        metrics: &RuntimeMetrics,
        thread_manager: &impl ThreadManagerOps,
        gc_coordinator: &GCCoordinator,
    ) -> LockResult;
    fn enter_with_timeout_safe(
        &self,
        thread_id: ArenaId,
        deadline: std::time::Instant,
        metrics: &RuntimeMetrics,
        thread_manager: &impl ThreadManagerOps,
        gc_coordinator: &GCCoordinator,
    ) -> LockResult;
    fn exit(&self, thread_id: ArenaId) -> bool;
    fn wait(&self, thread_id: ArenaId, timeout_ms: Option<u64>) -> Result<(), &'static str>;
    fn pulse(&self, thread_id: ArenaId) -> Result<(), &'static str>;
    fn pulse_all(&self, thread_id: ArenaId) -> Result<(), &'static str>;
}

pub trait SyncManagerOps {
    type Block: SyncBlockOps;

    fn get_or_create_sync_block(
        &self,
        get_index: impl FnOnce() -> Option<usize>,
        set_index: impl FnOnce(usize),
    ) -> (usize, Arc<Self::Block>);

    fn get_sync_block(&self, index: usize) -> Option<Arc<Self::Block>>;

    fn try_enter_block(
        &self,
        block: Arc<Self::Block>,
        thread_id: ArenaId,
        metrics: &RuntimeMetrics,
    ) -> bool;
}

#[doc(hidden)]
pub trait SyncBlockBackend {
    fn try_enter(&self, thread_id: ArenaId) -> bool;
    fn enter(&self, thread_id: ArenaId, metrics: &RuntimeMetrics);
    fn enter_with_timeout(
        &self,
        thread_id: ArenaId,
        timeout_ms: u64,
        metrics: &RuntimeMetrics,
    ) -> bool;
    fn enter_safe(
        &self,
        thread_id: ArenaId,
        metrics: &RuntimeMetrics,
        thread_manager: &impl ThreadManagerOps,
        gc_coordinator: &GCCoordinator,
    ) -> LockResult;
    fn enter_with_timeout_safe(
        &self,
        thread_id: ArenaId,
        deadline: std::time::Instant,
        metrics: &RuntimeMetrics,
        thread_manager: &impl ThreadManagerOps,
        gc_coordinator: &GCCoordinator,
    ) -> LockResult;
    fn exit(&self, thread_id: ArenaId) -> bool;
    fn wait(&self, thread_id: ArenaId, timeout_ms: Option<u64>) -> Result<(), &'static str>;
    fn pulse(&self, thread_id: ArenaId) -> Result<(), &'static str>;
    fn pulse_all(&self, thread_id: ArenaId) -> Result<(), &'static str>;
}

#[doc(hidden)]
pub trait SyncManagerBackend {
    type Block: SyncBlockBackend;

    fn get_or_create_sync_block(
        &self,
        get_index: impl FnOnce() -> Option<usize>,
        set_index: impl FnOnce(usize),
    ) -> (usize, Arc<Self::Block>);

    fn get_sync_block(&self, index: usize) -> Option<Arc<Self::Block>>;

    fn try_enter_block(
        &self,
        block: Arc<Self::Block>,
        thread_id: ArenaId,
        metrics: &RuntimeMetrics,
    ) -> bool;
}

impl<T: SyncBlockBackend + ?Sized> SyncBlockOps for T {
    fn try_enter(&self, thread_id: ArenaId) -> bool {
        SyncBlockBackend::try_enter(self, thread_id)
    }

    fn enter(&self, thread_id: ArenaId, metrics: &RuntimeMetrics) {
        SyncBlockBackend::enter(self, thread_id, metrics)
    }

    fn enter_with_timeout(
        &self,
        thread_id: ArenaId,
        timeout_ms: u64,
        metrics: &RuntimeMetrics,
    ) -> bool {
        SyncBlockBackend::enter_with_timeout(self, thread_id, timeout_ms, metrics)
    }

    fn enter_safe(
        &self,
        thread_id: ArenaId,
        metrics: &RuntimeMetrics,
        thread_manager: &impl ThreadManagerOps,
        gc_coordinator: &GCCoordinator,
    ) -> LockResult {
        SyncBlockBackend::enter_safe(self, thread_id, metrics, thread_manager, gc_coordinator)
    }

    fn enter_with_timeout_safe(
        &self,
        thread_id: ArenaId,
        deadline: std::time::Instant,
        metrics: &RuntimeMetrics,
        thread_manager: &impl ThreadManagerOps,
        gc_coordinator: &GCCoordinator,
    ) -> LockResult {
        SyncBlockBackend::enter_with_timeout_safe(
            self,
            thread_id,
            deadline,
            metrics,
            thread_manager,
            gc_coordinator,
        )
    }

    fn exit(&self, thread_id: ArenaId) -> bool {
        SyncBlockBackend::exit(self, thread_id)
    }

    fn wait(&self, thread_id: ArenaId, timeout_ms: Option<u64>) -> Result<(), &'static str> {
        SyncBlockBackend::wait(self, thread_id, timeout_ms)
    }

    fn pulse(&self, thread_id: ArenaId) -> Result<(), &'static str> {
        SyncBlockBackend::pulse(self, thread_id)
    }

    fn pulse_all(&self, thread_id: ArenaId) -> Result<(), &'static str> {
        SyncBlockBackend::pulse_all(self, thread_id)
    }
}

impl<T: SyncManagerBackend + ?Sized> SyncManagerOps for T {
    type Block = <Self as SyncManagerBackend>::Block;

    fn get_or_create_sync_block(
        &self,
        get_index: impl FnOnce() -> Option<usize>,
        set_index: impl FnOnce(usize),
    ) -> (usize, Arc<Self::Block>) {
        SyncManagerBackend::get_or_create_sync_block(self, get_index, set_index)
    }

    fn get_sync_block(&self, index: usize) -> Option<Arc<Self::Block>> {
        SyncManagerBackend::get_sync_block(self, index)
    }

    fn try_enter_block(
        &self,
        block: Arc<Self::Block>,
        thread_id: ArenaId,
        metrics: &RuntimeMetrics,
    ) -> bool {
        SyncManagerBackend::try_enter_block(self, block, thread_id, metrics)
    }
}

// Re-export basic sync primitives from utils::sync
// This allows existing code to continue using vm::sync::Arc, etc.
pub use dotnet_utils::sync::*;

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

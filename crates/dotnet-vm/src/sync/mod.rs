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

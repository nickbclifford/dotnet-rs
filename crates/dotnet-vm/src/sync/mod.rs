use crate::{gc::coordinator::GCCoordinator, metrics::RuntimeMetrics, threading::ThreadManagerOps};

#[cfg(not(feature = "multithreading"))]
mod single_threaded;
#[cfg(feature = "multithreading")]
mod threaded;

#[cfg(not(feature = "multithreading"))]
pub use single_threaded::*;

#[cfg(feature = "multithreading")]
pub use threaded::*;

pub trait SyncBlockOps {
    fn try_enter(&self, thread_id: dotnet_utils::ArenaId) -> bool;
    fn enter(&self, thread_id: dotnet_utils::ArenaId, metrics: &RuntimeMetrics);
    fn enter_with_timeout(&self, thread_id: dotnet_utils::ArenaId, timeout_ms: u64, metrics: &RuntimeMetrics)
    -> bool;
    fn enter_safe(
        &self,
        thread_id: dotnet_utils::ArenaId,
        metrics: &RuntimeMetrics,
        thread_manager: &impl ThreadManagerOps,
        gc_coordinator: &GCCoordinator,
    );
    fn enter_with_timeout_safe(
        &self,
        thread_id: dotnet_utils::ArenaId,
        timeout_ms: u64,
        metrics: &RuntimeMetrics,
        thread_manager: &impl ThreadManagerOps,
        gc_coordinator: &GCCoordinator,
    ) -> bool;
    fn exit(&self, thread_id: dotnet_utils::ArenaId) -> bool;
    fn wait(&self, thread_id: dotnet_utils::ArenaId, timeout_ms: Option<u64>) -> Result<(), &'static str>;
    fn pulse(&self, thread_id: dotnet_utils::ArenaId) -> Result<(), &'static str>;
    fn pulse_all(&self, thread_id: dotnet_utils::ArenaId) -> Result<(), &'static str>;
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
        thread_id: dotnet_utils::ArenaId,
        metrics: &RuntimeMetrics,
    ) -> bool;
}

// Re-export basic sync primitives from utils::sync
// This allows existing code to continue using vm::sync::Arc, etc.
pub use dotnet_utils::sync::*;

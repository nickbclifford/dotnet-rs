use crate::{
    ExecutorResult, SharedGlobalState,
    gc::coordinator::{GCCommand, GCCoordinator},
    threading::STWGuardOps,
};
use dotnet_tracer::Tracer;
use dotnet_utils::ArenaId;
use std::sync::Arc;

/// The single-threaded backend cannot host a fresh executor for managed ThreadStart.
pub(crate) const fn managed_thread_workers_supported() -> bool {
    false
}

/// Matches the multithreaded worker-entry seam without spawning in this backend.
pub(crate) fn spawn_thread_start(
    _shared: Arc<SharedGlobalState>,
    _method: dotnet_types::members::MethodDescription,
    _lookup: dotnet_types::generics::GenericLookup,
) -> Option<std::thread::JoinHandle<ExecutorResult>> {
    None
}

pub struct ThreadManager;

impl ThreadManager {
    pub fn new(_stw_in_progress: Arc<dotnet_utils::sync::AtomicBool>) -> Arc<Self> {
        Arc::new(Self)
    }

    pub fn set_coordinator(&self, _coordinator: std::sync::Weak<GCCoordinator>) {}
}

impl super::ThreadManagerBackend for ThreadManager {
    type Guard<'a> = StopTheWorldGuard;

    fn register_thread(&self) -> ArenaId {
        ArenaId::new(1)
    }

    fn register_thread_traced(&self, _tracer: &mut Tracer, _name: &str) -> ArenaId {
        ArenaId::new(1)
    }

    fn unregister_thread(&self, _managed_id: ArenaId) {}

    fn unregister_thread_traced(&self, _managed_id: ArenaId, _tracer: &mut Tracer) {}

    fn current_thread_id(&self) -> Option<ArenaId> {
        Some(ArenaId::new(1))
    }

    fn thread_count(&self) -> usize {
        1
    }

    fn is_gc_stop_requested(&self) -> bool {
        false
    }

    fn safe_point(&self, _managed_id: ArenaId, _coordinator: &GCCoordinator) {}

    fn execute_gc_command(&self, _command: GCCommand, _coordinator: &GCCoordinator) {}

    fn safe_point_traced(
        &self,
        _managed_id: ArenaId,
        _coordinator: &GCCoordinator,
        _tracer: &mut Tracer,
        _location: &str,
    ) {
    }

    fn request_stop_the_world(&self) -> Self::Guard<'_> {
        StopTheWorldGuard
    }

    fn request_stop_the_world_traced(&self, _tracer: &mut Tracer) -> Self::Guard<'_> {
        StopTheWorldGuard
    }
}

pub struct StopTheWorldGuard;

impl STWGuardOps for StopTheWorldGuard {
    fn elapsed_micros(&self) -> u64 {
        0
    }
}

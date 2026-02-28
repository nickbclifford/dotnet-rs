use crate::{
    gc::coordinator::{GCCommand, GCCoordinator},
    threading::{STWGuardOps, ThreadManagerOps},
    tracer::Tracer,
};
use dotnet_utils::ArenaId;
use std::sync::Arc;

pub struct ThreadManager;

impl ThreadManager {
    pub fn new(_stw_in_progress: Arc<dotnet_utils::sync::AtomicBool>) -> Arc<Self> {
        Arc::new(Self)
    }
}

impl ThreadManagerOps for ThreadManager {
    type Guard = StopTheWorldGuard;

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

    fn request_stop_the_world(&self) -> Self::Guard {
        StopTheWorldGuard
    }

    fn request_stop_the_world_traced(&self, _tracer: &mut Tracer) -> Self::Guard {
        StopTheWorldGuard
    }
}

pub struct StopTheWorldGuard;

impl STWGuardOps for StopTheWorldGuard {
    fn elapsed_micros(&self) -> u64 {
        0
    }
}

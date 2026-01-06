use crate::vm::{
    gc::{
        coordinator::{GCCommand, GCCoordinator},
        tracer::Tracer,
    },
    threading::{STWGuardOps, ThreadManagerOps},
};
use std::sync::Arc;

pub struct ThreadManager;

impl ThreadManager {
    pub fn new() -> Arc<Self> {
        Arc::new(Self)
    }
}

impl ThreadManagerOps for ThreadManager {
    type Guard = StopTheWorldGuard;

    fn register_thread(&self) -> u64 {
        1
    }

    fn register_thread_traced(&self, _tracer: &mut Tracer, _name: &str) -> u64 {
        1
    }

    fn unregister_thread(&self, _managed_id: u64) {}

    fn unregister_thread_traced(&self, _managed_id: u64, _tracer: &mut Tracer) {}

    fn current_thread_id(&self) -> Option<u64> {
        Some(1)
    }

    fn thread_count(&self) -> usize {
        1
    }

    fn is_gc_stop_requested(&self) -> bool {
        false
    }

    fn safe_point(&self, _managed_id: u64, _coordinator: &GCCoordinator) {}

    fn execute_gc_command(&self, _command: GCCommand, _coordinator: &GCCoordinator) {}

    fn safe_point_traced(
        &self,
        _managed_id: u64,
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

pub fn get_current_thread_id() -> u64 {
    1
}

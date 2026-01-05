use crate::vm::gc_coordinator::{GCCommand, GCCoordinator};
use crate::vm::tracer::Tracer;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThreadState {
    Running,
    AtSafePoint,
    Suspended,
    Exited,
}

pub struct ThreadManager;

impl ThreadManager {
    pub fn new() -> Self {
        Self
    }

    pub fn register_thread(&self) -> u64 {
        1
    }

    pub fn register_thread_traced(&self, _tracer: &Tracer, _name: &str) -> u64 {
        1
    }

    pub fn unregister_thread(&self, _managed_id: u64) {}

    pub fn unregister_thread_traced(&self, _managed_id: u64, _tracer: &Tracer) {}

    pub fn current_thread_id(&self) -> Option<u64> {
        Some(1)
    }

    pub fn thread_count(&self) -> usize {
        1
    }

    pub fn is_gc_stop_requested(&self) -> bool {
        false
    }

    pub fn safe_point(&self, _managed_id: u64, _coordinator: &GCCoordinator) {}

    pub fn execute_gc_command(&self, _command: GCCommand, _coordinator: &GCCoordinator) {}

    pub fn safe_point_traced(
        &self,
        _managed_id: u64,
        _coordinator: &GCCoordinator,
        _tracer: &Tracer,
        _location: &str,
    ) {}

    pub fn request_stop_the_world(&self) -> StopTheWorldGuard {
        StopTheWorldGuard
    }

    pub fn request_stop_the_world_traced(&self, _tracer: &Tracer) -> StopTheWorldGuard {
        StopTheWorldGuard
    }
}

pub struct StopTheWorldGuard;

impl StopTheWorldGuard {
    pub fn elapsed_micros(&self) -> u64 {
        0
    }
}

pub fn get_current_thread_id() -> u64 {
    1
}

impl Default for ThreadManager {
    fn default() -> Self {
        Self::new()
    }
}

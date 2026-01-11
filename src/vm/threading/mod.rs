use crate::vm::{
    gc::coordinator::{GCCommand, GCCoordinator},
    tracer::Tracer,
};

#[cfg(feature = "multithreading")]
mod basic;
pub mod lock;
#[cfg(not(feature = "multithreading"))]
mod stub;

#[cfg(feature = "multithreading")]
pub use basic::*;

#[cfg(not(feature = "multithreading"))]
pub use stub::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThreadState {
    Running,
    AtSafePoint,
    Suspended,
    Exited,
}

pub trait STWGuardOps {
    fn elapsed_micros(&self) -> u64;
}

pub trait ThreadManagerOps {
    type Guard: STWGuardOps;

    fn register_thread(&self) -> u64;
    fn register_thread_traced(&self, tracer: &mut Tracer, name: &str) -> u64;
    fn unregister_thread(&self, managed_id: u64);
    fn unregister_thread_traced(&self, managed_id: u64, tracer: &mut Tracer);
    fn current_thread_id(&self) -> Option<u64>;
    fn thread_count(&self) -> usize;
    fn is_gc_stop_requested(&self) -> bool;
    fn safe_point(&self, managed_id: u64, coordinator: &GCCoordinator);
    fn execute_gc_command(&self, command: GCCommand, coordinator: &GCCoordinator);
    fn safe_point_traced(
        &self,
        managed_id: u64,
        coordinator: &GCCoordinator,
        tracer: &mut Tracer,
        location: &str,
    );
    fn request_stop_the_world(&self) -> Self::Guard;
    fn request_stop_the_world_traced(&self, tracer: &mut Tracer) -> Self::Guard;
}

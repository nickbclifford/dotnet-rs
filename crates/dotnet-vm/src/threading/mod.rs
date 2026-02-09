//! Thread management and synchronization for the .NET virtual machine.
//!
//! This module provides thread coordination primitives for both single-threaded
//! and multi-threaded execution modes, with special support for garbage collection
//! safe points and stop-the-world pauses.
//!
//! # Architecture
//!
//! The threading system supports two modes via feature flags:
//!
//! - **`multithreading` enabled**: Uses [`basic`] module with real OS thread tracking,
//!   parking, and safe point coordination. Supports concurrent execution of managed
//!   threads with proper GC synchronization.
//!
//! - **`multithreading` disabled**: Uses [`stub`] module with minimal overhead for
//!   single-threaded execution. Safe point operations become no-ops.
//!
//! # Key Concepts
//!
//! ## Thread States
//!
//! Managed threads can be in one of four states ([`ThreadState`]):
//! - `Running`: Actively executing managed code
//! - `AtSafePoint`: Paused at a GC-safe location
//! - `Suspended`: Stopped by the GC coordinator for a stop-the-world pause
//! - `Exited`: Thread has terminated
//!
//! ## Safe Points
//!
//! Safe points are locations in the code where the GC can safely inspect and move
//! objects. Threads must check [`ThreadManagerOps::is_gc_stop_requested`] and call
//! [`ThreadManagerOps::safe_point`] at regular intervals (e.g., method entry, loop
//! backedges).
//!
//! ## Stop-The-World
//!
//! Garbage collection phases that require all threads to be paused use
//! [`ThreadManagerOps::request_stop_the_world`], which:
//! 1. Signals all threads to stop
//! 2. Waits for all threads to reach safe points
//! 3. Returns a guard that releases threads when dropped
//!
//! # Trait-Based Abstraction
//!
//! The [`ThreadManagerOps`] trait abstracts thread management operations, allowing
//! different implementations for single-threaded vs. multi-threaded modes. The
//! [`STWGuardOps`] trait provides timing information for stop-the-world pauses.
//!
//! # Example
//!
//! ```ignore
//! // Register a new managed thread
//! let thread_id = thread_manager.register_thread();
//!
//! // Check for GC safe point during execution
//! if thread_manager.is_gc_stop_requested() {
//!     thread_manager.safe_point(thread_id, &gc_coordinator);
//! }
//!
//! // Unregister when thread exits
//! thread_manager.unregister_thread(thread_id);
//! ```
use crate::{
    gc::coordinator::{GCCommand, GCCoordinator},
    tracer::Tracer,
};

#[cfg(feature = "multithreading")]
mod basic;
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

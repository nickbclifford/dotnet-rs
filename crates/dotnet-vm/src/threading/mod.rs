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
//! - **`multithreading` enabled**: Uses the multi-threaded implementation with real
//!   OS thread tracking, parking, and safe point coordination. Supports concurrent
//!   execution of managed threads with proper GC synchronization.
//!
//! - **`multithreading` disabled**: Uses the single-threaded implementation with
//!   minimal overhead for single-threaded execution. Safe point operations become
//!   no-ops.
//!
//! # Key Concepts
//!
//! ## Thread States
//!
//! Managed threads can be in one of three states ([`ThreadState`]):
//! - `Running`: Actively executing managed code
//! - `AtSafePoint`: Paused at a GC-safe location (e.g., for GC collection)
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

thread_local! {
    /// Flag indicating if this thread is currently performing GC
    pub(crate) static IS_PERFORMING_GC: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

#[cfg(feature = "multithreading")]
pub use basic::*;

#[cfg(not(feature = "multithreading"))]
pub use stub::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u64)]
pub enum ThreadState {
    Running = 0,
    AtSafePoint = 1,
    Exited = 2,
}

impl TryFrom<u64> for ThreadState {
    type Error = u64;

    fn try_from(value: u64) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(ThreadState::Running),
            1 => Ok(ThreadState::AtSafePoint),
            2 => Ok(ThreadState::Exited),
            _ => Err(value),
        }
    }
}

pub trait STWGuardOps {
    fn elapsed_micros(&self) -> u64;
}

#[doc(hidden)]
pub trait ThreadManagerBackend {
    type Guard<'a>: STWGuardOps
    where
        Self: 'a;

    fn register_thread(&self) -> dotnet_utils::ArenaId;
    fn register_thread_traced(&self, tracer: &mut Tracer, name: &str) -> dotnet_utils::ArenaId;
    fn unregister_thread(&self, managed_id: dotnet_utils::ArenaId);
    fn unregister_thread_traced(&self, managed_id: dotnet_utils::ArenaId, tracer: &mut Tracer);
    fn current_thread_id(&self) -> Option<dotnet_utils::ArenaId>;
    fn thread_count(&self) -> usize;
    fn is_gc_stop_requested(&self) -> bool;
    fn safe_point(&self, managed_id: dotnet_utils::ArenaId, coordinator: &GCCoordinator);
    fn execute_gc_command(&self, command: GCCommand, coordinator: &GCCoordinator);
    fn safe_point_traced(
        &self,
        managed_id: dotnet_utils::ArenaId,
        coordinator: &GCCoordinator,
        tracer: &mut Tracer,
        location: &str,
    );
    fn request_stop_the_world(&self) -> Self::Guard<'_>;
    fn request_stop_the_world_traced(&self, tracer: &mut Tracer) -> Self::Guard<'_>;
}

pub trait ThreadManagerOps {
    type Guard<'a>: STWGuardOps
    where
        Self: 'a;

    fn register_thread(&self) -> dotnet_utils::ArenaId;
    fn register_thread_traced(&self, tracer: &mut Tracer, name: &str) -> dotnet_utils::ArenaId;
    fn unregister_thread(&self, managed_id: dotnet_utils::ArenaId);
    fn unregister_thread_traced(&self, managed_id: dotnet_utils::ArenaId, tracer: &mut Tracer);
    fn current_thread_id(&self) -> Option<dotnet_utils::ArenaId>;
    fn thread_count(&self) -> usize;
    fn is_gc_stop_requested(&self) -> bool;
    fn safe_point(&self, managed_id: dotnet_utils::ArenaId, coordinator: &GCCoordinator);
    fn execute_gc_command(&self, command: GCCommand, coordinator: &GCCoordinator);
    fn safe_point_traced(
        &self,
        managed_id: dotnet_utils::ArenaId,
        coordinator: &GCCoordinator,
        tracer: &mut Tracer,
        location: &str,
    );
    fn request_stop_the_world(&self) -> Self::Guard<'_>;
    fn request_stop_the_world_traced(&self, tracer: &mut Tracer) -> Self::Guard<'_>;
}

impl<T: ThreadManagerBackend + ?Sized> ThreadManagerOps for T {
    type Guard<'a>
        = <Self as ThreadManagerBackend>::Guard<'a>
    where
        Self: 'a;

    fn register_thread(&self) -> dotnet_utils::ArenaId {
        ThreadManagerBackend::register_thread(self)
    }

    fn register_thread_traced(&self, tracer: &mut Tracer, name: &str) -> dotnet_utils::ArenaId {
        ThreadManagerBackend::register_thread_traced(self, tracer, name)
    }

    fn unregister_thread(&self, managed_id: dotnet_utils::ArenaId) {
        ThreadManagerBackend::unregister_thread(self, managed_id)
    }

    fn unregister_thread_traced(&self, managed_id: dotnet_utils::ArenaId, tracer: &mut Tracer) {
        ThreadManagerBackend::unregister_thread_traced(self, managed_id, tracer)
    }

    fn current_thread_id(&self) -> Option<dotnet_utils::ArenaId> {
        ThreadManagerBackend::current_thread_id(self)
    }

    fn thread_count(&self) -> usize {
        ThreadManagerBackend::thread_count(self)
    }

    fn is_gc_stop_requested(&self) -> bool {
        ThreadManagerBackend::is_gc_stop_requested(self)
    }

    fn safe_point(&self, managed_id: dotnet_utils::ArenaId, coordinator: &GCCoordinator) {
        ThreadManagerBackend::safe_point(self, managed_id, coordinator)
    }

    fn execute_gc_command(&self, command: GCCommand, coordinator: &GCCoordinator) {
        ThreadManagerBackend::execute_gc_command(self, command, coordinator)
    }

    fn safe_point_traced(
        &self,
        managed_id: dotnet_utils::ArenaId,
        coordinator: &GCCoordinator,
        tracer: &mut Tracer,
        location: &str,
    ) {
        ThreadManagerBackend::safe_point_traced(self, managed_id, coordinator, tracer, location)
    }

    fn request_stop_the_world(&self) -> Self::Guard<'_> {
        ThreadManagerBackend::request_stop_the_world(self)
    }

    fn request_stop_the_world_traced(&self, tracer: &mut Tracer) -> Self::Guard<'_> {
        ThreadManagerBackend::request_stop_the_world_traced(self, tracer)
    }
}

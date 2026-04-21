//! Garbage collection subsystem for the dotnet-rs VM.
//!
//! This module contains all GC-related functionality:
//! - Arena-based memory management
//! - Cross-thread GC coordination (when multithreading feature is enabled)
//! - Runtime execution tracing for GC events
use crate::stack::GCArenaRoot;
use gc_arena::arena::MarkedArena;

#[cfg(feature = "multithreading")]
pub mod arena;
pub mod coordinator;

#[cfg(feature = "multithreading")]
pub use arena::THREAD_ARENA;

#[cfg(feature = "multithreading")]
pub use coordinator::{
    ALLOCATION_THRESHOLD, ArenaHandle, CollectionSession, GCCommand, GCCoordinator, GcCycleGuard,
    MarkObjectPointers, MarkPhaseCommand, OpaqueObjectPtr, SweepPhaseCommand, clear_tracing_state,
    get_currently_tracing, set_currently_tracing,
};
#[cfg(not(feature = "multithreading"))]
pub use coordinator::{GCCommand, GCCoordinator};

/// Perform post-marking finalization check for a GC arena.
/// This handles object finalization by checking which objects are no longer reachable.
pub fn finalize_arena(marked: MarkedArena<'_, GCArenaRoot>) {
    marked.finalize(|fc, c| {
        c.stack
            .local
            .heap
            .finalize_check(fc, c.stack.shared.as_ref(), c.stack.indent())
    });
}

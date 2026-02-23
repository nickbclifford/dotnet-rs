//! Garbage collection subsystem for the dotnet-rs VM.
//!
//! This module contains all GC-related functionality:
//! - Arena-based memory management
//! - Cross-thread GC coordination (when multithreaded-gc feature is enabled)
//! - Runtime execution tracing for GC events
#[cfg(feature = "multithreaded-gc")]
pub mod arena;
pub mod coordinator;

#[cfg(feature = "multithreaded-gc")]
pub use arena::THREAD_ARENA;

pub use coordinator::*;

use gc_arena::arena::MarkedArena;

/// Perform post-marking finalization check for a GC arena.
/// This handles object finalization by checking which objects are no longer reachable.
pub fn finalize_arena(
    marked: MarkedArena<gc_arena::Rootable!['gc => crate::dispatch::ExecutionEngine<'gc, 'static>]>,
) {
    marked.finalize(|fc, c| {
        c.stack
            .local
            .heap
            .finalize_check(fc, &c.stack.shared, c.stack.indent())
    });
}

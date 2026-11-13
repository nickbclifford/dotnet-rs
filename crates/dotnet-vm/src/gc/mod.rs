//! Garbage collection subsystem for the dotnet-rs VM.
//!
//! This module contains all GC-related functionality:
//! - Arena-based memory management
//! - Cross-thread GC coordination (when multithreading feature is enabled)
//! - Runtime execution tracing for GC events
use crate::stack::GCArenaRoot;
use gc_arena::arena::MarkedArena;

#[cfg(feature = "multithreading")]
pub(crate) mod arena;
pub mod coordinator;

#[cfg(feature = "multithreading")]
pub use coordinator::{
    ALLOCATION_THRESHOLD, ArenaHandle, CollectionSession, GCCommand, GCCoordinator, GcCycleGuard,
    MarkObjectPointers, MarkPhaseCommand, OpaqueObjectPtr, SweepPhaseCommand, clear_tracing_state,
    get_currently_tracing, set_currently_tracing,
};

#[cfg(not(feature = "multithreading"))]
pub use coordinator::{GCCommand, GCCoordinator};

/// How much single-threaded collection work to do.
#[cfg(not(feature = "multithreading"))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CollectionWork {
    /// Pay the arena's allocation debt incrementally.
    Debt,
    /// Run a complete collection cycle, as `GC.Collect` requests.
    Full,
}

/// Clear this arena's short weak handles whose targets are unreachable. It only reads colors, so
/// every arena can run it concurrently; it must complete everywhere before any arena finalizes.
#[cfg(feature = "multithreading")]
pub fn clear_weak_arena(marked: MarkedArena<'_, GCArenaRoot>) {
    marked.finalize(|fc, c| {
        // SAFETY: ClearWeak runs on every key owner before sweeping.
        unsafe { c.stack.shared.gc_handles.clear_dead_dependents(fc) };
        // SAFETY: ClearWeak runs on each parked owner before any arena sweeps.
        unsafe {
            c.stack.shared.gc_handles.clear_dead(
                fc,
                dotnet_utils::gc::GCHandleType::Weak,
                &Default::default(),
            )
        };
    });
}

/// Perform post-marking finalization check for a GC arena.
/// This handles object finalization by checking which objects are no longer reachable.
pub fn finalize_arena(marked: MarkedArena<'_, GCArenaRoot>) {
    marked.finalize(|fc, c| {
        c.stack
            .local
            .heap
            .finalize_check(fc, c.stack.shared.as_ref(), c.stack.indent());
        let heap = &c.stack.local.heap;
        c.stack.shared.publish_finalizers(
            heap,
            heap.has_pending_finalizers() || c.stack.parked_finalizer_caller.is_some(),
        );
    });
}

/// Clear long weak handles and prune object registries after promotion has been marked.
pub fn finish_finalization_arena(marked: MarkedArena<'_, GCArenaRoot>) {
    marked.finalize(|fc, c| {
        c.stack
            .local
            .heap
            .finish_finalization(fc, c.stack.shared.as_ref());
    });
}

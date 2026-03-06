//! Garbage collection utility types.
use gc_arena::{Collect, Mutation};
use std::ops::Deref;

#[cfg(feature = "multithreading")]
use std::collections::HashSet;

#[cfg(feature = "multithreading")]
pub mod arena;
#[cfg(feature = "multithreading")]
pub mod cross_arena;
pub mod thread_safe_lock;

pub use thread_safe_lock::*;

#[cfg(feature = "multithreading")]
pub use arena::*;
#[cfg(feature = "multithreading")]
pub use cross_arena::*;

/// A handle to the GC mutation context.
#[derive(Copy, Clone)]
pub struct GCHandle<'gc> {
    pub(crate) mutation: &'gc Mutation<'gc>,
    #[cfg(feature = "multithreading")]
    pub(crate) arena: &'gc arena::ArenaHandleInner,
    #[cfg(feature = "memory-validation")]
    pub(crate) thread_id: crate::ArenaId,
}

impl<'gc> GCHandle<'gc> {
    pub fn new(
        mutation: &'gc Mutation<'gc>,
        #[cfg(feature = "multithreading")] arena: &'gc arena::ArenaHandleInner,
        #[cfg(feature = "memory-validation")] thread_id: crate::ArenaId,
    ) -> Self {
        #[cfg(all(feature = "memory-validation", feature = "multithreading"))]
        {
            if arena.thread_id != thread_id {
                tracing::error!(
                    "GCHandle initialized with mismatched thread/arena: thread_id={}, arena.thread_id={}",
                    thread_id,
                    arena.thread_id
                );
            }
        }
        Self {
            mutation,
            #[cfg(feature = "multithreading")]
            arena,
            #[cfg(feature = "memory-validation")]
            thread_id,
        }
    }

    /// Get the underlying mutation context.
    pub fn mutation(&self) -> &'gc Mutation<'gc> {
        #[cfg(feature = "memory-validation")]
        self.validate_thread();
        self.mutation
    }

    /// Record an allocation of the given size in the arena.
    pub fn record_allocation(&self, _size: usize) {
        #[cfg(feature = "memory-validation")]
        self.validate_thread();

        #[cfg(feature = "multithreading")]
        {
            self.arena.record_allocation(_size);
        }
    }

    #[cfg(feature = "memory-validation")]
    fn validate_thread(&self) {
        let current_thread = crate::sync::get_current_thread_id();
        if self.thread_id != current_thread {
            tracing::error!(
                "GCHandle used on wrong thread (expected {:?}, got {:?})",
                self.thread_id,
                current_thread
            );
            panic!(
                "GCHandle used on wrong thread (expected {:?}, got {:?})",
                self.thread_id, current_thread
            );
        }
        #[cfg(feature = "multithreading")]
        {
            if self.arena.thread_id != self.thread_id {
                tracing::error!(
                    "GCHandle arena mismatch: handle.thread_id={}, arena.thread_id={}",
                    self.thread_id,
                    self.arena.thread_id
                );
                panic!(
                    "GCHandle arena mismatch: handle.thread_id={}, arena.thread_id={}",
                    self.thread_id, self.arena.thread_id
                );
            }
        }
    }
}

impl<'gc> Deref for GCHandle<'gc> {
    type Target = Mutation<'gc>;

    fn deref(&self) -> &Self::Target {
        #[cfg(feature = "memory-validation")]
        self.validate_thread();
        self.mutation
    }
}

/// Type of GC handle, determines how the garbage collector treats the reference.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Collect)]
#[collect(require_static)]
pub enum GCHandleType {
    /// Weak reference that does not track resurrection
    Weak = 0,
    /// Weak reference that tracks resurrection (survives finalization)
    WeakTrackResurrection = 1,
    /// Normal (strong) GC handle
    Normal = 2,
    /// Pinned GC handle (prevents object from being moved by GC)
    Pinned = 3,
}

impl From<i32> for GCHandleType {
    fn from(i: i32) -> Self {
        match i {
            0 => GCHandleType::Weak,
            1 => GCHandleType::WeakTrackResurrection,
            2 => GCHandleType::Normal,
            3 => GCHandleType::Pinned,
            _ => panic!("invalid GCHandleType: {}", i),
        }
    }
}

#[cfg(feature = "multithreading")]
/// GC commands sent from the coordinator to worker threads.
#[derive(Debug, Clone)]
pub enum GCCommand {
    /// Start the marking phase (clear roots, mark local roots).
    MarkAll,
    /// Mark specific objects in the local arena (for cross-arena resurrection).
    /// Stores opaque pointers (usize) to avoid dependency on ObjectPtr.
    MarkObjects(HashSet<usize>),
    /// Run finalizers for dead objects.
    Finalize,
    /// Finish collection (sweep).
    Sweep,
}

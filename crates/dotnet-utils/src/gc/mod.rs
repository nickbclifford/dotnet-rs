//! Garbage collection utility types.
use gc_arena::{Collect, Mutation};
use std::{marker::PhantomData, ops::Deref};

#[cfg(feature = "multithreading")]
use std::collections::HashSet;

#[cfg(feature = "multithreading")]
pub mod arena;
#[cfg(feature = "multithreading")]
pub mod cross_arena;
pub mod thread_safe_lock;

pub use thread_safe_lock::{ThreadSafeLock, ThreadSafeReadGuard, ThreadSafeWriteGuard};

#[cfg(feature = "multithreading")]
pub use arena::{ALLOCATION_THRESHOLD, ArenaHandle, ArenaHandleInner};
#[cfg(feature = "multithreading")]
pub use cross_arena::{
    ArenaLease, ArenaState, clear_tracing_state, get_currently_tracing, is_stw_in_progress,
    is_valid_cross_arena_ref, record_cross_arena_ref, register_arena, reset_arena_registry,
    set_currently_tracing, set_stw_in_progress, take_found_cross_arena_refs_with_generation,
    try_acquire_lease, unregister_arena,
};

/// A handle to the GC mutation context.
#[derive(Copy, Clone)]
pub struct GCHandle<'gc> {
    pub(crate) mutation: &'gc Mutation<'gc>,
    #[cfg(feature = "multithreading")]
    pub(crate) arena: &'gc ArenaHandleInner,
    #[cfg(feature = "memory-validation")]
    pub(crate) thread_id: crate::ArenaId,
}

#[derive(Copy, Clone)]
pub struct GcLifetime<'gc> {
    _marker: PhantomData<&'gc mut Mutation<'gc>>,
}

impl<'gc> GCHandle<'gc> {
    pub fn new(
        mutation: &'gc Mutation<'gc>,
        #[cfg(feature = "multithreading")] arena: &'gc ArenaHandleInner,
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

    pub fn lifetime(self) -> GcLifetime<'gc> {
        GcLifetime {
            _marker: PhantomData,
        }
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct OpaqueObjectPtr(usize);

#[cfg(feature = "multithreading")]
impl OpaqueObjectPtr {
    pub fn from_raw(ptr: *const ()) -> Self {
        Self(ptr as usize)
    }

    pub fn as_ptr(self) -> *const () {
        self.0 as *const ()
    }
}

#[cfg(feature = "multithreading")]
#[derive(Debug, Clone, Default)]
pub struct MarkObjectPointers(HashSet<OpaqueObjectPtr>);

#[cfg(feature = "multithreading")]
impl MarkObjectPointers {
    pub fn new() -> Self {
        Self(HashSet::new())
    }

    pub fn with_capacity(capacity: usize) -> Self {
        Self(HashSet::with_capacity(capacity))
    }

    pub fn insert(&mut self, ptr: OpaqueObjectPtr) -> bool {
        self.0.insert(ptr)
    }
}

#[cfg(feature = "multithreading")]
impl FromIterator<OpaqueObjectPtr> for MarkObjectPointers {
    fn from_iter<T: IntoIterator<Item = OpaqueObjectPtr>>(iter: T) -> Self {
        Self(iter.into_iter().collect())
    }
}

#[cfg(feature = "multithreading")]
impl IntoIterator for MarkObjectPointers {
    type Item = OpaqueObjectPtr;
    type IntoIter = std::collections::hash_set::IntoIter<OpaqueObjectPtr>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}

#[cfg(feature = "multithreading")]
/// Mark-phase commands sent from the coordinator to worker threads.
#[derive(Debug, Clone)]
pub enum MarkPhaseCommand {
    /// Start the marking phase (clear roots, mark local roots).
    All,
    /// Mark specific objects in the local arena (for cross-arena resurrection).
    Objects(MarkObjectPointers),
}

#[cfg(feature = "multithreading")]
/// Reclaim-phase commands sent from the coordinator to worker threads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SweepPhaseCommand {
    /// Run finalizers for dead objects.
    Finalize,
    /// Finish collection (sweep).
    Sweep,
}

#[cfg(feature = "multithreading")]
/// GC commands sent from the coordinator to worker threads.
#[derive(Debug, Clone)]
pub enum GCCommand {
    Mark(MarkPhaseCommand),
    Sweep(SweepPhaseCommand),
}

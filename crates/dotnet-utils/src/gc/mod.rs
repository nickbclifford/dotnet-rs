//! Garbage collection utility types.
use gc_arena::{Collect, Mutation};
use std::{marker::PhantomData, ops::Deref};

#[cfg(feature = "multithreading")]
use std::{collections::HashSet, ptr::NonNull};

#[cfg(feature = "multithreading")]
pub mod arena;
#[cfg(feature = "multithreading")]
pub mod cross_arena;
pub mod thread_safe_lock;

#[cfg(feature = "multithreading")]
pub use thread_safe_lock::has_active_thread_safe_write_guard;
pub use thread_safe_lock::{ThreadSafeLock, ThreadSafeReadGuard, ThreadSafeWriteGuard};

#[cfg(feature = "multithreading")]
pub use arena::{ALLOCATION_THRESHOLD, ArenaHandle, ArenaHandleInner};
#[cfg(feature = "multithreading")]
pub use cross_arena::{
    ArenaIdentity, ArenaLease, ArenaRegistration, ArenaRegistrationError, ArenaState,
    RegistrationToken, TeardownOutcome, clear_tracing_state, current_thread_holds_lease,
    get_currently_tracing, is_stw_in_progress, is_valid_cross_arena_ref, record_cross_arena_object,
    record_cross_arena_ref, register_arena, registration_for_identity, reset_arena_registry,
    set_currently_tracing, set_stw_in_progress, take_found_cross_arena_refs, try_acquire_lease,
    try_acquire_lease_for_identity, try_acquire_lease_for_token, try_register_arena,
    unregister_arena, unregister_arena_identity,
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
    ///
    /// Returns `true` when this call flipped `needs_collection` from `false` to `true`.
    pub fn record_allocation(&self, _size: usize) -> bool {
        #[cfg(feature = "memory-validation")]
        self.validate_thread();

        #[cfg(feature = "multithreading")]
        {
            self.arena.record_allocation(_size)
        }

        #[cfg(not(feature = "multithreading"))]
        {
            false
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

/// Inert, type-erased object pointer; this is neither an owner nor a GC root.
/// Address equality and hashing do not establish liveness or arena identity.
#[cfg(feature = "multithreading")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct OpaqueObjectPtr(NonNull<()>);

#[cfg(feature = "multithreading")]
impl OpaqueObjectPtr {
    /// Erase an object allocation pointer without exposing its provenance.
    ///
    /// # Safety
    ///
    /// `ptr` must be non-null and identify a `Gc::as_ptr` allocation of the
    /// runtime's `ThreadSafeLock<ObjectInner>` with usable allocation provenance.
    /// No access is authorized by this wrapper: callers must separately
    /// establish liveness, rooting, owner identity and synchronization before use.
    pub unsafe fn from_raw(ptr: *const ()) -> Self {
        Self(NonNull::new(ptr.cast_mut()).expect("object pointer must not be null"))
    }

    /// Returns an inert raw pointer; dereferencing it requires its own proof.
    pub fn as_ptr(self) -> *const () {
        self.0.as_ptr()
    }

    /// Test-only: an address-only value that points at nothing.
    ///
    /// This deliberately does **not** satisfy [`Self::from_raw`]'s contract (it is not a
    /// `Gc::as_ptr` allocation), so it is confined to this crate's tests and must only be
    /// used for registry/buffer bookkeeping that compares or hashes the address. It must
    /// never be converted back to an object pointer, dereferenced, or sent to marking.
    #[cfg(test)]
    pub(crate) fn address_only_for_tests(addr: usize) -> Self {
        Self(NonNull::new(std::ptr::without_provenance_mut(addr)).expect("non-zero test address"))
    }
}

// SAFETY: F11.ObjectPointerThreadSafe — Construction is restricted to pointers
// to the runtime's ThreadSafeLock<ObjectInner>, whose Send + Sync bounds are
// asserted in dotnet-value. The erased pointer is inert and exposes no safe
// dereference; transport does not grant liveness, rooting or access permission.
#[cfg(feature = "multithreading")]
unsafe impl Send for OpaqueObjectPtr {}

// SAFETY: F11.ObjectPointerThreadSafe — The same concrete pointee and inert-only
// construction contract applies to sharing; all actual access needs independent
// synchronization and liveness proofs.
#[cfg(feature = "multithreading")]
unsafe impl Sync for OpaqueObjectPtr {}

/// Inert, generation-qualified object metadata. It does not root the object or
/// hold a lease; a fresh exact-identity lease and a GC liveness protocol are
/// required before any access. Never refresh an old pointer's identity by ID.
#[cfg(feature = "multithreading")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CrossArenaRef {
    identity: ArenaIdentity,
    ptr: OpaqueObjectPtr,
}

#[cfg(feature = "multithreading")]
impl CrossArenaRef {
    /// Pair a pointer with its original arena registration.
    ///
    /// # Safety
    ///
    /// `identity` must be captured while a live lease protects the pointer's
    /// original owner registration, or be the identity already captured for that
    /// same origin. A lease on a later registration of the same ID is insufficient;
    /// never refresh a stale pointer's identity from that registration.
    pub unsafe fn new(identity: ArenaIdentity, ptr: OpaqueObjectPtr) -> Self {
        Self { identity, ptr }
    }

    pub fn identity(self) -> ArenaIdentity {
        self.identity
    }

    pub fn ptr(self) -> OpaqueObjectPtr {
        self.ptr
    }
}

#[cfg(feature = "multithreading")]
#[derive(Debug, Clone, Default)]
pub struct MarkObjectPointers(HashSet<CrossArenaRef>);

#[cfg(feature = "multithreading")]
impl MarkObjectPointers {
    pub fn new() -> Self {
        Self(HashSet::new())
    }

    pub fn with_capacity(capacity: usize) -> Self {
        Self(HashSet::with_capacity(capacity))
    }

    pub fn insert(&mut self, reference: CrossArenaRef) -> bool {
        self.0.insert(reference)
    }
}

#[cfg(feature = "multithreading")]
impl FromIterator<CrossArenaRef> for MarkObjectPointers {
    fn from_iter<T: IntoIterator<Item = CrossArenaRef>>(iter: T) -> Self {
        Self(iter.into_iter().collect())
    }
}

#[cfg(feature = "multithreading")]
impl IntoIterator for MarkObjectPointers {
    type Item = CrossArenaRef;
    type IntoIter = std::collections::hash_set::IntoIter<CrossArenaRef>;

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

#[cfg(all(test, feature = "multithreading"))]
mod opaque_transport_recreation_tests {
    use super::{
        ArenaIdentity, CrossArenaRef, MarkObjectPointers, MarkPhaseCommand, OpaqueObjectPtr,
    };
    use crate::ArenaId;
    use static_assertions::assert_impl_all;
    use std::{collections::HashSet, ptr::NonNull};

    assert_impl_all!(OpaqueObjectPtr: Send, Sync, Copy);
    assert_impl_all!(CrossArenaRef: Send, Sync, Copy);

    /// Independent recreation (not recovered originals): isolate the command
    /// wrapper with a Box stand-in, not a GC object or a whole-VM test. No raw
    /// address is exposed; only inert metadata crosses the worker thread.
    #[test]
    fn m1e_recreation_opaque_transport() {
        let source = include_str!("mod.rs");
        let wrapper = source
            .split("pub struct OpaqueObjectPtr")
            .nth(1)
            .unwrap()
            .split("pub struct MarkObjectPointers")
            .next()
            .unwrap();
        assert!(
            wrapper.contains("NonNull<()>")
                && !wrapper.contains("expose_provenance")
                && !wrapper.contains("with_exposed_provenance"),
            "opaque command wrapper discards pointer provenance"
        );

        let raw = Box::into_raw(Box::new(0xA5u8));
        // This private test fixture bypasses the object-only constructor. The
        // stand-in is never interpreted as a GC object and remains live until
        // its original owner reclaims it after the worker has joined.
        let ptr = OpaqueObjectPtr(NonNull::new(raw.cast()).unwrap());
        let identity = ArenaIdentity {
            arena_id: ArenaId::new(123),
            generation: 7,
        };
        let reference = CrossArenaRef { identity, ptr };
        let different_generation = CrossArenaRef {
            identity: ArenaIdentity {
                generation: 8,
                ..identity
            },
            ptr,
        };
        let mut set = HashSet::new();
        assert!(set.insert(reference));
        assert!(!set.insert(reference));
        assert!(set.insert(different_generation));
        let mut pointers = MarkObjectPointers::new();
        assert!(pointers.insert(reference));
        assert!(pointers.insert(different_generation));
        assert!(!pointers.insert(reference));
        let command = MarkPhaseCommand::Objects(pointers);
        let (returned, command) = std::thread::spawn(move || (reference, command))
            .join()
            .unwrap();
        assert_eq!(returned.identity(), identity);
        let MarkPhaseCommand::Objects(pointers) = command else {
            panic!("expected object command");
        };
        let transported: HashSet<_> = pointers.into_iter().collect();
        assert_eq!(transported, HashSet::from([returned, different_generation]));
        // SAFETY: F10.RawAllocationOwnership — The Box remains allocated and
        // unaliased; the worker only moved inert metadata and has joined.
        unsafe { returned.ptr().as_ptr().cast_mut().cast::<u8>().write(0x5A) };
        // SAFETY: F10.RawAllocationOwnership — `raw` is the unique pointer
        // returned by Box::into_raw, and it is reclaimed exactly once.
        let value = unsafe { Box::from_raw(raw) };
        assert_eq!(*value, 0x5A);
    }
}

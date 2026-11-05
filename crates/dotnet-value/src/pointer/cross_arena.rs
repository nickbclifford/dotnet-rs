#[cfg(feature = "multithreading")]
use crate::{
    ByteOffset, ValidationTag,
    object::{ObjectInner, ObjectPtr},
    pointer::{CrossArenaOrigin, MANAGED_PTR_MAGIC, ManagedPtr, PointerOrigin},
};
#[cfg(feature = "multithreading")]
use dotnet_utils::gc::{
    ArenaLease, RegistrationToken, ThreadSafeLock, try_acquire_lease_for_identity,
    try_acquire_lease_for_token,
};
#[cfg(feature = "multithreading")]
use std::ptr::NonNull;

#[cfg(feature = "multithreading")]
impl ObjectPtr {
    pub(crate) fn try_with_data_range<T>(
        &self,
        offset: usize,
        size: usize,
        f: impl FnOnce(&[u8]) -> T,
    ) -> Option<T> {
        // SAFETY: F10.BorrowedStorageStable — the existing cross-arena origin protocol guarantees
        // that this `ObjectPtr` is live and aligned; this shared borrow stabilizes `ObjectInner`
        // but does not acquire an arena lease or create a root. The delegated HeapStorage read
        // holds the applicable object/FieldStorage read guard through `f`; a raw address returned
        // by `f` does not retain those guards or grant access beyond the checked read.
        let lock = unsafe { &*self.as_ptr() };
        let inner = lock.borrow();
        inner.storage.try_with_data_range(offset, size, f)
    }

    pub(crate) fn with_data_up_to<T>(
        &self,
        offset: usize,
        max_size: usize,
        f: impl FnOnce(&[u8]) -> T,
    ) -> Option<T> {
        // SAFETY: F10.BorrowedStorageStable — the existing cross-arena origin protocol guarantees
        // that this `ObjectPtr` is live and aligned; this shared borrow stabilizes `ObjectInner`
        // but does not acquire an arena lease or create a root. The delegated HeapStorage read
        // holds the applicable object/FieldStorage read guard through `f`; a raw address returned
        // by `f` does not retain those guards or grant access beyond the checked read.
        let lock = unsafe { &*self.as_ptr() };
        let inner = lock.borrow();
        inner.storage.with_data_up_to(offset, max_size, f)
    }
}

/// Reconstruct a serialized cross-arena lock pointer while its arena is pinned.
///
/// `token` is resolved to a lease for its exact original registration; a token
/// whose registration was removed never resolves (tokens are not reused), so
/// stale bytes are rejected before any pointer is formed. The callback is
/// deliberately scoped to that lease. Any dereference of the reconstructed
/// [`ObjectPtr`] must happen in `f`; a returned raw pointer needs its own
/// cross-arena coordination before it can be dereferenced.
///
/// # Safety
///
/// If `token` resolves, `lock_ptr` must carry provenance for a live, correctly
/// aligned `ThreadSafeLock<ObjectInner>` owned by that registration, as
/// previously serialized together with `token`. The caller must independently
/// keep the object rooted. This helper acquires a lease for `f`, preventing
/// arena teardown from completing, but the lease itself is not a root.
#[cfg(feature = "multithreading")]
pub(crate) unsafe fn cross_arena_ptr_from_ptr<T>(
    lock_ptr: *mut u8,
    token: RegistrationToken,
    f: impl FnOnce(ObjectPtr, &ArenaLease) -> T,
) -> Option<T> {
    let lease = try_acquire_lease_for_token(token)?;
    if lock_ptr.is_null() {
        return None;
    }

    // Mask the ManagedPtr tag while preserving the pointer's provenance.
    let lock_ptr = lock_ptr
        .map_addr(|addr| addr & !7)
        .cast::<ThreadSafeLock<ObjectInner<'static>>>();

    // SAFETY: F1.ArenaGenerationMatch — `token` resolved to its original registration, so this function's `# Safety` contract makes `lock_ptr` (tag masked, provenance kept) a live, aligned `ThreadSafeLock<ObjectInner>` of that registration; `lease` prevents arena teardown during `f`, and rooting stays with the caller.
    let ptr = unsafe { ObjectPtr::from_raw(lock_ptr) }?;
    Some(f(ptr, &lease))
}

#[cfg(feature = "multithreading")]
impl<'gc> ManagedPtr<'gc> {
    pub(crate) fn retained_stack_value_lease(
        value: &crate::StackValue<'gc>,
    ) -> Result<Option<dotnet_utils::gc::ArenaLease>, dotnet_types::error::ExecutionError> {
        let pointer = match value {
            crate::StackValue::ManagedPtr(pointer) => pointer,
            crate::StackValue::TypedRef(slot) => slot.value(),
            _ => return Ok(None),
        };
        match pointer.origin() {
            PointerOrigin::CrossArenaObjectRef(origin) => {
                try_acquire_lease_for_identity(origin.identity())
                    .map(Some)
                    .ok_or_else(|| {
                        dotnet_types::error::ExecutionError::InternalError(
                            "cross-arena owner is unavailable".into(),
                        )
                    })
            }
            _ => Ok(None),
        }
    }

    /// # Safety
    /// The object must be live and rooted and belong to the exact original
    /// registration pinned by `lease`. Its liveness and access discipline must
    /// hold during each subsequent use.
    pub unsafe fn new_cross_arena(
        value: Option<NonNull<u8>>,
        inner_type: dotnet_types::TypeDescription,
        ptr: ObjectPtr,
        lease: &ArenaLease,
        offset: ByteOffset,
    ) -> Self {
        // SAFETY: F1.ArenaGenerationMatch — this function's `# Safety` contract (a live, rooted object of the registration pinned by `lease`) is `CrossArenaOrigin::new`'s.
        let origin =
            PointerOrigin::CrossArenaObjectRef(unsafe { CrossArenaOrigin::new(ptr, lease) });
        Self {
            magic: ValidationTag::new(MANAGED_PTR_MAGIC as u64),
            _value: value,
            inner_type,
            offset: Self::pack_offset(&origin, value, offset),
            origin,
            flags: 0,
            _marker: std::marker::PhantomData,
        }
    }
}

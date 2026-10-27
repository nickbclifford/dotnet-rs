#[cfg(feature = "multithreading")]
use crate::{
    ArenaId, ByteOffset, ValidationTag,
    object::{ObjectInner, ObjectPtr},
    pointer::{MANAGED_PTR_MAGIC, ManagedPtr, PointerOrigin},
};
#[cfg(feature = "multithreading")]
use dotnet_utils::gc::{ThreadSafeLock, try_acquire_lease};
#[cfg(feature = "multithreading")]
use std::ptr::NonNull;

/// Reconstruct a serialized cross-arena lock pointer while its arena is pinned.
///
/// The callback is deliberately scoped to the arena lease. Any dereference of
/// the reconstructed [`ObjectPtr`] must happen in `f`; a returned raw pointer
/// needs its own cross-arena coordination before it can be dereferenced.
///
/// # Safety
///
/// `lock_ptr` must carry provenance for a live, correctly aligned
/// `ThreadSafeLock<ObjectInner>` owned by `owner_id`, as previously serialized
/// by the CrossArena representation. This helper acquires the `owner_id` arena
/// lease before reconstructing the pointer and holds it for `f`, so that arena
/// cannot be unregistered while `f` dereferences the pointer.
#[cfg(feature = "multithreading")]
pub(crate) unsafe fn cross_arena_ptr_from_ptr<T>(
    lock_ptr: *mut u8,
    owner_id: ArenaId,
    f: impl FnOnce(ObjectPtr) -> T,
) -> Option<T> {
    let _lease = try_acquire_lease(owner_id)?;
    if lock_ptr.is_null() {
        return None;
    }

    // Mask the ManagedPtr tag while preserving the pointer's provenance.
    let lock_ptr = lock_ptr
        .map_addr(|addr| addr & !7)
        .cast::<ThreadSafeLock<ObjectInner<'static>>>();

    // SAFETY: F4.WidthAligned — The caller guarantees that `lock_ptr` names a live, aligned
    // cross-arena lock owned by `owner_id`; `_lease` keeps that arena live for
    // the callback's duration.
    let ptr = unsafe { ObjectPtr::from_raw(lock_ptr) }?;
    Some(f(ptr))
}

#[cfg(feature = "multithreading")]
impl<'gc> ManagedPtr<'gc> {
    pub fn new_cross_arena(
        value: Option<NonNull<u8>>,
        inner_type: dotnet_types::TypeDescription,
        ptr: ObjectPtr,
        tid: ArenaId,
        offset: ByteOffset,
    ) -> Self {
        let origin = PointerOrigin::CrossArenaObjectRef(ptr, tid);
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

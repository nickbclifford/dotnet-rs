#[cfg(feature = "multithreading")]
use crate::{
    ArenaId, ByteOffset, ValidationTag,
    object::ObjectPtr,
    pointer::{ManagedPtr, PointerOrigin},
};
#[cfg(feature = "multithreading")]
use std::ptr::NonNull;

#[cfg(feature = "multithreading")]
#[cfg(any(feature = "memory-validation", debug_assertions))]
use crate::pointer::MANAGED_PTR_MAGIC;

#[cfg(feature = "multithreading")]
impl<'gc> ManagedPtr<'gc> {
    pub fn new_cross_arena(
        value: Option<NonNull<u8>>,
        inner_type: dotnet_types::TypeDescription,
        ptr: ObjectPtr,
        tid: ArenaId,
        offset: ByteOffset,
    ) -> Self {
        Self {
            magic: ValidationTag::new(MANAGED_PTR_MAGIC as u64),
            _value: value,
            inner_type,
            origin: PointerOrigin::CrossArenaObjectRef(ptr, tid),
            offset,
            pinned: false,
            _marker: std::marker::PhantomData,
        }
    }
}

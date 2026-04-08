#[cfg(feature = "multithreading")]
use crate::{
    ArenaId, ByteOffset, ValidationTag,
    object::ObjectPtr,
    pointer::{MANAGED_PTR_MAGIC, ManagedPtr, PointerOrigin},
};
#[cfg(feature = "multithreading")]
use std::ptr::NonNull;

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

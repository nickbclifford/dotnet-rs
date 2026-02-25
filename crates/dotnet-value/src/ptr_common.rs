use core::ptr::NonNull;

/// Common abstraction for types that can expose a raw data pointer.
/// Implemented for `ObjectRef` and `ManagedPtr` to consolidate common
/// pointer utility methods without coupling the two types.
pub trait PointerLike {
    /// Returns the underlying data pointer if available.
    fn pointer(&self) -> Option<NonNull<u8>>;

    /// Returns true if the pointer is null.
    #[inline]
    fn is_null(&self) -> bool {
        self.pointer().is_none()
    }

    /// Returns the exposed address of the pointer, if available.
    #[inline]
    fn addr(&self) -> Option<usize> {
        self.pointer().map(|p| p.as_ptr() as usize)
    }
}

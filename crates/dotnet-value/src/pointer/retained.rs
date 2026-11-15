use crate::{StackValue, stack_value::stack_value_kind};
use dotnet_types::error::ExecutionError;
use std::ops::Deref;

/// An observed address together with the stack value from which it was obtained.
///
/// Retaining the value retains any owner it contains and, for cross-arena
/// managed pointers, an arena lease. It does not retain a heap borrow or create
/// a GC root. Drop it before parking at a safepoint. Observing the address does
/// not grant access to it. An access must go through the address
/// [`checked_range`](Self::checked_range) returns, which resolves owned origins and checks the
/// range against the owner. It also requires the `RetainedPtr` to remain alive and all other
/// applicable provenance, lifetime, alignment, synchronization, and GC-rooting guarantees.
pub struct RetainedPtr<'gc> {
    ptr: *mut u8,
    _owner: StackValue<'gc>,
    #[cfg(feature = "multithreading")]
    _lease: Option<dotnet_utils::gc::ArenaLease>,
}

impl<'gc> RetainedPtr<'gc> {
    /// Retains a pointer-like stack value and observes its address.
    ///
    /// Accepts exactly the variants supported by [`StackValue::as_ptr`]; other
    /// variants return a type mismatch without observing an address. A torn-down
    /// cross-arena owner is rejected rather than returning an unprotected pointer.
    pub fn try_from_stack_value(value: StackValue<'gc>) -> Result<Self, ExecutionError> {
        match &value {
            StackValue::NativeInt(_)
            | StackValue::UnmanagedPtr(_)
            | StackValue::ManagedPtr(_)
            | StackValue::TypedRef(_) => {}
            other => {
                return Err(ExecutionError::TypeMismatch {
                    expected: "pointer",
                    actual: stack_value_kind(other).into(),
                });
            }
        }
        // Observation takes its own temporary lease. Acquire the retained lease
        // afterwards: if teardown began between them, reject without accessing
        // the observed pointer. This avoids a nested acquisition failing after
        // the retained lease was already obtained.
        let ptr = value.as_ptr();
        Ok(Self {
            ptr,
            #[cfg(feature = "multithreading")]
            _lease: super::ManagedPtr::retained_stack_value_lease(&value)?,
            _owner: value,
        })
    }

    /// Resolves and bounds-checks `size` bytes at this pointer before access, preserving retained
    /// ownership, and returns the address to access them through.
    ///
    /// Every owned origin is checked against its owner's extent: Heap and cross-arena objects
    /// against the actual payload length (the check `ManagedPtr::with_data` makes), Static storage
    /// and FrameHome blocks through `resolver`, and Transient owners against their data. Static
    /// pointers carry no cached address, so their address comes from the resolved block. Native
    /// integers, unmanaged pointers and the Unmanaged origin are returned unchecked; their
    /// validity is the guest's unsafe-memory contract.
    pub fn checked_range<'r, R: super::ManagedPtrResolver<'r> + ?Sized>(
        &self,
        resolver: &R,
        size: usize,
    ) -> Result<*mut u8, dotnet_types::error::MemoryAccessError> {
        use dotnet_types::error::MemoryAccessError;

        let pointer = match &self._owner {
            StackValue::ManagedPtr(pointer) => pointer.deref(),
            StackValue::TypedRef(slot) => slot.value().deref(),
            _ => return Ok(self.ptr),
        };
        if pointer.is_null() {
            return if size == 0 {
                Ok(self.ptr)
            } else {
                Err(MemoryAccessError::NullPointer(
                    "retained managed pointer range",
                ))
            };
        }
        let offset = pointer.byte_offset()?.as_usize();
        let out_of_bounds = |len: usize| MemoryAccessError::BoundsCheck { offset, size, len };
        match pointer.origin() {
            super::PointerOrigin::FrameHome(serial) => {
                super::frame_home_window(resolver, *serial, offset, size)
                    .map(std::ptr::NonNull::as_ptr)
            }
            super::PointerOrigin::Static(metadata) => {
                let (base, len) = resolver
                    .static_storage_block(metadata)
                    .ok_or(MemoryAccessError::InvalidOrigin)?;
                if offset.checked_add(size).is_none_or(|end| end > len) {
                    return Err(out_of_bounds(len));
                }
                // INVARIANT: F10.RawMemoryAccessValid — `offset + size <= len`, so the address stays
                // inside the static block (one past its end only when `size` is zero).
                Ok(base.as_ptr().wrapping_add(offset))
            }
            super::PointerOrigin::Heap(_) => self.checked_heap_range(pointer, offset, size),
            #[cfg(feature = "multithreading")]
            super::PointerOrigin::CrossArenaObjectRef(_) => {
                self.checked_heap_range(pointer, offset, size)
            }
            super::PointerOrigin::Transient(owner) => {
                let len = owner.with_data(<[u8]>::len);
                if offset.checked_add(size).is_none_or(|end| end > len) {
                    return Err(out_of_bounds(len));
                }
                Ok(self.ptr)
            }
            super::PointerOrigin::Unmanaged => Ok(self.ptr),
        }
    }

    /// Checks `size` bytes at `offset` against a heap owner's actual payload length. Access then
    /// goes through the retained allocation-base address, not the checking slice.
    fn checked_heap_range(
        &self,
        pointer: &super::ManagedPtr<'gc>,
        offset: usize,
        size: usize,
    ) -> Result<*mut u8, dotnet_types::error::MemoryAccessError> {
        // `allow_short` reports how many bytes remain after `offset`, so a short range can name
        // the payload length; `None` means the offset itself lies outside the payload.
        let available = pointer.try_with_heap_data(size, true, <[u8]>::len);
        match available {
            Some(available) if available == size => Ok(self.ptr),
            available => Err(dotnet_types::error::MemoryAccessError::BoundsCheck {
                offset,
                size,
                len: offset + available.unwrap_or(0),
            }),
        }
    }

    /// Observes the raw pointer; it does not itself authorize an access.
    ///
    /// A caller accessing it must keep `self` alive and prove the range and all
    /// other access requirements described on [`RetainedPtr`].
    pub fn as_ptr(&self) -> *mut u8 {
        self.ptr
    }

    /// Observes the address without exposing provenance for reconstruction.
    pub fn addr(&self) -> usize {
        self.ptr.addr()
    }

    /// Returns whether the observed pointer is null.
    pub fn is_null(&self) -> bool {
        self.ptr.is_null()
    }
}

impl std::fmt::Debug for RetainedPtr<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RetainedPtr")
            .field("ptr", &self.ptr)
            .field("owner", &self._owner)
            .finish_non_exhaustive()
    }
}

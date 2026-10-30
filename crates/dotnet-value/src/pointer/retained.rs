use crate::{StackValue, stack_value::stack_value_kind};
use dotnet_types::error::ExecutionError;

/// An observed address together with the stack value from which it was obtained.
///
/// Retaining the value retains any owner it contains and, for cross-arena
/// managed pointers, an arena lease. It does not retain a heap borrow or create
/// a GC root. Drop it before parking at a safepoint. Observing the address does
/// not grant access to it. An access through the pointer requires the
/// `RetainedPtr` to remain alive, a caller-provided range proof, and all other
/// applicable provenance, lifetime, alignment, synchronization, and GC-rooting
/// guarantees.
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

use dotnetdll::prelude::*;
use gc_arena::{Collect, unsafe_empty_collect};
use std::{
    fmt::{Debug, Formatter},
    hash::Hash,
    mem::size_of,
    ops::Deref,
    ptr::{self, NonNull},
};

#[repr(transparent)]
#[derive(Clone, Copy)]
pub struct ResolutionS(Option<NonNull<Resolution<'static>>>);
unsafe_empty_collect!(ResolutionS);
// SAFETY: ResolutionS is a transparent wrapper around a NonNull pointer to Resolution.
// Resolution data is managed by the VM and persists for the program lifetime.
// Thread-safety is guaranteed by the read-only nature of metadata once loaded.
unsafe impl Send for ResolutionS {}
unsafe impl Sync for ResolutionS {}
impl ResolutionS {
    pub const fn new(ptr: *const Resolution<'static>) -> Self {
        Self(NonNull::new(ptr as *mut _))
    }

    pub const fn as_raw(self) -> *const Resolution<'static> {
        match self.0 {
            Some(p) => p.as_ptr() as *const _,
            None => ptr::null(),
        }
    }

    /// # Safety
    ///
    /// The `data` slice must contain a valid pointer to a `Resolution<'static>` in native endianness.
    pub unsafe fn from_raw(data: &[u8]) -> Self {
        let mut res_data = [0u8; size_of::<usize>()];
        res_data.copy_from_slice(data);
        let res = usize::from_ne_bytes(res_data) as *const _;
        Self::new(res)
    }

    pub const fn is_null(&self) -> bool {
        self.0.is_none()
    }

    pub fn definition(&self) -> &'static Resolution<'static> {
        match self.0 {
            Some(p) => {
                // SAFETY: The pointer in ResolutionS is derived from a reference during construction,
                // ensuring it points to valid metadata that persists for the program lifetime.
                unsafe { &*p.as_ptr() }
            }
            None => {
                panic!("ResolutionS NULL");
            }
        }
    }
}
impl Deref for ResolutionS {
    type Target = Resolution<'static>;
    fn deref(&self) -> &'static Self::Target {
        self.definition()
    }
}
impl Debug for ResolutionS {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self.0 {
            None => write!(f, "ResolutionS(NULL)"),
            Some(_) => write!(
                f,
                "ResolutionS({} @ {:#?})",
                self.definition().assembly.as_ref().unwrap().name,
                self.as_raw()
            ),
        }
    }
}
impl PartialEq for ResolutionS {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}
impl Eq for ResolutionS {}
impl Hash for ResolutionS {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.0.hash(state);
    }
}

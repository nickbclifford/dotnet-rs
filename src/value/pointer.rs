use std::cmp::Ordering;
use std::collections::HashSet;
use std::fmt::{Debug, Formatter};
use gc_arena::{unsafe_empty_collect, Collect, Collection, Gc};
use crate::value::description::TypeDescription;
use crate::value::object::{ObjectHandle, ObjectRef};

#[derive(Copy, Clone, Debug, PartialEq, PartialOrd)]
pub struct UnmanagedPtr(pub *mut u8);
unsafe_empty_collect!(UnmanagedPtr);

#[derive(Copy, Clone)]
pub struct ManagedPtr<'gc> {
    pub value: *mut u8,
    pub inner_type: TypeDescription,
    pub owner: Option<ObjectHandle<'gc>>,
    pub pinned: bool,
}

impl Debug for ManagedPtr<'_> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "[{}] {:#?} (owner: {:?}, pinned: {})",
            self.inner_type.type_name(),
            self.value,
            self.owner.map(Gc::as_ptr),
            self.pinned
        )
    }
}

impl PartialEq for ManagedPtr<'_> {
    fn eq(&self, other: &Self) -> bool {
        self.value == other.value
    }
}

impl PartialOrd for ManagedPtr<'_> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        self.value.partial_cmp(&other.value)
    }
}

impl<'gc> ManagedPtr<'gc> {
    // Storage format: pointer (8) + TypeDescription (16) + ObjectRef (8) + pinned (1) = 33 bytes
    // But we need to account for actual sizes, not hardcoded values
    pub const SIZE: usize =
        size_of::<*mut u8>() + size_of::<TypeDescription>() + ObjectRef::SIZE + 1;

    pub fn new(
        value: *mut u8,
        inner_type: TypeDescription,
        owner: Option<ObjectHandle<'gc>>,
        pinned: bool,
    ) -> Self {
        Self {
            value,
            inner_type,
            owner,
            pinned,
        }
    }

    pub fn read(source: &[u8]) -> Self {
        // SAFETY: ManagedPtr should not actually be stored in object fields in most cases.
        // This function exists for completeness but reading a ManagedPtr from storage
        // that wasn't properly initialized will cause UB. The proper solution is to
        // avoid storing ManagedPtr values entirely, as they should be stack-only.
        //
        // For now, we panic if we try to read from what looks like uninitialized storage.

        // Check if the entire ManagedPtr storage appears to be zero-initialized
        let expected_size =
            size_of::<*mut u8>() + size_of::<TypeDescription>() + ObjectRef::SIZE + 1;
        if source.len() < expected_size {
            panic!("Attempted to read ManagedPtr from insufficiently sized storage");
        }

        // If all bytes are zero, this is likely uninitialized storage
        if source[..expected_size].iter().all(|&b| b == 0) {
            // This should never happen in correct code, so we panic rather than
            // trying to create a "safe" dummy value
            panic!("Attempted to read ManagedPtr from zero-initialized storage");
        }

        let mut value_bytes = [0u8; size_of::<*mut u8>()];
        value_bytes.copy_from_slice(&source[0..size_of::<*mut u8>()]);
        let value = usize::from_ne_bytes(value_bytes) as *mut u8;

        let mut type_bytes = [0u8; size_of::<TypeDescription>()];
        type_bytes.copy_from_slice(
            &source[size_of::<*mut u8>()..(size_of::<*mut u8>() + size_of::<TypeDescription>())],
        );
        let inner_type: TypeDescription = unsafe { std::mem::transmute(type_bytes) };

        let owner =
            ObjectRef::read(&source[(size_of::<*mut u8>() + size_of::<TypeDescription>())..]).0;

        let pinned =
            source[size_of::<*mut u8>() + size_of::<TypeDescription>() + ObjectRef::SIZE] != 0;

        Self {
            value,
            inner_type,
            owner,
            pinned,
        }
    }

    pub fn write(&self, dest: &mut [u8]) {
        let value_bytes = (self.value as usize).to_ne_bytes();
        dest[0..size_of::<*mut u8>()].copy_from_slice(&value_bytes);

        let type_bytes: [u8; size_of::<TypeDescription>()] =
            unsafe { std::mem::transmute(self.inner_type) };
        dest[size_of::<*mut u8>()..(size_of::<*mut u8>() + size_of::<TypeDescription>())]
            .copy_from_slice(&type_bytes);

        ObjectRef(self.owner)
            .write(&mut dest[(size_of::<*mut u8>() + size_of::<TypeDescription>())..]);

        dest[size_of::<*mut u8>() + size_of::<TypeDescription>() + ObjectRef::SIZE] =
            if self.pinned { 1 } else { 0 };
    }

    pub fn map_value(self, transform: impl FnOnce(*mut u8) -> *mut u8) -> Self {
        ManagedPtr {
            value: transform(self.value),
            inner_type: self.inner_type,
            owner: self.owner,
            pinned: self.pinned,
        }
    }
}

unsafe impl<'gc> Collect for ManagedPtr<'gc> {
    fn trace(&self, cc: &Collection) {
        if let Some(o) = self.owner {
            o.trace(cc);
        }
    }
}

impl<'gc> ManagedPtr<'gc> {
    pub fn resurrect(&self, fc: &gc_arena::Finalization<'gc>, visited: &mut HashSet<usize>) {
        if let Some(owner) = self.owner {
            let ptr = Gc::as_ptr(owner) as usize;
            if visited.insert(ptr) {
                Gc::resurrect(fc, owner);
                owner.borrow().resurrect(fc, visited);
            }
        }
    }
}
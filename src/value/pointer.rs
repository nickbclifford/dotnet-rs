use crate::{
    types::TypeDescription, utils::ResolutionS, value::object::{ObjectHandle, ObjectRef},
};
use gc_arena::{Collect, Collection, Gc, unsafe_empty_collect};
use std::{
    cmp::Ordering, collections::HashSet, fmt::{Debug, Formatter},
    ptr::NonNull,
};

#[derive(Copy, Clone, Debug, PartialEq, PartialOrd)]
pub struct UnmanagedPtr(pub NonNull<u8>);
unsafe_empty_collect!(UnmanagedPtr);

#[derive(Copy, Clone)]
pub struct ManagedPtr<'gc> {
    pub value: NonNull<u8>,
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
        size_of::<NonNull<u8>>() + size_of::<TypeDescription>() + ObjectRef::SIZE + 1;

    pub fn new(
        value: NonNull<u8>,
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
        let expected_size =
            size_of::<NonNull<u8>>() + size_of::<TypeDescription>() + ObjectRef::SIZE + 1;
        if source.len() < expected_size {
            panic!("Attempted to read ManagedPtr from insufficiently sized storage");
        }

        let mut value_bytes = [0u8; size_of::<NonNull<u8>>()];
        value_bytes.copy_from_slice(&source[0..size_of::<NonNull<u8>>()]);
        let value_ptr = usize::from_ne_bytes(value_bytes) as *mut u8;
        let value = NonNull::new(value_ptr).expect("ManagedPtr value should not be null");

        let resolution = unsafe {
            ResolutionS::from_raw(
                &source[size_of::<NonNull<u8>>()..(size_of::<NonNull<u8>>() + size_of::<usize>())],
            )
        };

        let mut def_bytes = [0u8; size_of::<usize>()];
        def_bytes.copy_from_slice(
            &source[(size_of::<NonNull<u8>>() + size_of::<usize>())
                ..(size_of::<NonNull<u8>>() + 2 * size_of::<usize>())],
        );
        let definition_ptr = NonNull::new(usize::from_ne_bytes(def_bytes) as *mut _);
        let inner_type = TypeDescription::from_raw(resolution, definition_ptr);

        debug_assert!(
            inner_type.is_null() || !inner_type.resolution.is_null(),
            "ManagedPtr has invalid TypeDescription (null resolution but non-null definition)"
        );

        let owner =
            ObjectRef::read(&source[(size_of::<NonNull<u8>>() + size_of::<TypeDescription>())..]).0;

        let pinned =
            source[size_of::<NonNull<u8>>() + size_of::<TypeDescription>() + ObjectRef::SIZE] != 0;

        Self {
            value,
            inner_type,
            owner,
            pinned,
        }
    }

    pub fn write(&self, dest: &mut [u8]) {
        let value_bytes = (self.value.as_ptr() as usize).to_ne_bytes();
        dest[0..size_of::<NonNull<u8>>()].copy_from_slice(&value_bytes);

        let res_bytes = (self.inner_type.resolution.as_raw() as usize).to_ne_bytes();
        dest[size_of::<NonNull<u8>>()..(size_of::<NonNull<u8>>() + size_of::<usize>())]
            .copy_from_slice(&res_bytes);

        let def_bytes = self
            .inner_type
            .definition_ptr()
            .map(|p| p.as_ptr() as usize)
            .unwrap_or(0)
            .to_ne_bytes();
        dest[(size_of::<NonNull<u8>>() + size_of::<usize>())
            ..(size_of::<NonNull<u8>>() + 2 * size_of::<usize>())]
            .copy_from_slice(&def_bytes);

        ObjectRef(self.owner)
            .write(&mut dest[(size_of::<NonNull<u8>>() + size_of::<TypeDescription>())..]);

        dest[size_of::<NonNull<u8>>() + size_of::<TypeDescription>() + ObjectRef::SIZE] =
            if self.pinned { 1 } else { 0 };
    }

    pub fn map_value(self, transform: impl FnOnce(NonNull<u8>) -> NonNull<u8>) -> Self {
        ManagedPtr {
            value: transform(self.value),
            inner_type: self.inner_type,
            owner: self.owner,
            pinned: self.pinned,
        }
    }

    /// # Safety
    ///
    /// The caller must ensure that the resulting pointer is within the bounds of the same
    /// allocated object as the original pointer.
    pub unsafe fn offset(self, bytes: isize) -> Self {
        self.map_value(|p| NonNull::new_unchecked(p.as_ptr().offset(bytes)))
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
                owner.borrow().storage.resurrect(fc, visited);
            }
        }
    }
}

use crate::{
    types::TypeDescription,
    utils::ResolutionS,
    value::object::{Object, ObjectHandle, ObjectRef},
};
use gc_arena::{unsafe_empty_collect, Collect, Collection, Gc};
use std::{
    cmp::Ordering,
    collections::HashSet,
    fmt::{self, Debug, Formatter},
    ptr::NonNull,
};

#[derive(Copy, Clone, Debug, PartialEq, PartialOrd)]
pub struct UnmanagedPtr(pub NonNull<u8>);
unsafe_empty_collect!(UnmanagedPtr);

#[derive(Copy, Clone)]
pub enum ManagedPtrOwner<'gc> {
    Heap(ObjectHandle<'gc>),
    Stack(NonNull<Object<'gc>>),
}

impl<'gc> ManagedPtrOwner<'gc> {
    pub fn as_ptr(&self) -> *const u8 {
        match self {
            Self::Heap(h) => Gc::as_ptr(*h) as *const u8,
            Self::Stack(s) => s.as_ptr() as *const u8,
        }
    }
}

unsafe impl<'gc> Collect for ManagedPtrOwner<'gc> {
    fn trace(&self, cc: &Collection) {
        match self {
            Self::Heap(h) => h.trace(cc),
            // Stack objects are already traced through the VM stack.
            // Do NOT trace them here to avoid infinite recursion:
            // Object → StackValue → ManagedPtr → ManagedPtrOwner::Stack → Object (cycle!)
            Self::Stack(_) => {}
        }
    }
}

impl Debug for ManagedPtrOwner<'_> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Heap(h) => write!(f, "Heap({:?})", Gc::as_ptr(*h)),
            Self::Stack(s) => write!(f, "Stack({:?})", s.as_ptr()),
        }
    }
}

#[derive(Copy, Clone)]
pub struct ManagedPtr<'gc> {
    pub value: NonNull<u8>,
    pub inner_type: TypeDescription,
    pub owner: Option<ManagedPtrOwner<'gc>>,
    pub pinned: bool,
}

impl Debug for ManagedPtr<'_> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "[{}] {:#?} (owner: {:?}, pinned: {})",
            self.inner_type.type_name(),
            self.value,
            self.owner,
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
    /// DEPRECATED: Old storage format size (33 bytes).
    /// In the new implementation, managed pointers are pointer-sized in memory (8 bytes).
    /// Metadata is stored in Object's side-table.
    pub const SIZE: usize =
        size_of::<NonNull<u8>>() + size_of::<TypeDescription>() + ObjectRef::SIZE + 1;

    /// Size of managed pointer in memory (pointer-sized, 8 bytes on 64-bit).
    /// This is the .NET-compliant size per ECMA-335.
    pub const MEMORY_SIZE: usize = size_of::<usize>();

    pub fn new(
        value: NonNull<u8>,
        inner_type: TypeDescription,
        owner: Option<ManagedPtrOwner<'gc>>,
        pinned: bool,
    ) -> Self {
        Self {
            value,
            inner_type,
            owner,
            pinned,
        }
    }

    /// Read just the pointer value from memory (8 bytes).
    /// This is the new memory format. Metadata must be retrieved from the Object's side-table.
    pub fn read_ptr_only(source: &[u8]) -> NonNull<u8> {
        let mut value_bytes = [0u8; size_of::<usize>()];
        value_bytes.copy_from_slice(&source[0..size_of::<usize>()]);
        let value_ptr = usize::from_ne_bytes(value_bytes) as *mut u8;
        NonNull::new(value_ptr).expect("ManagedPtr value should not be null")
    }

    /// Write just the pointer value to memory (8 bytes).
    /// Metadata should be stored in the Object's side-table separately.
    pub fn write_ptr_only(&self, dest: &mut [u8]) {
        let value_bytes = (self.value.as_ptr() as usize).to_ne_bytes();
        dest[0..size_of::<usize>()].copy_from_slice(&value_bytes);
    }

    /// DEPRECATED: Read the old 33-byte format.
    /// This is only for backward compatibility during transition.
    #[deprecated(note = "Use read_ptr_only and side-table lookup instead")]
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
            owner: owner.map(ManagedPtrOwner::Heap),
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

        let heap_owner = self.owner.and_then(|o| match o {
            ManagedPtrOwner::Heap(h) => Some(h),
            ManagedPtrOwner::Stack(_) => None, // Cannot write stack owner to persistent memory
        });

        ObjectRef(heap_owner)
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
            match owner {
                ManagedPtrOwner::Heap(h) => {
                    let ptr = Gc::as_ptr(h) as usize;
                    if visited.insert(ptr) {
                        Gc::resurrect(fc, h);
                        h.borrow().storage.resurrect(fc, visited);
                    }
                }
                ManagedPtrOwner::Stack(s) => {
                    unsafe { s.as_ref().resurrect(fc, visited) };
                }
            }
        }
    }
}

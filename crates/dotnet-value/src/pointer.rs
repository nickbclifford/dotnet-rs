use crate::object::ObjectRef;
use dotnet_types::TypeDescription;
use gc_arena::{Collect, Collection, unsafe_empty_collect};
use std::{
    cmp::Ordering,
    collections::HashSet,
    fmt::{self, Debug, Formatter},
    mem::size_of,
    ptr::NonNull,
};

#[derive(Copy, Clone, Debug, PartialEq, PartialOrd)]
pub struct UnmanagedPtr(pub NonNull<u8>);
unsafe_empty_collect!(UnmanagedPtr);

#[derive(Copy, Clone)]
#[repr(C)]
pub struct ManagedPtr<'gc> {
    _value: Option<NonNull<u8>>,
    /// The object that owns the memory pointed to by this pointer.
    /// This ensures the object stays alive while the pointer is in use.
    pub owner: Option<ObjectRef<'gc>>,
    /// The offset from the owner's base pointer.
    pub offset: usize,
    pub inner_type: TypeDescription,
    pub pinned: bool,
    pub _marker: std::marker::PhantomData<&'gc ()>,
}

impl Debug for ManagedPtr<'_> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "[{}] {:?} (owner: {:?}, pinned: {})",
            self.inner_type.type_name(),
            self.pointer(),
            self.owner,
            self.pinned
        )
    }
}

impl PartialEq for ManagedPtr<'_> {
    fn eq(&self, other: &Self) -> bool {
        self.pointer() == other.pointer()
    }
}

impl PartialOrd for ManagedPtr<'_> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        self.pointer().partial_cmp(&other.pointer())
    }
}

impl<'gc> ManagedPtr<'gc> {
    /// One word for the pointer, one word for the owner.
    pub const MEMORY_SIZE: usize = size_of::<usize>() * 2;

    pub fn new(
        value: Option<NonNull<u8>>,
        inner_type: TypeDescription,
        owner: Option<ObjectRef<'gc>>,
        pinned: bool,
    ) -> Self {
        let owner = owner.and_then(|o| if o.0.is_some() { Some(o) } else { None });
        let offset = match (value, owner) {
            (Some(ptr), Some(owner)) => {
                if let Some(handle) = owner.0 {
                    let base_ptr = handle.borrow().storage.as_ptr();
                    if base_ptr.is_null() {
                        0usize
                    } else {
                        (ptr.as_ptr() as usize).wrapping_sub(base_ptr as usize)
                    }
                } else {
                    0usize
                }
            }
            (Some(ptr), None) => ptr.as_ptr() as usize,
            _ => 0usize,
        };

        Self {
            _value: value,
            inner_type,
            owner,
            offset,
            pinned,
            _marker: std::marker::PhantomData,
        }
    }

    /// Read pointer and owner from memory.
    /// Returns (pointer, owner). Type info must be supplied by caller.
    ///
    /// # Safety
    ///
    /// The `source` slice must be at least `ManagedPtr::MEMORY_SIZE` bytes long.
    /// It must contain valid bytes representing a `ManagedPtr`.
    pub unsafe fn read_from_bytes(source: &[u8]) -> (Option<NonNull<u8>>, ObjectRef<'gc>, usize) {
        unsafe {
            let ptr_size = size_of::<usize>();

            // Memory layout: (Owner ObjectRef at offset 0, Offset at offset 8)
            // Read Owner (Offset 0)
            let owner = ObjectRef::read_unchecked(&source[0..ptr_size]);

            // Read Offset (Offset 8)
            let mut offset_bytes = [0u8; size_of::<usize>()];
            offset_bytes.copy_from_slice(&source[ptr_size..ptr_size * 2]);
            let offset = usize::from_ne_bytes(offset_bytes);

            // Compute pointer from owner's data + offset
            let ptr = if let Some(handle) = owner.0 {
                let base_ptr = handle.borrow().storage.as_ptr();
                if base_ptr.is_null() {
                    None
                } else {
                    NonNull::new(base_ptr.wrapping_add(offset) as *mut u8)
                }
            } else if offset == 0 {
                // Null owner with zero offset means null pointer
                None
            } else {
                // Null owner but non-zero offset - this is a static data pointer
                // Store raw pointer directly in offset field for static data
                NonNull::new(offset as *mut u8)
            };

            (ptr, owner, offset)
        }
    }

    /// Write owner and offset to memory.
    /// Memory layout: (Owner ObjectRef at offset 0, Offset at offset 8)
    pub fn write(&self, dest: &mut [u8]) {
        let ptr_size = size_of::<usize>();

        // Write Owner (Offset 0)
        self.owner
            .unwrap_or(ObjectRef(None))
            .write(&mut dest[0..ptr_size]);

        // Write Offset (Offset 8)
        let offset_bytes = self.offset.to_ne_bytes();
        dest[ptr_size..ptr_size * 2].copy_from_slice(&offset_bytes);
    }

    pub fn pointer(&self) -> Option<NonNull<u8>> {
        if let Some(owner) = self.owner {
            let handle = owner.0?;
            let base_ptr = handle.borrow().storage.as_ptr();
            if base_ptr.is_null() {
                None
            } else {
                NonNull::new(base_ptr.wrapping_add(self.offset) as *mut u8)
            }
        } else if self.offset == 0 {
            None
        } else {
            // Static data pointer
            NonNull::new(self.offset as *mut u8)
        }
    }

    pub fn managed_ptr_with_owner(
        ptr: *mut u8,
        inner_type: TypeDescription,
        owner: Option<ObjectRef<'gc>>,
        pinned: bool,
    ) -> Self {
        Self::new(NonNull::new(ptr), inner_type, owner, pinned)
    }

    pub fn map_value(
        self,
        transform: impl FnOnce(Option<NonNull<u8>>) -> Option<NonNull<u8>>,
    ) -> Self {
        let new_value = transform(self.pointer());
        Self::new(new_value, self.inner_type, self.owner, self.pinned)
    }

    /// # Safety
    ///
    /// The caller must ensure that the resulting pointer is within the bounds of the same
    /// allocated object as the original pointer.
    pub unsafe fn offset(self, bytes: isize) -> Self {
        self.map_value(|p| {
            p.and_then(|ptr| {
                // SAFETY: generic caller safety invariant regarding object bounds still applies.
                let new_ptr = unsafe { ptr.as_ptr().offset(bytes) };
                NonNull::new(new_ptr)
            })
        })
    }
}

unsafe impl<'gc> Collect for ManagedPtr<'gc> {
    fn trace(&self, cc: &Collection) {
        if let Some(owner) = &self.owner {
            owner.trace(cc);
        }
    }
}

impl<'gc> ManagedPtr<'gc> {
    pub fn resurrect(&self, fc: &gc_arena::Finalization<'gc>, visited: &mut HashSet<usize>) {
        if let Some(owner) = &self.owner {
            owner.resurrect(fc, visited);
        }
    }
}

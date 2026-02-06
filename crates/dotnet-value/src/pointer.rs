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
    pub(crate) _value: Option<NonNull<u8>>,
    /// The object that owns the memory pointed to by this pointer.
    /// This ensures the object stays alive while the pointer is in use.
    pub owner: Option<ObjectRef<'gc>>,
    /// The offset from the owner's base pointer.
    pub offset: usize,
    /// If this is a pointer to a stack slot, this stores the absolute index of that slot and the offset within it.
    pub stack_slot_origin: Option<(usize, usize)>,
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
            stack_slot_origin: None,
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
    #[allow(clippy::type_complexity)]
    pub unsafe fn read_from_bytes(
        source: &[u8],
    ) -> (
        Option<NonNull<u8>>,
        ObjectRef<'gc>,
        usize,
        Option<(usize, usize)>,
    ) {
        unsafe {
            let ptr_size = size_of::<usize>();

            let mut word0_bytes = [0u8; size_of::<usize>()];
            word0_bytes.copy_from_slice(&source[0..ptr_size]);
            let word0 = usize::from_ne_bytes(word0_bytes);

            let mut word1_bytes = [0u8; size_of::<usize>()];
            word1_bytes.copy_from_slice(&source[ptr_size..ptr_size * 2]);
            let word1 = usize::from_ne_bytes(word1_bytes);

            if word0 & 1 != 0 {
                // Stack pointer
                let slot_idx = (word0 >> 1) & 0xFFFFFFFF;
                let slot_offset = word0 >> 33;
                let raw_ptr = NonNull::new(word1 as *mut u8);
                (
                    raw_ptr,
                    ObjectRef(None),
                    slot_offset,
                    Some((slot_idx, slot_offset)),
                )
            } else {
                // Memory layout: (Owner ObjectRef at offset 0, Offset at offset 8)
                // Read Owner (Offset 0)
                let owner = ObjectRef::read_unchecked(&source[0..ptr_size]);

                // Read Offset (Offset 8)
                let offset = word1;

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

                (ptr, owner, offset, None)
            }
        }
    }

    /// Write owner and offset to memory.
    /// Memory layout: (Owner ObjectRef at offset 0, Offset at offset 8)
    pub fn write(&self, dest: &mut [u8]) {
        let ptr_size = size_of::<usize>();

        if let Some((slot_idx, slot_offset)) = self.stack_slot_origin {
            let word0 = 1 | ((slot_idx & 0xFFFFFFFF) << 1) | (slot_offset << 33);
            let word1 = self._value.map_or(0, |p| p.as_ptr() as usize);
            dest[0..ptr_size].copy_from_slice(&word0.to_ne_bytes());
            dest[ptr_size..ptr_size * 2].copy_from_slice(&word1.to_ne_bytes());
        } else {
            // Write Owner (Offset 0)
            self.owner
                .unwrap_or(ObjectRef(None))
                .write(&mut dest[0..ptr_size]);

            // Write Offset (Offset 8)
            let offset_bytes = self.offset.to_ne_bytes();
            dest[ptr_size..ptr_size * 2].copy_from_slice(&offset_bytes);
        }
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
        } else if let Some((_idx, _offset)) = self.stack_slot_origin {
            // We can't re-resolve here because we don't have the stack.
            // But we can return the cached value which should have been updated
            // by the stack on reallocation.
            self._value
        } else if self.offset == 0 {
            None
        } else {
            // Static data pointer
            NonNull::new(self.offset as *mut u8)
        }
    }

    pub fn update_cached_ptr(&mut self, ptr: NonNull<u8>) {
        self._value = Some(ptr);
        if self.owner.is_none() && self.stack_slot_origin.is_none() {
            // If it's a static pointer, also update the offset
            self.offset = ptr.as_ptr() as usize;
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
        let old_ptr = self.pointer();
        let new_value = transform(old_ptr);
        let mut m = Self::new(new_value, self.inner_type, self.owner, self.pinned);
        if let Some((idx, offset)) = self.stack_slot_origin {
            let diff = if let (Some(new_p), Some(old_p)) = (new_value, old_ptr) {
                (new_p.as_ptr() as isize).wrapping_sub(old_p.as_ptr() as isize)
            } else {
                0
            };
            m.stack_slot_origin = Some((idx, (offset as isize + diff) as usize));
            m.offset = (self.offset as isize + diff) as usize;
        }
        m
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

    pub fn with_stack_origin(mut self, slot_index: usize, offset: usize) -> Self {
        self.stack_slot_origin = Some((slot_index, offset));
        self
    }

    pub fn is_stack_local(&self) -> bool {
        self.stack_slot_origin.is_some()
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

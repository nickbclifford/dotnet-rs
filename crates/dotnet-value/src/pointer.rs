use crate::object::ObjectRef;
use dotnet_types::TypeDescription;
use gc_arena::{Collect, Collection, Mutation, unsafe_empty_collect};
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

/// Stack-related metadata for a [`ManagedPtr`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ManagedPtrStackInfo {
    /// The actual memory address being pointed to.
    /// NOTE: For heap pointers, this is `None` because the absolute address
    /// cannot be computed without the owner's base pointer.
    pub address: Option<NonNull<u8>>,
    /// The offset from the base of the owner (either an object on the heap or a stack slot).
    pub offset: crate::ByteOffset,
    /// If this is a stack pointer, this contains the stack slot index and the offset within that slot.
    pub stack_origin: Option<(crate::StackSlotIndex, crate::ByteOffset)>,
}

/// Detailed information about a [`ManagedPtr`] read from memory.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ManagedPtrInfo<'gc> {
    /// The actual memory address being pointed to.
    pub address: Option<NonNull<u8>>,
    /// The object that owns the memory, if any.
    pub owner: ObjectRef<'gc>,
    /// The offset from the base of the owner (either an object on the heap or a stack slot).
    pub offset: crate::ByteOffset,
    /// If this is a stack pointer, this contains the stack slot index and the offset within that slot.
    pub stack_origin: Option<(crate::StackSlotIndex, crate::ByteOffset)>,
}

#[cfg(any(feature = "memory-validation", debug_assertions))]
const MANAGED_PTR_MAGIC: u32 = 0x504F_494E;

#[derive(Copy, Clone)]
#[repr(C)]
pub struct ManagedPtr<'gc> {
    #[cfg(any(feature = "memory-validation", debug_assertions))]
    pub(crate) magic: u32,
    pub(crate) _value: Option<NonNull<u8>>,
    /// The object that owns the memory pointed to by this pointer.
    /// This ensures the object stays alive while the pointer is in use.
    pub owner: Option<ObjectRef<'gc>>,
    /// The offset from the owner's base pointer.
    pub offset: crate::ByteOffset,
    /// If this is a pointer to a stack slot, this stores the absolute index of that slot and the offset within it.
    pub stack_slot_origin: Option<(crate::StackSlotIndex, crate::ByteOffset)>,
    pub inner_type: TypeDescription,
    pub pinned: bool,
    pub _marker: std::marker::PhantomData<&'gc ()>,
}

impl Debug for ManagedPtr<'_> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        self.validate_magic();
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
        self.validate_magic();
        other.validate_magic();
        self.pointer() == other.pointer()
    }
}

impl PartialOrd for ManagedPtr<'_> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        self.validate_magic();
        other.validate_magic();
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
                        crate::ByteOffset(0)
                    } else {
                        crate::ByteOffset((ptr.as_ptr() as usize).wrapping_sub(base_ptr as usize))
                    }
                } else {
                    crate::ByteOffset(0)
                }
            }
            (Some(ptr), None) => crate::ByteOffset(ptr.as_ptr() as usize),
            _ => crate::ByteOffset(0),
        };

        Self {
            #[cfg(any(feature = "memory-validation", debug_assertions))]
            magic: MANAGED_PTR_MAGIC,
            _value: value,
            inner_type,
            owner,
            offset,
            stack_slot_origin: None,
            pinned,
            _marker: std::marker::PhantomData,
        }
    }

    pub(crate) fn validate_magic(&self) {
        #[cfg(any(feature = "memory-validation", debug_assertions))]
        {
            if self.magic != MANAGED_PTR_MAGIC {
                panic!("ManagedPtr magic number corrupted: {:#x}", self.magic);
            }
        }
    }

    /// Read only the stack-related metadata from memory. This does not require a GCHandle
    /// because it does not return an ObjectRef.
    ///
    /// # Safety
    ///
    /// The `source` slice must be at least `ManagedPtr::MEMORY_SIZE` bytes long.
    pub unsafe fn read_stack_info(source: &[u8]) -> ManagedPtrStackInfo {
        let ptr_size = size_of::<usize>();
        if source.len() < ptr_size * 2 {
            panic!("ManagedPtr::read: buffer too small");
        }

        let word0 = unsafe { (source.as_ptr() as *const usize).read_unaligned() };
        let word1 = unsafe { (source.as_ptr().add(ptr_size) as *const usize).read_unaligned() };

        if word0 & 1 == 1 {
            // Stack pointer
            let idx = (word0 >> 1) & 0xFFFFFFFF;
            let off = word0 >> 33;
            ManagedPtrStackInfo {
                address: NonNull::new(word1 as *mut u8),
                offset: crate::ByteOffset(off),
                stack_origin: Some((crate::StackSlotIndex(idx), crate::ByteOffset(off))),
            }
        } else {
            // Heap pointer
            ManagedPtrStackInfo {
                address: None,
                offset: crate::ByteOffset(word1),
                stack_origin: None,
            }
        }
    }

    /// Read pointer and owner from memory, branded with a GCHandle.
    /// Returns detailed metadata about the pointer. Type info must be supplied by caller.
    ///
    /// # Safety
    ///
    /// The `source` slice must be at least `ManagedPtr::MEMORY_SIZE` bytes long.
    /// It must contain valid bytes representing a `ManagedPtr`.
    pub unsafe fn read_branded(source: &[u8], _gc: &Mutation<'gc>) -> ManagedPtrInfo<'gc> {
        unsafe { Self::read_unchecked(source) }
    }

    /// Read pointer and owner from memory.
    /// Returns detailed metadata about the pointer. Type info must be supplied by caller.
    ///
    /// # Safety
    ///
    /// The `source` slice must be at least `ManagedPtr::MEMORY_SIZE` bytes long.
    /// It must contain valid bytes representing a `ManagedPtr`.
    pub unsafe fn read_unchecked(source: &[u8]) -> ManagedPtrInfo<'gc> {
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
            ManagedPtrInfo {
                address: raw_ptr,
                owner: ObjectRef(None),
                offset: crate::ByteOffset(slot_offset),
                stack_origin: Some((
                    crate::StackSlotIndex(slot_idx),
                    crate::ByteOffset(slot_offset),
                )),
            }
        } else {
            // Memory layout: (Owner ObjectRef at offset 0, Offset at offset 8)
            // Read Owner (Offset 0)
            let owner = unsafe { ObjectRef::read_unchecked(&source[0..ptr_size]) };

            // Read Offset (Offset 8)
            let offset = crate::ByteOffset(word1);

            // Compute pointer from owner's data + offset
            let ptr = if let Some(handle) = owner.0 {
                let base_ptr = handle.borrow().storage.as_ptr();
                if base_ptr.is_null() {
                    None
                } else {
                    NonNull::new(base_ptr.wrapping_add(offset.as_usize()) as *mut u8)
                }
            } else if offset == crate::ByteOffset::ZERO {
                // Null owner with zero offset means null pointer
                None
            } else {
                // Null owner but non-zero offset - this is a static data pointer
                // Store raw pointer directly in offset field for static data
                NonNull::new(offset.as_usize() as *mut u8)
            };

            ManagedPtrInfo {
                address: ptr,
                owner,
                offset,
                stack_origin: None,
            }
        }
    }

    /// Write owner and offset to memory.
    /// Memory layout: (Owner ObjectRef at offset 0, Offset at offset 8)
    pub fn write(&self, dest: &mut [u8]) {
        self.validate_magic();
        let ptr_size = size_of::<usize>();

        if let Some((slot_idx, slot_offset)) = self.stack_slot_origin {
            let word0 =
                1 | ((slot_idx.as_usize() & 0xFFFFFFFF) << 1) | (slot_offset.as_usize() << 33);
            let word1 = self._value.map_or(0, |p| p.as_ptr() as usize);
            dest[0..ptr_size].copy_from_slice(&word0.to_ne_bytes());
            dest[ptr_size..ptr_size * 2].copy_from_slice(&word1.to_ne_bytes());
        } else {
            // Write Owner (Offset 0)
            self.owner
                .unwrap_or(ObjectRef(None))
                .write(&mut dest[0..ptr_size]);

            // Write Offset (Offset 8)
            let offset_bytes = self.offset.as_usize().to_ne_bytes();
            dest[ptr_size..ptr_size * 2].copy_from_slice(&offset_bytes);
        }
    }

    pub fn pointer(&self) -> Option<NonNull<u8>> {
        self.validate_magic();
        if let Some(owner) = self.owner {
            let handle = owner.0?;
            let base_ptr = handle.borrow().storage.as_ptr();
            if base_ptr.is_null() {
                None
            } else {
                NonNull::new(base_ptr.wrapping_add(self.offset.as_usize()) as *mut u8)
            }
        } else if let Some((_idx, _offset)) = self.stack_slot_origin {
            // We can't re-resolve here because we don't have the stack.
            // But we can return the cached value which should have been updated
            // by the stack on reallocation.
            self._value
        } else if self.offset == crate::ByteOffset::ZERO {
            None
        } else {
            // Static data pointer
            NonNull::new(self.offset.as_usize() as *mut u8)
        }
    }

    pub fn update_cached_ptr(&mut self, ptr: NonNull<u8>) {
        self.validate_magic();
        self._value = Some(ptr);
        if self.owner.is_none() && self.stack_slot_origin.is_none() {
            // If it's a static pointer, also update the offset
            self.offset = crate::ByteOffset(ptr.as_ptr() as usize);
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
        self.validate_magic();
        let old_ptr = self.pointer();
        let new_value = transform(old_ptr);
        let mut m = Self::new(new_value, self.inner_type, self.owner, self.pinned);
        if let Some((idx, offset)) = self.stack_slot_origin {
            let diff = if let (Some(new_p), Some(old_p)) = (new_value, old_ptr) {
                (new_p.as_ptr() as isize).wrapping_sub(old_p.as_ptr() as isize)
            } else {
                0
            };
            m.stack_slot_origin = Some((
                idx,
                crate::ByteOffset((offset.as_usize() as isize + diff) as usize),
            ));
            m.offset = crate::ByteOffset((self.offset.as_usize() as isize + diff) as usize);
        }
        m
    }

    #[cfg(feature = "memory-validation")]
    fn validate_offset(&self, bytes: isize) {
        if let Some(owner) = self.owner {
            if let Some(handle) = owner.0 {
                // SAFETY: We are only reading the size and magic, which are immutable for the object's lifetime.
                // Borrowing the lock is safe here.
                let inner = handle.borrow();
                inner.validate_magic();
                let obj_size = inner.storage.size_bytes();
                let current_offset = self.offset.as_usize();
                let new_offset = (current_offset as isize).saturating_add(bytes);
                if new_offset < 0 || (new_offset as usize) > obj_size {
                    panic!(
                        "ManagedPtr::offset: bounds violation (offset={}, bytes={}, size={})",
                        current_offset, bytes, obj_size
                    );
                }
            }
        }
    }

    /// # Safety
    ///
    /// The caller must ensure that the resulting pointer is within the bounds of the same
    /// allocated object as the original pointer.
    pub unsafe fn offset(self, bytes: isize) -> Self {
        #[cfg(feature = "memory-validation")]
        self.validate_offset(bytes);

        self.map_value(|p| {
            p.and_then(|ptr| {
                // SAFETY: generic caller safety invariant regarding object bounds still applies.
                let new_ptr = unsafe { ptr.as_ptr().offset(bytes) };
                NonNull::new(new_ptr)
            })
        })
    }

    pub fn with_stack_origin(
        mut self,
        slot_index: crate::StackSlotIndex,
        offset: crate::ByteOffset,
    ) -> Self {
        self.validate_magic();
        self.stack_slot_origin = Some((slot_index, offset));
        self
    }

    pub fn is_stack_local(&self) -> bool {
        self.validate_magic();
        self.stack_slot_origin.is_some()
    }
}

unsafe impl<'gc> Collect for ManagedPtr<'gc> {
    fn trace(&self, cc: &Collection) {
        self.validate_magic();
        if let Some(owner) = &self.owner {
            owner.trace(cc);
        }
    }
}

impl<'gc> ManagedPtr<'gc> {
    pub fn resurrect(
        &self,
        fc: &'gc gc_arena::Finalization<'gc>,
        visited: &mut HashSet<usize>,
        depth: usize,
    ) {
        self.validate_magic();
        if let Some(owner) = &self.owner {
            owner.resurrect(fc, visited, depth);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::object::{HeapStorage, ObjectRef, ValueType};
    use gc_arena::{Arena, Rootable};

    #[test]
    #[cfg_attr(
        feature = "memory-validation",
        should_panic(expected = "ManagedPtr::offset: bounds violation")
    )]
    fn test_managed_ptr_offset_oob() {
        type TestRoot = Rootable![()];
        let arena = Arena::<TestRoot>::new(|_mc| ());
        #[cfg(feature = "multithreaded-gc")]
        let arena_handle = Box::leak(Box::new(dotnet_utils::gc::ArenaHandle::new(
            dotnet_utils::ArenaId(0),
        )));

        arena.mutate(|gc, _root| {
            let gc_handle = dotnet_utils::gc::GCHandle::new(
                gc,
                #[cfg(feature = "multithreaded-gc")]
                arena_handle.as_inner(),
                #[cfg(feature = "memory-validation")]
                dotnet_utils::ArenaId(0),
            );

            // Create a small object (4 bytes for Int32)
            let storage = HeapStorage::Boxed(ValueType::Int32(42));
            let obj = ObjectRef::new(gc_handle, storage);

            let ptr = ManagedPtr::new(
                obj.pointer(),
                dotnet_types::TypeDescription::NULL,
                Some(obj),
                false,
            );

            // Offset by much more than size of ValueType should panic
            unsafe {
                ptr.offset(1000);
            }
        });
    }

    #[test]
    fn test_managed_ptr_offset_valid() {
        type TestRoot = Rootable![()];
        let arena = Arena::<TestRoot>::new(|_mc| ());
        #[cfg(feature = "multithreaded-gc")]
        let arena_handle = Box::leak(Box::new(dotnet_utils::gc::ArenaHandle::new(
            dotnet_utils::ArenaId(0),
        )));

        arena.mutate(|gc, _root| {
            let gc_handle = dotnet_utils::gc::GCHandle::new(
                gc,
                #[cfg(feature = "multithreaded-gc")]
                arena_handle.as_inner(),
                #[cfg(feature = "memory-validation")]
                dotnet_utils::ArenaId(0),
            );

            let storage = HeapStorage::Boxed(ValueType::Int32(42));
            let obj = ObjectRef::new(gc_handle, storage);

            let ptr = ManagedPtr::new(
                obj.pointer(),
                dotnet_types::TypeDescription::NULL,
                Some(obj),
                false,
            );

            // Offset by 4 bytes (end of object) should be valid
            unsafe {
                ptr.offset(4);
            }
        });
    }
}

use crate::object::ObjectRef;
#[cfg(feature = "multithreaded-gc")]
use crate::object::ObjectPtr;
#[cfg(feature = "multithreaded-gc")]
use crate::ArenaId;
use dotnet_types::TypeDescription;
use dotnet_types::generics::GenericLookup;
use gc_arena::{Collect, Collection, Mutation, unsafe_empty_collect};
use std::sync::{Arc, OnceLock};
use dashmap::DashMap;
use std::sync::atomic::{AtomicU32, Ordering as AtomicOrdering};
use std::{
    cmp::Ordering as CmpOrdering,
    collections::HashSet,
    fmt::{self, Debug, Formatter},
    ptr::NonNull,
};

pub struct StaticMetadata {
    pub type_desc: TypeDescription,
    pub generics: GenericLookup,
}

fn static_registry() -> &'static DashMap<u32, Arc<StaticMetadata>> {
    static REGISTRY: OnceLock<DashMap<u32, Arc<StaticMetadata>>> = OnceLock::new();
    REGISTRY.get_or_init(DashMap::new)
}

static NEXT_STATIC_ID: AtomicU32 = AtomicU32::new(1);

#[derive(Copy, Clone, Debug, PartialEq, PartialOrd)]
pub struct UnmanagedPtr(pub NonNull<u8>);
unsafe_empty_collect!(UnmanagedPtr);

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PointerOrigin<'gc> {
    Heap(ObjectRef<'gc>),
    Stack(crate::StackSlotIndex, crate::ByteOffset),
    Static(TypeDescription, GenericLookup),
    Unmanaged,
    #[cfg(feature = "multithreaded-gc")]
    CrossArenaObjectRef(ObjectPtr, ArenaId),
    /// A value type resident on the evaluation stack (transient).
    Transient(crate::object::Object<'gc>),
}

// SAFETY: PointerOrigin contains several variants that hold GC-managed references.
// We manually implement trace and resurrect to ensure all such references (ObjectRef, Object)
// are correctly visited by the GC. Cross-arena references are recorded for coordinated GC.
unsafe impl<'gc> Collect for PointerOrigin<'gc> {
    fn trace(&self, cc: &Collection) {
        match self {
            Self::Heap(r) => r.trace(cc),
            #[cfg(feature = "multithreaded-gc")]
            Self::CrossArenaObjectRef(ptr, tid) => {
                dotnet_utils::gc::record_cross_arena_ref(*tid, ptr.as_ptr() as usize);
            }
            Self::Transient(obj) => obj.trace(cc),
            _ => {}
        }
    }
}

impl<'gc> PointerOrigin<'gc> {
    pub fn resurrect(
        &self,
        fc: &'gc gc_arena::Finalization<'gc>,
        visited: &mut HashSet<usize>,
        depth: usize,
    ) {
        match self {
            PointerOrigin::Heap(r) => r.resurrect(fc, visited, depth),
            #[cfg(feature = "multithreaded-gc")]
            PointerOrigin::CrossArenaObjectRef(ptr, tid) => {
                dotnet_utils::gc::record_cross_arena_ref(*tid, ptr.as_ptr() as usize);
            }
            PointerOrigin::Transient(obj) => obj.resurrect(fc, visited, depth),
            _ => {}
        }
    }
}

/// Stack-related metadata for a [`ManagedPtr`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManagedPtrStackInfo {
    /// The actual memory address being pointed to.
    pub address: Option<NonNull<u8>>,
    /// The offset from the base of the owner (either an object on the heap or a stack slot).
    pub offset: crate::ByteOffset,
    pub origin: PointerOrigin<'static>,
}

/// Detailed information about a [`ManagedPtr`] read from memory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManagedPtrInfo<'gc> {
    /// The actual memory address being pointed to.
    pub address: Option<NonNull<u8>>,
    pub origin: PointerOrigin<'gc>,
    /// The offset from the base of the owner (either an object on the heap or a stack slot).
    pub offset: crate::ByteOffset,
}

#[cfg(any(feature = "memory-validation", debug_assertions))]
const MANAGED_PTR_MAGIC: u32 = 0x504F_494E;

#[derive(Clone)]
#[repr(C)]
pub struct ManagedPtr<'gc> {
    #[cfg(any(feature = "memory-validation", debug_assertions))]
    pub(crate) magic: u32,
    pub(crate) _value: Option<NonNull<u8>>,
    pub origin: PointerOrigin<'gc>,
    /// The offset from the owner's base pointer.
    pub offset: crate::ByteOffset,
    pub inner_type: TypeDescription,
    pub pinned: bool,
    pub _marker: std::marker::PhantomData<&'gc ()>,
}

impl Debug for ManagedPtr<'_> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        self.validate_magic();
        write!(
            f,
            "[{}] offset: {:?} (origin: {:?}, pinned: {})",
            self.inner_type.type_name(),
            self.offset,
            self.origin,
            self.pinned
        )
    }
}

impl PartialEq for ManagedPtr<'_> {
    fn eq(&self, other: &Self) -> bool {
        self.validate_magic();
        other.validate_magic();
        self.origin == other.origin && self.offset == other.offset
    }
}

impl PartialOrd for ManagedPtr<'_> {
    fn partial_cmp(&self, other: &Self) -> Option<CmpOrdering> {
        self.validate_magic();
        other.validate_magic();
        // This is a bit arbitrary but consistent
        match format!("{:?}", self.origin).partial_cmp(&format!("{:?}", other.origin)) {
            Some(CmpOrdering::Equal) => self.offset.partial_cmp(&other.offset),
            ord => ord,
        }
    }
}

impl<'gc> ManagedPtr<'gc> {
    pub fn owner(&self) -> Option<ObjectRef<'gc>> {
        if let PointerOrigin::Heap(o) = self.origin {
            Some(o)
        } else {
            None
        }
    }

    pub fn stack_slot_origin(&self) -> Option<(crate::StackSlotIndex, crate::ByteOffset)> {
        if let PointerOrigin::Stack(idx, off) = self.origin {
            Some((idx, off))
        } else {
            None
        }
    }
    /// One word for the pointer, one word for the owner.
    pub const SIZE: usize = ObjectRef::SIZE * 2;

    #[cfg(feature = "multithreaded-gc")]
    pub fn new_cross_arena(
        value: Option<NonNull<u8>>,
        inner_type: TypeDescription,
        ptr: ObjectPtr,
        tid: ArenaId,
        offset: crate::ByteOffset,
    ) -> Self {
        Self {
            #[cfg(any(feature = "memory-validation", debug_assertions))]
            magic: MANAGED_PTR_MAGIC,
            _value: value,
            inner_type,
            origin: PointerOrigin::CrossArenaObjectRef(ptr, tid),
            offset,
            pinned: false,
            _marker: std::marker::PhantomData,
        }
    }

    pub fn new_transient(
        value: Option<NonNull<u8>>,
        inner_type: TypeDescription,
        obj: crate::object::Object<'gc>,
        offset: crate::ByteOffset,
    ) -> Self {
        Self {
            #[cfg(any(feature = "memory-validation", debug_assertions))]
            magic: MANAGED_PTR_MAGIC,
            _value: value,
            inner_type,
            origin: PointerOrigin::Transient(obj),
            offset,
            pinned: false,
            _marker: std::marker::PhantomData,
        }
    }

    pub fn new(
        value: Option<NonNull<u8>>,
        inner_type: TypeDescription,
        owner: Option<ObjectRef<'gc>>,
        pinned: bool,
        offset: Option<crate::ByteOffset>,
    ) -> Self {
        let owner = owner.and_then(|o| if o.0.is_some() { Some(o) } else { None });
        let offset = offset.unwrap_or(crate::ByteOffset(0));
        let origin = if let Some(o) = owner {
            PointerOrigin::Heap(o)
        } else {
            PointerOrigin::Unmanaged
        };

        Self {
            #[cfg(any(feature = "memory-validation", debug_assertions))]
            magic: MANAGED_PTR_MAGIC,
            _value: value,
            inner_type,
            origin,
            offset,
            pinned,
            _marker: std::marker::PhantomData,
        }
    }


    pub fn from_info(info: ManagedPtrInfo<'gc>, inner_type: TypeDescription) -> Self {
        Self {
            #[cfg(any(feature = "memory-validation", debug_assertions))]
            magic: MANAGED_PTR_MAGIC,
            _value: info.address,
            inner_type,
            origin: info.origin,
            offset: info.offset,
            pinned: false,
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
    /// The `source` slice must be at least `ManagedPtr::SIZE` bytes long.
    pub unsafe fn read_stack_info(source: &[u8]) -> ManagedPtrStackInfo {
        let ptr_size = ObjectRef::SIZE;
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
                origin: PointerOrigin::Stack(crate::StackSlotIndex(idx), crate::ByteOffset(off)),
            }
        } else {
            // Heap pointer or Static/Unmanaged
            // We can't fully reconstruct Static/Heap without GCHandle, so we mark as Unmanaged
            // for stack info purposes.
            ManagedPtrStackInfo {
                address: NonNull::new(word1 as *mut u8),
                offset: crate::ByteOffset(word1),
                origin: PointerOrigin::Unmanaged,
            }
        }
    }

    /// Read pointer and owner from memory, branded with a GCHandle.
    /// Returns detailed metadata about the pointer. Type info must be supplied by caller.
    ///
    /// # Safety
    ///
    /// The `source` slice must be at least `ManagedPtr::SIZE` bytes long.
    /// It must contain valid bytes representing a `ManagedPtr`.
    pub unsafe fn read_branded(source: &[u8], _gc: &Mutation<'gc>) -> ManagedPtrInfo<'gc> {
        unsafe { Self::read_unchecked(source) }
    }

    /// Read pointer and owner from memory.
    /// Returns detailed metadata about the pointer. Type info must be supplied by caller.
    ///
    /// # Safety
    ///
    /// The `source` slice must be at least `ManagedPtr::SIZE` bytes long.
    /// It must contain valid bytes representing a `ManagedPtr`.
    pub unsafe fn read_unchecked(source: &[u8]) -> ManagedPtrInfo<'gc> {
        let ptr_size = ObjectRef::SIZE;

        let mut word0_bytes = [0u8; ObjectRef::SIZE];
        word0_bytes.copy_from_slice(&source[0..ptr_size]);
        let word0 = usize::from_ne_bytes(word0_bytes);

        let mut word1_bytes = [0u8; ObjectRef::SIZE];
        word1_bytes.copy_from_slice(&source[ptr_size..ptr_size * 2]);
        let word1 = usize::from_ne_bytes(word1_bytes);

        if word0 & 1 != 0 {
            // Tagged pointer or Stack
            let tag = word0 & 7;
            match tag {
                1 | 5 => {
                    // Stack (historical tags 1 and 5 used for stack)
                    // We preserve the 33-bit offset and 30-bit slot index
                    let slot_idx = (word0 >> 3) & 0x3FFFFFFF;
                    let slot_offset = word0 >> 33;
                    let raw_ptr = NonNull::new(word1 as *mut u8);
                    ManagedPtrInfo {
                        address: raw_ptr,
                        origin: PointerOrigin::Stack(
                            crate::StackSlotIndex(slot_idx),
                            crate::ByteOffset(slot_offset),
                        ),
                        offset: crate::ByteOffset(slot_offset),
                    }
                }
                3 => {
                    // ValueType - Metadata lost, recover as unmanaged
                    let raw_ptr = NonNull::new(word1 as *mut u8);
                    ManagedPtrInfo {
                        address: raw_ptr,
                        origin: PointerOrigin::Unmanaged,
                        offset: crate::ByteOffset::ZERO,
                    }
                }
                7 => {
                    // Static with metadata (if ID > 0) or Unmanaged
                    let id = ((word0 >> 3) & 0xFFFFFFFF) as u32;
                    let slot_offset = word0 >> 35;
                    let raw_ptr = NonNull::new(word1 as *mut u8);

                    if id > 0 && let Some(meta) = static_registry().get(&id) {
                        ManagedPtrInfo {
                            address: raw_ptr,
                            origin: PointerOrigin::Static(
                                meta.type_desc,
                                meta.generics.clone(),
                            ),
                            offset: crate::ByteOffset(slot_offset),
                        }
                    } else {
                        // Unmanaged or Static (metadata lost)
                        ManagedPtrInfo {
                            address: raw_ptr,
                            origin: PointerOrigin::Unmanaged,
                            offset: crate::ByteOffset::ZERO,
                        }
                    }
                }
                _ => {
                    // Fallback for old stack format (word0 & 1 != 0)
                    let slot_idx = (word0 >> 1) & 0xFFFFFFFF;
                    let slot_offset = word0 >> 33;
                    let raw_ptr = NonNull::new(word1 as *mut u8);
                    ManagedPtrInfo {
                        address: raw_ptr,
                        origin: PointerOrigin::Stack(
                            crate::StackSlotIndex(slot_idx),
                            crate::ByteOffset(slot_offset),
                        ),
                        offset: crate::ByteOffset(slot_offset),
                    }
                }
            }
        } else {
            // Memory layout: (Owner ObjectRef at offset 0, Offset at offset 8)
            // Read Owner (Offset 0)
            let owner = unsafe { ObjectRef::read_unchecked(&source[0..ptr_size]) };

            // Read Offset (Offset 8)
            let offset = crate::ByteOffset(word1);

            // Compute pointer from owner's data + offset
            let ptr = if let Some(handle) = owner.0 {
                let base_ptr = unsafe { handle.borrow().storage.raw_data_ptr() };
                if base_ptr.is_null() {
                    None
                } else {
                    NonNull::new(base_ptr.wrapping_add(offset.as_usize()))
                }
            } else if offset == crate::ByteOffset::ZERO {
                // Null owner with zero offset means null pointer
                None
            } else {
                // Null owner but non-zero offset - this is a static data pointer or unmanaged
                // Store raw pointer directly in word1 for absolute addresses
                NonNull::new(offset.as_usize() as *mut u8)
            };

            ManagedPtrInfo {
                address: ptr,
                origin: owner.0.map_or(PointerOrigin::Unmanaged, |h| PointerOrigin::Heap(ObjectRef(Some(h)))),
                offset,
            }
        }
    }

    /// Write owner and offset to memory.
    /// Memory layout: (Owner ObjectRef at offset 0, Offset at offset 8)
    pub fn write(&self, dest: &mut [u8]) {
        self.validate_magic();
        let ptr_size = ObjectRef::SIZE;

        match &self.origin {
            PointerOrigin::Stack(slot_idx, slot_offset) => {
                let word0: usize =
                    1 | ((slot_idx.as_usize() & 0x3FFFFFFF) << 3) | (slot_offset.as_usize() << 33);
                let word1 = self._value.map_or(0, |p| p.as_ptr() as usize);
                dest[0..ptr_size].copy_from_slice(&word0.to_ne_bytes());
                dest[ptr_size..ptr_size * 2].copy_from_slice(&word1.to_ne_bytes());
            }
            PointerOrigin::Heap(owner) => {
                owner.write(&mut dest[0..ptr_size]);
                let offset_bytes = self.offset.as_usize().to_ne_bytes();
                dest[ptr_size..ptr_size * 2].copy_from_slice(&offset_bytes);
            }
            PointerOrigin::Static(type_desc, generics) => {
                // Register metadata to get an ID
                // TODO: Optimization to avoid duplicate registrations for the same type/generics
                let id = NEXT_STATIC_ID.fetch_add(1, AtomicOrdering::SeqCst);
                static_registry().insert(
                    id,
                    Arc::new(StaticMetadata {
                        type_desc: *type_desc,
                        generics: generics.clone(),
                    }),
                );

                let word0: usize = 7
                    | ((id as usize & 0xFFFFFFFF) << 3)
                    | (self.offset.as_usize() << 35);
                let word1 = self._value.map_or(0, |p| p.as_ptr() as usize);
                dest[0..ptr_size].copy_from_slice(&word0.to_ne_bytes());
                dest[ptr_size..ptr_size * 2].copy_from_slice(&word1.to_ne_bytes());
            }
            PointerOrigin::Unmanaged => {
                // For unmanaged, we write 7 to word0 (tag 7, id 0) and the absolute pointer to word1
                let word0: usize = 7;
                let word1 = self._value.map_or(0, |p| p.as_ptr() as usize);
                dest[0..ptr_size].copy_from_slice(&word0.to_ne_bytes());
                dest[ptr_size..ptr_size * 2].copy_from_slice(&word1.to_ne_bytes());
            }
            #[cfg(feature = "multithreaded-gc")]
            PointerOrigin::CrossArenaObjectRef(ptr, _) => {
                // For cross-arena, treat as unmanaged for now when serializing to memory.
                let word0: usize = 0;
                let word1 = ptr.as_ptr() as usize + self.offset.as_usize();
                dest[0..ptr_size].copy_from_slice(&word0.to_ne_bytes());
                dest[ptr_size..ptr_size * 2].copy_from_slice(&word1.to_ne_bytes());
            }
            PointerOrigin::Transient(_) => {
                // Transient objects cannot be easily serialized to memory as they don't have a stable address.
                // We treat them as unmanaged and hope the caller isn't doing anything too crazy.
                // In practice, ManagedPtrs to transient stack values should not be stored in heap memory.
                let word0: usize = 0;
                let word1 = self._value.map_or(0, |p| p.as_ptr() as usize);
                dest[0..ptr_size].copy_from_slice(&word0.to_ne_bytes());
                dest[ptr_size..ptr_size * 2].copy_from_slice(&word1.to_ne_bytes());
            }
        }
    }

    #[deprecated(note = "Use with_data instead to ensure memory safety and lock protection")]
    pub fn pointer(&self) -> Option<NonNull<u8>> {
        self.validate_magic();
        if let Some(owner) = self.owner() {
            let handle = owner.0?;
            let base_ptr = unsafe { handle.borrow().storage.raw_data_ptr() };
            if base_ptr.is_null() {
                None
            } else {
                NonNull::new(base_ptr.wrapping_add(self.offset.as_usize()))
            }
        } else {
            // For both stack and static/absolute pointers, use the cached value.
            // Stack pointers should have their cached value updated on stack reallocation.
            // Static/absolute pointers are stable or don't have enough info to re-resolve.
            self._value
        }
    }

    /// Safely accesses the data pointed to by this ManagedPtr.
    ///
    /// # Safety
    /// If there is no owner, this method assumes the cached pointer is valid for at least `size` bytes.
    pub unsafe fn with_data<T>(&self, size: usize, f: impl FnOnce(&[u8]) -> T) -> T {
        self.validate_magic();
        if let Some(owner) = self.owner() {
            let handle = owner.0.expect("ManagedPtr::with_data: null owner handle");
            let inner = handle.borrow();
            let ptr = unsafe { inner.storage.raw_data_ptr() };
            let slice = unsafe { std::slice::from_raw_parts(ptr, inner.storage.size_bytes()) };
            let offset = self.offset.as_usize();
            let available = slice.len().saturating_sub(offset);
            let to_access = std::cmp::min(size, available);
            f(&slice[offset..offset + to_access])
        } else if let PointerOrigin::Transient(obj) = &self.origin {
            obj.with_data(|data| {
                let offset = self.offset.as_usize();
                let available = data.len().saturating_sub(offset);
                let to_access = std::cmp::min(size, available);
                f(&data[offset..offset + to_access])
            })
        } else {
            // SAFETY: Caller must ensure the pointer is valid.
            // Inline pointer resolution to avoid calling deprecated pointer()
            let ptr = if let Some((_idx, _offset)) = self.stack_slot_origin() {
                // Stack pointer - use cached value
                self._value
                    .expect("ManagedPtr::with_data: null stack pointer")
            } else {
                // Static data pointer or absolute pointer - use cached value
                self._value.expect("ManagedPtr::with_data: null pointer")
            };
            let slice = unsafe { std::slice::from_raw_parts(ptr.as_ptr(), size) };
            f(slice)
        }
    }

    pub fn update_cached_ptr(&mut self, ptr: NonNull<u8>) {
        self.validate_magic();
        self._value = Some(ptr);
        if let PointerOrigin::Unmanaged = self.origin {
            // If it's a static pointer, also update the offset
            self.offset = crate::ByteOffset(ptr.as_ptr() as usize);
        }
    }

    pub fn new_static(
        value: Option<NonNull<u8>>,
        inner_type: TypeDescription,
        type_desc: TypeDescription,
        generics: GenericLookup,
        pinned: bool,
        offset: crate::ByteOffset,
    ) -> Self {
        Self {
            #[cfg(any(feature = "memory-validation", debug_assertions))]
            magic: MANAGED_PTR_MAGIC,
            _value: value,
            inner_type,
            origin: PointerOrigin::Static(type_desc, generics),
            offset,
            pinned,
            _marker: std::marker::PhantomData,
        }
    }

    pub fn map_value(
        self,
        transform: impl FnOnce(Option<NonNull<u8>>) -> Option<NonNull<u8>>,
    ) -> Self {
        self.validate_magic();
        let old_ptr = if let Some(owner) = self.owner() {
            owner.0.and_then(|h| {
                let base_ptr = unsafe { h.borrow().storage.raw_data_ptr() };
                if base_ptr.is_null() {
                    None
                } else {
                    NonNull::new(base_ptr.wrapping_add(self.offset.as_usize()))
                }
            })
        } else {
            self._value
        };
        let new_value = transform(old_ptr);
        let mut m = self.clone();
        m._value = new_value;
        if let PointerOrigin::Stack(_idx, ref mut offset) = m.origin {
            let diff = if let (Some(new_p), Some(old_p)) = (new_value, old_ptr) {
                (new_p.as_ptr() as isize).wrapping_sub(old_p.as_ptr() as isize)
            } else {
                0
            };
            *offset = crate::ByteOffset((offset.as_usize() as isize + diff) as usize);
            m.offset = crate::ByteOffset((m.offset.as_usize() as isize + diff) as usize);
        }
        m
    }

    #[cfg(feature = "memory-validation")]
    fn validate_offset(&self, bytes: isize) {
        if let PointerOrigin::Heap(owner) = self.origin
            && let Some(handle) = owner.0
        {
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

    /// # Safety
    ///
    /// The caller must ensure that the resulting pointer is within the bounds of the same
    /// allocated object as the original pointer.
    pub unsafe fn offset(self, bytes: isize) -> Self {
        #[cfg(feature = "memory-validation")]
        self.validate_offset(bytes);

        let mut m = self;
        m.offset = crate::ByteOffset((m.offset.as_usize() as isize + bytes) as usize);
        m._value = m
            ._value
            .map(|p| unsafe { NonNull::new_unchecked(p.as_ptr().offset(bytes)) });
        if let PointerOrigin::Stack(_idx, ref mut off) = m.origin {
            *off = crate::ByteOffset((off.as_usize() as isize + bytes) as usize);
        }
        m
    }

    pub fn with_stack_origin(
        mut self,
        slot_index: crate::StackSlotIndex,
        offset: crate::ByteOffset,
    ) -> Self {
        self.validate_magic();
        self.origin = PointerOrigin::Stack(slot_index, offset);
        self
    }

    pub fn is_stack_local(&self) -> bool {
        self.validate_magic();
        matches!(self.origin, PointerOrigin::Stack(_, _))
    }

    /// Returns true if this ManagedPtr represents a null pointer.
    /// A null pointer has no owner, no stack origin, and zero offset.
    pub fn is_null(&self) -> bool {
        self.validate_magic();
        match &self.origin {
            PointerOrigin::Unmanaged => self.offset == crate::ByteOffset::ZERO,
            PointerOrigin::Heap(o) => o.0.is_none(),
            _ => false,
        }
    }
}

unsafe impl<'gc> Collect for ManagedPtr<'gc> {
    fn trace(&self, cc: &Collection) {
        self.validate_magic();
        self.origin.trace(cc);
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
        self.origin.resurrect(fc, visited, depth);
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
            ArenaId(0),
        )));

        arena.mutate(|gc, _root| {
            let gc_handle = dotnet_utils::gc::GCHandle::new(
                gc,
                #[cfg(feature = "multithreaded-gc")]
                arena_handle.as_inner(),
                #[cfg(feature = "memory-validation")]
                ArenaId(0),
            );

            // Create a small object (4 bytes for Int32)
            let storage = HeapStorage::Boxed(ValueType::Int32(42));
            let obj = ObjectRef::new(gc_handle, storage);

            let ptr = ManagedPtr::new(
                obj.with_data(|d| NonNull::new(d.as_ptr() as *mut u8)),
                TypeDescription::NULL,
                Some(obj),
                false,
                Some(crate::ByteOffset(0)),
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
            ArenaId(0),
        )));

        arena.mutate(|gc, _root| {
            let gc_handle = dotnet_utils::gc::GCHandle::new(
                gc,
                #[cfg(feature = "multithreaded-gc")]
                arena_handle.as_inner(),
                #[cfg(feature = "memory-validation")]
                ArenaId(0),
            );

            let storage = HeapStorage::Boxed(ValueType::Int32(42));
            let obj = ObjectRef::new(gc_handle, storage);

            let ptr = ManagedPtr::new(
                obj.with_data(|d| NonNull::new(d.as_ptr() as *mut u8)),
                TypeDescription::NULL,
                Some(obj),
                false,
                Some(crate::ByteOffset(0)),
            );

            // Offset by 4 bytes (end of object) should be valid
            unsafe {
                ptr.offset(4);
            }
        });
    }
}

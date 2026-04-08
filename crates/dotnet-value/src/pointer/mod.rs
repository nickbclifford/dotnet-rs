use crate::{
    ByteOffset, StackSlotIndex, ValidationTag,
    object::{Object, ObjectRef},
    ptr_common::PointerLike,
};
use dashmap::DashMap;
use dotnet_types::{TypeDescription, generics::GenericLookup};
use dotnet_utils::ManagedByteOffset;
use gc_arena::{Collect, collect::Trace, static_collect};
use sptr::Strict;
use std::{
    cmp::Ordering as CmpOrdering,
    fmt::{self, Debug, Formatter},
    ptr::NonNull,
    sync::{
        Arc, OnceLock,
        atomic::{AtomicU32, Ordering as AtomicOrdering},
    },
};

#[cfg(feature = "fuzzing")]
use arbitrary::Arbitrary;

pub mod cross_arena;
pub mod origin;
pub mod serde;

pub use origin::*;

#[inline]
pub(crate) fn nonnull_from_exposed_addr(addr: usize) -> Option<NonNull<u8>> {
    NonNull::new(sptr::from_exposed_addr_mut::<u8>(addr))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StaticMetadata {
    pub type_desc: TypeDescription,
    pub generics: GenericLookup,
}

pub(crate) fn static_registry() -> &'static DashMap<u32, Arc<StaticMetadata>> {
    static REGISTRY: OnceLock<DashMap<u32, Arc<StaticMetadata>>> = OnceLock::new();
    REGISTRY.get_or_init(DashMap::new)
}

pub(crate) fn static_dedup_map() -> &'static DashMap<(TypeDescription, GenericLookup), u32> {
    static DEDUP: OnceLock<DashMap<(TypeDescription, GenericLookup), u32>> = OnceLock::new();
    DEDUP.get_or_init(DashMap::new)
}

pub fn reset_static_registry() {
    static_registry().clear();
    static_dedup_map().clear();
    NEXT_STATIC_ID.store(1, AtomicOrdering::SeqCst);
}

pub(crate) static NEXT_STATIC_ID: AtomicU32 = AtomicU32::new(1);

#[derive(Copy, Clone, Debug, PartialEq, PartialOrd)]
pub struct UnmanagedPtr(pub NonNull<u8>);

#[cfg(feature = "fuzzing")]
impl<'a> Arbitrary<'a> for UnmanagedPtr {
    fn arbitrary(u: &mut arbitrary::Unstructured<'a>) -> arbitrary::Result<Self> {
        let ptr_val: usize = u.arbitrary()?;
        Ok(UnmanagedPtr(
            nonnull_from_exposed_addr(ptr_val).unwrap_or(NonNull::dangling()),
        ))
    }
}
static_collect!(UnmanagedPtr);

/// Stack-related metadata for a [`ManagedPtr`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManagedPtrStackInfo {
    /// The actual memory address being pointed to.
    pub address: Option<NonNull<u8>>,
    /// The offset from the base of the owner (either an object on the heap or a stack slot).
    pub offset: ByteOffset,
    pub origin: PointerOrigin<'static>,
}

#[cfg(feature = "fuzzing")]
impl<'a> Arbitrary<'a> for ManagedPtrStackInfo {
    fn arbitrary(u: &mut arbitrary::Unstructured<'a>) -> arbitrary::Result<Self> {
        let ptr_val: usize = u.arbitrary()?;
        Ok(Self {
            address: nonnull_from_exposed_addr(ptr_val),
            offset: u.arbitrary()?,
            origin: u.arbitrary()?,
        })
    }
}

pub use dotnet_types::error::PointerDeserializationError;

/// Detailed information about a [`ManagedPtr`] read from memory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManagedPtrInfo<'gc> {
    /// The actual memory address being pointed to.
    pub address: Option<NonNull<u8>>,
    pub origin: PointerOrigin<'gc>,
    /// The offset from the base of the owner (either an object on the heap or a stack slot).
    pub offset: ByteOffset,
}

#[cfg(feature = "fuzzing")]
impl<'a, 'gc> Arbitrary<'a> for ManagedPtrInfo<'gc> {
    fn arbitrary(u: &mut arbitrary::Unstructured<'a>) -> arbitrary::Result<Self> {
        let ptr_val: usize = u.arbitrary()?;
        Ok(Self {
            address: nonnull_from_exposed_addr(ptr_val),
            origin: u.arbitrary()?,
            offset: u.arbitrary()?,
        })
    }
}

impl<'gc> ManagedPtrInfo<'gc> {
    pub fn owner(&self) -> Option<ObjectRef<'gc>> {
        self.origin.owner()
    }
}

pub(crate) const MANAGED_PTR_MAGIC: u32 = 0x504F_494E;

const MANAGED_PTR_FLAG_PINNED: u8 = 0b0000_0001;
const MANAGED_PTR_FLAG_UNMANAGED_INLINE_OFFSET: u8 = 0b0000_0010;

#[derive(Clone)]
#[repr(C)]
pub struct ManagedPtr<'gc> {
    pub(crate) magic: ValidationTag,
    pub(crate) _value: Option<NonNull<u8>>,
    pub(crate) origin: PointerOrigin<'gc>,
    /// Compact byte offset for managed origins; unmanaged absolute addresses are read from `_value`.
    pub(crate) offset: ManagedByteOffset,
    pub(crate) inner_type: TypeDescription,
    pub(crate) flags: u8,
    pub(crate) _marker: std::marker::PhantomData<&'gc ()>,
}

#[cfg(feature = "fuzzing")]
impl<'a, 'gc> Arbitrary<'a> for ManagedPtr<'gc> {
    fn arbitrary(u: &mut arbitrary::Unstructured<'a>) -> arbitrary::Result<Self> {
        let ptr_val: usize = u.arbitrary()?;
        let origin: PointerOrigin<'gc> = u.arbitrary()?;
        let offset = u.arbitrary()?;
        let pinned: bool = u.arbitrary()?;
        let value = nonnull_from_exposed_addr(ptr_val);
        let packed_offset = Self::pack_offset(&origin, value, offset);
        let unmanaged_inline_offset = matches!(origin, PointerOrigin::Unmanaged)
            && packed_offset.as_usize() == offset.as_usize();
        Ok(Self {
            magic: ValidationTag::new(MANAGED_PTR_MAGIC as u64),
            _value: value,
            offset: packed_offset,
            origin,
            inner_type: u.arbitrary()?,
            flags: Self::with_unmanaged_inline_offset_flag(
                Self::flags_from_pinned(pinned),
                unmanaged_inline_offset,
            ),
            _marker: std::marker::PhantomData,
        })
    }
}

impl Debug for ManagedPtr<'_> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        self.validate_magic();
        write!(
            f,
            "[{}] offset: {:?} (origin: {:?}, pinned: {})",
            self.inner_type.type_name(),
            self.byte_offset(),
            self.origin,
            self.is_pinned()
        )
    }
}

impl PartialEq for ManagedPtr<'_> {
    fn eq(&self, other: &Self) -> bool {
        self.validate_magic();
        other.validate_magic();
        self.origin == other.origin && self.byte_offset() == other.byte_offset()
    }
}

impl PartialOrd for ManagedPtr<'_> {
    fn partial_cmp(&self, other: &Self) -> Option<CmpOrdering> {
        self.validate_magic();
        other.validate_magic();

        // 1. Compare addresses (computed value)
        match (self._value, other._value) {
            (Some(a), Some(b)) => match a.as_ptr().partial_cmp(&b.as_ptr()) {
                Some(CmpOrdering::Equal) => {}
                ord => return ord,
            },
            (None, None) => {}
            (None, _) => return Some(CmpOrdering::Less),
            (_, None) => return Some(CmpOrdering::Greater),
        }

        // 2. Compare origins as tie-breaker
        match self.origin.discriminant().cmp(&other.origin.discriminant()) {
            CmpOrdering::Equal => {
                // If they are the same variant, we use offset as a tie-breaker.
                // We could compare the inner origin data, but that would require
                // implementing PartialOrd for TypeDescription, etc.
                self.byte_offset().partial_cmp(&other.byte_offset())
            }
            ord => Some(ord),
        }
    }
}

impl<'gc> ManagedPtr<'gc> {
    #[inline]
    fn flags_from_pinned(pinned: bool) -> u8 {
        if pinned { MANAGED_PTR_FLAG_PINNED } else { 0 }
    }

    #[inline]
    fn with_unmanaged_inline_offset_flag(flags: u8, enabled: bool) -> u8 {
        if enabled {
            flags | MANAGED_PTR_FLAG_UNMANAGED_INLINE_OFFSET
        } else {
            flags & !MANAGED_PTR_FLAG_UNMANAGED_INLINE_OFFSET
        }
    }

    #[inline]
    fn has_unmanaged_inline_offset(&self) -> bool {
        self.flags & MANAGED_PTR_FLAG_UNMANAGED_INLINE_OFFSET != 0
    }

    #[inline]
    fn pack_offset(
        origin: &PointerOrigin<'gc>,
        value: Option<NonNull<u8>>,
        offset: ByteOffset,
    ) -> ManagedByteOffset {
        if matches!(origin, PointerOrigin::Unmanaged) {
            // Unmanaged pointers keep compact offset when representable; otherwise `_value`
            // remains the authoritative absolute address.
            return ManagedByteOffset::try_from(offset.as_usize())
                .unwrap_or(ManagedByteOffset::ZERO);
        }

        match ManagedByteOffset::try_from(offset.as_usize()) {
            Ok(compact) => compact,
            Err(_) => {
                // For unmanaged pointers we preserve full-width absolute address in `_value`.
                // Any managed origin offset that exceeds u32 is unsupported by the compact form.
                let resolved = value.map(|p| p.as_ptr().expose_addr()).unwrap_or(0);
                panic!(
                    "ManagedPtr offset exceeds compact range: offset={}, origin={:?}, value=0x{resolved:X}",
                    offset.as_usize(),
                    origin
                );
            }
        }
    }

    #[inline]
    fn unpack_offset(&self) -> ByteOffset {
        if matches!(self.origin, PointerOrigin::Unmanaged) {
            if self.has_unmanaged_inline_offset() {
                ByteOffset(self.offset.as_usize())
            } else {
                ByteOffset(self._value.map_or(0, |p| p.as_ptr().expose_addr()))
            }
        } else {
            ByteOffset(self.offset.as_usize())
        }
    }

    // Read-only accessors
    pub fn origin(&self) -> &PointerOrigin<'gc> {
        &self.origin
    }
    pub fn inner_type(&self) -> TypeDescription {
        self.inner_type.clone()
    }
    pub fn is_pinned(&self) -> bool {
        self.flags & MANAGED_PTR_FLAG_PINNED != 0
    }
    pub fn byte_offset(&self) -> ByteOffset {
        self.unpack_offset()
    }

    pub fn into_info(self) -> ManagedPtrInfo<'gc> {
        let offset = self.unpack_offset();
        ManagedPtrInfo {
            address: self._value,
            origin: self.origin,
            offset,
        }
    }

    pub fn with_origin(&self, origin: PointerOrigin<'gc>) -> Self {
        let mut clone = self.clone();
        let byte_offset = self.byte_offset();
        let packed_offset = Self::pack_offset(&origin, clone._value, byte_offset);
        clone.origin = origin;
        clone.offset = packed_offset;
        clone.flags = Self::with_unmanaged_inline_offset_flag(
            clone.flags,
            matches!(clone.origin, PointerOrigin::Unmanaged)
                && packed_offset.as_usize() == byte_offset.as_usize(),
        );
        clone
    }

    pub fn with_inner_type(&self, inner_type: TypeDescription) -> Self {
        Self {
            inner_type,
            ..self.clone()
        }
    }

    pub fn owner(&self) -> Option<ObjectRef<'gc>> {
        match &self.origin {
            PointerOrigin::Heap(o) => Some(*o),
            _ => None,
        }
    }

    pub fn stack_slot_origin(&self) -> Option<(StackSlotIndex, ByteOffset)> {
        if let PointerOrigin::Stack(idx) = self.origin {
            Some((idx, self.byte_offset()))
        } else {
            None
        }
    }

    pub fn new_transient(
        value: Option<NonNull<u8>>,
        inner_type: TypeDescription,
        obj: Object<'gc>,
        offset: ByteOffset,
    ) -> Self {
        let origin = PointerOrigin::new_transient(obj);
        Self {
            magic: ValidationTag::new(MANAGED_PTR_MAGIC as u64),
            _value: value,
            inner_type,
            offset: Self::pack_offset(&origin, value, offset),
            origin,
            flags: 0,
            _marker: std::marker::PhantomData,
        }
    }

    pub fn new(
        value: Option<NonNull<u8>>,
        inner_type: TypeDescription,
        owner: Option<ObjectRef<'gc>>,
        pinned: bool,
        offset: Option<ByteOffset>,
    ) -> Self {
        let owner = owner.and_then(|o| if o.0.is_some() { Some(o) } else { None });
        let origin = if let Some(o) = owner {
            PointerOrigin::Heap(o)
        } else {
            PointerOrigin::Unmanaged
        };
        let offset = offset.unwrap_or_else(|| {
            if let PointerOrigin::Unmanaged = origin {
                ByteOffset(value.map_or(0, |p| p.as_ptr() as usize))
            } else {
                ByteOffset(0)
            }
        });
        let packed_offset = Self::pack_offset(&origin, value, offset);
        let unmanaged_inline_offset = matches!(origin, PointerOrigin::Unmanaged)
            && packed_offset.as_usize() == offset.as_usize();

        Self {
            magic: ValidationTag::new(MANAGED_PTR_MAGIC as u64),
            _value: value,
            inner_type,
            offset: packed_offset,
            origin,
            flags: Self::with_unmanaged_inline_offset_flag(
                Self::flags_from_pinned(pinned),
                unmanaged_inline_offset,
            ),
            _marker: std::marker::PhantomData,
        }
    }

    pub fn null() -> Self {
        Self {
            magic: ValidationTag::new(MANAGED_PTR_MAGIC as u64),
            _value: None,
            inner_type: TypeDescription::NULL,
            origin: PointerOrigin::Unmanaged,
            offset: ManagedByteOffset::ZERO,
            flags: 0,
            _marker: std::marker::PhantomData,
        }
    }

    pub fn from_info_full(
        info: ManagedPtrInfo<'gc>,
        inner_type: TypeDescription,
        pinned: bool,
    ) -> Self {
        let mut origin = info.origin.normalize();
        let offset = info.offset;
        let value = info.address;

        if origin.is_null() && offset == ByteOffset::ZERO && value.is_none() {
            origin = PointerOrigin::Unmanaged;
        }
        let packed_offset = Self::pack_offset(&origin, value, offset);
        let unmanaged_inline_offset = matches!(origin, PointerOrigin::Unmanaged)
            && packed_offset.as_usize() == offset.as_usize();

        Self {
            magic: ValidationTag::new(MANAGED_PTR_MAGIC as u64),
            _value: value,
            inner_type,
            offset: packed_offset,
            origin,
            flags: Self::with_unmanaged_inline_offset_flag(
                Self::flags_from_pinned(pinned),
                unmanaged_inline_offset,
            ),
            _marker: std::marker::PhantomData,
        }
    }

    pub(crate) fn validate_magic(&self) {
        self.magic.validate(MANAGED_PTR_MAGIC as u64, "ManagedPtr");
    }

    /// Safely accesses the data pointed to by this ManagedPtr.
    ///
    /// # Safety
    /// If there is no owner, this method assumes the cached pointer is valid for at least `size` bytes.
    pub unsafe fn with_data<T>(&self, size: usize, f: impl FnOnce(&[u8]) -> T) -> T {
        self.validate_magic();
        match &self.origin {
            PointerOrigin::Heap(owner) => {
                let handle = owner
                    .0
                    .expect("ManagedPtr::with_data: null heap owner handle");
                let inner = handle.borrow();
                // SAFETY: `inner` holds a shared lock on the object storage;
                // we only read the raw base pointer.
                let ptr = unsafe { inner.storage.raw_data_ptr() };
                // SAFETY: `ptr` points to `inner.storage` which remains locked
                // for this scope, and `size_bytes()` is the exact allocation size.
                let slice = unsafe { std::slice::from_raw_parts(ptr, inner.storage.size_bytes()) };
                let offset = self.byte_offset().as_usize();
                let available = slice.len().saturating_sub(offset);
                let to_access = std::cmp::min(size, available);
                f(&slice[offset..offset + to_access])
            }
            #[cfg(feature = "multithreading")]
            PointerOrigin::CrossArenaObjectRef(ptr, _) => ptr.with_data(|data| {
                let offset = self.byte_offset().as_usize();
                let available = data.len().saturating_sub(offset);
                let to_access = std::cmp::min(size, available);
                f(&data[offset..offset + to_access])
            }),
            PointerOrigin::Transient(obj) => obj.with_data(|data| {
                let offset = self.byte_offset().as_usize();
                let available = data.len().saturating_sub(offset);
                let to_access = std::cmp::min(size, available);
                f(&data[offset..offset + to_access])
            }),
            _ => {
                // SAFETY: Caller must ensure the pointer is valid.
                // Inline pointer resolution to avoid lockless aliasing.
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
    }

    pub fn update_cached_ptr(&mut self, ptr: NonNull<u8>) {
        self.validate_magic();
        self._value = Some(ptr);
        if let PointerOrigin::Unmanaged = self.origin {
            self.offset = ManagedByteOffset::ZERO;
            self.flags = Self::with_unmanaged_inline_offset_flag(self.flags, false);
        }
    }

    pub fn new_static(
        value: Option<NonNull<u8>>,
        inner_type: TypeDescription,
        type_desc: TypeDescription,
        generics: GenericLookup,
        pinned: bool,
        offset: ByteOffset,
    ) -> Self {
        let origin = PointerOrigin::new_static(type_desc, generics);
        Self {
            magic: ValidationTag::new(MANAGED_PTR_MAGIC as u64),
            _value: value,
            inner_type,
            offset: Self::pack_offset(&origin, value, offset),
            origin,
            flags: Self::flags_from_pinned(pinned),
            _marker: std::marker::PhantomData,
        }
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
            let current_offset = self.byte_offset().as_usize();
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
        let current_offset = m.byte_offset().as_usize();
        let new_offset_isize = (current_offset as isize)
            .checked_add(bytes)
            .expect("ManagedPtr::offset overflow");

        if new_offset_isize < 0 {
            panic!(
                "ManagedPtr::offset: negative result (offset={}, bytes={})",
                current_offset, bytes
            );
        }

        let new_offset = ByteOffset(new_offset_isize as usize);
        let packed_offset = Self::pack_offset(&m.origin, m._value, new_offset);
        m.flags = Self::with_unmanaged_inline_offset_flag(
            m.flags,
            matches!(m.origin, PointerOrigin::Unmanaged)
                && packed_offset.as_usize() == new_offset.as_usize(),
        );
        m.offset = packed_offset;
        m._value = m
            ._value
            .map(|p| unsafe { NonNull::new_unchecked(p.as_ptr().offset(bytes)) });
        m
    }

    pub fn with_stack_origin(mut self, slot_index: StackSlotIndex, _offset: ByteOffset) -> Self {
        self.validate_magic();
        let byte_offset = self.byte_offset();
        self.origin = PointerOrigin::Stack(slot_index);
        self.offset = Self::pack_offset(&self.origin, self._value, byte_offset);
        self.flags = Self::with_unmanaged_inline_offset_flag(self.flags, false);
        self
    }

    /// Returns true if this ManagedPtr represents a null pointer.
    /// A null pointer has no owner, no stack origin, and zero offset.
    pub fn is_null(&self) -> bool {
        self.validate_magic();
        // A canonical null ManagedPtr MUST have origin Unmanaged, offset 0, AND no cached address.
        // We also check for non-canonical nulls (like Heap(None)) for robustness.
        match &self.origin {
            PointerOrigin::Unmanaged => self._value.is_none(),
            PointerOrigin::Heap(o) => o.0.is_none(),
            _ => false,
        }
    }
}

unsafe impl<'gc> Collect<'gc> for ManagedPtr<'gc> {
    fn trace<Tr: Trace<'gc>>(&self, cc: &mut Tr) {
        self.origin.trace(cc);
    }
}

impl<'gc> ManagedPtr<'gc> {
    pub fn resurrect(
        &self,
        fc: &'gc gc_arena::Finalization<'gc>,
        visited: &mut std::collections::HashSet<usize>,
        depth: usize,
    ) {
        self.validate_magic();
        self.origin.resurrect(fc, visited, depth);
    }
}

impl<'gc> PointerLike for ManagedPtr<'gc> {
    #[inline]
    fn pointer(&self) -> Option<NonNull<u8>> {
        self.validate_magic();
        match &self.origin {
            PointerOrigin::Heap(owner) => {
                let handle = owner.0?;
                // SAFETY: `handle` is a live object handle and we only read the
                // immutable base pointer for offset computation.
                let base_ptr = unsafe { handle.borrow().storage.raw_data_ptr() };
                if base_ptr.is_null() {
                    None
                } else {
                    // SAFETY: offset calculation mirrors ManagedPtr::with_data bounds scheme; caller must ensure validity
                    NonNull::new(unsafe { base_ptr.add(self.byte_offset().as_usize()) })
                }
            }
            #[cfg(feature = "multithreading")]
            PointerOrigin::CrossArenaObjectRef(ptr, _) => {
                let base_ptr = ptr.as_heap_storage(|storage| {
                    // SAFETY: `as_heap_storage` validates the object and keeps
                    // the lock held for the closure duration; we only read the
                    // base storage pointer.
                    unsafe { storage.raw_data_ptr() }
                });
                if base_ptr.is_null() {
                    None
                } else {
                    // SAFETY: offset calculation mirrors ManagedPtr::with_data bounds scheme; caller must ensure validity
                    NonNull::new(unsafe { base_ptr.add(self.byte_offset().as_usize()) })
                }
            }
            _ => {
                // For stack/static/absolute pointers, use cached value
                self._value
            }
        }
    }
}

#[cfg(test)]
mod tests;

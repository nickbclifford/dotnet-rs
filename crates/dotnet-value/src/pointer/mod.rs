//! Managed pointer access.
//!
//! **M1a heap access contract:** For `Heap` and `CrossArenaObjectRef` origins,
//! address observation is distinct from reading, writing, copying, or atomic
//! access. `ManagedPtr::observe_address(&self) -> Option<NonNull<u8>>`,
//! used by `PointerLike::pointer`, obtains the retained payload base under the
//! shared object borrow and forms `base.wrapping_add(offset)` without a payload
//! slice or a bounds check. A null base or a result wrapping to zero yields
//! `None`. The raw address neither keeps the borrow/arena lease nor roots the
//! owner; it grants no right to dereference, and later access needs its own
//! owner, lifetime, bounds, alignment, synchronization, and barrier proofs.
//! Expose provenance explicitly when an address is converted to an integer for
//! later pointer reconstruction.
//!
//! Heap reads are exact-or-reject: `with_data` checks
//! `offset.checked_add(size) <= payload_len` before forming only the requested
//! slice. Offset equal to payload length is valid for size zero; overflow or an
//! out-of-range window is rejected, never silently shortened. The unsafe
//! `try_with_data<T>(&self, size: usize, f: impl FnOnce(&[u8]) -> T) -> Option<T>`
//! returns `None` without calling `f` for a rejected heap range. This is not a
//! promise to catch callback panics or change legacy non-heap behavior.
//! `Stack` and `Static` retain their legacy fallback (M3); `Transient` and
//! owner/guard escape remain M1d debt. `Unmanaged` retains its existing caller
//! contract. None of these paths is covered by the heap observation/read rules.
//!
//! This design does not change `ObjectRef`/`ManagedPtr` serialization words,
//! pointer atomics (`AtomicPtr`), or write barriers. `memory-validation`'s
//! `validate_offset` uses accounting `size_bytes()` as a debug trap, not as an
//! access guard, and remains untouched.

use crate::{
    ByteOffset, StackSlotIndex, ValidationTag,
    object::{Object, ObjectRef},
    ptr_common::PointerLike,
};
use dashmap::DashMap;
use dotnet_types::{TypeDescription, generics::GenericLookup};
use dotnet_utils::ManagedByteOffset;
use gc_arena::{Collect, collect::Trace, static_collect};
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

/// Reconstructs a raw pointer at an unmanaged integer address.
///
/// This is the single reconstruction primitive for address-only `Unmanaged`,
/// P/Invoke, and `Unsafe.*` boundaries. Managed origins must instead derive a
/// pointer from their provenance-carrying owner or resolver.
///
/// # Safety
///
/// For every later access through the returned pointer, the caller must prove
/// that `addr` is zero or denotes live storage with the required provenance,
/// bounds, alignment, lifetime, and synchronization. Nonzero `addr` values
/// that are later dereferenced must carry provenance previously exposed from
/// the corresponding pointer; arbitrary integers do not root allocations or
/// acquire access rights. This function validates none of those properties. It
/// must never recover a managed `ManagedPtr` origin from serialized address bits.
#[inline]
pub unsafe fn unmanaged_ptr_from_addr(addr: usize) -> *mut u8 {
    std::ptr::with_exposed_provenance_mut::<u8>(addr)
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

/// Resolves live storage bases for executable managed-pointer deserialization.
///
/// Serialized Stack and Static origins retain only a stable slot or metadata
/// handle plus an offset. Implementors must return a currently live base for
/// that handle; [`ManagedPtr::read_resolved_unchecked`] derives the resulting
/// pointer from that provenance-carrying base and the encoded offset.
pub trait ManagedPtrResolver<'gc> {
    /// Returns the live start address of an evaluation-stack slot.
    fn stack_slot_base(&self, slot: StackSlotIndex) -> Option<NonNull<u8>>;

    /// Returns the live start address of static storage for `metadata`.
    fn static_storage_base(&self, metadata: &StaticMetadata) -> Option<NonNull<u8>>;
}

/// Caller-owned cache for decoded non-null Heap handles.
///
/// Implementations must own values from a GC-traced root and must discard every
/// entry after the collection cycle that can recycle serialized handle-address
/// words. Implementations must never retain an object-data pointer, storage
/// guard, or CrossArena lease: a cache hit only avoids decoding the serialized
/// Heap handle and still resolves current object storage.
///
/// This trait deliberately covers Heap handles only. Stack and Static storage
/// remains resolver-owned, while CrossArena decoding must acquire a fresh arena
/// lease for every traversal.
pub trait HeapManagedPtrDecodeCache<'gc> {
    /// Returns the rooted handle previously decoded from `serialized_handle`.
    fn get_heap_handle(&mut self, serialized_handle: usize) -> Option<ObjectRef<'gc>>;

    /// Retains a newly decoded non-null Heap handle under its complete wire word.
    fn insert_heap_handle(&mut self, serialized_handle: usize, owner: ObjectRef<'gc>);
}

/// A resolver for contexts that deliberately cannot deserialize Stack or Static
/// pointers into executable addresses.
///
/// This is suitable only when a caller accepts Heap, CrossArena, and Unmanaged
/// origins and wants Stack/Static origins rejected with a contextual error.
#[derive(Debug, Default, Copy, Clone)]
pub struct NoManagedPtrResolver;

impl<'gc> ManagedPtrResolver<'gc> for NoManagedPtrResolver {
    fn stack_slot_base(&self, _slot: StackSlotIndex) -> Option<NonNull<u8>> {
        None
    }

    fn static_storage_base(&self, _metadata: &StaticMetadata) -> Option<NonNull<u8>> {
        None
    }
}

#[derive(Copy, Clone, Debug, PartialEq, PartialOrd)]
pub struct UnmanagedPtr(pub NonNull<u8>);

#[cfg(feature = "fuzzing")]
impl<'a> Arbitrary<'a> for UnmanagedPtr {
    fn arbitrary(u: &mut arbitrary::Unstructured<'a>) -> arbitrary::Result<Self> {
        let ptr_val: usize = u.arbitrary()?;
        Ok(UnmanagedPtr(
            NonNull::new(std::ptr::without_provenance_mut(ptr_val)).unwrap_or(NonNull::dangling()),
        ))
    }
}
static_collect!(UnmanagedPtr);

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
            address: NonNull::new(std::ptr::without_provenance_mut(ptr_val)),
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
pub struct ManagedPtr<'gc> {
    pub(crate) magic: ValidationTag,
    pub(crate) _value: Option<NonNull<u8>>,
    pub(crate) origin: PointerOrigin<'gc>,
    pub(crate) inner_type: TypeDescription,
    /// Compact byte offset for managed origins; unmanaged absolute addresses are read from `_value`.
    pub(crate) offset: ManagedByteOffset,
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
        let value = NonNull::new(std::ptr::without_provenance_mut(ptr_val));
        let packed_offset = Self::pack_offset(&origin, value, offset);
        let unmanaged_inline_offset = matches!(origin, PointerOrigin::Unmanaged)
            && packed_offset.as_usize() == offset.as_usize();
        Ok(Self {
            magic: ValidationTag::new(MANAGED_PTR_MAGIC as u64),
            _value: value,
            origin,
            inner_type: u.arbitrary()?,
            offset: packed_offset,
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
                let resolved = value.map(|p| p.as_ptr().addr()).unwrap_or(0);
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
                ByteOffset::new(self.offset.as_usize())
            } else {
                ByteOffset::new(self._value.map_or(0, |p| p.as_ptr().addr()))
            }
        } else {
            ByteOffset::new(self.offset.as_usize())
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
                ByteOffset::new(value.map_or(0, |p| p.as_ptr() as usize))
            } else {
                ByteOffset::new(0)
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

    #[inline]
    fn access_window(&self, source_len: usize, requested_size: usize) -> (usize, usize) {
        let offset = self.byte_offset().as_usize();
        let available = source_len.saturating_sub(offset);
        let to_access = std::cmp::min(requested_size, available);
        (offset, to_access)
    }

    fn try_with_heap_data<T>(
        &self,
        size: usize,
        allow_short: bool,
        f: impl FnOnce(&[u8]) -> T,
    ) -> Option<T> {
        let offset = self.byte_offset().as_usize();
        match &self.origin {
            PointerOrigin::Heap(owner) => {
                let handle = owner
                    .0
                    .expect("ManagedPtr::with_data: null heap owner handle");
                let inner = handle.borrow();
                if allow_short {
                    inner.storage.with_data_up_to(offset, size, f)
                } else {
                    inner.storage.try_with_data_range(offset, size, f)
                }
            }
            #[cfg(feature = "multithreading")]
            PointerOrigin::CrossArenaObjectRef(ptr, _) => {
                if allow_short {
                    ptr.with_data_up_to(offset, size, f)
                } else {
                    ptr.try_with_data_range(offset, size, f)
                }
            }
            _ => unreachable!("heap range access requires a heap origin"),
        }
    }

    /// Attempts an exact read from a heap-backed managed pointer.
    ///
    /// For Heap and CrossArenaObjectRef origins, the requested window is checked
    /// against the actual payload length before any byte slice is formed; an
    /// invalid or overflowing window returns `None` without invoking `f`.
    /// Other origins retain their legacy `with_data` behavior and return
    /// `Some` unless that behavior panics.
    ///
    /// # Safety
    /// If there is no heap owner, this method inherits `with_data`'s requirement
    /// that the cached pointer be valid for the requested range.
    pub unsafe fn try_with_data<T>(&self, size: usize, f: impl FnOnce(&[u8]) -> T) -> Option<T> {
        self.validate_magic();
        match &self.origin {
            PointerOrigin::Heap(_) => self.try_with_heap_data(size, false, f),
            #[cfg(feature = "multithreading")]
            PointerOrigin::CrossArenaObjectRef(_, _) => self.try_with_heap_data(size, false, f),
            _ => {
                // SAFETY: F10.RawMemoryAccessValid — Non-heap origins deliberately retain the
                // pre-existing `with_data` behavior and its caller-provided validity contract.
                Some(unsafe { self.with_data(size, f) })
            }
        }
    }

    /// Reads up to `max_size` bytes from a managed pointer.
    ///
    /// Heap reads explicitly clamp to the bytes remaining after the logical
    /// offset, for callers such as P/Invoke that preserve a zero-filled buffer
    /// tail. An offset beyond the payload is rejected. Other origins retain
    /// their previous behavior.
    ///
    /// # Safety
    /// If there is no heap owner, this method inherits `with_data`'s requirement
    /// that the cached pointer be valid for the requested range.
    pub unsafe fn with_data_up_to<T>(&self, max_size: usize, f: impl FnOnce(&[u8]) -> T) -> T {
        self.validate_magic();
        match &self.origin {
            PointerOrigin::Heap(_) => {
                self.try_with_heap_data(max_size, true, f)
                    .unwrap_or_else(|| {
                        panic!(
                            "ManagedPtr::with_data_up_to: heap offset {} exceeds payload extent",
                            self.byte_offset().as_usize()
                        )
                    })
            }
            #[cfg(feature = "multithreading")]
            PointerOrigin::CrossArenaObjectRef(_, _) => self
                .try_with_heap_data(max_size, true, f)
                .unwrap_or_else(|| {
                    panic!(
                        "ManagedPtr::with_data_up_to: heap offset {} exceeds payload extent",
                        self.byte_offset().as_usize()
                    )
                }),
            _ => {
                // SAFETY: F10.RawMemoryAccessValid — Non-heap origins deliberately retain the
                // pre-existing `with_data` behavior and its caller-provided validity contract.
                unsafe { self.with_data(max_size, f) }
            }
        }
    }

    /// Safely accesses the data pointed to by this ManagedPtr.
    ///
    /// Heap and cross-arena heap reads are exact: the complete requested range
    /// must fit within the actual payload extent, not allocation-pressure
    /// `size_bytes()`. Invalid offsets, size overflow, or short heap ranges panic
    /// rather than returning a truncated success. Other origins retain their
    /// existing access behavior.
    ///
    /// # Safety
    /// If there is no owner, this method assumes the cached pointer is valid for at least `size` bytes.
    pub unsafe fn with_data<T>(&self, size: usize, f: impl FnOnce(&[u8]) -> T) -> T {
        self.validate_magic();
        match &self.origin {
            PointerOrigin::Heap(_) => self
                .try_with_heap_data(size, false, f)
                .unwrap_or_else(|| {
                    panic!(
                        "ManagedPtr::with_data: heap range overflow or out of bounds (offset {}, size {})",
                        self.byte_offset().as_usize(),
                        size
                    )
                }),
            #[cfg(feature = "multithreading")]
            PointerOrigin::CrossArenaObjectRef(_, _) => self
                .try_with_heap_data(size, false, f)
                .unwrap_or_else(|| {
                    panic!(
                        "ManagedPtr::with_data: heap range overflow or out of bounds (offset {}, size {})",
                        self.byte_offset().as_usize(),
                        size
                    )
                }),
            // Legacy Transient access remains outside the heap payload contract (M1d).
            PointerOrigin::Transient(obj) => obj.with_data(|data| {
                let (offset, to_access) = self.access_window(data.len(), size);
                f(&data[offset..offset + to_access])
            }),
            _ => {
                // SAFETY: F10.RawMemoryAccessValid — This retains the legacy non-heap
                // slice path and its caller-provided range/lifetime contract; it does
                // not use the heap payload extent or establish a new access right.
                // Stack/Static migration belongs to M3; Transient owner/guard
                // escape belongs to M1d. Inline resolution preserves existing aliasing behavior.
                let ptr = if let Some((_idx, _offset)) = self.stack_slot_origin() {
                    // Stack pointer - use cached value
                    self._value
                        .expect("ManagedPtr::with_data: null stack pointer")
                } else {
                    // Static data pointer or absolute pointer - use cached value
                    self._value.expect("ManagedPtr::with_data: null pointer")
                };
                // SAFETY: F3.InteriorPointerRebased — The ManagedPtr representation invariant established by its constructors satisfies this operation's preconditions.
                let slice = unsafe { NonNull::slice_from_raw_parts(ptr, size).as_ref() };
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
            // SAFETY: F1.GcHandleRooted — We are only reading the size and magic, which are immutable for the object's lifetime.
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
    /// This performs a raw pointer offset using wrapping arithmetic, so creating the intermediate
    /// pointer value is UB-free even if it leaves allocation bounds. However, callers must ensure
    /// the resulting `ManagedPtr` is only used in ways that are valid for the same allocation
    /// provenance as the original pointer (for example, any eventual dereference must be in
    /// bounds and properly aligned for the accessed type). When `memory-validation` is enabled,
    /// an additional bounds check traps diagnostically, but that check is not a correctness guard.
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

        let new_offset = ByteOffset::new(new_offset_isize as usize);
        let packed_offset = Self::pack_offset(&m.origin, m._value, new_offset);
        m.flags = Self::with_unmanaged_inline_offset_flag(
            m.flags,
            matches!(m.origin, PointerOrigin::Unmanaged)
                && packed_offset.as_usize() == new_offset.as_usize(),
        );
        m.offset = packed_offset;
        m._value = m._value.map(|p| {
            NonNull::new(p.as_ptr().wrapping_offset(bytes))
                .expect("ManagedPtr::offset produced null pointer")
        });
        m
    }

    pub fn with_stack_origin(mut self, slot_index: StackSlotIndex) -> Self {
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

// SAFETY: F3.InteriorPointerRebased — The ManagedPtr representation invariant established by its constructors satisfies this operation's preconditions.
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

// Wrapping address formation is observation only, not a bounds check or access right.
fn observed_address(base: *const u8, offset: usize) -> Option<NonNull<u8>> {
    if base.is_null() {
        return None;
    }
    NonNull::new(base.wrapping_add(offset).cast_mut())
}

impl<'gc> ManagedPtr<'gc> {
    /// Observes a heap address without forming a payload slice or checking bounds.
    ///
    /// Wrapping formation is observation only, not an access check or permission
    /// to dereference. The returned address retains allocation provenance only;
    /// it neither roots the owner nor retains a borrow, arena lease, or lock.
    /// Other origins return their legacy cached value (Stack/Static: M3;
    /// Transient owner/guard escape: M1d).
    pub fn observe_address(&self) -> Option<NonNull<u8>> {
        self.validate_magic();
        let base = match &self.origin {
            PointerOrigin::Heap(owner) => {
                let handle = owner.0?;
                handle.borrow().storage.observe_payload_base()
            }
            #[cfg(feature = "multithreading")]
            PointerOrigin::CrossArenaObjectRef(ptr, _) => {
                ptr.as_heap_storage(|storage| storage.observe_payload_base())
            }
            _ => return self._value,
        };
        observed_address(base, self.byte_offset().as_usize())
    }
}

impl<'gc> PointerLike for ManagedPtr<'gc> {
    #[inline]
    fn pointer(&self) -> Option<NonNull<u8>> {
        self.validate_magic();
        match &self.origin {
            PointerOrigin::Heap(_) => self.observe_address(),
            #[cfg(feature = "multithreading")]
            PointerOrigin::CrossArenaObjectRef(_, _) => self.observe_address(),
            // Legacy cached-address fallback for non-heap origins; Stack/Static
            // migration is M3 and Transient owner/guard escape is M1d.
            _ => self._value,
        }
    }
}

#[cfg(test)]
mod tests;

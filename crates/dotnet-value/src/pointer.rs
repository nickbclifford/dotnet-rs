use crate::{ArenaId, object::ObjectRef};
use dashmap::DashMap;
use dotnet_types::{TypeDescription, generics::GenericLookup};
use gc_arena::{Collect, Collection, Mutation, unsafe_empty_collect};
use sptr::Strict;
use std::{
    cmp::Ordering as CmpOrdering,
    collections::HashSet,
    fmt::{self, Debug, Formatter},
    ptr::NonNull,
    sync::{
        Arc, OnceLock,
        atomic::{AtomicU32, Ordering as AtomicOrdering},
    },
};

#[inline]
fn nonnull_from_exposed_addr(addr: usize) -> Option<NonNull<u8>> {
    NonNull::new(sptr::from_exposed_addr_mut::<u8>(addr))
}

#[cfg(feature = "fuzzing")]
use arbitrary::Arbitrary;

#[cfg(feature = "multithreading")]
use crate::object::ObjectPtr;
#[cfg(feature = "multithreading")]
use dotnet_utils::gc::ThreadSafeLock;

pub struct StaticMetadata {
    pub type_desc: TypeDescription,
    pub generics: GenericLookup,
}

fn static_registry() -> &'static DashMap<u32, Arc<StaticMetadata>> {
    static REGISTRY: OnceLock<DashMap<u32, Arc<StaticMetadata>>> = OnceLock::new();
    REGISTRY.get_or_init(DashMap::new)
}

fn static_dedup_map() -> &'static DashMap<(TypeDescription, GenericLookup), u32> {
    static DEDUP: OnceLock<DashMap<(TypeDescription, GenericLookup), u32>> = OnceLock::new();
    DEDUP.get_or_init(DashMap::new)
}

pub fn reset_static_registry() {
    static_registry().clear();
    static_dedup_map().clear();
    NEXT_STATIC_ID.store(1, AtomicOrdering::SeqCst);
}

static NEXT_STATIC_ID: AtomicU32 = AtomicU32::new(1);

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
unsafe_empty_collect!(UnmanagedPtr);

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PointerOrigin<'gc> {
    Heap(ObjectRef<'gc>),
    Stack(crate::StackSlotIndex),
    Static(TypeDescription, GenericLookup),
    Unmanaged,
    #[cfg(feature = "multithreading")]
    CrossArenaObjectRef(ObjectPtr, ArenaId),
    /// A value type resident on the evaluation stack (transient).
    Transient(crate::object::Object<'gc>),
}

#[cfg(feature = "fuzzing")]
impl<'a, 'gc> Arbitrary<'a> for PointerOrigin<'gc> {
    fn arbitrary(u: &mut arbitrary::Unstructured<'a>) -> arbitrary::Result<Self> {
        let variant = u.int_in_range(0..=5)?;
        match variant {
            0 => Ok(Self::Heap(u.arbitrary()?)),
            1 => Ok(Self::Stack(u.arbitrary()?)),
            2 => Ok(Self::Static(u.arbitrary()?, u.arbitrary()?)),
            3 => Ok(Self::Unmanaged),
            #[cfg(feature = "multithreading")]
            4 => Ok(Self::CrossArenaObjectRef(u.arbitrary()?, u.arbitrary()?)),
            #[cfg(not(feature = "multithreading"))]
            4 => Ok(Self::Unmanaged),
            5 => Ok(Self::Transient(u.arbitrary()?)),
            _ => unreachable!(),
        }
    }
}

// SAFETY: PointerOrigin contains several variants that hold GC-managed references.
// We manually implement trace and resurrect to ensure all such references (ObjectRef, Object)
// are correctly visited by the GC. Cross-arena references are recorded for coordinated GC.
unsafe impl<'gc> Collect for PointerOrigin<'gc> {
    fn trace(&self, cc: &Collection) {
        match self {
            Self::Heap(r) => r.trace(cc),
            #[cfg(feature = "multithreading")]
            Self::CrossArenaObjectRef(ptr, tid) => {
                dotnet_utils::gc::record_cross_arena_ref(*tid, ptr.as_ptr() as usize);
            }
            Self::Transient(obj) => obj.trace(cc),
            _ => {}
        }
    }
}

impl<'gc> PointerOrigin<'gc> {
    pub fn is_null(&self) -> bool {
        match self {
            Self::Unmanaged => true,
            Self::Heap(r) => r.0.is_none(),
            _ => false,
        }
    }

    pub fn normalize(self) -> Self {
        match self {
            Self::Heap(r) if r.0.is_none() => Self::Unmanaged,
            Self::Transient(_) => Self::Unmanaged,
            other => other,
        }
    }

    pub fn owner(&self) -> Option<ObjectRef<'gc>> {
        if let Self::Heap(r) = self {
            Some(*r)
        } else {
            None
        }
    }

    pub fn write_barrier_owner_id(&self) -> Option<ArenaId> {
        match self {
            Self::Heap(r) => r.0.as_ref().map(|h| unsafe { (*h.as_ptr()).owner_id }),
            #[cfg(feature = "multithreading")]
            Self::CrossArenaObjectRef(_, tid) => Some(*tid),
            _ => None,
        }
    }

    #[cfg(feature = "multithreading")]
    pub fn write_barrier_owner(&self) -> Option<(ObjectPtr, ArenaId)> {
        match self {
            Self::Heap(r) => r.as_ptr_info(),
            Self::CrossArenaObjectRef(ptr, tid) => Some((*ptr, *tid)),
            _ => None,
        }
    }

    pub fn resurrect(
        &self,
        fc: &'gc gc_arena::Finalization<'gc>,
        visited: &mut HashSet<usize>,
        depth: usize,
    ) {
        match self {
            PointerOrigin::Heap(r) => r.resurrect(fc, visited, depth),
            #[cfg(feature = "multithreading")]
            PointerOrigin::CrossArenaObjectRef(ptr, tid) => {
                dotnet_utils::gc::record_cross_arena_ref(*tid, ptr.as_ptr().expose_addr());
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
    pub offset: crate::ByteOffset,
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

#[cfg(any(feature = "memory-validation", debug_assertions))]
const MANAGED_PTR_MAGIC: u32 = 0x504F_494E;

#[derive(Clone)]
#[repr(C)]
pub struct ManagedPtr<'gc> {
    #[cfg(any(feature = "memory-validation", debug_assertions))]
    pub(crate) magic: u32,
    pub(crate) _value: Option<NonNull<u8>>,
    pub(crate) origin: PointerOrigin<'gc>,
    /// The offset from the owner's base pointer.
    pub(crate) offset: crate::ByteOffset,
    pub(crate) inner_type: TypeDescription,
    pub(crate) pinned: bool,
    pub(crate) _marker: std::marker::PhantomData<&'gc ()>,
}

#[cfg(feature = "fuzzing")]
impl<'a, 'gc> Arbitrary<'a> for ManagedPtr<'gc> {
    fn arbitrary(u: &mut arbitrary::Unstructured<'a>) -> arbitrary::Result<Self> {
        let ptr_val: usize = u.arbitrary()?;
        Ok(Self {
            #[cfg(any(feature = "memory-validation", debug_assertions))]
            magic: MANAGED_PTR_MAGIC,
            _value: nonnull_from_exposed_addr(ptr_val),
            origin: u.arbitrary()?,
            offset: u.arbitrary()?,
            inner_type: u.arbitrary()?,
            pinned: u.arbitrary()?,
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
    // Read-only accessors
    pub fn origin(&self) -> &PointerOrigin<'gc> { &self.origin }
    pub fn inner_type(&self) -> TypeDescription { self.inner_type }
    pub fn is_pinned(&self) -> bool { self.pinned }
    pub fn byte_offset(&self) -> crate::ByteOffset { self.offset }

    pub fn offset_by(&self, bytes: isize) -> Result<Self, dotnet_types::error::MemoryError> {
        Ok(Self {
            offset: crate::ByteOffset(self.offset.0.wrapping_add_signed(bytes)),
            _value: self._value.map(|p| unsafe {
                NonNull::new_unchecked(p.as_ptr().wrapping_offset(bytes))
            }),
            ..self.clone()
        })
    }

    pub fn into_info(self) -> ManagedPtrInfo<'gc> {
        ManagedPtrInfo {
            address: self._value,
            origin: self.origin,
            offset: self.offset,
        }
    }

    pub fn with_origin(&self, origin: PointerOrigin<'gc>) -> Self {
        Self { origin, ..self.clone() }
    }

    pub fn with_inner_type(&self, inner_type: TypeDescription) -> Self {
        Self { inner_type, ..self.clone() }
    }

    pub fn with_pinned(&self, pinned: bool) -> Self {
        Self { pinned, ..self.clone() }
    }

    pub fn owner(&self) -> Option<ObjectRef<'gc>> {
        if let PointerOrigin::Heap(o) = self.origin {
            Some(o)
        } else {
            None
        }
    }

    pub fn stack_slot_origin(&self) -> Option<(crate::StackSlotIndex, crate::ByteOffset)> {
        if let PointerOrigin::Stack(idx) = self.origin {
            Some((idx, self.offset))
        } else {
            None
        }
    }
    /// One word for the pointer, one word for the owner, and one word for the checksum.
    pub const SIZE: usize = ObjectRef::SIZE * 3;

    pub fn serialization_buffer() -> [u8; ObjectRef::SIZE * 3] {
        [0u8; ObjectRef::SIZE * 3]
    }

    #[cfg(feature = "multithreading")]
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
        let origin = if let Some(o) = owner {
            PointerOrigin::Heap(o)
        } else {
            PointerOrigin::Unmanaged
        };
        let offset = offset.unwrap_or_else(|| {
            if let PointerOrigin::Unmanaged = origin {
                crate::ByteOffset(value.map_or(0, |p| p.as_ptr() as usize))
            } else {
                crate::ByteOffset(0)
            }
        });

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

    pub fn null() -> Self {
        Self {
            #[cfg(any(feature = "memory-validation", debug_assertions))]
            magic: MANAGED_PTR_MAGIC,
            _value: None,
            inner_type: TypeDescription::NULL,
            origin: PointerOrigin::Unmanaged,
            offset: crate::ByteOffset::ZERO,
            pinned: false,
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

        if origin.is_null() && offset == crate::ByteOffset::ZERO && value.is_none() {
            origin = PointerOrigin::Unmanaged;
        }

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

    #[deprecated(note = "Use from_info_full to explicitly specify pinning status")]
    pub fn from_info(info: ManagedPtrInfo<'gc>, inner_type: TypeDescription) -> Self {
        Self::from_info_full(info, inner_type, false)
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

        if word0 & 1 != 0 {
            let tag = word0 & 7;
            match tag {
                1 => {
                    // Stack pointer
                    let idx = (word0 >> 3) & 0x3FFFFFFF;
                    let off = word0 >> 33;
                    return ManagedPtrStackInfo {
                        address: nonnull_from_exposed_addr(word1),
                        offset: crate::ByteOffset(off),
                        origin: PointerOrigin::Stack(crate::StackSlotIndex(idx)),
                    };
                }
                7 if ((word0 >> 3) & 7) == 2 => {
                    // Transient (Tag 7, Subtag 2)
                    let off = word0 >> 6;
                    return ManagedPtrStackInfo {
                        address: nonnull_from_exposed_addr(word1),
                        offset: crate::ByteOffset(off),
                        origin: PointerOrigin::Unmanaged, // Can't recover Object without GCHandle
                    };
                }
                _ => {}
            }
        }

        // Heap pointer or Static/Unmanaged
        // We can't fully reconstruct Static/Heap without GCHandle, so we mark as Unmanaged
        // for stack info purposes.
        ManagedPtrStackInfo {
            address: nonnull_from_exposed_addr(word1),
            offset: crate::ByteOffset(word1),
            origin: PointerOrigin::Unmanaged,
        }
    }

    /// Read pointer and owner from memory, branded with a GCHandle.
    /// Returns detailed metadata about the pointer. Type info must be supplied by caller.
    ///
    /// # Safety
    ///
    /// The `source` slice must be at least `ManagedPtr::SIZE` bytes long.
    /// It must contain valid bytes representing a `ManagedPtr`.
    pub unsafe fn read_branded(
        source: &[u8],
        _gc: &Mutation<'gc>,
    ) -> Result<ManagedPtrInfo<'gc>, PointerDeserializationError> {
        unsafe { Self::read_unchecked(source) }
    }

    /// Read pointer and owner from memory.
    /// Returns detailed metadata about the pointer. Type info must be supplied by caller.
    ///
    /// # Safety
    ///
    /// The `source` slice must be at least `ManagedPtr::SIZE` bytes long.
    /// It must contain valid bytes representing a `ManagedPtr`.
    pub unsafe fn read_unchecked(
        source: &[u8],
    ) -> Result<ManagedPtrInfo<'gc>, PointerDeserializationError> {
        let ptr_size = ObjectRef::SIZE;

        let mut word0_bytes = [0u8; ObjectRef::SIZE];
        word0_bytes.copy_from_slice(&source[0..ptr_size]);
        let word0 = usize::from_ne_bytes(word0_bytes);

        let mut word1_bytes = [0u8; ObjectRef::SIZE];
        word1_bytes.copy_from_slice(&source[ptr_size..ptr_size * 2]);
        let word1 = usize::from_ne_bytes(word1_bytes);

        let mut word2_bytes = [0u8; ObjectRef::SIZE];
        word2_bytes.copy_from_slice(&source[ptr_size * 2..ptr_size * 3]);
        let word2 = usize::from_ne_bytes(word2_bytes);

        if word2 != word0 ^ word1 {
            return Err(PointerDeserializationError::ChecksumMismatch);
        }

        let tag = word0 & 7;
        if word0 & 1 != 0 {
            // Tagged pointer or Stack
            match tag {
                1 => {
                    // Stack (Tag 1)
                    let slot_idx = (word0 >> 3) & 0x3FFFFFFF;
                    let slot_offset = word0 >> 33;
                    let raw_ptr = nonnull_from_exposed_addr(word1);
                    Ok(ManagedPtrInfo {
                        address: raw_ptr,
                        origin: PointerOrigin::Stack(crate::StackSlotIndex(slot_idx)),
                        offset: crate::ByteOffset(slot_offset),
                    })
                }
                5 => {
                    // CrossArenaObjectRef (Tag 5)
                    // Recover ObjectPtr from Word 0 and offset from Word 1
                    #[cfg(feature = "multithreading")]
                    {
                        let lock_ptr = sptr::from_exposed_addr::<
                            ThreadSafeLock<crate::object::ObjectInner<'static>>,
                        >(word0 & !7);
                        // SAFETY: During GC or stable execution, cross-arena pointers are valid.
                        // We need the owner_id from the object itself.
                        #[cfg(feature = "fuzzing")]
                        if !crate::object::is_valid_object_ptr(lock_ptr.expose_addr()) {
                            return Err(PointerDeserializationError::UnknownTag(tag));
                        }

                        let ptr = unsafe {
                            ObjectPtr::from_raw(lock_ptr)
                                .ok_or(PointerDeserializationError::UnknownTag(tag))?
                        };

                        #[cfg(any(feature = "memory-validation", debug_assertions))]
                        {
                            let inner_ptr = unsafe { (*lock_ptr).as_ptr() };
                            if unsafe { (*inner_ptr).magic } != crate::object::OBJECT_MAGIC {
                                panic!(
                                    "ManagedPtr::read: CrossArena pointer {:#x} points to invalid object (bad magic)",
                                    lock_ptr.expose_addr()
                                );
                            }
                        }

                        let owner_id = ptr.owner_id();

                        // RECOVERY: We must use the data storage pointer as the base, not ObjectInner.
                        let inner_ptr = unsafe { (*lock_ptr).as_ptr() };
                        let base_ptr = unsafe { (*inner_ptr).storage.raw_data_ptr() };
                        let base_addr = base_ptr.expose_addr();

                        Ok(ManagedPtrInfo {
                            address: nonnull_from_exposed_addr(base_addr + word1),
                            origin: PointerOrigin::CrossArenaObjectRef(ptr, owner_id),
                            offset: crate::ByteOffset(word1),
                        })
                    }
                    #[cfg(not(feature = "multithreading"))]
                    {
                        // Fallback for non-multithreading mode (should not happen)
                        Err(PointerDeserializationError::UnknownTag(tag))
                    }
                }
                7 => {
                    // Extended Tags
                    let subtag = (word0 >> 3) & 7;
                    match subtag {
                        1 => {
                            // Static (Subtag 1)
                            let id = ((word0 >> 6) & 0xFFFFFFFF) as u32;
                            let slot_offset = word0 >> 38;
                            let raw_ptr = nonnull_from_exposed_addr(word1);

                            if id > 0
                                && let Some(meta) = static_registry().get(&id)
                            {
                                Ok(ManagedPtrInfo {
                                    address: raw_ptr,
                                    origin: PointerOrigin::Static(
                                        meta.type_desc,
                                        meta.generics.clone(),
                                    ),
                                    offset: crate::ByteOffset(slot_offset),
                                })
                            } else {
                                Err(PointerDeserializationError::InvalidStaticId(id))
                            }
                        }
                        2 => {
                            // Transient (Subtag 2)
                            // NOTE: Object reference is lost on serialization/deserialization.
                            // Recovering as Unmanaged is unsafe if the stack relocates.
                            Err(PointerDeserializationError::UnknownSubtag(subtag))
                        }
                        _ => {
                            // Unmanaged or unknown subtag
                            Err(PointerDeserializationError::UnknownSubtag(subtag))
                        }
                    }
                }
                _ => {
                    // Invalid tag or unhandled format
                    Err(PointerDeserializationError::UnknownTag(tag))
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
                nonnull_from_exposed_addr(offset.as_usize())
            };

            Ok(ManagedPtrInfo {
                address: ptr,
                origin: owner.0.map_or(PointerOrigin::Unmanaged, |h| {
                    PointerOrigin::Heap(ObjectRef(Some(h)))
                }),
                offset,
            })
        }
    }

    /// Write owner and offset to memory.
    /// Memory layout: (Owner ObjectRef at offset 0, Offset at offset 8)
    pub fn write(&self, dest: &mut [u8]) {
        self.validate_magic();

        let ptr_size = ObjectRef::SIZE;

        let (word0, word1) = match &self.origin {
            PointerOrigin::Stack(slot_idx) => {
                let w0: usize =
                    1 | ((slot_idx.as_usize() & 0x3FFFFFFF) << 3) | (self.offset.as_usize() << 33);
                let w1 = self._value.map_or(0, |p| p.as_ptr().expose_addr());
                (w0, w1)
            }
            PointerOrigin::Heap(owner) => {
                let w0 = match owner.0 {
                    Some(h) => gc_arena::Gc::as_ptr(h).expose_addr(),
                    None => 0,
                };
                let w1 = self.offset.as_usize();
                (w0, w1)
            }
            PointerOrigin::Static(type_desc, generics) => {
                let key = (*type_desc, generics.clone());
                let id = *static_dedup_map().entry(key).or_insert_with(|| {
                    let new_id = NEXT_STATIC_ID.fetch_add(1, AtomicOrdering::SeqCst);
                    static_registry().insert(
                        new_id,
                        Arc::new(StaticMetadata {
                            type_desc: *type_desc,
                            generics: generics.clone(),
                        }),
                    );
                    new_id
                });

                let w0: usize = 7
                    | (1 << 3)
                    | ((id as usize & 0xFFFFFFFF) << 6)
                    | (self.offset.as_usize() << 38);
                let w1 = self._value.map_or(0, |p| p.as_ptr().expose_addr());
                (w0, w1)
            }
            PointerOrigin::Unmanaged => {
                let w0: usize = 0;
                let w1 = self._value.map_or(0, |p| p.as_ptr().expose_addr());
                (w0, w1)
            }
            #[cfg(feature = "multithreading")]
            PointerOrigin::CrossArenaObjectRef(ptr, _) => {
                let w0: usize = ptr.as_ptr().expose_addr() | 5;
                let w1 = self.offset.as_usize();
                (w0, w1)
            }
            PointerOrigin::Transient(_) => {
                let w0: usize = 7 | (2 << 3) | (self.offset.as_usize() << 6);
                let w1 = self._value.map_or(0, |p| p.as_ptr().expose_addr());
                (w0, w1)
            }
        };

        dest[0..ptr_size].copy_from_slice(&word0.to_ne_bytes());
        dest[ptr_size..ptr_size * 2].copy_from_slice(&word1.to_ne_bytes());
        let word2 = word0 ^ word1;
        dest[ptr_size * 2..ptr_size * 3].copy_from_slice(&word2.to_ne_bytes());

        #[cfg(debug_assertions)]
        if !matches!(self.origin, PointerOrigin::Transient(_)) {
            let recovered =
                unsafe { Self::read_unchecked(dest) }.expect("ManagedPtr::write: recovery failed");
            let self_origin_norm = self.origin.clone().normalize();
            let recovered_origin_norm = recovered.origin.clone().normalize();

            assert_eq!(
                recovered_origin_norm, self_origin_norm,
                "ManagedPtr serialization round-trip failed: origin mismatch. Original: {:?}, Recovered: {:?}",
                self.origin, recovered.origin
            );

            if !matches!(self_origin_norm, PointerOrigin::Unmanaged) {
                assert_eq!(
                    recovered.offset, self.offset,
                    "ManagedPtr serialization round-trip failed: offset mismatch. Original: {:?}, Recovered: {:?}",
                    self.offset, recovered.offset
                );
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
        if let PointerOrigin::Stack(_idx) = m.origin {
            let diff = if let (Some(new_p), Some(old_p)) = (new_value, old_ptr) {
                (new_p.as_ptr() as isize).wrapping_sub(old_p.as_ptr() as isize)
            } else {
                0
            };
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
        let new_offset_isize = (m.offset.as_usize() as isize)
            .checked_add(bytes)
            .expect("ManagedPtr::offset overflow");

        if new_offset_isize < 0 {
            panic!(
                "ManagedPtr::offset: negative result (offset={}, bytes={})",
                m.offset.as_usize(),
                bytes
            );
        }

        m.offset = crate::ByteOffset(new_offset_isize as usize);
        m._value = m
            ._value
            .map(|p| unsafe { NonNull::new_unchecked(p.as_ptr().offset(bytes)) });
        m
    }

    pub fn with_stack_origin(
        mut self,
        slot_index: crate::StackSlotIndex,
        _offset: crate::ByteOffset,
    ) -> Self {
        self.validate_magic();
        self.origin = PointerOrigin::Stack(slot_index);
        self
    }

    pub fn is_stack_local(&self) -> bool {
        self.validate_magic();
        matches!(self.origin, PointerOrigin::Stack(_))
    }

    /// Returns true if this ManagedPtr represents a null pointer.
    /// A null pointer has no owner, no stack origin, and zero offset.
    pub fn is_null(&self) -> bool {
        self.validate_magic();
        // A canonical null ManagedPtr MUST have origin Unmanaged, offset 0, AND no cached address.
        // We also check for non-canonical nulls (like Heap(None)) for robustness.
        match &self.origin {
            PointerOrigin::Unmanaged => {
                self.offset == crate::ByteOffset::ZERO && self._value.is_none()
            }
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

impl<'gc> crate::ptr_common::PointerLike for ManagedPtr<'gc> {
    #[inline]
    fn pointer(&self) -> Option<NonNull<u8>> {
        self.validate_magic();
        if let Some(owner) = self.owner() {
            let handle = owner.0?;
            let base_ptr = unsafe { handle.borrow().storage.raw_data_ptr() };
            if base_ptr.is_null() {
                None
            } else {
                // SAFETY: offset calculation mirrors ManagedPtr::with_data bounds scheme; caller must ensure validity
                NonNull::new(unsafe { base_ptr.add(self.offset.as_usize()) })
            }
        } else {
            // For stack/static/absolute pointers, use cached value
            self._value
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::object::{HeapStorage, ObjectRef};
    use gc_arena::{Arena, Gc, Rootable};
    use std::sync::{Mutex, OnceLock};

    fn static_reg_test_lock() -> &'static Mutex<()> {
        static L: OnceLock<Mutex<()>> = OnceLock::new();
        L.get_or_init(|| Mutex::new(()))
    }

    #[test]
    #[cfg_attr(
        feature = "memory-validation",
        should_panic(expected = "ManagedPtr::offset: bounds violation")
    )]
    fn test_managed_ptr_offset_oob() {
        type TestRoot = Rootable![()];
        let arena = Arena::<TestRoot>::new(|_mc| ());
        #[cfg(feature = "multithreading")]
        let arena_handle = Box::leak(Box::new(dotnet_utils::gc::ArenaHandle::new(ArenaId(0))));

        arena.mutate(|gc, _root| {
            let gc_handle = dotnet_utils::gc::GCHandle::new(
                gc,
                #[cfg(feature = "multithreading")]
                arena_handle.as_inner(),
                #[cfg(feature = "memory-validation")]
                ArenaId(0),
            );

            // Create a small object (4 bytes for Int32)
            let storage = HeapStorage::Str(crate::string::CLRString::from("test"));
            let obj = ObjectRef::new(gc_handle, storage);

            let ptr = ManagedPtr::new(
                obj.with_data(|d| NonNull::new(d.as_ptr().cast_mut())),
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
        #[cfg(feature = "multithreading")]
        let arena_handle = Box::leak(Box::new(dotnet_utils::gc::ArenaHandle::new(ArenaId(0))));

        arena.mutate(|gc, _root| {
            let gc_handle = dotnet_utils::gc::GCHandle::new(
                gc,
                #[cfg(feature = "multithreading")]
                arena_handle.as_inner(),
                #[cfg(feature = "memory-validation")]
                ArenaId(0),
            );

            let storage = HeapStorage::Str(crate::string::CLRString::from("test"));
            let obj = ObjectRef::new(gc_handle, storage);

            let ptr = ManagedPtr::new(
                obj.with_data(|d| NonNull::new(d.as_ptr().cast_mut())),
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

    #[test]
    fn test_managed_ptr_serialization_roundtrip() {
        let _guard = static_reg_test_lock().lock().unwrap();
        type TestRoot = Rootable![()];
        let arena = Arena::<TestRoot>::new(|_mc| ());
        #[cfg(feature = "multithreading")]
        let arena_handle = Box::leak(Box::new(dotnet_utils::gc::ArenaHandle::new(ArenaId(0))));

        arena.mutate(|gc, _root| {
            let gc_handle = dotnet_utils::gc::GCHandle::new(
                gc,
                #[cfg(feature = "multithreading")]
                arena_handle.as_inner(),
                #[cfg(feature = "memory-validation")]
                ArenaId(0),
            );

            let mut buf = ManagedPtr::serialization_buffer();

            // 1. Unmanaged
            let unmanaged_addr = 0xDEADBEEFusize;
            let ptr_unmanaged = ManagedPtr::new(
                nonnull_from_exposed_addr(unmanaged_addr),
                TypeDescription::NULL,
                None,
                false,
                None,
            );

            ptr_unmanaged.write(&mut buf);
            let info = unsafe { ManagedPtr::read_unchecked(&buf) }.unwrap();
            assert_eq!(info.address, nonnull_from_exposed_addr(unmanaged_addr));
            assert_eq!(info.origin, PointerOrigin::Unmanaged);
            assert_eq!(info.offset.as_usize(), unmanaged_addr);

            // 2. Stack
            let stack_slot = crate::StackSlotIndex(123);
            let stack_addr = 0x1000usize;
            let ptr_stack = ManagedPtr::new(
                nonnull_from_exposed_addr(stack_addr),
                TypeDescription::NULL,
                None,
                false,
                Some(crate::ByteOffset(456)),
            )
            .with_stack_origin(stack_slot, crate::ByteOffset(0));

            ptr_stack.write(&mut buf);
            let info = unsafe { ManagedPtr::read_unchecked(&buf) }.unwrap();
            assert_eq!(info.address, nonnull_from_exposed_addr(stack_addr));
            assert_eq!(info.origin, PointerOrigin::Stack(stack_slot));
            assert_eq!(info.offset.as_usize(), 456);

            // 3. Heap
            let s = crate::string::CLRString::from("test");
            let obj = ObjectRef::new(gc_handle, HeapStorage::Str(s));
            let base_addr = obj.with_data(|d| d.as_ptr().expose_addr());
            let offset = 2;
            let ptr_heap = ManagedPtr::new(
                nonnull_from_exposed_addr(base_addr + offset),
                TypeDescription::NULL,
                Some(obj),
                false,
                Some(crate::ByteOffset(offset)),
            );

            ptr_heap.write(&mut buf);
            let info = unsafe { ManagedPtr::read_unchecked(&buf) }.unwrap();
            assert_eq!(info.address, nonnull_from_exposed_addr(base_addr + offset));
            assert_eq!(info.origin, PointerOrigin::Heap(obj));
            assert_eq!(info.offset.as_usize(), offset);

            // 4. Static
            let type_desc = TypeDescription::NULL;
            let generics = GenericLookup::default();
            let static_addr = 0x2000usize;
            let static_offset = 8;
            let ptr_static = ManagedPtr::new_static(
                nonnull_from_exposed_addr(static_addr + static_offset),
                TypeDescription::NULL,
                type_desc,
                generics.clone(),
                false,
                crate::ByteOffset(static_offset),
            );

            ptr_static.write(&mut buf);
            let info = unsafe { ManagedPtr::read_unchecked(&buf) }.unwrap();
            assert_eq!(
                info.address,
                nonnull_from_exposed_addr(static_addr + static_offset)
            );
            assert_eq!(info.origin, PointerOrigin::Static(type_desc, generics));
            assert_eq!(info.offset.as_usize(), static_offset);

            // 5. CrossArenaObjectRef (if enabled)
            #[cfg(feature = "multithreading")]
            {
                use crate::object::ObjectPtr;
                let ptr_raw = obj.with_data(|d| d.as_ptr());
                let base_addr = ptr_raw.expose_addr();
                // SAFETY: `obj` is kept alive for the duration of this test closure.
                // We only use this raw pointer to validate serialization/deserialization logic.
                let ptr_lock = Gc::as_ptr(obj.0.unwrap())
                    .cast::<ThreadSafeLock<crate::object::ObjectInner<'static>>>();
                let ptr = unsafe { ObjectPtr::from_raw(ptr_lock).unwrap() };
                let arena_id = ptr.owner_id();
                let cross_offset = 12;
                let ptr_cross = ManagedPtr::new_cross_arena(
                    nonnull_from_exposed_addr(base_addr + cross_offset),
                    TypeDescription::NULL,
                    ptr,
                    arena_id,
                    crate::ByteOffset(cross_offset),
                );

                ptr_cross.write(&mut buf);
                let info = unsafe { ManagedPtr::read_unchecked(&buf) }.unwrap();
                assert_eq!(
                    info.address,
                    nonnull_from_exposed_addr(base_addr + cross_offset)
                );
                if let PointerOrigin::CrossArenaObjectRef(recovered_ptr, recovered_arena) =
                    info.origin
                {
                    assert_eq!(recovered_ptr, ptr);
                    assert_eq!(recovered_arena, arena_id);
                } else {
                    panic!("Expected CrossArenaObjectRef, got {:?}", info.origin);
                }
                assert_eq!(info.offset.as_usize(), cross_offset);
            }
        });
    }

    #[test]
    fn test_gc_alignment() {
        type TestRoot = Rootable![()];
        let arena = Arena::<TestRoot>::new(|_mc| ());
        arena.mutate(|mc, _| {
            for _ in 0..1000 {
                let gc = Gc::new(mc, 0u64); // u64 should need 8-byte alignment
                let ptr = Gc::as_ptr(gc) as usize;
                assert_eq!(ptr % 8, 0, "Gc pointer {:#x} is not 8-byte aligned", ptr);
            }
        });
    }

    #[test]
    fn test_managed_ptr_serialization_bugs_reproduction() {
        type TestRoot = Rootable![()];
        let arena = Arena::<TestRoot>::new(|_mc| ());
        #[cfg(feature = "multithreading")]
        let arena_handle = Box::leak(Box::new(dotnet_utils::gc::ArenaHandle::new(ArenaId(0))));

        arena.mutate(|gc, _root| {
            let _gc_handle = &dotnet_utils::gc::GCHandle::new(
                gc,
                #[cfg(feature = "multithreading")]
                arena_handle.as_inner(),
                #[cfg(feature = "memory-validation")]
                ArenaId(0),
            );
            let mut buf = [0u8; ManagedPtr::SIZE];

            // 1. Transient origin (Fixed behavior in Stage 1)
            let layout = Arc::new(crate::layout::FieldLayoutManager {
                fields: std::collections::HashMap::new(),
                total_size: 0,
                alignment: 1,
                gc_desc: crate::layout::GcDesc::default(),
                has_ref_fields: false,
            });
            let storage = crate::storage::FieldStorage::new(layout, vec![]);
            let obj = crate::object::Object::new(
                TypeDescription::NULL,
                GenericLookup::default(),
                storage,
            );
            let transient_addr = 0x5000usize;
            let ptr_transient = ManagedPtr::new_transient(
                nonnull_from_exposed_addr(transient_addr),
                TypeDescription::NULL,
                obj.clone(),
                crate::ByteOffset(123), // Use a non-zero offset
            );

            ptr_transient.write(&mut buf);
            let result = unsafe { ManagedPtr::read_unchecked(&buf) };
            assert!(result.is_err(), "Transient recovery should fail for safety");

            let word0 = usize::from_ne_bytes(buf[0..8].try_into().unwrap());
            assert_eq!(word0 & 7, 7, "Transient should use Tag 7");
            assert_eq!((word0 >> 3) & 7, 2, "Transient should use Subtag 2");

            // 2. Tag Collision (Verified safe)
            // Any pointer with bit 0 = 0 is now guaranteed to be treated as Heap or Unmanaged,
            // never as Stack/Static/Transient.
            // We don't test misaligned pointers here as they would (correctly) panic in ObjectRef::read_unchecked.
        });
    }

    #[test]
    fn test_static_registry_deduplication() {
        let _guard = static_reg_test_lock().lock().unwrap();
        reset_static_registry();

        let type_desc = TypeDescription::NULL;
        let generics = GenericLookup::default();
        let mut buf1 = ManagedPtr::serialization_buffer();
        let mut buf2 = ManagedPtr::serialization_buffer();

        let ptr1 = ManagedPtr::new_static(
            nonnull_from_exposed_addr(0x1000),
            TypeDescription::NULL,
            type_desc,
            generics.clone(),
            false,
            crate::ByteOffset(0),
        );

        let ptr2 = ManagedPtr::new_static(
            nonnull_from_exposed_addr(0x2000),
            TypeDescription::NULL,
            type_desc,
            generics.clone(),
            false,
            crate::ByteOffset(0),
        );

        ptr1.write(&mut buf1);
        ptr2.write(&mut buf2);

        let word0_1 = usize::from_ne_bytes(buf1[0..8].try_into().unwrap());
        let word0_2 = usize::from_ne_bytes(buf2[0..8].try_into().unwrap());

        let id1 = (word0_1 >> 6) & 0xFFFFFFFF;
        let id2 = (word0_2 >> 6) & 0xFFFFFFFF;

        assert_eq!(
            id1, id2,
            "Static pointers with same metadata should have same ID"
        );
        assert_eq!(id1, 1, "First ID should be 1");
    }

    #[test]
    fn test_read_stack_info_miri() {
        let mut buf = [0u8; ManagedPtr::SIZE];
        let slot_idx = crate::StackSlotIndex(42);
        let offset = 8;
        let addr = 0x12345678usize;

        let w0: usize = 1 | ((slot_idx.as_usize() & 0x3FFFFFFF) << 3) | (offset << 33);
        let w1 = addr;

        buf[0..8].copy_from_slice(&w0.to_ne_bytes());
        buf[8..16].copy_from_slice(&w1.to_ne_bytes());

        let info = unsafe { ManagedPtr::read_stack_info(&buf) };
        assert_eq!(info.address, nonnull_from_exposed_addr(addr));
        assert_eq!(info.offset.as_usize(), offset);
        assert_eq!(info.origin, PointerOrigin::Stack(slot_idx));
    }

    #[test]
    fn test_managed_ptr_unmanaged_roundtrip_miri() {
        let mut buf = [0u8; ManagedPtr::SIZE];
        let addr = 0xAAAA_BBBB_CCCC_DDDDusize;
        let ptr = ManagedPtr::new(
            nonnull_from_exposed_addr(addr),
            TypeDescription::NULL,
            None,
            false,
            None,
        );

        ptr.write(&mut buf);
        let info = unsafe { ManagedPtr::read_unchecked(&buf) }.unwrap();
        assert_eq!(info.address, nonnull_from_exposed_addr(addr));
        assert_eq!(info.origin, PointerOrigin::Unmanaged);
        assert_eq!(info.offset.as_usize(), addr);
    }
}

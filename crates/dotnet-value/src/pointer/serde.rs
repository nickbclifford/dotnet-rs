use crate::{
    ByteOffset,
    object::ObjectRef,
    pointer::{
        FrameHomeSerial, HeapManagedPtrDecodeCache, ManagedPtr, ManagedPtrInfo, ManagedPtrResolver,
        PointerOrigin, unmanaged_ptr_from_addr,
    },
};
use dotnet_types::error::PointerDeserializationError;
use gc_arena::Mutation;
use std::ptr::NonNull;

#[cfg(feature = "multithreading")]
use crate::pointer::{CrossArenaOrigin, cross_arena::cross_arena_ptr_from_ptr};

/// Tag-7 sub-tag of a frame-home pointer. Word 0 is
/// `7 | (3 << 3) | (serial_low << 6) | (offset_mirror << 38)`, where `serial_low` is the low
/// 32 bits of the frame serial and `offset_mirror` the low 26 bits of the offset; word 1 is
/// `offset | (serial_high << 32)` with the full 32-bit offset.
const FRAME_HOME_SUBTAG: usize = 3;
const FRAME_HOME_SERIAL_SHIFT: usize = 6;
const FRAME_HOME_OFFSET_SHIFT: usize = 38;
const FRAME_HOME_OFFSET_MIRROR_MASK: usize = 0x03FF_FFFF;
const LOW_32_BITS: usize = 0xFFFF_FFFF;

fn frame_home_managed_ptr_words(serial: FrameHomeSerial, offset: ByteOffset) -> (usize, usize) {
    let serial = serial.get() as usize;
    let offset = offset.as_usize();
    let word0 = 7
        | (FRAME_HOME_SUBTAG << 3)
        | ((serial & LOW_32_BITS) << FRAME_HOME_SERIAL_SHIFT)
        | ((offset & FRAME_HOME_OFFSET_MIRROR_MASK) << FRAME_HOME_OFFSET_SHIFT);
    let word1 = offset | ((serial >> 32) << 32);
    (word0, word1)
}

fn parse_frame_home_managed_ptr(
    word0: usize,
    word1: usize,
) -> Result<(FrameHomeSerial, ByteOffset), PointerDeserializationError> {
    let offset = word1 & LOW_32_BITS;
    if word0 >> FRAME_HOME_OFFSET_SHIFT != offset & FRAME_HOME_OFFSET_MIRROR_MASK {
        return Err(PointerDeserializationError::OffsetMismatch);
    }
    let serial = ((word0 >> FRAME_HOME_SERIAL_SHIFT) & LOW_32_BITS) | ((word1 >> 32) << 32);
    Ok((FrameHomeSerial::new(serial as u64), ByteOffset::new(offset)))
}

// Preserve provenance in the first serialized word.
struct SerializedManagedPtrWords {
    word0: *mut u8,
    word1: usize,
    word2: usize,
}

/// Writes a complete serialized managed-pointer representation.
///
/// Callers must first establish that `dest` contains at least
/// [`ManagedPtr::SIZE`] bytes.
fn write_serialized_managed_ptr_words(dest: &mut [u8], words: SerializedManagedPtrWords) {
    let ptr_size = ObjectRef::SIZE;
    assert!(
        dest.len() >= ManagedPtr::SIZE,
        "ManagedPtr buffer too small"
    );
    // SAFETY: F10.RawMemoryAccessValid — Callers establish at least three pointer-sized words of space.
    // This raw pointer copy is intentional: serializing through `usize` would
    // discard provenance from managed owner pointers.
    unsafe {
        dest.as_mut_ptr()
            .cast::<*mut u8>()
            .write_unaligned(words.word0);
    }
    dest[ptr_size..ptr_size * 2].copy_from_slice(&words.word1.to_ne_bytes());
    dest[ptr_size * 2..ptr_size * 3].copy_from_slice(&words.word2.to_ne_bytes());
}

#[cfg(feature = "multithreading")]
fn cross_arena_storage_base_with_lease(ptr: crate::object::ObjectPtr) -> *mut u8 {
    // SAFETY: F1.ArenaGenerationMatch; F1.GcHandleRooted — the caller holds the
    // exact arena lease and independently guarantees a live, rooted object. The
    // lease prevents arena teardown but does not keep the object from collection.
    unsafe { ptr.as_heap_storage(|storage| storage.raw_data_ptr()) }
}

/// Supplies the cached live base while `ManagedPtr::write` checks its debug
/// round trip. The serialized reader must still recover the FrameHome/Static
/// origin and apply its encoded offset before the resulting address is checked.
#[cfg(debug_assertions)]
struct DebugWriteResolver {
    base: Option<NonNull<u8>>,
}

#[cfg(debug_assertions)]
impl<'gc> ManagedPtrResolver<'gc> for DebugWriteResolver {
    fn static_storage_base(
        &self,
        _metadata: &crate::pointer::StaticMetadata,
    ) -> Option<NonNull<u8>> {
        self.base
    }

    fn frame_home_block(&self, _serial: FrameHomeSerial) -> Option<(NonNull<u8>, usize)> {
        self.base.map(|base| (base, usize::MAX))
    }
}

impl<'gc> ManagedPtr<'gc> {
    /// The serialized representation always occupies three words.
    ///
    /// # Origin-handle convention
    ///
    /// - `Heap`: word 0 is the provenance-carrying GC handle pointer from
    ///   `Gc::as_ptr(handle)`; word 1 is the byte offset. Its address is used
    ///   only for tag/checksum inspection, and `ObjectRef::from_storage_ptr` is
    ///   the GC-handle reconstruction boundary.
    /// - Tag 1 (retired Stack encoding) is rejected.
    /// - `Static`: word 0 is
    ///   `7 | (1 << 3) | (registry_id << 6) | (offset << 38)`; word 1 is the
    ///   full compact byte offset, never a cached address. The packed word-0
    ///   field mirrors its low 26 bits.
    /// - `FrameHome`: tag 7, sub-tag 3. Word 0 carries the low 32 bits of the frame
    ///   serial and mirrors the low 26 bits of the offset; word 1 is the full 32-bit
    ///   offset with the high 32 bits of the serial above it. Never a cached address.
    /// - `Unmanaged`: word 0 is zero and word 1 is the absolute address. The
    ///   address is the value because this origin has no recoverable handle.
    /// - `CrossArenaObjectRef`: word 0 is the provenance-carrying lock pointer
    ///   tagged with low bits `5`; word 1 is `offset | (token << 32)`, where
    ///   `token` is the origin's non-reused, nonzero `RegistrationToken` copied
    ///   from its original registration (never a reusable `ArenaId` and never
    ///   issued at encode time). Decoding resolves the token to a lease for that
    ///   exact registration and uses the pointer already carried in word 0; a
    ///   token whose registration was removed is rejected with
    ///   `PointerDeserializationError::UnresolvedCrossArenaToken` before any
    ///   pointer is formed. These are process-local bytes, not a persisted or
    ///   foreign format: P/Invoke rejects by-value aggregates with ref fields.
    /// - `Transient`: word 0 carries its tag and offset and word 1 is the byte
    ///   offset, but it remains unserializable: metadata and resolved reads return
    ///   `PointerDeserializationError::UnknownSubtag`.
    ///
    /// Word 2 remains the integrity word `word0 ^ word1`, computed from the
    /// first pointer's address. Do not turn managed pointers into integers for
    /// storage or reconstruction.
    pub const SIZE: usize = ObjectRef::SIZE * 3;

    pub fn serialization_buffer() -> [u8; ObjectRef::SIZE * 3] {
        [0u8; ObjectRef::SIZE * 3]
    }

    /// Read only the serialized origin and offset metadata.
    ///
    /// This is the deserialization entry point for tracing and resurrection:
    /// those operations need the origin but must not manufacture executable
    /// FrameHome or Static addresses. Use [`ManagedPtr::read_resolved_unchecked`]
    /// for an execution path and supply a live-base resolver there.
    ///
    /// # Safety
    ///
    /// The `source` slice must contain valid bytes representing a `ManagedPtr`.
    /// If a CrossArena registration token resolves, the bytes must have been
    /// written for that registration and the original object allocation must
    /// be live and rooted. A token whose registration was removed (including
    /// after unregister/re-register of the same `ArenaId`) is rejected before
    /// use, because tokens are never reused; a lease alone does not root the
    /// object.
    pub unsafe fn read_metadata_unchecked(
        source: &[u8],
    ) -> Result<ManagedPtrInfo<'gc>, PointerDeserializationError> {
        let ptr_size = ObjectRef::SIZE;
        if source.len() < Self::SIZE {
            panic!("ManagedPtr::read_metadata_unchecked: buffer too small");
        }

        // SAFETY: F10.RawMemoryAccessValid — The source length was checked. Preserve the pointer
        // provenance carried by the ObjectRef storage slot.
        let word0: *mut u8 = unsafe { source.as_ptr().cast::<*mut u8>().read_unaligned() };
        let word0_addr = word0.addr();
        let word1 = usize::from_ne_bytes(
            source[ptr_size..ptr_size * 2]
                .try_into()
                .expect("slice length was checked above"),
        );
        let word2 = usize::from_ne_bytes(
            source[ptr_size * 2..ptr_size * 3]
                .try_into()
                .expect("slice length was checked above"),
        );

        if word2 != word0_addr ^ word1 {
            return Err(PointerDeserializationError::ChecksumMismatch);
        }

        match word0_addr & 7 {
            1 => Err(PointerDeserializationError::UnknownSubtag(1)),
            5 => {
                #[cfg(feature = "multithreading")]
                {
                    let raw_token = (word1 >> 32) as u32;
                    let token = dotnet_utils::gc::RegistrationToken::from_raw(raw_token).ok_or(
                        PointerDeserializationError::UnresolvedCrossArenaToken(raw_token),
                    )?;
                    #[expect(
                        clippy::multiple_unsafe_ops_per_block,
                        reason = "reconstructing the pointer and association share the decoder's original-registration contract"
                    )]
                    // SAFETY: F1.ArenaGenerationMatch; F1.GcHandleRooted — this function's `# Safety` contract makes `source` a valid encoding, so `word0` is a live lock address owned by the registration `token` names; the helper holds that registration's lease while the callback runs, and the callback does not access object storage.
                    let origin = unsafe {
                        cross_arena_ptr_from_ptr(word0, token, |ptr, lease| {
                            // SAFETY: F1.ArenaGenerationMatch; F1.GcHandleRooted — `lease` is the exact original registration named by the serialized token, and `ptr` is the live object the valid encoding pairs with it.
                            CrossArenaOrigin::new(ptr, lease)
                        })
                    }
                    .ok_or(
                        PointerDeserializationError::UnresolvedCrossArenaToken(raw_token),
                    )?;
                    Ok(ManagedPtrInfo {
                        address: None,
                        origin: PointerOrigin::CrossArenaObjectRef(origin),
                        offset: ByteOffset::new(word1 & 0xFFFF_FFFF),
                    })
                }
                #[cfg(not(feature = "multithreading"))]
                {
                    Err(PointerDeserializationError::UnknownTag(5))
                }
            }
            7 => match (word0_addr >> 3) & 7 {
                1 => {
                    let id = ((word0_addr >> 6) & 0xFFFF_FFFF) as u32;
                    let slot_offset = word0_addr >> 38;
                    if word1 > u32::MAX as usize || slot_offset != (word1 & 0x03FF_FFFF) {
                        return Err(PointerDeserializationError::OffsetMismatch);
                    }
                    let metadata = super::static_registry()
                        .get(&id)
                        .map(|metadata| metadata.clone())
                        .ok_or(PointerDeserializationError::InvalidStaticId(id))?;
                    Ok(ManagedPtrInfo {
                        address: None,
                        origin: PointerOrigin::Static(metadata),
                        offset: ByteOffset::new(word1),
                    })
                }
                2 => Err(PointerDeserializationError::UnknownSubtag(2)),
                FRAME_HOME_SUBTAG => {
                    let (serial, offset) = parse_frame_home_managed_ptr(word0_addr, word1)?;
                    Ok(ManagedPtrInfo {
                        address: None,
                        origin: PointerOrigin::FrameHome(serial),
                        offset,
                    })
                }
                subtag => Err(PointerDeserializationError::UnknownSubtag(subtag)),
            },
            0 => {
                // The untagged encoding is either a Heap ObjectRef or an
                // Unmanaged absolute address. Metadata decoding retains no
                // executable address in either case.
                // SAFETY: F1.GcHandleRooted — this function's `# Safety` contract makes `source` a valid encoding, so the untagged `word0` is null (Unmanaged) or a live object's storage pointer, as `from_storage_ptr` requires.
                let owner = unsafe { ObjectRef::from_storage_ptr(word0) };
                Ok(ManagedPtrInfo {
                    address: None,
                    origin: owner.0.map_or(PointerOrigin::Unmanaged, |h| {
                        PointerOrigin::Heap(ObjectRef(Some(h)))
                    }),
                    offset: ByteOffset::new(word1),
                })
            }
            tag => Err(PointerDeserializationError::UnknownTag(tag)),
        }
    }

    /// Read only managed-pointer metadata, branded with a GC token.
    ///
    /// The token carries the lifetime of decoded Heap and CrossArena origin
    /// handles; it does not grant FrameHome or Static storage access.
    ///
    /// # Safety
    ///
    /// The `source` slice must contain valid bytes representing a `ManagedPtr`.
    pub unsafe fn read_metadata_branded(
        source: &[u8],
        _gc: &Mutation<'gc>,
    ) -> Result<ManagedPtrInfo<'gc>, PointerDeserializationError> {
        // SAFETY: F6.NoEscapeAcrossArena — this function's `# Safety` contract (valid `ManagedPtr` bytes) is `read_metadata_unchecked`'s, and `_gc` brands the decoded handles with this arena's lifetime.
        unsafe { Self::read_metadata_unchecked(source) }
    }

    /// Read a `ManagedPtr` into an executable provenance-carrying address.
    ///
    /// FrameHome and Static encodings carry an offset, not an address. `resolver`
    /// must provide their live storage bases; this method applies the encoded
    /// offset to that pointer. Heap and CrossArena addresses likewise derive from
    /// their retained allocation bases. Wrapping formation is observation only:
    /// it proves neither bounds nor read/write/copy/atomic permission, and the
    /// raw address retains no root or borrow/lease. Later access needs the
    /// separately retained origin/owner and its own guards and range checks.
    /// FrameHome and Static origins are not owner-checked. A resolver that cannot supply
    /// a base receives a contextual deserialization error instead of an address
    /// reconstructed from an integer.
    ///
    /// # Safety
    ///
    /// The `source` slice must contain valid bytes representing a `ManagedPtr`.
    pub unsafe fn read_resolved_unchecked<R: ManagedPtrResolver<'gc> + ?Sized>(
        source: &[u8],
        resolver: &R,
    ) -> Result<ManagedPtrInfo<'gc>, PointerDeserializationError> {
        // SAFETY: F1.GcHandleRooted — this function's `# Safety` contract (valid `ManagedPtr` bytes) is `read_metadata_unchecked`'s.
        let info = unsafe { Self::read_metadata_unchecked(source) }?;
        // SAFETY: F1.GcHandleRooted — `info` was just decoded from `source`, which this function's `# Safety` contract makes a valid encoding.
        unsafe { Self::resolve_metadata_unchecked(source, resolver, info) }
    }

    /// Resolves the executable address for metadata decoded from `_source`.
    ///
    /// # Safety
    ///
    /// `info` must have been decoded from a valid `ManagedPtr` encoding, so its Heap and
    /// CrossArena origins name live objects.
    unsafe fn resolve_metadata_unchecked<R: ManagedPtrResolver<'gc> + ?Sized>(
        _source: &[u8],
        resolver: &R,
        mut info: ManagedPtrInfo<'gc>,
    ) -> Result<ManagedPtrInfo<'gc>, PointerDeserializationError> {
        let offset = info.offset.as_usize();
        info.address = match &info.origin {
            PointerOrigin::Static(metadata) => Some(
                resolver
                    .static_storage_base(metadata)
                    .and_then(|base| NonNull::new(base.as_ptr().wrapping_add(offset)))
                    .ok_or(PointerDeserializationError::UnresolvedStaticStorage)?,
            ),
            PointerOrigin::FrameHome(serial) => Some(
                crate::pointer::frame_home_window(resolver, *serial, offset, 0)
                    .map_err(|_| PointerDeserializationError::UnresolvedFrameHome(serial.get()))?,
            ),
            PointerOrigin::Heap(owner) => match owner.0 {
                Some(handle) => {
                    // SAFETY: F1.GcHandleRooted — The decoded Heap origin retains this
                    // handle, which roots the owner during base observation. The raw
                    // address is derived from the retained allocation base while the
                    // storage is borrowed; it carries provenance only and retains no
                    // root or borrow/lock after this statement. Later access needs its
                    // own owner and guard, range, and access-right checks.
                    let base = unsafe { handle.borrow().storage.raw_data_ptr() };
                    NonNull::new(base.wrapping_add(offset))
                }
                None => None,
            },
            PointerOrigin::Unmanaged => {
                // SAFETY: F10.RawMemoryAccessValid — `unmanaged_ptr_from_addr` only forms the address; its validity obligation moves to whoever later accesses through it.
                NonNull::new(unsafe { unmanaged_ptr_from_addr(offset) })
            }
            #[cfg(feature = "multithreading")]
            PointerOrigin::CrossArenaObjectRef(origin) => {
                let _lease = dotnet_utils::gc::try_acquire_lease_for_identity(origin.identity())
                    .ok_or(PointerDeserializationError::UnknownTag(5))?;
                // SAFETY: F1.ArenaGenerationMatch; F1.GcHandleRooted — `_lease` holds the origin's exact identity across the base read, and this function's `# Safety` contract (`info` decoded from a valid encoding) makes the object live.
                let ptr = unsafe { origin.object_ptr() };
                let base = cross_arena_storage_base_with_lease(ptr);
                NonNull::new(base.wrapping_add(offset))
            }
            PointerOrigin::Transient(_) => unreachable!("Transient cannot be decoded"),
        };
        Ok(info)
    }

    /// Read an executable managed pointer using a caller-owned Heap-handle cache.
    ///
    /// The cache applies only to a non-null, untagged Heap word. A hit still
    /// validates all three serialized words, resolves live object storage from
    /// the rooted handle, and applies the encoded offset. Other origins use the
    /// ordinary resolver path unchanged.
    ///
    /// # Safety
    ///
    /// The `source` slice must contain valid bytes representing a `ManagedPtr`.
    /// `cache` must retain its Heap values in a GC-traced owner and invalidate
    /// them at its collection-epoch boundary before a serialized handle address
    /// can be reused.
    pub unsafe fn read_resolved_with_heap_cache_unchecked<
        R: ManagedPtrResolver<'gc> + ?Sized,
        C: HeapManagedPtrDecodeCache<'gc> + ?Sized,
    >(
        source: &[u8],
        resolver: &R,
        cache: &mut C,
    ) -> Result<ManagedPtrInfo<'gc>, PointerDeserializationError> {
        let ptr_size = ObjectRef::SIZE;
        if source.len() < Self::SIZE {
            panic!("ManagedPtr::read_resolved_with_heap_cache_unchecked: buffer too small");
        }

        // SAFETY: F10.RawMemoryAccessValid — The complete three-word buffer was checked above. Reading
        // the first word as a pointer retains provenance for cache misses and
        // lets the numeric address be used only as a cache key/tag.
        let word0: *mut u8 = unsafe { source.as_ptr().cast::<*mut u8>().read_unaligned() };
        let word0_addr = word0.addr();
        let word1 = usize::from_ne_bytes(
            source[ptr_size..ptr_size * 2]
                .try_into()
                .expect("slice length was checked above"),
        );
        let word2 = usize::from_ne_bytes(
            source[ptr_size * 2..ptr_size * 3]
                .try_into()
                .expect("slice length was checked above"),
        );
        if word2 != word0_addr ^ word1 {
            return Err(PointerDeserializationError::ChecksumMismatch);
        }

        if word0_addr != 0 && word0_addr & 7 == 0 {
            let owner = match cache.get_heap_handle(word0_addr) {
                Some(owner) => owner,
                None => {
                    // SAFETY: F1.GcHandleRooted — `source` starts with the complete Heap ObjectRef
                    // representation validated as an untagged non-null word above.
                    let owner = unsafe { ObjectRef::from_storage_ptr(word0) };
                    if owner.0.is_some() {
                        cache.insert_heap_handle(word0_addr, owner);
                    }
                    owner
                }
            };
            let info = ManagedPtrInfo {
                address: None,
                origin: PointerOrigin::Heap(owner),
                offset: ByteOffset::new(word1),
            };
            // SAFETY: F10.RawMemoryAccessValid — `source` was completely validated above and `info` carries
            // either a rooted cached Heap handle or the normal decoded handle.
            return unsafe { Self::resolve_metadata_unchecked(source, resolver, info) };
        }

        // SAFETY: F1.GcHandleRooted — this function's `# Safety` contract (valid `ManagedPtr` bytes) is `read_resolved_unchecked`'s; the cache is not used for this encoding.
        unsafe { Self::read_resolved_unchecked(source, resolver) }
    }

    /// Read an executable managed pointer and brand decoded GC handles.
    ///
    /// # Safety
    ///
    /// The `source` slice must contain valid bytes representing a `ManagedPtr`.
    pub unsafe fn read_resolved_branded<R: ManagedPtrResolver<'gc> + ?Sized>(
        source: &[u8],
        gc: &Mutation<'gc>,
        resolver: &R,
    ) -> Result<ManagedPtrInfo<'gc>, PointerDeserializationError> {
        let _ = gc;
        // SAFETY: F6.NoEscapeAcrossArena — this function's `# Safety` contract (valid `ManagedPtr` bytes) is `read_resolved_unchecked`'s, and `gc` brands the decoded handles with this arena's lifetime.
        unsafe { Self::read_resolved_unchecked(source, resolver) }
    }

    /// Read a branded executable managed pointer using a caller-owned Heap cache.
    ///
    /// # Safety
    ///
    /// The `source` slice must contain valid bytes representing a `ManagedPtr`.
    /// The cache must satisfy [`HeapManagedPtrDecodeCache`]'s tracing and
    /// collection-epoch invalidation contract.
    pub unsafe fn read_resolved_branded_with_heap_cache_unchecked<
        R: ManagedPtrResolver<'gc> + ?Sized,
        C: HeapManagedPtrDecodeCache<'gc> + ?Sized,
    >(
        source: &[u8],
        gc: &Mutation<'gc>,
        resolver: &R,
        cache: &mut C,
    ) -> Result<ManagedPtrInfo<'gc>, PointerDeserializationError> {
        let _ = gc;
        // SAFETY: F6.NoEscapeAcrossArena — this function's `# Safety` contract (valid bytes and a traced, epoch-invalidated cache) is `read_resolved_with_heap_cache_unchecked`'s, and `gc` brands the decoded handles.
        unsafe { Self::read_resolved_with_heap_cache_unchecked(source, resolver, cache) }
    }

    /// Writes the three-word origin handle, offset, and XOR integrity word
    /// described by [`ManagedPtr::SIZE`].
    pub fn write(&self, dest: &mut [u8]) {
        self.validate_magic();

        let byte_offset = self.byte_offset();

        let (word0, word1) = match &self.origin {
            PointerOrigin::Heap(owner) => {
                let w0 = match owner.0 {
                    Some(handle) => gc_arena::Gc::as_ptr(handle).cast::<u8>().cast_mut(),
                    None => std::ptr::without_provenance_mut(0),
                };
                let w1 = byte_offset.as_usize();
                (w0, w1)
            }
            PointerOrigin::Static(metadata) => {
                let key = metadata.as_ref().clone();
                let id = *super::static_dedup_map().entry(key).or_insert_with(|| {
                    let new_id = super::NEXT_STATIC_ID.fetch_add(1, super::AtomicOrdering::SeqCst);
                    super::static_registry().insert(new_id, metadata.clone());
                    new_id
                });

                let w0: usize = 7
                    | (1 << 3)
                    | ((id as usize & 0xFFFFFFFF) << 6)
                    | (byte_offset.as_usize() << 38);
                let w1 = byte_offset.as_usize();
                (std::ptr::without_provenance_mut(w0), w1)
            }
            PointerOrigin::Unmanaged => {
                let w0 = std::ptr::without_provenance_mut(0);
                let w1 = self._value.map_or(0, |p| p.as_ptr().addr());
                (w0, w1)
            }
            #[cfg(feature = "multithreading")]
            PointerOrigin::CrossArenaObjectRef(origin) => {
                let w0 = origin
                    .as_ptr()
                    .cast::<u8>()
                    .cast_mut()
                    .map_addr(|addr| addr | 5);
                // Store the origin's non-reused registration token in the high 32 bits of
                // word1 and the 32-bit offset in the low bits. The token is copied from the
                // origin's original registration, never issued here, so stale origins encode
                // a token that cannot resolve. Decoding needs no pointer dereference.
                let offset_u32 = byte_offset.as_usize() & 0xFFFFFFFF;
                let token = origin.token().as_u32() as usize;
                let w1 = offset_u32 | (token << 32);
                (w0, w1)
            }
            PointerOrigin::FrameHome(serial) => {
                let (w0, w1) = frame_home_managed_ptr_words(*serial, byte_offset);
                (std::ptr::without_provenance_mut(w0), w1)
            }
            PointerOrigin::Transient(_) => {
                let w0: usize = 7 | (2 << 3) | (byte_offset.as_usize() << 6);
                let w1 = byte_offset.as_usize();
                (std::ptr::without_provenance_mut(w0), w1)
            }
        };

        write_serialized_managed_ptr_words(
            dest,
            SerializedManagedPtrWords {
                word0,
                word1,
                word2: word0.addr() ^ word1,
            },
        );

        // A frame-home pointer decoded without its frame's resolver has no address to recover.
        #[cfg(debug_assertions)]
        if !(matches!(self.origin, PointerOrigin::Transient(_))
            || (matches!(self.origin, PointerOrigin::FrameHome(_)) && self._value.is_none()))
        {
            let expected_address = self._value.map(|address| address.as_ptr().addr());
            let base = self._value.and_then(|address| {
                NonNull::new(address.as_ptr().wrapping_sub(byte_offset.as_usize()))
            });
            let resolver = DebugWriteResolver { base };
            let recovered =
                // SAFETY: F10.RawMemoryAccessValid — `dest` was just filled with one complete ManagedPtr encoding above.
                unsafe { Self::read_resolved_unchecked(dest, &resolver) }
                    .expect("ManagedPtr::write: recovery failed");
            // Canonicalize null Heap origins for comparison. Transient origins
            // retain their owner but are excluded here because they cannot be decoded.
            let self_origin_norm = self.origin.clone().normalize();
            let recovered_origin_norm = recovered.origin.clone().normalize();

            assert_eq!(
                recovered_origin_norm, self_origin_norm,
                "ManagedPtr serialization round-trip failed: origin mismatch. Original: {:?}, Recovered: {:?}",
                self.origin, recovered.origin
            );

            if !matches!(self_origin_norm, PointerOrigin::Unmanaged) {
                assert_eq!(
                    recovered.offset, byte_offset,
                    "ManagedPtr serialization round-trip failed: offset mismatch. Original: {:?}, Recovered: {:?}",
                    byte_offset, recovered.offset
                );
            }

            assert_eq!(
                recovered.address.map(|address| address.as_ptr().addr()),
                expected_address,
                "ManagedPtr serialization round-trip failed: resolved address mismatch. Original: {:?}, Recovered: {:?}",
                expected_address,
                recovered.address.map(|address| address.as_ptr().addr())
            );
        }
    }
}

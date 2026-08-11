use crate::{
    ByteOffset, StackSlotIndex,
    object::ObjectRef,
    pointer::{
        HeapManagedPtrDecodeCache, ManagedPtr, ManagedPtrInfo, ManagedPtrResolver,
        ManagedPtrStackInfo, PointerOrigin, unmanaged_ptr_from_addr,
    },
};
use dotnet_types::error::PointerDeserializationError;
use gc_arena::Mutation;
use std::ptr::NonNull;

#[cfg(feature = "multithreading")]
use crate::{ArenaId, pointer::cross_arena::cross_arena_ptr_from_addr};

#[cfg(feature = "multithreading")]
fn cross_arena_storage_base_with_lease(ptr: crate::object::ObjectPtr) -> *mut u8 {
    // This private helper is called only by `cross_arena_ptr_from_addr` callbacks,
    // whose contract holds the target arena lease for the callback duration.
    // SAFETY: F1.ArenaGenerationMatch; F1.GcHandleRooted — That callback-scoped lease keeps `ptr`'s object storage live while
    // its base address is read.
    unsafe { ptr.as_heap_storage(|storage| storage.raw_data_ptr()) }
}

/// Supplies the cached live base while `ManagedPtr::write` checks its debug
/// round trip. The serialized reader must still recover the Stack/Static
/// origin and apply its encoded offset before the resulting address is checked.
#[cfg(debug_assertions)]
struct DebugWriteResolver {
    base: Option<NonNull<u8>>,
}

#[cfg(debug_assertions)]
impl<'gc> ManagedPtrResolver<'gc> for DebugWriteResolver {
    fn stack_slot_base(&self, _slot: StackSlotIndex) -> Option<NonNull<u8>> {
        self.base
    }

    fn static_storage_base(
        &self,
        _metadata: &crate::pointer::StaticMetadata,
    ) -> Option<NonNull<u8>> {
        self.base
    }
}

impl<'gc> ManagedPtr<'gc> {
    /// The serialized representation always occupies three words.
    ///
    /// # Origin-handle convention
    ///
    /// - `Heap`: word 0 is the GC handle address from `Gc::as_ptr(handle).addr()`;
    ///   word 1 is the byte offset. `ObjectRef::read_unchecked` is the sole
    ///   GC-handle reconstruction boundary.
    /// - `Stack`: word 0 is `1 | (slot_idx << 3) | (offset << 33)`; word 1 is
    ///   the full compact byte offset, never a cached address. The packed
    ///   word-0 field mirrors its low 31 bits for legacy tag compatibility.
    /// - `Static`: word 0 is
    ///   `7 | (1 << 3) | (registry_id << 6) | (offset << 38)`; word 1 is the
    ///   full compact byte offset, never a cached address. The packed word-0
    ///   field mirrors its low 26 bits.
    /// - `Unmanaged`: word 0 is zero and word 1 is the absolute address. The
    ///   address is the value because this origin has no recoverable handle.
    /// - `CrossArenaObjectRef`: word 0 is `lock_ptr.addr() | 5`; word 1 is
    ///   `offset | (arena_id << 32)`. Its lock-pointer reconstruction is
    ///   confined to the scoped helper established before this redesign.
    /// - `Transient`: word 0 carries its tag and offset and word 1 is the byte
    ///   offset, but it remains unserializable: metadata and resolved reads return
    ///   `PointerDeserializationError::UnknownSubtag`.
    ///
    /// Word 2 remains the integrity word `word0 ^ word1`. Address words must
    /// be obtained with `ptr::addr()`, not provenance exposure.
    pub const SIZE: usize = ObjectRef::SIZE * 3;

    pub fn serialization_buffer() -> [u8; ObjectRef::SIZE * 3] {
        [0u8; ObjectRef::SIZE * 3]
    }

    /// Read stack-related metadata without constructing an executable pointer.
    ///
    /// Stack reallocation uses this to preserve the encoded slot and offset.
    /// Call [`ManagedPtr::read_resolved_unchecked`] when an executable pointer
    /// is required.
    ///
    /// # Safety
    ///
    /// The `source` slice must contain one complete serialized `ManagedPtr`.
    pub unsafe fn read_stack_info(source: &[u8]) -> ManagedPtrStackInfo {
        let ptr_size = ObjectRef::SIZE;
        if source.len() < Self::SIZE {
            panic!("ManagedPtr::read_stack_info: buffer too small");
        }

        let word0 = usize::from_ne_bytes(
            source[..ptr_size]
                .try_into()
                .expect("slice length was checked above"),
        );
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
        if word2 != word0 ^ word1 {
            panic!("ManagedPtr::read_stack_info: checksum mismatch");
        }

        if word0 & 1 != 0 {
            match word0 & 7 {
                1 => {
                    let packed_offset = word0 >> 33;
                    if word1 > u32::MAX as usize || packed_offset != (word1 & 0x7FFF_FFFF) {
                        panic!("ManagedPtr::read_stack_info: encoded offset mismatch");
                    }
                    let idx = (word0 >> 3) & 0x3FFF_FFFF;
                    return ManagedPtrStackInfo {
                        offset: ByteOffset::new(word1),
                        origin: PointerOrigin::Stack(StackSlotIndex::new(idx)),
                    };
                }
                7 if ((word0 >> 3) & 7) == 2 => {
                    return ManagedPtrStackInfo {
                        offset: ByteOffset::new(word0 >> 6),
                        // A transient origin has no recoverable metadata owner.
                        origin: PointerOrigin::Unmanaged,
                    };
                }
                _ => {}
            }
        }

        // This helper is intentionally metadata-only. Its only consumer is
        // stack reallocation, which acts solely on the Stack case above.
        ManagedPtrStackInfo {
            offset: ByteOffset::new(word1),
            origin: PointerOrigin::Unmanaged,
        }
    }

    /// Read only the serialized origin and offset metadata.
    ///
    /// This is the deserialization entry point for tracing and resurrection:
    /// those operations need the origin but must not manufacture executable
    /// Stack or Static addresses. Use [`ManagedPtr::read_resolved_unchecked`]
    /// for an execution path and supply a live-base resolver there.
    ///
    /// # Safety
    ///
    /// The `source` slice must contain valid bytes representing a `ManagedPtr`.
    pub unsafe fn read_metadata_unchecked(
        source: &[u8],
    ) -> Result<ManagedPtrInfo<'gc>, PointerDeserializationError> {
        let ptr_size = ObjectRef::SIZE;
        if source.len() < Self::SIZE {
            panic!("ManagedPtr::read_metadata_unchecked: buffer too small");
        }

        let word0 = usize::from_ne_bytes(
            source[..ptr_size]
                .try_into()
                .expect("slice length was checked above"),
        );
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

        if word2 != word0 ^ word1 {
            return Err(PointerDeserializationError::ChecksumMismatch);
        }

        match word0 & 7 {
            1 => {
                let packed_offset = word0 >> 33;
                if word1 > u32::MAX as usize || packed_offset != (word1 & 0x7FFF_FFFF) {
                    return Err(PointerDeserializationError::OffsetMismatch);
                }
                let slot_idx = (word0 >> 3) & 0x3FFF_FFFF;
                Ok(ManagedPtrInfo {
                    address: None,
                    origin: PointerOrigin::Stack(StackSlotIndex::new(slot_idx)),
                    offset: ByteOffset::new(word1),
                })
            }
            5 => {
                #[cfg(feature = "multithreading")]
                {
                    let owner_id_val = (word1 >> 32) as u32;
                    // Sign-extend `ArenaId::INVALID` and other negative IDs.
                    let owner_id = ArenaId::new(owner_id_val as i32 as i64 as u64);
                    // SAFETY: F1.GcHandleRooted — Valid CrossArena serialization contains a live lock
                    // address in the encoded arena. The helper holds that arena's
                    // lease while it reconstructs the origin handle; the callback
                    // deliberately does not access object storage.
                    let ptr = unsafe { cross_arena_ptr_from_addr(word0 & !7, owner_id, |ptr| ptr) }
                        .ok_or(PointerDeserializationError::UnknownTag(5))?;
                    Ok(ManagedPtrInfo {
                        address: None,
                        origin: PointerOrigin::CrossArenaObjectRef(ptr, owner_id),
                        offset: ByteOffset::new(word1 & 0xFFFF_FFFF),
                    })
                }
                #[cfg(not(feature = "multithreading"))]
                {
                    Err(PointerDeserializationError::UnknownTag(5))
                }
            }
            7 => match (word0 >> 3) & 7 {
                1 => {
                    let id = ((word0 >> 6) & 0xFFFF_FFFF) as u32;
                    let slot_offset = word0 >> 38;
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
                subtag => Err(PointerDeserializationError::UnknownSubtag(subtag)),
            },
            0 => {
                // The untagged encoding is either a Heap ObjectRef or an
                // Unmanaged absolute address. Metadata decoding retains no
                // executable address in either case.
                // SAFETY: F3.InteriorPointerRebased — `source` is a full ManagedPtr representation; its
                // initial word is the ObjectRef encoding expected by this helper.
                let owner = unsafe { ObjectRef::read_unchecked(&source[..ptr_size]) };
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
    /// handles; it does not grant Stack or Static storage access.
    ///
    /// # Safety
    ///
    /// The `source` slice must contain valid bytes representing a `ManagedPtr`.
    pub unsafe fn read_metadata_branded(
        source: &[u8],
        _gc: &Mutation<'gc>,
    ) -> Result<ManagedPtrInfo<'gc>, PointerDeserializationError> {
        // SAFETY: F6.NoEscapeAcrossArena — The branded caller supplies one complete ManagedPtr encoding.
        unsafe { Self::read_metadata_unchecked(source) }
    }

    /// Read a `ManagedPtr` into an executable provenance-carrying address.
    ///
    /// Stack and Static encodings carry an offset, not an address. `resolver`
    /// must provide their live storage bases; this method applies the encoded
    /// offset directly to that live pointer. A resolver that cannot supply a
    /// base receives a contextual deserialization error instead of an address
    /// reconstructed from an integer.
    ///
    /// # Safety
    ///
    /// The `source` slice must contain valid bytes representing a `ManagedPtr`.
    pub unsafe fn read_resolved_unchecked<R: ManagedPtrResolver<'gc> + ?Sized>(
        source: &[u8],
        resolver: &R,
    ) -> Result<ManagedPtrInfo<'gc>, PointerDeserializationError> {
        // SAFETY: F3.InteriorPointerRebased — The caller supplies one complete ManagedPtr encoding.
        let info = unsafe { Self::read_metadata_unchecked(source) }?;
        // SAFETY: F2.DescriptorMatchesEcmaLayout — The metadata came from the complete encoded source above.
        unsafe { Self::resolve_metadata_unchecked(source, resolver, info) }
    }

    unsafe fn resolve_metadata_unchecked<R: ManagedPtrResolver<'gc> + ?Sized>(
        _source: &[u8],
        resolver: &R,
        mut info: ManagedPtrInfo<'gc>,
    ) -> Result<ManagedPtrInfo<'gc>, PointerDeserializationError> {
        let offset = info.offset.as_usize();
        info.address = match &info.origin {
            PointerOrigin::Stack(slot) => Some(
                resolver
                    .stack_slot_base(*slot)
                    .and_then(|base| NonNull::new(base.as_ptr().wrapping_add(offset)))
                    .ok_or(PointerDeserializationError::UnresolvedStackSlot(
                        slot.as_usize(),
                    ))?,
            ),
            PointerOrigin::Static(metadata) => Some(
                resolver
                    .static_storage_base(metadata)
                    .and_then(|base| NonNull::new(base.as_ptr().wrapping_add(offset)))
                    .ok_or(PointerDeserializationError::UnresolvedStaticStorage)?,
            ),
            PointerOrigin::Heap(owner) => match owner.0 {
                Some(handle) => {
                    // SAFETY: F6.NoEscapeAcrossArena — Heap origin serialization contains a live branded
                    // handle. The resulting pointer is derived while its storage
                    // is borrowed from that handle.
                    let base = unsafe { handle.borrow().storage.raw_data_ptr() };
                    NonNull::new(base.wrapping_add(offset))
                }
                None => None,
            },
            PointerOrigin::Unmanaged => {
                // SAFETY: F2.DescriptorMatchesEcmaLayout — This branch is exclusively the address-only Unmanaged
                // wire representation; the caller owns its validity contract.
                NonNull::new(unsafe { unmanaged_ptr_from_addr(offset) })
            }
            #[cfg(feature = "multithreading")]
            PointerOrigin::CrossArenaObjectRef(_, owner_id) => {
                let word0 = usize::from_ne_bytes(
                    _source[..ObjectRef::SIZE]
                        .try_into()
                        .expect("slice length was checked by metadata decoding"),
                );
                // SAFETY: F1.GcHandleRooted — Metadata decoding already validated the CrossArena
                // representation. The helper holds the encoded arena lease for
                // the complete origin-to-storage traversal below.
                let base = unsafe {
                    cross_arena_ptr_from_addr(word0 & !7, *owner_id, |ptr| {
                        cross_arena_storage_base_with_lease(ptr)
                    })
                }
                .ok_or(PointerDeserializationError::UnknownTag(5))?;
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

        let word0 = usize::from_ne_bytes(
            source[..ptr_size]
                .try_into()
                .expect("slice length was checked above"),
        );
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
        if word2 != word0 ^ word1 {
            return Err(PointerDeserializationError::ChecksumMismatch);
        }

        if word0 != 0 && word0 & 7 == 0 {
            let owner = match cache.get_heap_handle(word0) {
                Some(owner) => owner,
                None => {
                    // SAFETY: F1.GcHandleRooted — `source` starts with the complete Heap ObjectRef
                    // representation validated as an untagged non-null word above.
                    let owner = unsafe { ObjectRef::read_unchecked(&source[..ptr_size]) };
                    if owner.0.is_some() {
                        cache.insert_heap_handle(word0, owner);
                    }
                    owner
                }
            };
            let info = ManagedPtrInfo {
                address: None,
                origin: PointerOrigin::Heap(owner),
                offset: ByteOffset::new(word1),
            };
            // SAFETY: F2.DescriptorMatchesEcmaLayout — `source` was completely validated above and `info` carries
            // either a rooted cached Heap handle or the normal decoded handle.
            return unsafe { Self::resolve_metadata_unchecked(source, resolver, info) };
        }

        // SAFETY: F1.GcHandleRooted — Non-Heap encodings keep their established decoding behavior.
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
        // SAFETY: F6.NoEscapeAcrossArena — The branded caller supplies one complete ManagedPtr encoding.
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
        // SAFETY: F6.NoEscapeAcrossArena — The branded caller supplies one complete ManagedPtr encoding
        // and a cache satisfying its documented contract.
        unsafe { Self::read_resolved_with_heap_cache_unchecked(source, resolver, cache) }
    }

    /// Writes the three-word origin handle, offset, and XOR integrity word
    /// described by [`ManagedPtr::SIZE`].
    pub fn write(&self, dest: &mut [u8]) {
        self.validate_magic();

        let ptr_size = ObjectRef::SIZE;
        let byte_offset = self.byte_offset();

        let (word0, word1) = match &self.origin {
            PointerOrigin::Stack(slot_idx) => {
                let w0: usize =
                    1 | ((slot_idx.as_usize() & 0x3FFFFFFF) << 3) | (byte_offset.as_usize() << 33);
                let w1 = byte_offset.as_usize();
                (w0, w1)
            }
            PointerOrigin::Heap(owner) => {
                let w0 = match owner.0 {
                    Some(h) => gc_arena::Gc::as_ptr(h).addr(),
                    None => 0,
                };
                let w1 = byte_offset.as_usize();
                (w0, w1)
            }
            PointerOrigin::Static(metadata) => {
                let key = (metadata.type_desc.clone(), metadata.generics.clone());
                let id = *super::static_dedup_map().entry(key).or_insert_with(|| {
                    let new_id =
                        super::NEXT_STATIC_ID.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    super::static_registry().insert(new_id, metadata.clone());
                    new_id
                });

                let w0: usize = 7
                    | (1 << 3)
                    | ((id as usize & 0xFFFFFFFF) << 6)
                    | (byte_offset.as_usize() << 38);
                let w1 = byte_offset.as_usize();
                (w0, w1)
            }
            PointerOrigin::Unmanaged => {
                let w0: usize = 0;
                let w1 = self._value.map_or(0, |p| p.as_ptr().addr());
                (w0, w1)
            }
            #[cfg(feature = "multithreading")]
            PointerOrigin::CrossArenaObjectRef(ptr, tid) => {
                let w0: usize = ptr.as_ptr().addr() | 5;
                // Store 32-bit ArenaId in high bits of word1, 32-bit offset in low bits.
                // This avoids dereferencing the pointer during deserialization/GC.
                let offset_u32 = byte_offset.as_usize() & 0xFFFFFFFF;
                let tid_u32 = (tid.as_u64() & 0xFFFFFFFF) as usize;
                let w1 = offset_u32 | (tid_u32 << 32);
                (w0, w1)
            }
            PointerOrigin::Transient(_) => {
                let w0: usize = 7 | (2 << 3) | (byte_offset.as_usize() << 6);
                let w1 = byte_offset.as_usize();
                (w0, w1)
            }
        };

        dest[0..ptr_size].copy_from_slice(&word0.to_ne_bytes());
        dest[ptr_size..ptr_size * 2].copy_from_slice(&word1.to_ne_bytes());
        let word2 = word0 ^ word1;
        dest[ptr_size * 2..ptr_size * 3].copy_from_slice(&word2.to_ne_bytes());

        #[cfg(debug_assertions)]
        if !matches!(self.origin, PointerOrigin::Transient(_)) {
            let expected_address = self._value.map(|address| address.as_ptr().addr());
            let base = self._value.and_then(|address| {
                NonNull::new(address.as_ptr().wrapping_sub(byte_offset.as_usize()))
            });
            let resolver = DebugWriteResolver { base };
            let recovered =
                // SAFETY: F3.InteriorPointerRebased — `dest` was just filled with one complete ManagedPtr encoding above.
                unsafe { Self::read_resolved_unchecked(dest, &resolver) }
                    .expect("ManagedPtr::write: recovery failed");
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

use crate::{
    ByteOffset, StackSlotIndex,
    object::ObjectRef,
    pointer::{
        ManagedPtr, ManagedPtrInfo, ManagedPtrStackInfo, PointerOrigin, nonnull_from_exposed_addr,
    },
};
use dotnet_types::error::PointerDeserializationError;
use gc_arena::Mutation;
use sptr::Strict;
use std::ptr::NonNull;

#[cfg(feature = "multithreading")]
use crate::{
    ArenaId,
    object::{OBJECT_MAGIC, ObjectPtr},
};
#[cfg(feature = "multithreading")]
use dotnet_utils::gc::ThreadSafeLock;

impl<'gc> ManagedPtr<'gc> {
    /// One word for the pointer, one word for the owner, and one word for the checksum.
    pub const SIZE: usize = ObjectRef::SIZE * 3;

    pub fn serialization_buffer() -> [u8; ObjectRef::SIZE * 3] {
        [0u8; ObjectRef::SIZE * 3]
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
                        offset: ByteOffset(off),
                        origin: PointerOrigin::Stack(StackSlotIndex(idx)),
                    };
                }
                7 if ((word0 >> 3) & 7) == 2 => {
                    // Transient (Tag 7, Subtag 2)
                    let off = word0 >> 6;
                    return ManagedPtrStackInfo {
                        address: nonnull_from_exposed_addr(word1),
                        offset: ByteOffset(off),
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
            offset: ByteOffset(word1),
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
                        origin: PointerOrigin::Stack(StackSlotIndex(slot_idx)),
                        offset: ByteOffset(slot_offset),
                    })
                }
                5 => {
                    // CrossArenaObjectRef (Tag 5)
                    // Recover ObjectPtr from Word 0 and ArenaId/Offset from Word 1.
                    // This avoids dereferencing the pointer during deserialization,
                    // making it safe even if memory contains garbage or is half-written.
                    #[cfg(feature = "multithreading")]
                    {
                        let lock_ptr = sptr::from_exposed_addr::<
                            ThreadSafeLock<crate::object::ObjectInner<'static>>,
                        >(word0 & !7);

                        let ptr = unsafe {
                            ObjectPtr::from_raw(lock_ptr)
                                .ok_or(PointerDeserializationError::UnknownTag(tag))?
                        };

                        let offset_val = word1 & 0xFFFFFFFF;
                        let owner_id_val = (word1 >> 32) as u32;
                        // Sign-extend from 32-bit to correctly handle ArenaId::INVALID (u64::MAX)
                        let owner_id = ArenaId::new(owner_id_val as i32 as i64 as u64);

                        // If we are in debug/validation mode, we still want to ensure
                        // the pointer is valid, but we do it AFTER recovering owner_id.
                        #[cfg(any(feature = "memory-validation", debug_assertions))]
                        {
                            if !lock_ptr.is_null() {
                                let inner_ptr = unsafe { (*lock_ptr).as_ptr() };
                                unsafe {
                                    (*inner_ptr)
                                        .magic
                                        .validate(OBJECT_MAGIC, "ManagedPtr::read (CrossArena)")
                                };
                            }
                        }

                        // RECOVERY: We must use the data storage pointer as the base, not ObjectInner.
                        // We use the recovered offset directly.
                        // Note: If we need the absolute address, we still have to dereference base_ptr,
                        // but we can postpone this or make it safe.
                        let base_ptr = unsafe {
                            if lock_ptr.is_null() {
                                NonNull::dangling().as_ptr()
                            } else {
                                let inner_ptr = (*lock_ptr).as_ptr();
                                (*inner_ptr).storage.raw_data_ptr()
                            }
                        };
                        let base_addr = base_ptr.expose_addr();

                        Ok(ManagedPtrInfo {
                            address: if base_ptr.is_null() {
                                None
                            } else {
                                nonnull_from_exposed_addr(base_addr + offset_val)
                            },
                            origin: PointerOrigin::CrossArenaObjectRef(ptr, owner_id),
                            offset: ByteOffset(offset_val),
                        })
                    }
                    #[cfg(not(feature = "multithreading"))]
                    {
                        // Fallback for non-multithreading mode (should not happen)
                        let _ = tag;
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
                                && let Some(meta) = super::static_registry().get(&id)
                            {
                                Ok(ManagedPtrInfo {
                                    address: raw_ptr,
                                    origin: PointerOrigin::Static(
                                        meta.type_desc.clone(),
                                        meta.generics.clone(),
                                    ),
                                    offset: ByteOffset(slot_offset),
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
            let offset = ByteOffset(word1);

            // Compute pointer from owner's data + offset
            let ptr = if let Some(handle) = owner.0 {
                let base_ptr = unsafe { handle.borrow().storage.raw_data_ptr() };
                if base_ptr.is_null() {
                    None
                } else {
                    NonNull::new(base_ptr.wrapping_add(offset.as_usize()))
                }
            } else if offset == ByteOffset::ZERO {
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
                let key = (type_desc.clone(), generics.clone());
                let id = *super::static_dedup_map().entry(key).or_insert_with(|| {
                    let new_id =
                        super::NEXT_STATIC_ID.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    super::static_registry().insert(
                        new_id,
                        std::sync::Arc::new(super::StaticMetadata {
                            type_desc: type_desc.clone(),
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
            PointerOrigin::CrossArenaObjectRef(ptr, tid) => {
                let w0: usize = ptr.as_ptr().expose_addr() | 5;
                // Store 32-bit ArenaId in high bits of word1, 32-bit offset in low bits.
                // This avoids dereferencing the pointer during deserialization/GC.
                let offset_u32 = self.offset.as_usize() & 0xFFFFFFFF;
                let tid_u32 = (tid.as_u64() & 0xFFFFFFFF) as usize;
                let w1 = offset_u32 | (tid_u32 << 32);
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
}

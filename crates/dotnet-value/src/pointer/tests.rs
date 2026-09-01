use crate::{
    object::{HeapStorage, ObjectRef},
    pointer::*,
    test_helpers::with_test_gc_context,
};
use gc_arena::{Arena, Gc, Rootable};
use std::{
    cell::Cell,
    ptr::NonNull,
    sync::{Mutex, OnceLock},
};

use dotnet_utils::sync::Arc;

#[derive(Copy, Clone)]
struct TestManagedPtrResolver {
    stack_base: Option<NonNull<u8>>,
    static_base: Option<NonNull<u8>>,
}

impl<'gc> ManagedPtrResolver<'gc> for TestManagedPtrResolver {
    fn stack_slot_base(&self, _slot: StackSlotIndex) -> Option<NonNull<u8>> {
        self.stack_base
    }

    fn static_storage_base(&self, _metadata: &StaticMetadata) -> Option<NonNull<u8>> {
        self.static_base
    }
}

const NO_MANAGED_BASES: TestManagedPtrResolver = TestManagedPtrResolver {
    stack_base: None,
    static_base: None,
};

#[derive(Default)]
struct CountingHeapCache<'gc> {
    entries: hashbrown::HashMap<usize, ObjectRef<'gc>>,
    hits: usize,
    misses: usize,
}

impl<'gc> HeapManagedPtrDecodeCache<'gc> for CountingHeapCache<'gc> {
    fn get_heap_handle(&mut self, serialized_handle: usize) -> Option<ObjectRef<'gc>> {
        match self.entries.get(&serialized_handle).copied() {
            Some(owner) => {
                self.hits += 1;
                Some(owner)
            }
            None => {
                self.misses += 1;
                None
            }
        }
    }

    fn insert_heap_handle(&mut self, serialized_handle: usize, owner: ObjectRef<'gc>) {
        self.entries.insert(serialized_handle, owner);
    }
}

fn static_reg_test_lock() -> &'static Mutex<()> {
    static L: OnceLock<Mutex<()>> = OnceLock::new();
    L.get_or_init(|| Mutex::new(()))
}

fn managed_ptr_to_heap_object_start<'gc>(obj: ObjectRef<'gc>) -> ManagedPtr<'gc> {
    let ptr = obj.with_data(|d| NonNull::new(d.as_ptr().cast_mut()));
    ManagedPtr::new(
        ptr,
        TypeDescription::NULL,
        Some(obj),
        false,
        Some(ByteOffset::new(0)),
    )
}

#[test]
#[cfg(feature = "memory-validation")]
#[should_panic(expected = "ManagedPtr::offset: bounds violation")]
fn test_managed_ptr_offset_oob() {
    with_test_gc_context(|gc_handle| {
        // Create a small object (4 bytes for Int32)
        let storage = HeapStorage::Str(crate::string::CLRString::from("test"));
        let obj = ObjectRef::new(gc_handle, storage);

        let ptr = managed_ptr_to_heap_object_start(obj);

        // Offset by much more than size of ValueType should panic.
        // SAFETY: F10.RawMemoryAccessValid — The test invokes the unsafe operation specifically to verify its
        // runtime bounds validation rejects this out-of-bounds offset.
        unsafe {
            ptr.offset(1000);
        }
    });
}

#[test]
fn test_managed_ptr_offset_valid() {
    with_test_gc_context(|gc_handle| {
        let storage = HeapStorage::Str(crate::string::CLRString::from("test"));
        let obj = ObjectRef::new(gc_handle, storage);

        let ptr = managed_ptr_to_heap_object_start(obj);

        // Offset by 4 bytes (end of object) should be valid
        // SAFETY: F1.GcHandleRooted — This test constructs valid backing storage and uses the pointer only within that storage's lifetime.
        unsafe {
            ptr.offset(4);
        }
    });
}

#[test]
fn test_heap_decode_cache_hit_miss_and_integrity() {
    with_test_gc_context(|gc_handle| {
        let object = ObjectRef::new(
            gc_handle,
            HeapStorage::Str(crate::string::CLRString::from("cached pointer")),
        );
        let offset = ByteOffset::new(2);
        let address = object.with_data(|data| {
            NonNull::new(data.as_ptr().wrapping_add(offset.as_usize()).cast_mut())
        });
        let ptr = ManagedPtr::new(
            address,
            TypeDescription::NULL,
            Some(object),
            false,
            Some(offset),
        );
        let mut buffer = ManagedPtr::serialization_buffer();
        ptr.write(&mut buffer);
        let mut cache = CountingHeapCache::default();

        // SAFETY: F10.RawMemoryAccessValid — The buffer was written from the live object above. This test
        // cache is short-lived inside the GC fixture; production cache owners
        // must instead trace their retained ObjectRefs across collection.
        let miss = unsafe {
            ManagedPtr::read_resolved_with_heap_cache_unchecked(
                &buffer,
                &NO_MANAGED_BASES,
                &mut cache,
            )
        }
        .unwrap();
        assert_eq!(miss.address, address);
        assert_eq!(cache.misses, 1);
        assert_eq!(cache.hits, 0);
        assert_eq!(cache.entries.len(), 1);

        // SAFETY: F1.GcHandleRooted — The cache now contains the live Heap handle decoded above;
        // the reader must still derive an address from freshly borrowed storage.
        let hit = unsafe {
            ManagedPtr::read_resolved_with_heap_cache_unchecked(
                &buffer,
                &NO_MANAGED_BASES,
                &mut cache,
            )
        }
        .unwrap();
        assert_eq!(hit.address, address);
        assert_eq!(cache.misses, 1);
        assert_eq!(cache.hits, 1);

        buffer[ManagedPtr::SIZE - 1] ^= 1;
        // SAFETY: F10.RawMemoryAccessValid — The corrupted buffer remains one complete representation;
        // checksum validation must reject it before consulting the cache.
        let corrupt = unsafe {
            ManagedPtr::read_resolved_with_heap_cache_unchecked(
                &buffer,
                &NO_MANAGED_BASES,
                &mut cache,
            )
        };
        assert!(matches!(
            corrupt,
            Err(dotnet_types::error::PointerDeserializationError::ChecksumMismatch)
        ));
        assert_eq!((cache.hits, cache.misses), (1, 1));
    });
}

#[test]
fn test_managed_ptr_serialization_roundtrip() {
    let _guard = static_reg_test_lock().lock().unwrap();

    with_test_gc_context(|gc_handle| {
        let mut buf = ManagedPtr::serialization_buffer();

        // 1. Unmanaged
        let unmanaged_addr = 0xDEADBEEFusize;
        let ptr_unmanaged = ManagedPtr::new(
            NonNull::new(std::ptr::without_provenance_mut(unmanaged_addr)),
            TypeDescription::NULL,
            None,
            false,
            None,
        );
        ptr_unmanaged.write(&mut buf);
        // SAFETY: F3.InteriorPointerRebased — `NoManagedPtrResolver` is sufficient for the Unmanaged encoding.
        let info = unsafe { ManagedPtr::read_resolved_unchecked(&buf, &NO_MANAGED_BASES) }.unwrap();
        assert_eq!(
            info.address,
            NonNull::new(std::ptr::without_provenance_mut(unmanaged_addr))
        );
        assert_eq!(info.origin, PointerOrigin::Unmanaged);
        assert_eq!(info.offset.as_usize(), unmanaged_addr);

        // 2. Stack
        let stack_slot = StackSlotIndex::new(123);
        let mut stack_storage = [0u8; 512];
        let stack_base = NonNull::new(stack_storage.as_mut_ptr()).unwrap();
        let stack_offset = 456;
        let stack_addr = NonNull::new(stack_base.as_ptr().wrapping_add(stack_offset));
        let ptr_stack = ManagedPtr::new(
            stack_addr,
            TypeDescription::NULL,
            None,
            false,
            Some(ByteOffset::new(stack_offset)),
        )
        .with_stack_origin(stack_slot);
        ptr_stack.write(&mut buf);
        let word1 = usize::from_ne_bytes(buf[8..16].try_into().unwrap());
        assert_eq!(
            word1, stack_offset,
            "Stack word1 must encode the byte offset"
        );
        // SAFETY: F3.InteriorPointerRebased — The bytes are complete Stack ManagedPtr encoding, but this resolver deliberately has no stack base.
        let unresolved_stack =
            unsafe { ManagedPtr::read_resolved_unchecked(&buf, &NO_MANAGED_BASES) };
        assert!(matches!(
            unresolved_stack,
            Err(dotnet_types::error::PointerDeserializationError::UnresolvedStackSlot(123))
        ));
        // SAFETY: F3.StackSlotMatchesView — The resolver returns the live stack-slot base used above.
        let info = unsafe {
            ManagedPtr::read_resolved_unchecked(
                &buf,
                &TestManagedPtrResolver {
                    stack_base: Some(stack_base),
                    static_base: None,
                },
            )
        }
        .unwrap();
        assert_eq!(info.address, stack_addr);
        assert_eq!(info.origin, PointerOrigin::Stack(stack_slot));

        // 3. Heap
        let s = crate::string::CLRString::from("test");
        let obj = ObjectRef::new(gc_handle, HeapStorage::Str(s));
        let offset = 2;
        let ptr = obj.with_data(|d| NonNull::new(d.as_ptr().wrapping_add(offset).cast_mut()));
        let ptr_heap = ManagedPtr::new(
            ptr,
            TypeDescription::NULL,
            Some(obj),
            false,
            Some(ByteOffset::new(offset)),
        );
        ptr_heap.write(&mut buf);
        // SAFETY: F6.NoEscapeAcrossArena — Heap decoding gets its base from the branded ObjectRef handle.
        let info = unsafe { ManagedPtr::read_resolved_unchecked(&buf, &NO_MANAGED_BASES) }.unwrap();
        assert_eq!(info.address, ptr);
        assert_eq!(info.origin, PointerOrigin::Heap(obj));
        assert_eq!(info.offset.as_usize(), offset);

        // 4. Static
        let type_desc = TypeDescription::NULL;
        let generics = GenericLookup::default();
        let mut static_storage = [0u8; 16];
        let static_base = NonNull::new(static_storage.as_mut_ptr()).unwrap();
        let static_offset = 8;
        let static_addr = NonNull::new(static_base.as_ptr().wrapping_add(static_offset));
        let ptr_static = ManagedPtr::new_static(
            static_addr,
            TypeDescription::NULL,
            type_desc,
            generics,
            false,
            ByteOffset::new(static_offset),
        );
        ptr_static.write(&mut buf);
        let word1 = usize::from_ne_bytes(buf[8..16].try_into().unwrap());
        assert_eq!(
            word1, static_offset,
            "Static word1 must encode the byte offset"
        );
        // SAFETY: F3.InteriorPointerRebased — The bytes are complete Static ManagedPtr encoding, but this resolver deliberately has no static base.
        let unresolved_static =
            unsafe { ManagedPtr::read_resolved_unchecked(&buf, &NO_MANAGED_BASES) };
        assert!(matches!(
            unresolved_static,
            Err(dotnet_types::error::PointerDeserializationError::UnresolvedStaticStorage)
        ));
        // SAFETY: F1.GcHandleRooted — The resolver returns the live static-storage base used above.
        let info = unsafe {
            ManagedPtr::read_resolved_unchecked(
                &buf,
                &TestManagedPtrResolver {
                    stack_base: None,
                    static_base: Some(static_base),
                },
            )
        }
        .unwrap();
        assert_eq!(info.address, static_addr);
        assert_eq!(info.offset.as_usize(), static_offset);

        // 5. CrossArenaObjectRef (if enabled)
        #[cfg(feature = "multithreading")]
        {
            use crate::object::ObjectPtr;
            let ptr_raw = obj.with_data(|d| d.as_ptr());
            // SAFETY: F1.GcHandleRooted — `obj` is kept alive for the duration of this test closure.
            // We only use this raw pointer to validate serialization/deserialization logic.
            let ptr_lock = Gc::as_ptr(obj.0.unwrap())
                .cast::<dotnet_utils::gc::ThreadSafeLock<crate::object::ObjectInner<'static>>>();
            // SAFETY: F1.GcHandleRooted — This test constructs valid backing storage and uses the pointer only within that storage's lifetime.
            let ptr = unsafe { ObjectPtr::from_raw(ptr_lock).unwrap() };
            let arena_id = ptr.owner_id();
            let cross_offset = 12;
            let cross_ptr = NonNull::new(ptr_raw.wrapping_add(cross_offset).cast_mut());
            let ptr_cross = ManagedPtr::new_cross_arena(
                cross_ptr,
                TypeDescription::NULL,
                ptr,
                arena_id,
                ByteOffset::new(cross_offset),
            );

            ptr_cross.write(&mut buf);
            // SAFETY: F1.GcHandleRooted — CrossArena decoding obtains its base while holding the encoded arena lease.
            let info =
                unsafe { ManagedPtr::read_resolved_unchecked(&buf, &NO_MANAGED_BASES) }.unwrap();
            assert_eq!(info.address, cross_ptr);
            if let PointerOrigin::CrossArenaObjectRef(recovered_ptr, recovered_arena) = info.origin
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
    with_test_gc_context(|_gc_handle| {
        let mut buf = [0u8; ManagedPtr::SIZE];

        // 1. Transient origin (Fixed behavior in Stage 1)
        let layout = Arc::new(crate::layout::FieldLayoutManager {
            fields: hashbrown::HashMap::new(),
            total_size: 0,
            alignment: 1,
            gc_desc: crate::layout::GcDesc::default(),
            has_ref_fields: false,
        });
        let storage = crate::storage::FieldStorage::new(layout, vec![]);
        let obj = Object::new(TypeDescription::NULL, GenericLookup::default(), storage);
        let transient_addr = 0x5000usize;
        let ptr_transient = ManagedPtr::new_transient(
            NonNull::new(std::ptr::without_provenance_mut(transient_addr)),
            TypeDescription::NULL,
            obj.clone(),
            ByteOffset::new(123), // Use a non-zero offset
        );

        ptr_transient.write(&mut buf);
        // SAFETY: F1.GcHandleRooted — This test constructs valid backing storage and uses the pointer only within that storage's lifetime.
        let result = unsafe { ManagedPtr::read_metadata_unchecked(&buf) };
        assert!(result.is_err(), "Transient recovery should fail for safety");

        let word0 = usize::from_ne_bytes(buf[0..8].try_into().unwrap());
        assert_eq!(word0 & 7, 7, "Transient should use Tag 7");
        assert_eq!((word0 >> 3) & 7, 2, "Transient should use Subtag 2");
        let word1 = usize::from_ne_bytes(buf[8..16].try_into().unwrap());
        assert_eq!(word1, 123, "Transient word1 must encode the byte offset");

        // 2. Tag Collision (Verified safe)
        // Any pointer with bit 0 = 0 is now guaranteed to be treated as Heap or Unmanaged,
        // never as Stack/Static/Transient.
        // We don't test misaligned pointers here as they would (correctly) panic in ObjectRef::read_unchecked.
    });
}

#[test]
fn test_stack_rebase_preserves_golden_three_word_encoding_and_full_offset() {
    let _guard = static_reg_test_lock().lock().unwrap();
    reset_static_registry();

    let stack_offset = (1usize << 31) + 17;
    let stack_slot = StackSlotIndex::new(9);
    let mut old_stack_storage = [0u8; 1];
    let old_stack_base = NonNull::new(old_stack_storage.as_mut_ptr()).unwrap();
    let mut new_stack_storage = [0u8; 1];
    let new_stack_base = NonNull::new(new_stack_storage.as_mut_ptr()).unwrap();
    assert_ne!(old_stack_base, new_stack_base);
    let stack = ManagedPtr::new(
        NonNull::new(old_stack_base.as_ptr().wrapping_add(stack_offset)),
        TypeDescription::NULL,
        None,
        false,
        Some(ByteOffset::new(stack_offset)),
    )
    .with_origin(PointerOrigin::Stack(stack_slot));
    let mut buffer = ManagedPtr::serialization_buffer();
    stack.write(&mut buffer);

    let ptr_size = ObjectRef::SIZE;
    let expected_word0 = 1 | ((stack_slot.as_usize() & 0x3FFF_FFFF) << 3) | (stack_offset << 33);
    let expected_word1 = stack_offset;
    let expected_word2 = expected_word0 ^ expected_word1;
    assert_eq!(
        usize::from_ne_bytes(buffer[..ptr_size].try_into().unwrap()),
        expected_word0
    );
    assert_eq!(
        usize::from_ne_bytes(buffer[ptr_size..ptr_size * 2].try_into().unwrap()),
        expected_word1,
        "Stack word1 must retain the full compact offset"
    );
    assert_eq!(
        usize::from_ne_bytes(buffer[ptr_size * 2..ptr_size * 3].try_into().unwrap()),
        expected_word2,
        "Stack word2 must remain the word0 XOR word1 checksum"
    );
    let golden_bytes = buffer;

    assert!(
        ManagedPtr::rebase_stack_pointer(&mut buffer, |slot| {
            assert_eq!(slot, stack_slot);
            Some(new_stack_base)
        })
        .unwrap()
    );
    assert_eq!(
        buffer, golden_bytes,
        "Stack rebase must preserve the canonical three-word representation"
    );

    // SAFETY: F3.InteriorPointerRebased — The canonical bytes are complete and the resolver
    // supplies the distinct current base for their stable logical slot.
    let rebased = unsafe {
        ManagedPtr::read_resolved_unchecked(
            &buffer,
            &TestManagedPtrResolver {
                stack_base: Some(new_stack_base),
                static_base: None,
            },
        )
    }
    .unwrap();
    assert_eq!(
        rebased.address,
        NonNull::new(new_stack_base.as_ptr().wrapping_add(stack_offset))
    );
    assert_eq!(rebased.origin, PointerOrigin::Stack(stack_slot));
    assert_eq!(rebased.offset, ByteOffset::new(stack_offset));

    // SAFETY: F3.InteriorPointerRebased — `buffer` was just written as one complete Stack ManagedPtr.
    let stack_info = unsafe { ManagedPtr::read_metadata_unchecked(&buffer) }.unwrap();
    assert_eq!(stack_info.offset, ByteOffset::new(stack_offset));

    // A self-consistent XOR alone must not permit the canonical word-1 offset
    // to disagree with the legacy packed mirror in word 0.
    let word0 = usize::from_ne_bytes(buffer[..ptr_size].try_into().unwrap());
    let mismatched_word1 = stack_offset + 1;
    buffer[ptr_size..ptr_size * 2].copy_from_slice(&mismatched_word1.to_ne_bytes());
    buffer[ptr_size * 2..ptr_size * 3].copy_from_slice(&(word0 ^ mismatched_word1).to_ne_bytes());
    // SAFETY: F10.RawMemoryAccessValid — The complete buffer is deliberately malformed to exercise validation.
    let mismatched = unsafe { ManagedPtr::read_metadata_unchecked(&buffer) };
    assert!(matches!(
        mismatched,
        Err(dotnet_types::error::PointerDeserializationError::OffsetMismatch)
    ));

    let address = NonNull::new(std::ptr::without_provenance_mut(0x4000)).unwrap();
    let static_offset = (1usize << 26) + 23;
    let static_ptr = ManagedPtr::new_static(
        Some(address),
        TypeDescription::NULL,
        TypeDescription::NULL,
        GenericLookup::default(),
        false,
        ByteOffset::new(static_offset),
    );
    static_ptr.write(&mut buffer);
    // SAFETY: F3.InteriorPointerRebased — `buffer` was just written as one complete Static ManagedPtr and
    // its metadata remains registered for this test.
    let static_info = unsafe { ManagedPtr::read_metadata_unchecked(&buffer) }.unwrap();
    assert_eq!(static_info.offset, ByteOffset::new(static_offset));
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
        NonNull::new(std::ptr::without_provenance_mut(0x1000)),
        TypeDescription::NULL,
        type_desc.clone(),
        generics.clone(),
        false,
        ByteOffset::new(0),
    );

    let ptr2 = ManagedPtr::new_static(
        NonNull::new(std::ptr::without_provenance_mut(0x2000)),
        TypeDescription::NULL,
        type_desc,
        generics.clone(),
        false,
        ByteOffset::new(0),
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
fn test_stack_rebase_roundtrip_miri() {
    let mut buf = [0u8; ManagedPtr::SIZE];
    let slot_idx = StackSlotIndex::new(42);
    let offset = 8;
    let w0: usize = 1 | ((slot_idx.as_usize() & 0x3FFFFFFF) << 3) | (offset << 33);
    let w1 = offset;
    let ptr_size = ObjectRef::SIZE;

    buf[..ptr_size].copy_from_slice(&w0.to_ne_bytes());
    buf[ptr_size..ptr_size * 2].copy_from_slice(&w1.to_ne_bytes());
    buf[ptr_size * 2..ptr_size * 3].copy_from_slice(&(w0 ^ w1).to_ne_bytes());
    let original = buf;

    let mut old_stack_storage = [0u8; 16];
    let old_stack_base = NonNull::new(old_stack_storage.as_mut_ptr()).unwrap();
    let mut new_stack_storage = [0u8; 16];
    let new_stack_base = NonNull::new(new_stack_storage.as_mut_ptr()).unwrap();
    assert_ne!(old_stack_base, new_stack_base);

    assert!(
        ManagedPtr::rebase_stack_pointer(&mut buf, |slot| {
            assert_eq!(slot, slot_idx);
            Some(new_stack_base)
        })
        .unwrap()
    );
    assert_eq!(buf, original);

    // SAFETY: F3.InteriorPointerRebased — The hand-written canonical bytes are complete and
    // the resolver supplies the current base for their stable slot.
    let info = unsafe {
        ManagedPtr::read_resolved_unchecked(
            &buf,
            &TestManagedPtrResolver {
                stack_base: Some(new_stack_base),
                static_base: None,
            },
        )
    }
    .unwrap();
    assert_eq!(
        info.address,
        NonNull::new(new_stack_base.as_ptr().wrapping_add(offset))
    );
    assert_eq!(info.origin, PointerOrigin::Stack(slot_idx));
    assert_eq!(info.offset.as_usize(), offset);
}

#[test]
fn test_safe_stack_rebase_rejects_invalid_input_without_mutation() {
    let _guard = static_reg_test_lock().lock().unwrap();
    reset_static_registry();
    let slot = StackSlotIndex::new(27);
    let offset = 24usize;
    let ptr_size = ObjectRef::SIZE;
    let word0 = 1 | ((slot.as_usize() & 0x3FFF_FFFF) << 3) | (offset << 33);
    let word1 = offset;
    let mut valid_stack = ManagedPtr::serialization_buffer();
    valid_stack[..ptr_size].copy_from_slice(&word0.to_ne_bytes());
    valid_stack[ptr_size..ptr_size * 2].copy_from_slice(&word1.to_ne_bytes());
    valid_stack[ptr_size * 2..ptr_size * 3].copy_from_slice(&(word0 ^ word1).to_ne_bytes());

    for actual in [0, ManagedPtr::SIZE - 1] {
        let mut truncated = vec![0xA5; actual];
        let original = truncated.clone();
        let resolver_called = Cell::new(false);
        assert_eq!(
            ManagedPtr::rebase_stack_pointer(&mut truncated, |_| {
                resolver_called.set(true);
                None
            }),
            Err(
                dotnet_types::error::PointerDeserializationError::BufferTooSmall {
                    expected: ManagedPtr::SIZE,
                    actual,
                }
            )
        );
        assert_eq!(truncated, original);
        assert!(!resolver_called.get());
    }

    let mut bad_checksum = valid_stack;
    bad_checksum[ManagedPtr::SIZE - 1] ^= 1;
    let original = bad_checksum;
    let resolver_called = Cell::new(false);
    assert_eq!(
        ManagedPtr::rebase_stack_pointer(&mut bad_checksum, |_| {
            resolver_called.set(true);
            None
        }),
        Err(dotnet_types::error::PointerDeserializationError::ChecksumMismatch)
    );
    assert_eq!(bad_checksum, original);
    assert!(!resolver_called.get());

    let mut offset_mismatch = valid_stack;
    let mismatched_word1 = word1 + 1;
    offset_mismatch[ptr_size..ptr_size * 2].copy_from_slice(&mismatched_word1.to_ne_bytes());
    offset_mismatch[ptr_size * 2..ptr_size * 3]
        .copy_from_slice(&(word0 ^ mismatched_word1).to_ne_bytes());
    let original = offset_mismatch;
    let resolver_called = Cell::new(false);
    assert_eq!(
        ManagedPtr::rebase_stack_pointer(&mut offset_mismatch, |_| {
            resolver_called.set(true);
            None
        }),
        Err(dotnet_types::error::PointerDeserializationError::OffsetMismatch)
    );
    assert_eq!(offset_mismatch, original);
    assert!(!resolver_called.get());

    let mut unresolved = valid_stack;
    let original = unresolved;
    let resolved_slot = Cell::new(None);
    assert_eq!(
        ManagedPtr::rebase_stack_pointer(&mut unresolved, |resolved| {
            resolved_slot.set(Some(resolved));
            None
        }),
        Err(dotnet_types::error::PointerDeserializationError::UnresolvedStackSlot(slot.as_usize()))
    );
    assert_eq!(unresolved, original);
    assert_eq!(resolved_slot.get(), Some(slot));

    let unmanaged = ManagedPtr::new(
        Some(NonNull::new(std::ptr::without_provenance_mut(0x5000)).unwrap()),
        TypeDescription::NULL,
        None,
        false,
        None,
    );
    let mut non_stack = ManagedPtr::serialization_buffer();
    unmanaged.write(&mut non_stack);
    let mut static_storage = [0u8; 1];
    let static_ptr = ManagedPtr::new_static(
        NonNull::new(static_storage.as_mut_ptr()),
        TypeDescription::NULL,
        TypeDescription::NULL,
        GenericLookup::default(),
        false,
        ByteOffset::ZERO,
    );
    let mut static_non_stack = ManagedPtr::serialization_buffer();
    static_ptr.write(&mut static_non_stack);

    for non_stack in [&mut non_stack, &mut static_non_stack] {
        let original = *non_stack;
        let resolver_called = Cell::new(false);
        assert_eq!(
            ManagedPtr::rebase_stack_pointer(non_stack, |_| {
                resolver_called.set(true);
                None
            }),
            Ok(false)
        );
        assert_eq!(*non_stack, original);
        assert!(!resolver_called.get());
    }
}

#[test]
fn test_cached_stack_rebase_updates_address_and_preserves_unresolved_or_non_stack_pointers() {
    let slot = StackSlotIndex::new(12);
    let offset = ByteOffset::new(4);
    let mut old_storage = [0u8; 16];
    let old_base = NonNull::new(old_storage.as_mut_ptr()).unwrap();
    let old_address = NonNull::new(old_base.as_ptr().wrapping_add(offset.as_usize())).unwrap();
    let mut new_storage = [0u8; 16];
    let new_base = NonNull::new(new_storage.as_mut_ptr()).unwrap();
    let new_address = NonNull::new(new_base.as_ptr().wrapping_add(offset.as_usize())).unwrap();
    assert_ne!(old_base, new_base);

    let mut stack_pointer = ManagedPtr::new(
        Some(old_address),
        TypeDescription::NULL,
        None,
        false,
        Some(offset),
    )
    .with_stack_origin(slot);
    assert_eq!(
        stack_pointer.rebase_cached_stack_pointer(|resolved_slot| {
            assert_eq!(resolved_slot, slot);
            Some(new_base)
        }),
        Ok(true)
    );

    assert_eq!(
        stack_pointer.rebase_cached_stack_pointer(|_| None),
        Err(dotnet_types::error::PointerDeserializationError::UnresolvedStackSlot(slot.as_usize()))
    );
    let stack_info = stack_pointer.into_info();
    assert_eq!(stack_info.address, Some(new_address));
    assert_eq!(stack_info.origin, PointerOrigin::Stack(slot));
    assert_eq!(stack_info.offset, offset);

    let mut non_stack_pointer = ManagedPtr::null();
    let resolver_called = Cell::new(false);
    assert_eq!(
        non_stack_pointer.rebase_cached_stack_pointer(|_| {
            resolver_called.set(true);
            None
        }),
        Ok(false)
    );
    assert!(!resolver_called.get());
    let non_stack_info = non_stack_pointer.into_info();
    assert_eq!(non_stack_info.address, None);
    assert_eq!(non_stack_info.origin, PointerOrigin::Unmanaged);
    assert_eq!(non_stack_info.offset, ByteOffset::ZERO);
}

#[test]
fn test_managed_ptr_unmanaged_roundtrip_miri() {
    let mut buf = [0u8; ManagedPtr::SIZE];
    let addr = 0xAAAA_BBBB_CCCC_DDDDusize;
    let ptr = ManagedPtr::new(
        NonNull::new(std::ptr::without_provenance_mut(addr)),
        TypeDescription::NULL,
        None,
        false,
        None,
    );

    ptr.write(&mut buf);
    // SAFETY: F1.GcHandleRooted — This test constructs valid backing storage and uses the pointer only within that storage's lifetime.
    let info = unsafe { ManagedPtr::read_resolved_unchecked(&buf, &NO_MANAGED_BASES) }.unwrap();
    assert_eq!(
        info.address,
        NonNull::new(std::ptr::without_provenance_mut(addr))
    );
    assert_eq!(info.origin, PointerOrigin::Unmanaged);
    assert_eq!(info.offset.as_usize(), addr);
}

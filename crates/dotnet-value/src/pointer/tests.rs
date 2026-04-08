use crate::{
    object::{HeapStorage, ObjectRef},
    pointer::*,
};
use gc_arena::{Arena, Gc, Rootable};
use sptr::Strict;
use std::sync::{Mutex, OnceLock};

#[cfg(feature = "memory-validation")]
use dotnet_utils::sync::MANAGED_THREAD_ID;

#[cfg(feature = "memory-validation")]
struct ManagedThreadIdGuard {
    previous: Option<crate::ArenaId>,
}

#[cfg(feature = "memory-validation")]
impl ManagedThreadIdGuard {
    fn set(id: crate::ArenaId) -> Self {
        let previous = MANAGED_THREAD_ID.with(|thread_id| {
            let prev = thread_id.get();
            thread_id.set(Some(id));
            prev
        });
        Self { previous }
    }
}

#[cfg(feature = "memory-validation")]
impl Drop for ManagedThreadIdGuard {
    fn drop(&mut self) {
        MANAGED_THREAD_ID.with(|thread_id| thread_id.set(self.previous));
    }
}

fn static_reg_test_lock() -> &'static Mutex<()> {
    static L: OnceLock<Mutex<()>> = OnceLock::new();
    L.get_or_init(|| Mutex::new(()))
}

#[test]
#[cfg(feature = "memory-validation")]
#[should_panic(expected = "ManagedPtr::offset: bounds violation")]
fn test_managed_ptr_offset_oob() {
    type TestRoot = Rootable![()];
    let arena = Arena::<TestRoot>::new(|_mc| ());
    #[cfg(feature = "multithreading")]
    let _arena_handle_owner = dotnet_utils::gc::ArenaHandle::new(crate::ArenaId(0));
    #[cfg(feature = "memory-validation")]
    let _thread_id_guard = ManagedThreadIdGuard::set(crate::ArenaId(0));
    #[cfg(feature = "multithreading")]
    let arena_handle = unsafe {
        std::mem::transmute::<
            &dotnet_utils::gc::ArenaHandleInner,
            &'static dotnet_utils::gc::ArenaHandleInner,
        >(_arena_handle_owner.as_inner())
    };

    arena.mutate(|gc, _root| {
        let gc_handle = dotnet_utils::gc::GCHandle::new(
            gc,
            #[cfg(feature = "multithreading")]
            arena_handle,
            #[cfg(feature = "memory-validation")]
            crate::ArenaId(0),
        );

        // Create a small object (4 bytes for Int32)
        let storage = HeapStorage::Str(crate::string::CLRString::from("test"));
        let obj = ObjectRef::new(gc_handle, storage);

        let ptr = ManagedPtr::new(
            obj.with_data(|d| NonNull::new(d.as_ptr().cast_mut())),
            TypeDescription::NULL,
            Some(obj),
            false,
            Some(ByteOffset(0)),
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
    let _arena_handle_owner = dotnet_utils::gc::ArenaHandle::new(crate::ArenaId(0));
    #[cfg(feature = "memory-validation")]
    let _thread_id_guard = ManagedThreadIdGuard::set(crate::ArenaId(0));
    #[cfg(feature = "multithreading")]
    let arena_handle = unsafe {
        std::mem::transmute::<
            &dotnet_utils::gc::ArenaHandleInner,
            &'static dotnet_utils::gc::ArenaHandleInner,
        >(_arena_handle_owner.as_inner())
    };

    arena.mutate(|gc, _root| {
        let gc_handle = dotnet_utils::gc::GCHandle::new(
            gc,
            #[cfg(feature = "multithreading")]
            arena_handle,
            #[cfg(feature = "memory-validation")]
            crate::ArenaId(0),
        );

        let storage = HeapStorage::Str(crate::string::CLRString::from("test"));
        let obj = ObjectRef::new(gc_handle, storage);

        let ptr = ManagedPtr::new(
            obj.with_data(|d| NonNull::new(d.as_ptr().cast_mut())),
            TypeDescription::NULL,
            Some(obj),
            false,
            Some(ByteOffset(0)),
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
    let _arena_handle_owner = dotnet_utils::gc::ArenaHandle::new(crate::ArenaId(0));
    #[cfg(feature = "memory-validation")]
    let _thread_id_guard = ManagedThreadIdGuard::set(crate::ArenaId(0));
    #[cfg(feature = "multithreading")]
    let arena_handle = unsafe {
        std::mem::transmute::<
            &dotnet_utils::gc::ArenaHandleInner,
            &'static dotnet_utils::gc::ArenaHandleInner,
        >(_arena_handle_owner.as_inner())
    };

    arena.mutate(|gc, _root| {
        let gc_handle = dotnet_utils::gc::GCHandle::new(
            gc,
            #[cfg(feature = "multithreading")]
            arena_handle,
            #[cfg(feature = "memory-validation")]
            crate::ArenaId(0),
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
        let stack_slot = StackSlotIndex(123);
        let stack_addr = 0x1000usize;
        let ptr_stack = ManagedPtr::new(
            nonnull_from_exposed_addr(stack_addr),
            TypeDescription::NULL,
            None,
            false,
            Some(ByteOffset(456)),
        )
        .with_stack_origin(stack_slot, ByteOffset(0));

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
            Some(ByteOffset(offset)),
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
            type_desc.clone(),
            generics.clone(),
            false,
            ByteOffset(static_offset),
        );

        ptr_static.write(&mut buf);
        let info = unsafe { ManagedPtr::read_unchecked(&buf) }.unwrap();
        assert_eq!(
            info.address,
            nonnull_from_exposed_addr(static_addr + static_offset)
        );
        let (resolved_type, resolved_generics) =
            info.origin.static_parts().expect("expected static origin");
        assert_eq!(resolved_type, &type_desc);
        assert_eq!(resolved_generics, &generics);
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
                .cast::<dotnet_utils::gc::ThreadSafeLock<crate::object::ObjectInner<'static>>>();
            let ptr = unsafe { ObjectPtr::from_raw(ptr_lock).unwrap() };
            let arena_id = ptr.owner_id();
            let cross_offset = 12;
            let ptr_cross = ManagedPtr::new_cross_arena(
                nonnull_from_exposed_addr(base_addr + cross_offset),
                TypeDescription::NULL,
                ptr,
                arena_id,
                ByteOffset(cross_offset),
            );

            ptr_cross.write(&mut buf);
            let info = unsafe { ManagedPtr::read_unchecked(&buf) }.unwrap();
            assert_eq!(
                info.address,
                nonnull_from_exposed_addr(base_addr + cross_offset)
            );
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
    type TestRoot = Rootable![()];
    let arena = Arena::<TestRoot>::new(|_mc| ());
    #[cfg(feature = "multithreading")]
    let _arena_handle_owner = dotnet_utils::gc::ArenaHandle::new(crate::ArenaId(0));
    #[cfg(feature = "memory-validation")]
    let _thread_id_guard = ManagedThreadIdGuard::set(crate::ArenaId(0));
    #[cfg(feature = "multithreading")]
    let arena_handle = unsafe {
        std::mem::transmute::<
            &dotnet_utils::gc::ArenaHandleInner,
            &'static dotnet_utils::gc::ArenaHandleInner,
        >(_arena_handle_owner.as_inner())
    };

    arena.mutate(|gc, _root| {
        let _gc_handle = &dotnet_utils::gc::GCHandle::new(
            gc,
            #[cfg(feature = "multithreading")]
            arena_handle,
            #[cfg(feature = "memory-validation")]
            crate::ArenaId(0),
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
        let obj = Object::new(TypeDescription::NULL, GenericLookup::default(), storage);
        let transient_addr = 0x5000usize;
        let ptr_transient = ManagedPtr::new_transient(
            nonnull_from_exposed_addr(transient_addr),
            TypeDescription::NULL,
            obj.clone(),
            ByteOffset(123), // Use a non-zero offset
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
        type_desc.clone(),
        generics.clone(),
        false,
        ByteOffset(0),
    );

    let ptr2 = ManagedPtr::new_static(
        nonnull_from_exposed_addr(0x2000),
        TypeDescription::NULL,
        type_desc,
        generics.clone(),
        false,
        ByteOffset(0),
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
    let slot_idx = StackSlotIndex(42);
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

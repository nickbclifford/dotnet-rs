#[cfg(test)]
mod tests {
    use crate::{LoadType, StackValue, StoreType};
    use dotnet_utils::sync::Ordering as AtomicOrdering;
    #[repr(align(8))]
    struct Aligned8([u8; 8]);
    /// Runs `f` against a fresh, 8-byte-aligned slot that is only ever accessed through the
    /// returned raw pointer (never through the owning place while the pointer is live), so the
    /// pointer keeps write permission under Tree/Stacked Borrows.
    fn with_atomic_slot(f: impl FnOnce(*mut u8)) {
        let mut data = Aligned8([0u8; 8]);
        f((&raw mut data.0).cast::<u8>());
    }

    // Each width gets its own slot: mixed-size atomic accesses to one location trip a Miri ICE
    // in its weak-memory store buffer, and width coverage, not mixed-size overlap, is what this
    // test is for. Overlapping mixed-size access is not yet handled (shared raw-memory access;
    // see docs/plans/README.md).
    #[test]
    #[expect(
        clippy::multiple_unsafe_ops_per_block,
        reason = "the test intentionally exercises a sequence of atomic operations over one owned buffer"
    )]
    fn test_atomic_load_store() {
        // SAFETY: F4.WidthAligned — Every slot is a live, owned, 8-byte-aligned buffer accessed
        // only through `ptr` for the duration of each closure.
        with_atomic_slot(|ptr| unsafe {
            StackValue::Int32(42).store_atomic(ptr, StoreType::Int8, AtomicOrdering::SeqCst);
            let val = StackValue::load_atomic(ptr, LoadType::Int8, AtomicOrdering::SeqCst);
            assert_eq!(val, StackValue::Int32(42));
            assert_eq!(ptr.read(), 42);
            StackValue::Int32(-1).store_atomic(ptr, StoreType::Int8, AtomicOrdering::SeqCst);
            let val = StackValue::load_atomic(ptr, LoadType::Int8, AtomicOrdering::SeqCst);
            assert_eq!(val, StackValue::Int32(-1));
            assert_eq!(ptr.read(), 255);
            StackValue::Int32(200).store_atomic(ptr, StoreType::Int8, AtomicOrdering::SeqCst);
            let val = StackValue::load_atomic(ptr, LoadType::UInt8, AtomicOrdering::SeqCst);
            assert_eq!(val, StackValue::Int32(200));
        });
        // SAFETY: F4.WidthAligned — a fresh owned, 8-byte-aligned slot, accessed only through `ptr`.
        with_atomic_slot(|ptr| unsafe {
            StackValue::Int32(12345).store_atomic(ptr, StoreType::Int16, AtomicOrdering::SeqCst);
            let val = StackValue::load_atomic(ptr, LoadType::Int16, AtomicOrdering::SeqCst);
            assert_eq!(val, StackValue::Int32(12345));
        });
        // SAFETY: F4.WidthAligned — a fresh owned, 8-byte-aligned slot, accessed only through `ptr`.
        with_atomic_slot(|ptr| unsafe {
            StackValue::Int32(0x12345678).store_atomic(
                ptr,
                StoreType::Int32,
                AtomicOrdering::SeqCst,
            );
            let val = StackValue::load_atomic(ptr, LoadType::Int32, AtomicOrdering::SeqCst);
            assert_eq!(val, StackValue::Int32(0x12345678));
        });
        // SAFETY: F4.WidthAligned — a fresh owned, 8-byte-aligned slot, accessed only through `ptr`.
        with_atomic_slot(|ptr| unsafe {
            StackValue::Int64(0x123456789ABCDEF0).store_atomic(
                ptr,
                StoreType::Int64,
                AtomicOrdering::SeqCst,
            );
            let val = StackValue::load_atomic(ptr, LoadType::Int64, AtomicOrdering::SeqCst);
            assert_eq!(val, StackValue::Int64(0x123456789ABCDEF0));
        });
        // SAFETY: F4.WidthAligned — a fresh owned, 8-byte-aligned slot, accessed only through `ptr`.
        with_atomic_slot(|ptr| unsafe {
            StackValue::NativeFloat(1.25).store_atomic(
                ptr,
                StoreType::Float32,
                AtomicOrdering::SeqCst,
            );
            let val = StackValue::load_atomic(ptr, LoadType::Float32, AtomicOrdering::SeqCst);
            assert_eq!(val, StackValue::NativeFloat(1.25));
        });
        // SAFETY: F4.WidthAligned — a fresh owned, 8-byte-aligned slot, accessed only through `ptr`.
        with_atomic_slot(|ptr| unsafe {
            StackValue::NativeFloat(1.23456789).store_atomic(
                ptr,
                StoreType::Float64,
                AtomicOrdering::SeqCst,
            );
            let val = StackValue::load_atomic(ptr, LoadType::Float64, AtomicOrdering::SeqCst);
            assert_eq!(val, StackValue::NativeFloat(1.23456789));
        });
    }
    #[test]
    #[expect(
        clippy::multiple_unsafe_ops_per_block,
        reason = "the test intentionally exercises object atomic load/store sequencing over an initialized pointer slot"
    )]
    fn test_atomic_object_load_store() {
        use crate::{
            object::{HeapStorage, ObjectRef},
            test_helpers::with_test_gc_context,
        };

        with_test_gc_context(|gc_handle| {
            let storage = HeapStorage::Str(crate::string::CLRString::from("test"));
            let obj = ObjectRef::new(gc_handle, storage);
            // An initialized pointer slot lets Miri validate the provenance
            // preserved by the object storage encoding across atomic access.
            let mut stored_object_ptr: *mut u8 = std::ptr::null_mut();
            let ptr = (&mut stored_object_ptr as *mut *mut u8).cast::<u8>();
            // SAFETY: F4.WidthAligned — `ptr` is the address of the local `stored_object_ptr`, an initialized, aligned pointer slot only this test accesses, and `obj` stays live for the closure.
            unsafe {
                StackValue::ObjectRef(obj).store_atomic(
                    ptr,
                    StoreType::Object,
                    AtomicOrdering::SeqCst,
                );
                let encoded_ptr =
                    dotnet_utils::sync::AtomicPtr::<u8>::from_ptr(ptr.cast::<*mut u8>())
                        .load(dotnet_utils::sync::Ordering::SeqCst);
                assert_eq!(encoded_ptr, obj.to_storage_ptr());
                let val = StackValue::load_atomic(ptr, LoadType::Object, AtomicOrdering::SeqCst);
                if let StackValue::ObjectRef(read_obj) = val {
                    assert_eq!(read_obj, obj);
                } else {
                    panic!("Expected ObjectRef");
                }
                StackValue::ObjectRef(ObjectRef(None)).store_atomic(
                    ptr,
                    StoreType::Object,
                    AtomicOrdering::SeqCst,
                );
                assert!(
                    dotnet_utils::sync::AtomicPtr::<u8>::from_ptr(ptr.cast::<*mut u8>())
                        .load(dotnet_utils::sync::Ordering::SeqCst)
                        .is_null()
                );
                let val = StackValue::load_atomic(ptr, LoadType::Object, AtomicOrdering::SeqCst);
                if let StackValue::ObjectRef(read_obj) = val {
                    assert!(read_obj.0.is_none());
                } else {
                    panic!("Expected ObjectRef");
                }
            }
        });
    }
    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "Alignment violation")]
    fn test_misaligned_atomic_load_is_rejected() {
        let data = Aligned8([0u8; 8]);
        // SAFETY: F10.RawMemoryAccessValid — Offset one remains within the test's eight-byte backing allocation.
        let ptr = unsafe { data.0.as_ptr().add(1) };
        // SAFETY: F4.WidthAligned — `ptr` points into the live test allocation; the checked operation is
        // expected to reject its deliberately invalid alignment.
        unsafe {
            StackValue::load_atomic(ptr, LoadType::Int32, AtomicOrdering::Relaxed);
        }
    }

    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "Alignment violation")]
    fn test_misaligned_atomic_store_is_rejected() {
        let mut data = Aligned8([0u8; 8]);
        // SAFETY: F10.RawMemoryAccessValid — Offset one remains within the test's eight-byte backing allocation.
        let ptr = unsafe { data.0.as_mut_ptr().add(1) };
        // SAFETY: F4.WidthAligned — `ptr` points into the live test allocation; the checked operation is
        // expected to reject its deliberately invalid alignment.
        unsafe {
            StackValue::Int32(42).store_atomic(ptr, StoreType::Int32, AtomicOrdering::Relaxed);
        }
    }

    // Multithreading only: without that feature the VM is single-threaded by contract and
    // `AtomicRepr` deliberately lowers to plain unaligned accesses, so spawning concurrent
    // threads would violate that precondition (Miri correctly reports a data race).
    #[cfg(feature = "multithreading")]
    mod concurrent {
        use crate::{LoadType, StackValue, StoreType};
        use dotnet_utils::sync::Ordering as AtomicOrdering;
        use std::cell::UnsafeCell;

        #[test]
        fn test_concurrent_atomic_load_store() {
            use std::sync::Arc;
            use std::sync::atomic::{AtomicBool, Ordering};
            use std::thread;
            // Bounded so the test stays fast under Miri; the reader runs until the writer finishes.
            const WRITES: i64 = if cfg!(miri) { 200 } else { 100_000 };
            let data = Arc::new(ThreadSafeBox(UnsafeCell::new([0u8; 8])));
            let stop = Arc::new(AtomicBool::new(false));
            let data_clone = data.clone();
            let stop_clone = stop.clone();
            let writer = thread::spawn(move || {
                // Derived from the `UnsafeCell`, so the shared buffer is writable through `ptr`.
                let ptr = data_clone.0.get().cast::<u8>();
                for i in 0..WRITES {
                    // SAFETY: F4.WidthAligned — `ptr` addresses the live, 8-byte-aligned shared
                    // buffer, and every concurrent access to it is atomic.
                    unsafe {
                        StackValue::Int64(i).store_atomic(
                            ptr,
                            StoreType::Int64,
                            AtomicOrdering::Release,
                        );
                    }
                }
                stop_clone.store(true, Ordering::Release);
            });
            let data_clone2 = data.clone();
            let reader = thread::spawn(move || {
                let ptr = data_clone2.0.get().cast::<u8>().cast_const();
                let mut last = 0;
                loop {
                    let done = stop.load(Ordering::Acquire);
                    // SAFETY: F4.WidthAligned — as for the writer; this access is an atomic load.
                    let val = unsafe {
                        StackValue::load_atomic(ptr, LoadType::Int64, AtomicOrdering::Acquire)
                    };
                    let StackValue::Int64(v) = val else {
                        panic!("Expected Int64");
                    };
                    // A single writer stores increasing values, so observations never go backwards.
                    assert!(
                        v >= last && v < WRITES,
                        "torn or out-of-order read: {v} after {last}"
                    );
                    last = v;
                    if done {
                        assert_eq!(v, WRITES - 1);
                        break;
                    }
                }
            });
            writer.join().unwrap();
            reader.join().unwrap();
        }

        #[repr(align(8))]
        struct ThreadSafeBox(UnsafeCell<[u8; 8]>);
        // SAFETY: F4.WidthAligned — threads share the cell only through aligned, width-matched atomic
        // loads and stores, so concurrent access never races non-atomically.
        unsafe impl Sync for ThreadSafeBox {}
    }
}

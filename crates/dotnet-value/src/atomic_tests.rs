#[cfg(test)]
mod tests {
    use crate::{LoadType, StackValue, StoreType};
    use dotnet_utils::sync::Ordering as AtomicOrdering;
    #[test]
    fn test_atomic_load_store() {
        let mut data = [0u8; 8];
        let ptr = data.as_mut_ptr();
        unsafe {
            StackValue::Int32(42).store_atomic(ptr, StoreType::Int8, AtomicOrdering::SeqCst);
            let val = StackValue::load_atomic(ptr, LoadType::Int8, AtomicOrdering::SeqCst);
            assert_eq!(val, StackValue::Int32(42));
            assert_eq!(data[0], 42);
            StackValue::Int32(-1).store_atomic(ptr, StoreType::Int8, AtomicOrdering::SeqCst);
            let val = StackValue::load_atomic(ptr, LoadType::Int8, AtomicOrdering::SeqCst);
            assert_eq!(val, StackValue::Int32(-1));
            assert_eq!(data[0], 255);
            StackValue::Int32(200).store_atomic(ptr, StoreType::Int8, AtomicOrdering::SeqCst);
            let val = StackValue::load_atomic(ptr, LoadType::UInt8, AtomicOrdering::SeqCst);
            assert_eq!(val, StackValue::Int32(200));
            StackValue::Int32(12345).store_atomic(ptr, StoreType::Int16, AtomicOrdering::SeqCst);
            let val = StackValue::load_atomic(ptr, LoadType::Int16, AtomicOrdering::SeqCst);
            assert_eq!(val, StackValue::Int32(12345));
            StackValue::Int32(0x12345678).store_atomic(
                ptr,
                StoreType::Int32,
                AtomicOrdering::SeqCst,
            );
            let val = StackValue::load_atomic(ptr, LoadType::Int32, AtomicOrdering::SeqCst);
            assert_eq!(val, StackValue::Int32(0x12345678));
            StackValue::Int64(0x123456789ABCDEF0).store_atomic(
                ptr,
                StoreType::Int64,
                AtomicOrdering::SeqCst,
            );
            let val = StackValue::load_atomic(ptr, LoadType::Int64, AtomicOrdering::SeqCst);
            assert_eq!(val, StackValue::Int64(0x123456789ABCDEF0));
            StackValue::NativeFloat(1.25).store_atomic(
                ptr,
                StoreType::Float32,
                AtomicOrdering::SeqCst,
            );
            let val = StackValue::load_atomic(ptr, LoadType::Float32, AtomicOrdering::SeqCst);
            if let StackValue::NativeFloat(f) = val {
                assert_eq!(f, 1.25);
            } else {
                panic!("Expected NativeFloat");
            }
            StackValue::NativeFloat(1.23456789).store_atomic(
                ptr,
                StoreType::Float64,
                AtomicOrdering::SeqCst,
            );
            let val = StackValue::load_atomic(ptr, LoadType::Float64, AtomicOrdering::SeqCst);
            if let StackValue::NativeFloat(f) = val {
                assert_eq!(f, 1.23456789);
            } else {
                panic!("Expected NativeFloat");
            }
        }
    }
    #[test]
    fn test_concurrent_atomic_load_store() {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::thread;
        let data = Arc::new(ThreadSafeBox([0u8; 8]));
        let stop = Arc::new(AtomicBool::new(false));
        let data_clone = data.clone();
        let stop_clone = stop.clone();
        let writer = thread::spawn(move || {
            let ptr = data_clone.0.as_ptr() as *mut u8;
            let mut i = 0;
            while !stop_clone.load(Ordering::Relaxed) {
                unsafe {
                    StackValue::Int64(i).store_atomic(
                        ptr,
                        StoreType::Int64,
                        AtomicOrdering::Release,
                    );
                }
                i += 1;
            }
        });
        let data_clone2 = data.clone();
        let stop_clone2 = stop.clone();
        let reader = thread::spawn(move || {
            let ptr = data_clone2.0.as_ptr();
            while !stop_clone2.load(Ordering::Relaxed) {
                unsafe {
                    let val =
                        StackValue::load_atomic(ptr, LoadType::Int64, AtomicOrdering::Acquire);
                    if let StackValue::Int64(v) = val {
                        assert!(v >= 0);
                    }
                }
            }
        });
        thread::sleep(std::time::Duration::from_millis(100));
        stop.store(true, Ordering::Relaxed);
        writer.join().unwrap();
        reader.join().unwrap();
    }
    #[test]
    fn test_atomic_object_load_store() {
        use crate::object::{HeapStorage, ObjectRef};
        use gc_arena::{Arena, Rootable};
        type TestRoot = Rootable![()];
        let arena = Arena::<TestRoot>::new(|_mc| ());
        #[cfg(feature = "multithreaded-gc")]
        let arena_handle = Box::leak(Box::new(dotnet_utils::gc::ArenaHandle::new(
            dotnet_utils::ArenaId(0),
        )));
        arena.mutate(|gc, _root| {
            let storage = HeapStorage::Str(crate::string::CLRString::from("test"));
            let gc_handle = dotnet_utils::gc::GCHandle::new(
                gc,
                #[cfg(feature = "multithreaded-gc")]
                arena_handle.as_inner(),
                #[cfg(feature = "memory-validation")]
                dotnet_utils::ArenaId(0),
            );
            let obj = ObjectRef::new(gc_handle, storage);
            let mut buffer = [0u8; 8];
            let ptr = buffer.as_mut_ptr();
            unsafe {
                StackValue::ObjectRef(obj).store_atomic(
                    ptr,
                    StoreType::Object,
                    AtomicOrdering::SeqCst,
                );
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
    #[cfg_attr(
        feature = "memory-validation",
        should_panic(expected = "Alignment violation")
    )]
    fn test_misaligned_load() {
        let data = [0u8; 16];
        let ptr = unsafe { data.as_ptr().add(1) };
        unsafe {
            StackValue::load_atomic(ptr, LoadType::Int32, AtomicOrdering::Relaxed);
        }
    }
    struct ThreadSafeBox([u8; 8]);
    unsafe impl Sync for ThreadSafeBox {}
    unsafe impl Send for ThreadSafeBox {}
}

#[cfg(test)]
mod tests {
    use crate::object::{HeapStorage, ObjectRef, ValueType};
    use gc_arena::{Arena, Rootable};
    #[test]
    fn test_read_branded_null() {
        type TestRoot = Rootable![()];
        let arena = Arena::<TestRoot>::new(|_mc| ());
        #[cfg(feature = "multithreaded-gc")]
        let arena_handle = Box::leak(Box::new(dotnet_utils::gc::ArenaHandle::new(
            dotnet_utils::ArenaId(0),
        )));
        arena.mutate(|gc, _root| {
            let null_bytes = 0usize.to_ne_bytes();
            let gc_handle = dotnet_utils::gc::GCHandle::new(
                gc,
                #[cfg(feature = "multithreaded-gc")]
                arena_handle.as_inner(),
                #[cfg(feature = "memory-validation")]
                dotnet_utils::ArenaId(0),
            );
            unsafe {
                let obj = ObjectRef::read_branded(&null_bytes, &gc_handle);
                assert!(obj.0.is_none());
            }
        });
    }
    #[test]
    fn test_read_valid_object() {
        type TestRoot = Rootable![()];
        let arena = Arena::<TestRoot>::new(|_mc| ());
        #[cfg(feature = "multithreaded-gc")]
        let arena_handle = Box::leak(Box::new(dotnet_utils::gc::ArenaHandle::new(
            dotnet_utils::ArenaId(0),
        )));
        arena.mutate(|gc, _root| {
            let storage = HeapStorage::Boxed(ValueType::Int32(42));
            let gc_handle = dotnet_utils::gc::GCHandle::new(
                gc,
                #[cfg(feature = "multithreaded-gc")]
                arena_handle.as_inner(),
                #[cfg(feature = "memory-validation")]
                dotnet_utils::ArenaId(0),
            );
            let obj = ObjectRef::new(gc_handle, storage);
            let mut buffer = [0u8; size_of::<usize>()];
            obj.write(&mut buffer);
            unsafe {
                let read_obj = ObjectRef::read_branded(&buffer, &gc_handle);
                assert_eq!(read_obj, obj);
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use crate::object::{HeapStorage, ObjectRef};
    use gc_arena::{Arena, Rootable};
    #[test]
    fn test_read_branded_null() {
        type TestRoot = Rootable![()];
        let arena = Arena::<TestRoot>::new(|_mc| ());
        #[cfg(feature = "multithreading")]
        let _arena_handle_owner = dotnet_utils::gc::ArenaHandle::new(dotnet_utils::ArenaId(0));
        #[cfg(feature = "multithreading")]
        let arena_handle = unsafe {
            std::mem::transmute::<
                &dotnet_utils::gc::ArenaHandleInner,
                &'static dotnet_utils::gc::ArenaHandleInner,
            >(_arena_handle_owner.as_inner())
        };
        arena.mutate(|gc, _root| {
            let null_bytes = 0usize.to_ne_bytes();
            let gc_handle = dotnet_utils::gc::GCHandle::new(
                gc,
                #[cfg(feature = "multithreading")]
                arena_handle,
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
        #[cfg(feature = "multithreading")]
        let _arena_handle_owner = dotnet_utils::gc::ArenaHandle::new(dotnet_utils::ArenaId(0));
        #[cfg(feature = "multithreading")]
        let arena_handle = unsafe {
            std::mem::transmute::<
                &dotnet_utils::gc::ArenaHandleInner,
                &'static dotnet_utils::gc::ArenaHandleInner,
            >(_arena_handle_owner.as_inner())
        };
        arena.mutate(|gc, _root| {
            let storage = HeapStorage::Str(crate::string::CLRString::from("test"));
            let gc_handle = dotnet_utils::gc::GCHandle::new(
                gc,
                #[cfg(feature = "multithreading")]
                arena_handle,
                #[cfg(feature = "memory-validation")]
                dotnet_utils::ArenaId(0),
            );
            let obj = ObjectRef::new(gc_handle, storage);
            let mut buffer = [0u8; ObjectRef::SIZE];
            obj.write(&mut buffer);
            unsafe {
                let read_obj = ObjectRef::read_branded(&buffer, &gc_handle);
                assert_eq!(read_obj, obj);
            }
        });
    }
}

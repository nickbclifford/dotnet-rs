#[cfg(test)]
mod tests {
    use crate::object::{HeapStorage, ObjectRef};
    #[cfg(all(feature = "memory-validation", feature = "multithreading"))]
    use dotnet_utils::sync::MANAGED_THREAD_ID;
    use gc_arena::{Arena, Rootable};

    #[cfg(all(feature = "memory-validation", feature = "multithreading"))]
    struct ManagedThreadIdGuard {
        previous: Option<dotnet_utils::ArenaId>,
    }

    #[cfg(all(feature = "memory-validation", feature = "multithreading"))]
    impl ManagedThreadIdGuard {
        fn set(id: dotnet_utils::ArenaId) -> Self {
            let previous = MANAGED_THREAD_ID.with(|thread_id| {
                let prev = thread_id.get();
                thread_id.set(Some(id));
                prev
            });
            Self { previous }
        }
    }

    #[cfg(all(feature = "memory-validation", feature = "multithreading"))]
    impl Drop for ManagedThreadIdGuard {
        fn drop(&mut self) {
            MANAGED_THREAD_ID.with(|thread_id| thread_id.set(self.previous));
        }
    }

    #[test]
    fn test_read_branded_null() {
        type TestRoot = Rootable![()];
        let arena = Arena::<TestRoot>::new(|_mc| ());
        #[cfg(feature = "multithreading")]
        let arena_id = dotnet_utils::ArenaId(0);
        #[cfg(feature = "multithreading")]
        dotnet_utils::gc::register_arena(
            arena_id,
            dotnet_utils::sync::Arc::new(dotnet_utils::sync::AtomicBool::new(false)),
        );
        #[cfg(feature = "multithreading")]
        let _arena_handle_owner = dotnet_utils::gc::ArenaHandle::new(arena_id);
        #[cfg(all(feature = "memory-validation", feature = "multithreading"))]
        let _thread_id_guard = ManagedThreadIdGuard::set(arena_id);
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
        #[cfg(feature = "multithreading")]
        dotnet_utils::gc::unregister_arena(arena_id);
    }
    #[test]
    fn test_read_valid_object() {
        type TestRoot = Rootable![()];
        let arena = Arena::<TestRoot>::new(|_mc| ());
        #[cfg(feature = "multithreading")]
        let arena_id = dotnet_utils::ArenaId(0);
        #[cfg(feature = "multithreading")]
        dotnet_utils::gc::register_arena(
            arena_id,
            dotnet_utils::sync::Arc::new(dotnet_utils::sync::AtomicBool::new(false)),
        );
        #[cfg(feature = "multithreading")]
        let _arena_handle_owner = dotnet_utils::gc::ArenaHandle::new(arena_id);
        #[cfg(all(feature = "memory-validation", feature = "multithreading"))]
        let _thread_id_guard = ManagedThreadIdGuard::set(arena_id);
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
        #[cfg(feature = "multithreading")]
        dotnet_utils::gc::unregister_arena(arena_id);
    }
}

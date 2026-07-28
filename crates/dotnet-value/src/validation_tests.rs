#[cfg(test)]
mod tests {
    #[cfg(feature = "memory-validation")]
    use crate::object::{HeapStorage, ObjectRef};
    #[cfg(feature = "memory-validation")]
    use dotnet_utils::ArenaId;
    #[cfg(feature = "memory-validation")]
    use gc_arena::{Arena, Rootable};
    #[test]
    #[cfg(feature = "memory-validation")]
    fn test_arena_mismatch_panics() {
        #[cfg(feature = "multithreading")]
        {}
        #[cfg(not(feature = "multithreading"))]
        {
            type TestRoot = Rootable![()];
            let arena = Arena::<TestRoot>::new(|_mc| ());
            arena.mutate(|gc, _root| {
                dotnet_utils::sync::MANAGED_THREAD_ID.with(|id| id.set(Some(ArenaId::new(1))));
                let gc_handle = dotnet_utils::gc::GCHandle::new(gc, ArenaId::new(1));
                let obj = ObjectRef::new(
                    gc_handle,
                    HeapStorage::Str(crate::string::CLRString::from("test")),
                );
                obj.as_heap_storage(|_| ());
                dotnet_utils::sync::MANAGED_THREAD_ID.with(|id| id.set(Some(ArenaId::new(2))));
                let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
                    obj.as_heap_storage(|_| ());
                }));
                assert!(res.is_err(), "Access from wrong arena should panic");
            });
        }
    }
    #[test]
    #[cfg(all(feature = "memory-validation", feature = "multithreading"))]
    fn test_dangling_arena_panics() {
        type TestRoot = Rootable![()];
        let arena = Arena::<TestRoot>::new(|_mc| ());
        let arena_id = ArenaId::new(100);
        let stw_flag = dotnet_utils::sync::Arc::new(dotnet_utils::sync::AtomicBool::new(false));
        let _arena_handle_owner = dotnet_utils::gc::ArenaHandle::new(arena_id);
        // SAFETY: The local owner retains the ArenaHandleInner until after the
        // `arena.mutate` closure finishes, so the test-only `'static` borrow cannot
        // outlive the owner.
        let arena_handle = unsafe {
            std::mem::transmute::<
                &dotnet_utils::gc::ArenaHandleInner,
                &'static dotnet_utils::gc::ArenaHandleInner,
            >(_arena_handle_owner.as_inner())
        };
        dotnet_utils::gc::register_arena(arena_id, stw_flag.clone());
        arena.mutate(|gc, _root| {
            dotnet_utils::sync::MANAGED_THREAD_ID.set(Some(arena_id));
            let gc_handle = dotnet_utils::gc::GCHandle::new(gc, arena_handle, arena_id);
            let obj = ObjectRef::new(
                gc_handle,
                HeapStorage::Str(crate::string::CLRString::from("test")),
            );
            obj.as_heap_storage(|_| ());
            dotnet_utils::gc::unregister_arena(arena_id);
            let other_id = ArenaId::new(200);
            dotnet_utils::gc::register_arena(other_id, stw_flag.clone());
            dotnet_utils::sync::MANAGED_THREAD_ID.set(Some(other_id));
            let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
                obj.as_heap_storage(|_| ());
            }));
            assert!(res.is_err(), "Access to dangling arena should panic");
            dotnet_utils::gc::unregister_arena(other_id);
        });
    }
    #[test]
    #[cfg(all(feature = "memory-validation", feature = "multithreading"))]
    fn test_uncoordinated_stw_access_panics() {
        type TestRoot = Rootable![()];
        let arena = Arena::<TestRoot>::new(|_mc| ());
        let owner_id = ArenaId::new(101);
        let current_id = ArenaId::new(102);
        let stw_flag = dotnet_utils::sync::Arc::new(dotnet_utils::sync::AtomicBool::new(false));
        let _owner_handle_owner = dotnet_utils::gc::ArenaHandle::new(owner_id);
        // SAFETY: The local owner retains the ArenaHandleInner until after the
        // `arena.mutate` closure finishes, so the test-only `'static` borrow cannot
        // outlive the owner.
        let owner_handle = unsafe {
            std::mem::transmute::<
                &dotnet_utils::gc::ArenaHandleInner,
                &'static dotnet_utils::gc::ArenaHandleInner,
            >(_owner_handle_owner.as_inner())
        };
        dotnet_utils::gc::register_arena(owner_id, stw_flag.clone());
        dotnet_utils::gc::register_arena(current_id, stw_flag.clone());
        arena.mutate(|gc, _root| {
            dotnet_utils::sync::MANAGED_THREAD_ID.set(Some(owner_id));
            let gc_handle = dotnet_utils::gc::GCHandle::new(gc, owner_handle, owner_id);
            let obj = ObjectRef::new(
                gc_handle,
                HeapStorage::Str(crate::string::CLRString::from("test")),
            );
            dotnet_utils::sync::MANAGED_THREAD_ID.set(Some(current_id));
            obj.as_heap_storage(|_| ());
            dotnet_utils::gc::set_stw_in_progress(owner_id, true);
            let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
                obj.as_heap_storage(|_| ());
            }));
            assert!(res.is_err(), "Uncoordinated access during STW should panic");
            dotnet_utils::gc::set_currently_tracing(Some(current_id));
            obj.as_heap_storage(|_| ());
            dotnet_utils::gc::set_currently_tracing(None);
            dotnet_utils::gc::set_stw_in_progress(owner_id, false);
        });
        dotnet_utils::gc::unregister_arena(owner_id);
        dotnet_utils::gc::unregister_arena(current_id);
    }
}

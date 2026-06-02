use gc_arena::{Arena, Rootable};

#[cfg(feature = "memory-validation")]
use dotnet_utils::sync::MANAGED_THREAD_ID;

#[cfg(feature = "multithreading")]
use std::sync::atomic::{AtomicU64, Ordering};

#[cfg(feature = "multithreading")]
static NEXT_TEST_ARENA_ID: AtomicU64 = AtomicU64::new(1);

#[cfg(feature = "multithreading")]
fn next_test_arena_id() -> crate::ArenaId {
    crate::ArenaId(NEXT_TEST_ARENA_ID.fetch_add(1, Ordering::Relaxed))
}

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

#[cfg(feature = "multithreading")]
struct ArenaRegistrationGuard {
    arena_id: crate::ArenaId,
}

#[cfg(feature = "multithreading")]
impl ArenaRegistrationGuard {
    fn register(arena_id: crate::ArenaId) -> Self {
        dotnet_utils::gc::register_arena(
            arena_id,
            dotnet_utils::sync::Arc::new(dotnet_utils::sync::AtomicBool::new(false)),
        );
        Self { arena_id }
    }
}

#[cfg(feature = "multithreading")]
impl Drop for ArenaRegistrationGuard {
    fn drop(&mut self) {
        dotnet_utils::gc::unregister_arena(self.arena_id);
    }
}

pub(crate) fn with_test_gc_context<R>(f: impl for<'gc> FnOnce(dotnet_utils::gc::GCHandle<'gc>) -> R) -> R {
    type TestRoot = Rootable![()];
    let arena = Arena::<TestRoot>::new(|_mc| ());

    #[cfg(feature = "multithreading")]
    let arena_id = next_test_arena_id();
    #[cfg(all(not(feature = "multithreading"), feature = "memory-validation"))]
    let arena_id = crate::ArenaId(0);

    #[cfg(feature = "multithreading")]
    let _arena_registration = ArenaRegistrationGuard::register(arena_id);

    #[cfg(feature = "multithreading")]
    let arena_handle_owner = dotnet_utils::gc::ArenaHandle::new(arena_id);

    #[cfg(feature = "memory-validation")]
    let _thread_id_guard = ManagedThreadIdGuard::set(arena_id);

    #[cfg(feature = "multithreading")]
    let arena_handle = unsafe {
        std::mem::transmute::<
            &dotnet_utils::gc::ArenaHandleInner,
            &'static dotnet_utils::gc::ArenaHandleInner,
        >(arena_handle_owner.as_inner())
    };

    arena.mutate(|gc, _root| {
        let gc_handle = dotnet_utils::gc::GCHandle::new(
            gc,
            #[cfg(feature = "multithreading")]
            arena_handle,
            #[cfg(feature = "memory-validation")]
            arena_id,
        );
        f(gc_handle)
    })
}

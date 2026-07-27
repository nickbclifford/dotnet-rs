use gc_arena::{Collect, Mutation, barrier::Unlock, collect::Trace};
use std::ops::{Deref, DerefMut};

#[cfg(feature = "multithreading")]
use std::cell::Cell;

#[cfg(feature = "multithreading")]
use parking_lot::{
    MappedRwLockReadGuard, MappedRwLockWriteGuard, RwLock, RwLockReadGuard, RwLockWriteGuard,
};

#[cfg(not(feature = "multithreading"))]
use gc_arena::lock::RefLock as RwLock;
#[cfg(not(feature = "multithreading"))]
use std::cell::{Ref as MappedRwLockReadGuard, RefMut as MappedRwLockWriteGuard};

#[cfg(feature = "multithreading")]
thread_local! {
    static ACTIVE_WRITE_GUARDS: Cell<usize> = const { Cell::new(0) };
}

/// Returns whether this thread currently holds a mutable `ThreadSafeLock` guard.
///
/// A managed thread in this state must not park at a GC safepoint: the collector
/// traces lock contents under a read lock, which requires every stopped thread to
/// have released its write guards first.
#[cfg(feature = "multithreading")]
pub fn has_active_thread_safe_write_guard() -> bool {
    ACTIVE_WRITE_GUARDS.get() != 0
}

#[cfg(feature = "multithreading")]
struct SafepointWriteGuard;

#[cfg(feature = "multithreading")]
impl SafepointWriteGuard {
    fn enter() -> Self {
        ACTIVE_WRITE_GUARDS.set(
            ACTIVE_WRITE_GUARDS
                .get()
                .checked_add(1)
                .expect("ThreadSafeLock write-guard depth overflow"),
        );
        Self
    }
}

#[cfg(feature = "multithreading")]
impl Drop for SafepointWriteGuard {
    fn drop(&mut self) {
        let depth = ACTIVE_WRITE_GUARDS.get();
        debug_assert!(depth > 0, "unbalanced ThreadSafeLock write guard");
        ACTIVE_WRITE_GUARDS.set(depth - 1);
    }
}

/// A thread-safe lock for GC-managed objects.
///
/// In multi-threaded mode, this uses `parking_lot::RwLock` internally.
/// In single-threaded mode, this uses `gc_arena::lock::RefLock`.
#[derive(Debug)]
pub struct ThreadSafeLock<T: ?Sized> {
    inner: RwLock<T>,
}

impl<T: ?Sized> ThreadSafeLock<T> {
    /// Create a new `ThreadSafeLock` wrapping the given value.
    pub fn new(value: T) -> Self
    where
        T: Sized,
    {
        Self {
            inner: RwLock::new(value),
        }
    }
}

#[cfg(feature = "multithreading")]
impl<T: ?Sized> ThreadSafeLock<T> {
    /// Borrow the contents immutably.
    pub fn borrow(&self) -> ThreadSafeReadGuard<'_, T> {
        ThreadSafeReadGuard {
            guard: RwLockReadGuard::map(self.inner.read(), |x| x),
        }
    }

    /// Borrow the contents mutably.
    pub fn borrow_mut<'gc>(&self, _gc: &Mutation<'gc>) -> ThreadSafeWriteGuard<'_, T> {
        ThreadSafeWriteGuard {
            guard: RwLockWriteGuard::map(self.inner.write(), |x| x),
            safepoint_guard: SafepointWriteGuard::enter(),
        }
    }

    /// Try to borrow the contents immutably without blocking.
    ///
    /// Returns `None` if a write lock is currently held.
    pub fn try_borrow(&self) -> Option<ThreadSafeReadGuard<'_, T>> {
        self.inner.try_read().map(|guard| ThreadSafeReadGuard {
            guard: RwLockReadGuard::map(guard, |x| x),
        })
    }

    /// Try to borrow the contents mutably without blocking.
    ///
    /// Returns `None` if any locks (read_unchecked or write) are currently held.
    pub fn try_borrow_mut<'gc>(&self, _gc: &Mutation<'gc>) -> Option<ThreadSafeWriteGuard<'_, T>> {
        let _ = _gc;
        self.inner.try_write().map(|guard| ThreadSafeWriteGuard {
            guard: RwLockWriteGuard::map(guard, |x| x),
            safepoint_guard: SafepointWriteGuard::enter(),
        })
    }

    /// Get an immutable reference to the inner value.
    ///
    /// # Safety
    ///
    /// This bypasses the lock and should only be used when you can guarantee
    /// that no other threads are accessing the value.
    pub unsafe fn as_ptr(&self) -> *const T {
        self.inner.data_ptr()
    }
}

#[cfg(not(feature = "multithreading"))]
impl<T: ?Sized> ThreadSafeLock<T> {
    /// Borrow the contents immutably.
    pub fn borrow(&self) -> ThreadSafeReadGuard<'_, T> {
        ThreadSafeReadGuard {
            guard: self.inner.borrow(),
        }
    }

    /// Borrow the contents mutably.
    pub fn borrow_mut<'gc>(&self, _gc: &Mutation<'gc>) -> ThreadSafeWriteGuard<'_, T> {
        ThreadSafeWriteGuard {
            // SAFETY: `_gc` is a `&Mutation<'gc>` token which, by gc-arena's
            // contract, can only be obtained inside a `mutate` closure.  The
            // arena guarantees no GC cycle runs concurrently with mutation, so
            // no other code can observe the RefLock as immutably borrowed through
            // the GC tracing path at this point.  `unlock_unchecked` is the
            // gc-arena-blessed way to obtain a mutable borrow on a `RefLock`
            // inside a mutation context.
            guard: unsafe { self.inner.unlock_unchecked().borrow_mut() },
        }
    }

    /// Try to borrow the contents immutably without blocking.
    ///
    /// Returns `None` if a write lock is currently held.
    pub fn try_borrow(&self) -> Option<ThreadSafeReadGuard<'_, T>> {
        self.inner
            .try_borrow()
            .ok()
            .map(|guard| ThreadSafeReadGuard { guard })
    }

    /// Try to borrow the contents mutably without blocking.
    ///
    /// Returns `None` if any locks (read_unchecked or write) are currently held.
    pub fn try_borrow_mut<'gc>(&self, _gc: &Mutation<'gc>) -> Option<ThreadSafeWriteGuard<'_, T>> {
        // SAFETY: Same invariant as `borrow_mut`: holding a `&Mutation<'gc>`
        // token guarantees we are inside a mutation context where the arena's
        // GC cycle cannot run.  `unlock_unchecked` is the gc-arena-prescribed
        // way to access a `RefLock` mutably within a mutation context.  If
        // another borrow is already active `try_borrow_mut` returns `None`
        // instead of panicking, making this safe to call speculatively.
        unsafe {
            self.inner
                .unlock_unchecked()
                .try_borrow_mut()
                .ok()
                .map(|guard| ThreadSafeWriteGuard { guard })
        }
    }

    /// Get an immutable reference to the inner value.
    ///
    /// # Safety
    ///
    /// This bypasses the lock and should only be used when you can guarantee
    /// that no other threads are accessing the value.
    pub unsafe fn as_ptr(&self) -> *const T {
        self.inner.as_ptr()
    }
}

// SAFETY: Both lock backends expose `T` to the tracer only through a shared
// borrow. The multithreaded backend requires the GC safepoint protocol to keep
// write-guard holders running until they release their guards, then verifies
// that invariant with `try_read` before tracing. The single-threaded `RefLock`
// implementation delegates to its `Collect` implementation.
unsafe impl<'gc, T: Collect<'gc> + 'gc> Collect<'gc> for ThreadSafeLock<T> {
    fn trace<Tr: Trace<'gc>>(&self, cc: &mut Tr) {
        #[cfg(feature = "multithreading")]
        {
            let guard = self
                .inner
                .try_read()
                .expect("ThreadSafeLock write guard remained active during stop-the-world tracing");
            guard.trace(cc);
        }
        #[cfg(not(feature = "multithreading"))]
        {
            self.inner.trace(cc);
        }
    }
}

impl<T> Unlock for ThreadSafeLock<T> {
    type Unlocked = RwLock<T>;

    unsafe fn unlock_unchecked(&self) -> &Self::Unlocked {
        &self.inner
    }
}

/// RAII guard for immutable borrows.
pub struct ThreadSafeReadGuard<'a, T: ?Sized> {
    guard: MappedRwLockReadGuard<'a, T>,
}

impl<'a, T: ?Sized> ThreadSafeReadGuard<'a, T> {
    pub fn map<U: ?Sized, F>(this: Self, f: F) -> ThreadSafeReadGuard<'a, U>
    where
        F: FnOnce(&T) -> &U,
    {
        ThreadSafeReadGuard {
            #[cfg(feature = "multithreading")]
            guard: MappedRwLockReadGuard::map(this.guard, f),
            #[cfg(not(feature = "multithreading"))]
            guard: std::cell::Ref::map(this.guard, f),
        }
    }
}

impl<T: ?Sized> Deref for ThreadSafeReadGuard<'_, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.guard
    }
}

/// RAII guard for mutable borrows.
pub struct ThreadSafeWriteGuard<'a, T: ?Sized> {
    guard: MappedRwLockWriteGuard<'a, T>,
    #[cfg(feature = "multithreading")]
    safepoint_guard: SafepointWriteGuard,
}

impl<'a, T: ?Sized> ThreadSafeWriteGuard<'a, T> {
    pub fn map<U: ?Sized, F>(this: Self, f: F) -> ThreadSafeWriteGuard<'a, U>
    where
        F: FnOnce(&mut T) -> &mut U,
    {
        let ThreadSafeWriteGuard {
            guard,
            #[cfg(feature = "multithreading")]
            safepoint_guard,
        } = this;
        ThreadSafeWriteGuard {
            #[cfg(feature = "multithreading")]
            guard: MappedRwLockWriteGuard::map(guard, f),
            #[cfg(not(feature = "multithreading"))]
            guard: std::cell::RefMut::map(guard, f),
            #[cfg(feature = "multithreading")]
            safepoint_guard,
        }
    }
}

impl<T: ?Sized> Deref for ThreadSafeWriteGuard<'_, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.guard
    }
}

impl<T: ?Sized> DerefMut for ThreadSafeWriteGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.guard
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use static_assertions::assert_impl_all;
    use static_assertions::assert_not_impl_all;
    use std::{cell::Cell, rc::Rc};

    #[cfg(feature = "multithreading")]
    assert_impl_all!(ThreadSafeLock<i32>: Send, Sync);
    #[cfg(feature = "multithreading")]
    assert_impl_all!(ThreadSafeLock<Cell<u8>>: Send);
    #[cfg(feature = "multithreading")]
    assert_not_impl_all!(ThreadSafeLock<Cell<u8>>: Sync);
    #[cfg(feature = "multithreading")]
    assert_not_impl_all!(ThreadSafeLock<Rc<u8>>: Send);
    #[cfg(feature = "multithreading")]
    assert_not_impl_all!(ThreadSafeLock<Rc<u8>>: Sync);
    #[cfg(feature = "multithreading")]
    assert_not_impl_all!(ThreadSafeWriteGuard<'static, i32>: Send);

    #[cfg(not(feature = "multithreading"))]
    assert_impl_all!(ThreadSafeLock<i32>: Send);
    #[cfg(not(feature = "multithreading"))]
    assert_not_impl_all!(ThreadSafeLock<i32>: Sync);
    #[cfg(not(feature = "multithreading"))]
    assert_impl_all!(ThreadSafeLock<Cell<u8>>: Send);
    #[cfg(not(feature = "multithreading"))]
    assert_not_impl_all!(ThreadSafeLock<Cell<u8>>: Sync);
    #[cfg(not(feature = "multithreading"))]
    assert_not_impl_all!(ThreadSafeLock<Rc<u8>>: Send);
    #[cfg(not(feature = "multithreading"))]
    assert_not_impl_all!(ThreadSafeLock<Rc<u8>>: Sync);

    #[test]
    fn test_basic_borrow() {
        let lock = ThreadSafeLock::new(42);
        let guard = lock.borrow();
        assert_eq!(*guard, 42);
    }

    #[test]
    fn test_multiple_readers() {
        let lock = ThreadSafeLock::new(42);
        let guard1 = lock.borrow();
        let guard2 = lock.borrow();
        assert_eq!(*guard1, 42);
        assert_eq!(*guard2, 42);
    }

    #[test]
    fn test_exclusive_writer() {
        use gc_arena::{Arena, Rootable};

        type TestArena = Arena<Rootable![ThreadSafeLock<i32>]>;

        let mut arena = TestArena::new(|_mc| ThreadSafeLock::new(42));
        arena.mutate_root(|mc, lock| {
            {
                let mut guard = lock.borrow_mut(mc);
                *guard = 100;
            }
            let guard = lock.borrow();
            assert_eq!(*guard, 100);
        });
    }

    #[test]
    fn test_try_borrow() {
        use gc_arena::{Arena, Rootable};

        type TestArena = Arena<Rootable![ThreadSafeLock<i32>]>;

        let mut arena = TestArena::new(|_mc| ThreadSafeLock::new(42));
        arena.mutate_root(|mc, lock| {
            let _writer = lock.borrow_mut(mc);
            // Should fail to get reader while writer is active
            assert!(lock.try_borrow().is_none());
        });
    }

    #[cfg(feature = "multithreading")]
    #[test]
    fn write_guards_track_safepoint_exclusion_across_mapping() {
        use gc_arena::{Arena, Rootable};

        type TestArena = Arena<Rootable![ThreadSafeLock<(i32, i32)>]>;

        assert!(!has_active_thread_safe_write_guard());
        let mut arena = TestArena::new(|_mc| ThreadSafeLock::new((1, 2)));
        arena.mutate_root(|mc, lock| {
            let guard = lock.borrow_mut(mc);
            assert!(has_active_thread_safe_write_guard());

            let mut mapped = ThreadSafeWriteGuard::map(guard, |pair| &mut pair.1);
            assert!(has_active_thread_safe_write_guard());
            *mapped = 3;
            drop(mapped);

            assert!(!has_active_thread_safe_write_guard());
            assert_eq!(*lock.borrow(), (1, 3));
        });
    }
}

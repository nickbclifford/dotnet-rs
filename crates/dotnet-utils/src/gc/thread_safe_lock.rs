use gc_arena::{Collect, Mutation, barrier::Unlock, collect::Trace};
use std::ops::{Deref, DerefMut};

#[cfg(feature = "multithreading")]
use parking_lot::{
    MappedRwLockReadGuard, MappedRwLockWriteGuard, RwLock, RwLockReadGuard, RwLockWriteGuard,
};

#[cfg(not(feature = "multithreading"))]
use gc_arena::lock::RefLock as RwLock;
#[cfg(not(feature = "multithreading"))]
use std::cell::{Ref as MappedRwLockReadGuard, RefMut as MappedRwLockWriteGuard};

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

unsafe impl<'gc, T: Collect<'gc> + 'gc> Collect<'gc> for ThreadSafeLock<T> {
    fn trace<Tr: Trace<'gc>>(&self, cc: &mut Tr) {
        #[cfg(feature = "multithreading")]
        {
            // SAFETY: Tracing happens during a stop-the-world pause, so no other
            // threads are running. We can safely access the inner value without
            // acquiring the lock. This avoids deadlock (or panic) if a thread was
            // already holding the write lock when it reached a safe point.
            unsafe {
                (*self.inner.data_ptr()).trace(cc);
            }
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
}

impl<'a, T: ?Sized> ThreadSafeWriteGuard<'a, T> {
    pub fn map<U: ?Sized, F>(this: Self, f: F) -> ThreadSafeWriteGuard<'a, U>
    where
        F: FnOnce(&mut T) -> &mut U,
    {
        ThreadSafeWriteGuard {
            #[cfg(feature = "multithreading")]
            guard: MappedRwLockWriteGuard::map(this.guard, f),
            #[cfg(not(feature = "multithreading"))]
            guard: std::cell::RefMut::map(this.guard, f),
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

// SAFETY: `ThreadSafeLock<T>` can be sent to another thread as long as `T`
// itself can be sent.  Under both backends (parking_lot `RwLock` and
// gc-arena `RefLock`) ownership transfer is safe when `T: Send`.
unsafe impl<T: Send> Send for ThreadSafeLock<T> {}

// SAFETY: `ThreadSafeLock<T>` allows `&ThreadSafeLock<T>` to be used from
// multiple threads concurrently.  Read-borrows hand out `&T` references that
// may be observed by many threads at once, so `T: Sync` is required.
// Write-borrows hand out `&mut T` which may be sent across a thread boundary,
// so `T: Send` is also required.  These bounds mirror the requirements of
// `parking_lot::RwLock<T>` (the multithreading backend) and make the
// single-threaded (`RefLock`) path equally strict, preventing `!Sync` types
// from being incorrectly shared.
unsafe impl<T: Send + Sync> Sync for ThreadSafeLock<T> {}

#[cfg(test)]
mod tests {
    use super::*;
    use static_assertions::assert_impl_all;

    // Compile-time: ThreadSafeLock<i32> must be Send + Sync under every
    // feature configuration (i32 satisfies the T: Send + Sync bound).
    assert_impl_all!(ThreadSafeLock<i32>: Send, Sync);

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
}

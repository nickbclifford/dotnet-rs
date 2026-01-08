//! Thread-safe lock for heap objects in multi-threaded GC environment.
//!
//! This module provides a replacement for `gc_arena::lock::RefLock` that is safe
//! to use across multiple threads. Unlike `RefLock`, which uses non-atomic borrow
//! flags, `ThreadSafeLock` uses atomic operations to ensure thread-safe access.
use gc_arena::{barrier::Unlock, Collect, Collection};
#[cfg(feature = "multithreading")]
use parking_lot::{RwLock, RwLockReadGuard, RwLockWriteGuard};
#[cfg(not(feature = "multithreading"))]
use std::cell::{Ref as RwLockReadGuard, RefMut as RwLockWriteGuard};
#[cfg(not(feature = "multithreading"))]
use gc_arena::lock::RefLock as RwLock;
use std::ops::{Deref, DerefMut};

/// A thread-safe lock for GC-managed objects.
///
/// In multi-threaded mode, this uses `parking_lot::RwLock` internally.
/// In single-threaded mode, this uses `gc_arena::lock::RefLock`.
#[derive(Debug)]
pub struct ThreadSafeLock<T> {
    inner: RwLock<T>,
}

impl<T> ThreadSafeLock<T> {
    /// Create a new `ThreadSafeLock` wrapping the given value.
    pub fn new(value: T) -> Self {
        Self {
            inner: RwLock::new(value),
        }
    }

    /// Borrow the contents immutably.
    pub fn borrow(&self) -> ThreadSafeReadGuard<'_, T> {
        ThreadSafeReadGuard {
            #[cfg(feature = "multithreading")]
            guard: self.inner.read(),
            #[cfg(not(feature = "multithreading"))]
            guard: self.inner.borrow(),
        }
    }

    /// Borrow the contents mutably.
    pub fn borrow_mut<'gc>(&self, _gc: &gc_arena::Mutation<'gc>) -> ThreadSafeWriteGuard<'_, T> {
        ThreadSafeWriteGuard {
            #[cfg(feature = "multithreading")]
            guard: self.inner.write(),
            #[cfg(not(feature = "multithreading"))]
            guard: unsafe { self.inner.unlock_unchecked().borrow_mut() },
        }
    }

    /// Try to borrow the contents immutably without blocking.
    ///
    /// Returns `None` if a write lock is currently held.
    pub fn try_borrow(&self) -> Option<ThreadSafeReadGuard<'_, T>> {
        #[cfg(feature = "multithreading")]
        {
            self.inner
                .try_read()
                .map(|guard| ThreadSafeReadGuard { guard })
        }
        #[cfg(not(feature = "multithreading"))]
        {
            self.inner
                .try_borrow()
                .ok()
                .map(|guard| ThreadSafeReadGuard { guard })
        }
    }

    /// Try to borrow the contents mutably without blocking.
    ///
    /// Returns `None` if any locks (read or write) are currently held.
    pub fn try_borrow_mut<'gc>(
        &self,
        _gc: &gc_arena::Mutation<'gc>,
    ) -> Option<ThreadSafeWriteGuard<'_, T>> {
        #[cfg(feature = "multithreading")]
        {
            let _ = _gc;
            self.inner
                .try_write()
                .map(|guard| ThreadSafeWriteGuard { guard })
        }
        #[cfg(not(feature = "multithreading"))]
        {
            unsafe {
                self.inner
                    .unlock_unchecked()
                    .try_borrow_mut()
                    .ok()
                    .map(|guard| ThreadSafeWriteGuard { guard })
            }
        }
    }

    /// Get an immutable reference to the inner value.
    ///
    /// # Safety
    ///
    /// This bypasses the lock and should only be used when you can guarantee
    /// that no other threads are accessing the value.
    pub unsafe fn as_ptr(&self) -> *const T {
        #[cfg(feature = "multithreading")]
        {
            self.inner.data_ptr()
        }
        #[cfg(not(feature = "multithreading"))]
        {
            self.inner.as_ptr()
        }
    }
}

unsafe impl<T: Collect> Collect for ThreadSafeLock<T> {
    fn trace(&self, cc: &Collection) {
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
pub struct ThreadSafeReadGuard<'a, T> {
    guard: RwLockReadGuard<'a, T>,
}

impl<T> Deref for ThreadSafeReadGuard<'_, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.guard
    }
}

/// RAII guard for mutable borrows.
pub struct ThreadSafeWriteGuard<'a, T> {
    guard: RwLockWriteGuard<'a, T>,
}

impl<T> Deref for ThreadSafeWriteGuard<'_, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.guard
    }
}

impl<T> DerefMut for ThreadSafeWriteGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.guard
    }
}

// The lock itself is Send and Sync if the inner type is Send
unsafe impl<T: Send> Send for ThreadSafeLock<T> {}
unsafe impl<T: Send> Sync for ThreadSafeLock<T> {}

#[cfg(test)]
mod tests {
    use super::*;

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

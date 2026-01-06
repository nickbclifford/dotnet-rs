//! Thread-safe lock for heap objects in multi-threaded GC environment.
//!
//! This module provides a replacement for `gc_arena::lock::RefLock` that is safe
//! to use across multiple threads. Unlike `RefLock`, which uses non-atomic borrow
//! flags, `ThreadSafeLock` uses atomic operations to ensure thread-safe access.
use gc_arena::{barrier::Unlock, Collect, Collection};
use parking_lot::{RwLock, RwLockReadGuard, RwLockWriteGuard};
use std::ops::{Deref, DerefMut};

/// A thread-safe lock for GC-managed objects.
///
/// This lock uses `parking_lot::RwLock` internally to provide thread-safe
/// read/write access to objects. It implements `Collect` so it can be used
/// with `gc-arena`.
///
/// Unlike `RefLock`, this lock:
/// - Uses atomic operations for the borrow flag
/// - Is safe to use across multiple threads
/// - Allows multiple concurrent readers or one writer
/// - Does not panic on lock conflicts (uses OS-level blocking instead)
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
    ///
    /// This acquires a read lock, which allows multiple concurrent readers.
    /// If a write lock is held, this will block until the writer releases it.
    ///
    /// # GC Safety
    ///
    /// CAUTION: This method uses `parking_lot::RwLock`, which may block without
    /// checking GC safe points. While parking_lot's locks are typically very fast
    /// (using adaptive spinning), prolonged contention could delay reaching a safe
    /// point during stop-the-world GC pauses.
    ///
    /// For critical sections that may experience contention, consider using
    /// `try_borrow()` in a loop with explicit safe point checks between attempts.
    pub fn borrow(&self) -> ThreadSafeReadGuard<'_, T> {
        ThreadSafeReadGuard {
            guard: self.inner.read(),
        }
    }

    /// Borrow the contents mutably.
    ///
    /// This acquires a write lock, which provides exclusive access.
    /// If any readers or writers are active, this will block until they release.
    ///
    /// # GC Safety
    ///
    /// CAUTION: This method uses `parking_lot::RwLock`, which may block without
    /// checking GC safe points. While parking_lot's locks are typically very fast
    /// (using adaptive spinning), prolonged contention could delay reaching a safe
    /// point during stop-the-world GC pauses.
    ///
    /// For critical sections that may experience contention, consider using
    /// `try_borrow_mut()` in a loop with explicit safe point checks between attempts.
    ///
    /// Note: The `_gc` parameter is kept for API compatibility with `RefLock`,
    /// which requires a mutation context. We don't actually use it since we're
    /// using a different synchronization mechanism.
    pub fn borrow_mut<'gc>(&self, _gc: &gc_arena::Mutation<'gc>) -> ThreadSafeWriteGuard<'_, T> {
        ThreadSafeWriteGuard {
            guard: self.inner.write(),
        }
    }

    /// Try to borrow the contents immutably without blocking.
    ///
    /// Returns `None` if a write lock is currently held.
    pub fn try_borrow(&self) -> Option<ThreadSafeReadGuard<'_, T>> {
        self.inner
            .try_read()
            .map(|guard| ThreadSafeReadGuard { guard })
    }

    /// Try to borrow the contents mutably without blocking.
    ///
    /// Returns `None` if any locks (read or write) are currently held.
    pub fn try_borrow_mut(&self) -> Option<ThreadSafeWriteGuard<'_, T>> {
        self.inner
            .try_write()
            .map(|guard| ThreadSafeWriteGuard { guard })
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

unsafe impl<T: Collect> Collect for ThreadSafeLock<T> {
    fn trace(&self, cc: &Collection) {
        // SAFETY: Tracing happens during a stop-the-world pause, so no other
        // threads are running. We can safely access the inner value without
        // acquiring the lock. This avoids deadlock (or panic) if a thread was
        // already holding the write lock when it reached a safe point.
        unsafe {
            (*self.inner.data_ptr()).trace(cc);
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
                let mut guard = lock.borrow_mut(&mc);
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
            let _writer = lock.borrow_mut(&mc);
            // Should fail to get reader while writer is active
            assert!(lock.try_borrow().is_none());
        });
    }
}

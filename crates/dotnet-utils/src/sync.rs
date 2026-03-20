//! Basic synchronization primitives.
//!
//! This module provides a unified interface for synchronization primitives
//! that works across both single-threaded and multi-threaded configurations.
//! Low-level modules can depend on this without pulling in the entire VM subsystem.
#[cfg(not(feature = "multithreading"))]
pub mod compat {
    use std::cell::{Ref, RefCell, RefMut};
    use std::ops::{Deref, DerefMut};
    #[derive(Debug, Default)]
    pub struct Mutex<T>(RefCell<T>);
    impl<T> Mutex<T> {
        pub fn new(t: T) -> Self {
            Self(RefCell::new(t))
        }
        pub fn lock(&self) -> MutexGuard<'_, T> {
            MutexGuard(self.0.borrow_mut())
        }
        pub fn get_mut(&mut self) -> &mut T {
            self.0.get_mut()
        }
    }
    pub struct MutexGuard<'a, T>(RefMut<'a, T>);
    impl<T> Deref for MutexGuard<'_, T> {
        type Target = T;
        fn deref(&self) -> &T {
            &self.0
        }
    }
    impl<T> DerefMut for MutexGuard<'_, T> {
        fn deref_mut(&mut self) -> &mut T {
            &mut self.0
        }
    }
    #[derive(Debug, Default)]
    pub struct RwLock<T>(RefCell<T>);
    impl<T> RwLock<T> {
        pub fn new(t: T) -> Self {
            Self(RefCell::new(t))
        }
        pub fn read(&self) -> RwLockReadGuard<'_, T> {
            RwLockReadGuard(self.0.borrow())
        }
        pub fn write(&self) -> RwLockWriteGuard<'_, T> {
            RwLockWriteGuard(self.0.borrow_mut())
        }
        pub fn get_mut(&mut self) -> &mut T {
            self.0.get_mut()
        }
        /// Get a pointer to the inner data.
        ///
        /// # Safety
        ///
        /// Caller must ensure they have proper access to the data and that the
        /// pointer does not outlive the lock.
        pub unsafe fn data_ptr(&self) -> *mut T {
            self.0.as_ptr()
        }
        pub fn try_read(&self) -> Option<RwLockReadGuard<'_, T>> {
            self.0.try_borrow().ok().map(RwLockReadGuard)
        }
        pub fn try_write(&self) -> Option<RwLockWriteGuard<'_, T>> {
            self.0.try_borrow_mut().ok().map(RwLockWriteGuard)
        }
    }
    pub struct RwLockReadGuard<'a, T>(Ref<'a, T>);
    impl<T> Deref for RwLockReadGuard<'_, T> {
        type Target = T;
        fn deref(&self) -> &T {
            &self.0
        }
    }
    impl<'a, T> RwLockReadGuard<'a, T> {
        pub fn map<U: ?Sized, F>(this: Self, f: F) -> MappedRwLockReadGuard<'a, U>
        where
            F: FnOnce(&T) -> &U,
        {
            Ref::map(this.0, f)
        }
    }
    pub struct RwLockWriteGuard<'a, T>(RefMut<'a, T>);
    impl<T> Deref for RwLockWriteGuard<'_, T> {
        type Target = T;
        fn deref(&self) -> &T {
            &self.0
        }
    }
    impl<T> DerefMut for RwLockWriteGuard<'_, T> {
        fn deref_mut(&mut self) -> &mut T {
            &mut self.0
        }
    }
    impl<'a, T> RwLockWriteGuard<'a, T> {
        pub fn map<U: ?Sized, F>(this: Self, f: F) -> MappedRwLockWriteGuard<'a, U>
        where
            F: FnOnce(&mut T) -> &mut U,
        {
            RefMut::map(this.0, f)
        }
    }
    #[derive(Debug, Default)]
    pub struct Condvar(());
    impl Condvar {
        pub const fn new() -> Self {
            Self(())
        }
        pub fn notify_one(&self) {}
        pub fn notify_all(&self) {}
        pub fn wait<T>(&self, _guard: &mut MutexGuard<'_, T>) {}
    }
    pub type MappedRwLockReadGuard<'a, T> = Ref<'a, T>;
    pub type MappedRwLockWriteGuard<'a, T> = RefMut<'a, T>;
}

pub use std::sync::{
    Arc, Weak,
    atomic::{
        AtomicBool, AtomicI8, AtomicI16, AtomicI32, AtomicI64, AtomicIsize, AtomicU8, AtomicU16,
        AtomicU32, AtomicU64, AtomicUsize, Ordering,
    },
};

#[cfg(feature = "multithreading")]
thread_local! {
    /// Cached managed thread ID for the current thread
    pub static MANAGED_THREAD_ID: std::cell::Cell<Option<crate::ArenaId>> = const { std::cell::Cell::new(None) };
}

/// Get the current thread's managed ID from thread-local storage.
pub fn get_current_thread_id() -> crate::ArenaId {
    #[cfg(feature = "multithreading")]
    {
        MANAGED_THREAD_ID.with(|id| id.get().unwrap_or(crate::ArenaId::INVALID))
    }
    #[cfg(not(feature = "multithreading"))]
    {
        crate::ArenaId(1)
    }
}

#[cfg(feature = "multithreading")]
pub use parking_lot::{
    Condvar, MappedRwLockReadGuard, MappedRwLockWriteGuard, Mutex, MutexGuard, RwLock,
    RwLockReadGuard, RwLockWriteGuard,
};

#[cfg(not(feature = "multithreading"))]
pub use compat::*;

// ── compile-time + runtime tests for single-threaded compat sync primitives ──
//
// These tests are only meaningful (and only compilable) under
// `--no-default-features`, i.e. when the `compat` module is in use.
#[cfg(all(test, not(feature = "multithreading")))]
mod sync_send_sync_tests {
    use super::compat::{Condvar, Mutex, RwLock};
    use static_assertions::{assert_impl_all, assert_not_impl_all};

    // ── compile-time: Sync must NOT be implemented ──────────────────────────
    // `RefCell<T>` is unconditionally `!Sync`; wrapping it must not change that.
    assert_not_impl_all!(Mutex<i32>: Sync);
    assert_not_impl_all!(RwLock<i32>: Sync);
    // Guards borrow from the RefCell, so they must also be !Sync.
    assert_not_impl_all!(super::compat::MutexGuard<'static, i32>: Sync);
    assert_not_impl_all!(super::compat::RwLockReadGuard<'static, i32>: Sync);
    assert_not_impl_all!(super::compat::RwLockWriteGuard<'static, i32>: Sync);

    // ── compile-time: Send follows the inner T ───────────────────────────────
    // When T: Send, the wrapper must be Send.
    assert_impl_all!(Mutex<i32>: Send);
    assert_impl_all!(RwLock<i32>: Send);
    // When T: !Send (raw pointer), the wrapper must NOT be Send.
    assert_not_impl_all!(Mutex<*mut i32>: Send);
    assert_not_impl_all!(RwLock<*mut i32>: Send);

    // ── runtime: basic lock/unlock round-trips ────────────────────────────────
    #[test]
    fn compat_mutex_lock_unlock() {
        let m = Mutex::new(0u32);
        {
            let mut g = m.lock();
            *g = 42;
        }
        assert_eq!(*m.lock(), 42);
    }

    #[test]
    fn compat_rwlock_read_write() {
        let rw = RwLock::new(String::from("hello"));
        assert_eq!(*rw.read(), "hello");
        *rw.write() = String::from("world");
        assert_eq!(*rw.read(), "world");
    }

    #[test]
    fn compat_rwlock_try_variants_succeed_when_unlocked() {
        let rw = RwLock::new(99i32);
        assert!(rw.try_read().is_some());
        assert!(rw.try_write().is_some());
    }

    #[test]
    fn compat_rwlock_try_write_fails_while_borrowed() {
        let rw = RwLock::new(0i32);
        let _r = rw.read();
        // A second immutable borrow is allowed by RefCell.
        assert!(rw.try_read().is_some());
        // But a mutable borrow must fail.
        assert!(rw.try_write().is_none());
    }

    // ── runtime: confirm double-mutable-borrow panics (RefCell semantics) ────
    #[test]
    #[should_panic]
    fn compat_mutex_double_lock_panics() {
        let m = Mutex::new(0i32);
        let _g1 = m.lock();
        let _g2 = m.lock(); // RefCell: already mutably borrowed → panic
    }

    // ── runtime: Condvar no-ops must not panic ────────────────────────────────
    #[test]
    fn compat_condvar_noop_methods_do_not_panic() {
        let cv = Condvar::new();
        cv.notify_one();
        cv.notify_all();
        let m = Mutex::new(());
        let mut g = m.lock();
        cv.wait(&mut g); // single-threaded stub: immediate return
    }
}

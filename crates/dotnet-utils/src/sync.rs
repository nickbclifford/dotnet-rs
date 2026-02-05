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

    // SAFETY: In single-threaded mode, we can safely share across "threads"
    // because there is only one.
    unsafe impl<T> Sync for Mutex<T> {}
    unsafe impl<T> Send for Mutex<T> {}
    unsafe impl<T> Sync for RwLock<T> {}
    unsafe impl<T> Send for RwLock<T> {}

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
    Arc,
    atomic::{
        AtomicBool, AtomicI8, AtomicI16, AtomicI32, AtomicI64, AtomicIsize, AtomicU8, AtomicU16,
        AtomicU32, AtomicU64, AtomicUsize, Ordering,
    },
};

#[cfg(feature = "multithreading")]
thread_local! {
    /// Cached managed thread ID for the current thread
    pub static MANAGED_THREAD_ID: std::cell::Cell<Option<u64>> = const { std::cell::Cell::new(None) };
}

/// Get the current thread's managed ID from thread-local storage.
pub fn get_current_thread_id() -> u64 {
    #[cfg(feature = "multithreading")]
    {
        MANAGED_THREAD_ID.with(|id| id.get().unwrap_or(0))
    }
    #[cfg(not(feature = "multithreading"))]
    {
        1
    }
}

#[cfg(feature = "multithreading")]
pub use parking_lot::{
    Condvar, MappedRwLockReadGuard, MappedRwLockWriteGuard, Mutex, MutexGuard, RwLock,
    RwLockReadGuard, RwLockWriteGuard,
};

#[cfg(not(feature = "multithreading"))]
pub use compat::*;

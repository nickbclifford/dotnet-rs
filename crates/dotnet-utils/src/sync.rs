//! Basic synchronization primitives.
//!
//! This module provides a unified interface for synchronization primitives
//! that works across both single-threaded and multi-threaded configurations.
//! Low-level modules can depend on this without pulling in the entire VM subsystem.
#[cfg(loom)]
mod loom_compat {
    use std::ops::{Deref, DerefMut};
    use std::sync::TryLockError;

    /// A parking_lot-shaped wrapper around [`loom::sync::Mutex`].
    #[derive(Debug, Default)]
    pub struct Mutex<T: ?Sized>(loom::sync::Mutex<T>);

    impl<T> Mutex<T> {
        pub fn new(value: T) -> Self {
            Self(loom::sync::Mutex::new(value))
        }
    }

    impl<T: ?Sized> Mutex<T> {
        pub fn lock(&self) -> MutexGuard<'_, T> {
            MutexGuard(Some(self.0.lock().expect("loom mutex cannot be poisoned")))
        }

        pub fn try_lock(&self) -> Option<MutexGuard<'_, T>> {
            match self.0.try_lock() {
                Ok(guard) => Some(MutexGuard(Some(guard))),
                Err(TryLockError::WouldBlock) => None,
                Err(TryLockError::Poisoned(_)) => panic!("loom mutex cannot be poisoned"),
            }
        }

        pub fn get_mut(&mut self) -> &mut T {
            self.0.get_mut().expect("loom mutex cannot be poisoned")
        }
    }

    /// A mutex guard that can temporarily surrender its inner loom guard to a condition wait.
    #[derive(Debug)]
    pub struct MutexGuard<'a, T: ?Sized>(Option<loom::sync::MutexGuard<'a, T>>);

    impl<T: ?Sized> Deref for MutexGuard<'_, T> {
        type Target = T;

        fn deref(&self) -> &Self::Target {
            self.0
                .as_ref()
                .expect("mutex guard is temporarily held by a condition wait")
        }
    }

    impl<T: ?Sized> DerefMut for MutexGuard<'_, T> {
        fn deref_mut(&mut self) -> &mut Self::Target {
            self.0
                .as_mut()
                .expect("mutex guard is temporarily held by a condition wait")
        }
    }

    /// A parking_lot-shaped wrapper around [`loom::sync::RwLock`].
    #[derive(Debug, Default)]
    pub struct RwLock<T>(loom::sync::RwLock<T>);

    impl<T> RwLock<T> {
        pub fn new(value: T) -> Self {
            Self(loom::sync::RwLock::new(value))
        }

        pub fn read(&self) -> RwLockReadGuard<'_, T> {
            RwLockReadGuard(self.0.read().expect("loom rwlock cannot be poisoned"))
        }

        pub fn write(&self) -> RwLockWriteGuard<'_, T> {
            RwLockWriteGuard(self.0.write().expect("loom rwlock cannot be poisoned"))
        }

        pub fn try_read(&self) -> Option<RwLockReadGuard<'_, T>> {
            match self.0.try_read() {
                Ok(guard) => Some(RwLockReadGuard(guard)),
                Err(TryLockError::WouldBlock) => None,
                Err(TryLockError::Poisoned(_)) => panic!("loom rwlock cannot be poisoned"),
            }
        }

        pub fn try_write(&self) -> Option<RwLockWriteGuard<'_, T>> {
            match self.0.try_write() {
                Ok(guard) => Some(RwLockWriteGuard(guard)),
                Err(TryLockError::WouldBlock) => None,
                Err(TryLockError::Poisoned(_)) => panic!("loom rwlock cannot be poisoned"),
            }
        }

        pub fn get_mut(&mut self) -> &mut T {
            self.0.get_mut().expect("loom rwlock cannot be poisoned")
        }

        /// Get a pointer to the inner data.
        ///
        /// # Safety
        ///
        /// Caller must ensure proper access to the data and that the pointer
        /// does not outlive the lock. Acquiring and dropping the write guard
        /// obtains the stable inner pointer without exposing loom's poisoning API.
        pub unsafe fn data_ptr(&self) -> *mut T {
            let mut guard = self.write();
            &mut *guard
        }
    }

    /// Read guard for a [`RwLock`].
    #[derive(Debug)]
    pub struct RwLockReadGuard<'a, T>(loom::sync::RwLockReadGuard<'a, T>);

    impl<T> Deref for RwLockReadGuard<'_, T> {
        type Target = T;

        fn deref(&self) -> &Self::Target {
            &self.0
        }
    }

    impl<'a, T> RwLockReadGuard<'a, T> {
        pub fn map<U: ?Sized, F>(this: Self, f: F) -> MappedRwLockReadGuard<'a, U>
        where
            F: FnOnce(&T) -> &U,
        {
            let value = f(&this) as *const U;
            MappedRwLockReadGuard {
                // The erased guard remains owned until the mapped guard is dropped.
                // Consequently, `value` cannot be used after the source lock is released.
                value,
                _guard: Box::new(this),
            }
        }
    }

    /// Write guard for a [`RwLock`].
    #[derive(Debug)]
    pub struct RwLockWriteGuard<'a, T>(loom::sync::RwLockWriteGuard<'a, T>);

    impl<T> Deref for RwLockWriteGuard<'_, T> {
        type Target = T;

        fn deref(&self) -> &Self::Target {
            &self.0
        }
    }

    impl<T> DerefMut for RwLockWriteGuard<'_, T> {
        fn deref_mut(&mut self) -> &mut Self::Target {
            &mut self.0
        }
    }

    impl<'a, T> RwLockWriteGuard<'a, T> {
        pub fn map<U: ?Sized, F>(mut this: Self, f: F) -> MappedRwLockWriteGuard<'a, U>
        where
            F: FnOnce(&mut T) -> &mut U,
        {
            let value = f(&mut this) as *mut U;
            MappedRwLockWriteGuard {
                // The erased guard remains owned until the mapped guard is dropped.
                // Consequently, `value` cannot be used after the source lock is released.
                value,
                _guard: Box::new(this),
            }
        }
    }

    // These traits type-erase the source guard while retaining its drop behavior. They omit
    // Send/Sync bounds deliberately: a mapped guard must not become more transferable than the
    // loom guard that continues to protect its projection.
    trait ErasedReadGuard {}
    impl<T> ErasedReadGuard for RwLockReadGuard<'_, T> {}
    trait ErasedWriteGuard {}
    impl<T> ErasedWriteGuard for RwLockWriteGuard<'_, T> {}

    /// A projection of a read guard that preserves ownership of the original guard.
    pub struct MappedRwLockReadGuard<'a, T: ?Sized> {
        value: *const T,
        _guard: Box<dyn ErasedReadGuard + 'a>,
    }

    impl<T: ?Sized> Deref for MappedRwLockReadGuard<'_, T> {
        type Target = T;

        fn deref(&self) -> &Self::Target {
            // SAFETY: F10.BorrowedStorageStable — `value` was produced from a valid reference while `_guard` held the
            // corresponding lock. `_guard` is dropped after this field and keeps that lock held
            // for the mapped guard's entire lifetime.
            unsafe { &*self.value }
        }
    }

    impl<'a, T: ?Sized> MappedRwLockReadGuard<'a, T> {
        pub fn map<U: ?Sized, F>(this: Self, f: F) -> MappedRwLockReadGuard<'a, U>
        where
            F: FnOnce(&T) -> &U,
        {
            let value = f(&this) as *const U;
            MappedRwLockReadGuard {
                value,
                // Keep the original lock guard, rather than merely the prior projected pointer.
                _guard: this._guard,
            }
        }
    }

    /// A projection of a write guard that preserves ownership of the original guard.
    pub struct MappedRwLockWriteGuard<'a, T: ?Sized> {
        value: *mut T,
        _guard: Box<dyn ErasedWriteGuard + 'a>,
    }

    impl<T: ?Sized> Deref for MappedRwLockWriteGuard<'_, T> {
        type Target = T;

        fn deref(&self) -> &Self::Target {
            // SAFETY: F10.BorrowedStorageStable — see `MappedRwLockReadGuard::deref`; the retained write guard also
            // guarantees that no aliases to the projection are created by this adapter.
            unsafe { &*self.value }
        }
    }

    impl<T: ?Sized> DerefMut for MappedRwLockWriteGuard<'_, T> {
        fn deref_mut(&mut self) -> &mut Self::Target {
            // SAFETY: F10.BorrowedStorageStable — `&mut self` makes this the sole mutable access through the mapped guard,
            // while its retained source write guard remains locked.
            unsafe { &mut *self.value }
        }
    }

    impl<'a, T: ?Sized> MappedRwLockWriteGuard<'a, T> {
        pub fn map<U: ?Sized, F>(mut this: Self, f: F) -> MappedRwLockWriteGuard<'a, U>
        where
            F: FnOnce(&mut T) -> &mut U,
        {
            let value = f(&mut this) as *mut U;
            MappedRwLockWriteGuard {
                value,
                // Keep the original lock guard, rather than merely the prior projected pointer.
                _guard: this._guard,
            }
        }
    }

    /// A parking_lot-shaped wrapper around [`loom::sync::Condvar`].
    #[derive(Debug, Default)]
    pub struct Condvar(loom::sync::Condvar);

    impl Condvar {
        pub fn new() -> Self {
            Self(loom::sync::Condvar::new())
        }

        pub fn notify_one(&self) {
            self.0.notify_one();
        }

        pub fn notify_all(&self) {
            self.0.notify_all();
        }

        pub fn wait<'a, T>(&self, guard: &mut MutexGuard<'a, T>) {
            let inner = guard
                .0
                .take()
                .expect("mutex guard is already held by a condition wait");
            guard.0 = Some(self.0.wait(inner).expect("loom mutex cannot be poisoned"));
        }
    }
}

#[cfg(all(not(loom), not(feature = "multithreading")))]
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
        pub fn try_lock(&self) -> Option<MutexGuard<'_, T>> {
            self.0.try_borrow_mut().ok().map(MutexGuard)
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
    /// Single-threaded condition-variable stub.
    ///
    /// Notifications are harmless no-ops. [`Condvar::wait`] panics because
    /// waiting for another thread in this build configuration is a runtime
    /// invariant violation.
    #[derive(Debug, Default)]
    pub struct Condvar(());
    impl Condvar {
        pub const fn new() -> Self {
            Self(())
        }
        pub fn notify_one(&self) {}
        pub fn notify_all(&self) {}
        pub fn wait<T>(&self, _guard: &mut MutexGuard<'_, T>) {
            unreachable!("compat::Condvar::wait cannot be used in a single-threaded build")
        }
    }
    pub type MappedRwLockReadGuard<'a, T> = Ref<'a, T>;
    pub type MappedRwLockWriteGuard<'a, T> = RefMut<'a, T>;
}
#[path = "sync/lock_order.rs"]
pub mod lock_order;

pub use lock_order::{
    AcquireAfter, HeldLockLevel, LockLevel, OrderedMutex, OrderedMutexGuard, OrderedRwLock,
    OrderedRwLockReadGuard, OrderedRwLockWriteGuard, levels, unlocked,
};
// Loom does not provide `Weak` or the unsized-`Arc` behavior exposed by this facade.
pub use std::sync::{Arc, Weak};

#[cfg(loom)]
pub use loom::sync::atomic::{
    AtomicBool, AtomicI8, AtomicI16, AtomicI32, AtomicI64, AtomicIsize, AtomicU8, AtomicU16,
    AtomicU32, AtomicU64, AtomicUsize, Ordering,
};

#[cfg(not(loom))]
pub use std::sync::atomic::{
    AtomicBool, AtomicI8, AtomicI16, AtomicI32, AtomicI64, AtomicIsize, AtomicU8, AtomicU16,
    AtomicU32, AtomicU64, AtomicUsize, Ordering,
};

#[cfg(any(feature = "multithreading", feature = "memory-validation"))]
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
        // Single-threaded builds use 1 as their stable arena identity. INVALID remains reserved
        // for an absent owner or an uninitialized multithreaded identity.
        #[cfg(feature = "memory-validation")]
        {
            MANAGED_THREAD_ID.with(|id| id.get().unwrap_or(crate::ArenaId::new(1)))
        }
        #[cfg(not(feature = "memory-validation"))]
        {
            crate::ArenaId::new(1)
        }
    }
}

#[cfg(loom)]
pub use loom_compat::*;

#[cfg(all(not(loom), feature = "multithreading"))]
pub use parking_lot::{
    Condvar, MappedRwLockReadGuard, MappedRwLockWriteGuard, Mutex, MutexGuard, RwLock,
    RwLockReadGuard, RwLockWriteGuard,
};

#[cfg(all(not(loom), not(feature = "multithreading")))]
pub use compat::*;

// ── compile-time + runtime tests for single-threaded compat sync primitives ──
//
// These tests are only meaningful (and only compilable) under
// `--no-default-features`, i.e. when the `compat` module is in use.
#[cfg(all(test, not(loom), not(feature = "multithreading")))]
mod sync_send_sync_tests {
    use super::compat::{Condvar, Mutex, RwLock};
    use static_assertions::{assert_impl_all, assert_not_impl_all};

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
    // When T: !Send (raw pointer), the wrapper must be neither Send nor Sync.
    assert_not_impl_all!(Mutex<*mut i32>: Send);
    assert_not_impl_all!(RwLock<*mut i32>: Send);
    assert_not_impl_all!(Mutex<*mut i32>: Sync);
    assert_not_impl_all!(RwLock<*mut i32>: Sync);

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

    // ── runtime: RwLock enforces RefCell aliasing rules ───────────────────────
    #[test]
    #[should_panic]
    fn compat_rwlock_write_while_reading_panics() {
        let rw = RwLock::new(0i32);
        let _reader = rw.read();
        let _writer = rw.write();
    }

    #[test]
    #[should_panic]
    fn compat_rwlock_read_while_writing_panics() {
        let rw = RwLock::new(0i32);
        let _writer = rw.write();
        let _reader = rw.read();
    }

    // ── runtime: Condvar notifications are harmless no-ops ──────────────────
    #[test]
    fn compat_condvar_notifications_do_not_panic() {
        let cv = Condvar::new();
        cv.notify_one();
        cv.notify_all();
    }

    #[test]
    #[should_panic(expected = "compat::Condvar::wait cannot be used")]
    fn compat_condvar_wait_panics() {
        let cv = Condvar::new();
        let m = Mutex::new(());
        let mut guard = m.lock();
        cv.wait(&mut guard);
    }
}

// Loom synchronization objects must be created inside a model. These tests exercise the
// parking_lot-shaped facade rather than loom's native Result-returning APIs.
#[cfg(all(test, loom))]
mod loom_facade_tests {
    use super::{
        Condvar, Mutex, OrderedMutex, RwLock,
        levels::{CollectionLock, GcCoordination, ThreadRegistry},
    };
    use loom::{
        sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        },
        thread,
    };

    #[test]
    fn mutex_lock_and_try_lock_follow_facade_semantics() {
        loom::model(|| {
            let mutex = Arc::new(Mutex::new(0usize));
            let locked = Arc::new(AtomicBool::new(false));
            let release = Arc::new(AtomicBool::new(false));

            let worker_mutex = Arc::clone(&mutex);
            let worker_locked = Arc::clone(&locked);
            let worker_release = Arc::clone(&release);
            let worker = thread::spawn(move || {
                let mut guard = worker_mutex.lock();
                *guard = 1;
                worker_locked.store(true, Ordering::Release);
                while !worker_release.load(Ordering::Acquire) {
                    thread::yield_now();
                }
            });

            while !locked.load(Ordering::Acquire) {
                thread::yield_now();
            }
            assert!(mutex.try_lock().is_none());
            release.store(true, Ordering::Release);
            worker.join().unwrap();

            let mut guard = mutex.try_lock().expect("released mutex must be lockable");
            assert_eq!(*guard, 1);
            *guard = 2;
            drop(guard);
            assert_eq!(*mutex.lock(), 2);
        });
    }

    #[test]
    fn mapped_rwlock_guards_preserve_the_source_lock() {
        loom::model(|| {
            let lock = RwLock::new((String::from("facade"), vec![1usize, 2]));

            let read = super::RwLockReadGuard::map(lock.read(), |value| &value.0);
            let read = super::MappedRwLockReadGuard::map(read, |value| &value[..]);
            assert_eq!(&*read, "facade");
            drop(read);

            let write = super::RwLockWriteGuard::map(lock.write(), |value| &mut value.1);
            let mut write = super::MappedRwLockWriteGuard::map(write, |value| &mut value[1]);
            *write = 3;
            drop(write);

            assert_eq!(lock.read().1, vec![1, 3]);
        });
    }

    #[test]
    fn condition_wait_and_notify_use_a_mutable_facade_guard() {
        loom::model(|| {
            let pair = Arc::new((Mutex::new(false), Condvar::new()));
            let waiting_pair = Arc::clone(&pair);
            let waiter = thread::spawn(move || {
                let mut guard = waiting_pair.0.lock();
                while !*guard {
                    waiting_pair.1.wait(&mut guard);
                }
            });

            let mut guard = pair.0.lock();
            *guard = true;
            pair.1.notify_one();
            drop(guard);
            waiter.join().unwrap();
        });
    }

    #[test]
    fn ordered_locks_compose_acquisition_tokens() {
        loom::model(|| {
            let collection = OrderedMutex::<CollectionLock, usize>::new(1);
            let coordination = OrderedMutex::<GcCoordination, usize>::new(2);
            let threads = OrderedMutex::<ThreadRegistry, usize>::new(3);

            let mut collection_guard = collection.lock();
            let mut coordination_guard = coordination.lock_after(collection_guard.held_level());
            let mut threads_guard = threads.lock_after(coordination_guard.held_level());
            *collection_guard += 1;
            *coordination_guard += 1;
            *threads_guard += 1;

            assert_eq!(
                (*collection_guard, *coordination_guard, *threads_guard),
                (2, 3, 4)
            );
        });
    }
}

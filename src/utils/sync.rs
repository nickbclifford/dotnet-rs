//! Basic synchronization primitives.
//!
//! This module provides a unified interface for synchronization primitives
//! that works across both single-threaded and multi-threaded configurations.
//! Low-level modules can depend on this without pulling in the entire VM subsystem.
#[cfg(not(feature = "multithreading"))]
pub mod compat {
    use std::sync::{self, MutexGuard, RwLockReadGuard, RwLockWriteGuard};
    #[derive(Debug)]
    pub struct Mutex<T>(sync::Mutex<T>);
    impl<T> Mutex<T> {
        pub fn new(t: T) -> Self {
            Self(sync::Mutex::new(t))
        }
        pub fn lock(&self) -> MutexGuard<'_, T> {
            self.0.lock().unwrap()
        }
    }
    #[derive(Debug)]
    pub struct RwLock<T>(sync::RwLock<T>);
    impl<T> RwLock<T> {
        pub fn new(t: T) -> Self {
            Self(sync::RwLock::new(t))
        }
        pub fn read(&self) -> RwLockReadGuard<'_, T> {
            self.0.read().unwrap()
        }
        pub fn write(&self) -> RwLockWriteGuard<'_, T> {
            self.0.write().unwrap()
        }
    }
    #[derive(Debug, Default)]
    pub struct Condvar(());
    impl Condvar {
        pub fn new() -> Self {
            Self(())
        }
        pub fn notify_one(&self) {}
        pub fn notify_all(&self) {}
        pub fn wait<T>(&self, _guard: &mut MutexGuard<'_, T>) {}
    }
}

pub use std::sync::{
    atomic::{
        AtomicBool, AtomicI16, AtomicI32, AtomicI64, AtomicI8, AtomicIsize, AtomicU16, AtomicU32,
        AtomicU64, AtomicU8, AtomicUsize, Ordering,
    },
    Arc,
};

#[cfg(feature = "multithreading")]
thread_local! {
    /// Cached managed thread ID for the current thread
    pub(crate) static MANAGED_THREAD_ID: std::cell::Cell<Option<u64>> = const { std::cell::Cell::new(None) };
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
pub use parking_lot::{Condvar, Mutex, MutexGuard, RwLock, RwLockReadGuard, RwLockWriteGuard};

#[cfg(not(feature = "multithreading"))]
pub use compat::*;
#[cfg(not(feature = "multithreading"))]
pub use std::sync::{MutexGuard, RwLockReadGuard, RwLockWriteGuard};

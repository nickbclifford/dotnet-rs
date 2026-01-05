#[cfg(feature = "multithreading")]
mod threaded;
#[cfg(not(feature = "multithreading"))]
mod single_threaded;

#[cfg(feature = "multithreading")]
pub use threaded::*;
#[cfg(not(feature = "multithreading"))]
pub use single_threaded::*;

// Re-export Arc (same for both std and parking_lot)
pub use std::sync::Arc;

// Re-export atomic types (always from std::sync::atomic)
pub use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};

#[cfg(feature = "multithreading")]
pub use parking_lot::{Condvar, Mutex, MutexGuard, RwLock, RwLockReadGuard, RwLockWriteGuard};

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
    }
}

#[cfg(not(feature = "multithreading"))]
pub use compat::*;
#[cfg(not(feature = "multithreading"))]
pub use std::sync::{MutexGuard, RwLockReadGuard, RwLockWriteGuard};

//! Garbage collection utility types.
use gc_arena::{barrier::Unlock, Collect, Collection, Mutation};
use std::ops::{Deref, DerefMut};

#[cfg(feature = "multithreaded-gc")]
use crate::sync::{Arc, AtomicBool, AtomicUsize, Condvar, Mutex, Ordering};
#[cfg(feature = "multithreaded-gc")]
use std::{
    cell::{Cell, RefCell},
    collections::HashSet,
    mem,
};

#[cfg(feature = "multithreading")]
use parking_lot::{
    MappedRwLockReadGuard, MappedRwLockWriteGuard, RwLock, RwLockReadGuard, RwLockWriteGuard,
};

#[cfg(not(feature = "multithreading"))]
use gc_arena::lock::RefLock as RwLock;
#[cfg(not(feature = "multithreading"))]
use std::cell::{Ref as MappedRwLockReadGuard, RefMut as MappedRwLockWriteGuard};

/// A handle to the GC mutation context.
pub type GCHandle<'gc> = &'gc Mutation<'gc>;

/// Type of GC handle, determines how the garbage collector treats the reference.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Collect)]
#[collect(require_static)]
pub enum GCHandleType {
    /// Weak reference that does not track resurrection
    Weak = 0,
    /// Weak reference that tracks resurrection (survives finalization)
    WeakTrackResurrection = 1,
    /// Normal (strong) GC handle
    Normal = 2,
    /// Pinned GC handle (prevents object from being moved by GC)
    Pinned = 3,
}

impl From<i32> for GCHandleType {
    fn from(i: i32) -> Self {
        match i {
            0 => GCHandleType::Weak,
            1 => GCHandleType::WeakTrackResurrection,
            2 => GCHandleType::Normal,
            3 => GCHandleType::Pinned,
            _ => panic!("invalid GCHandleType: {}", i),
        }
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

    /// Borrow the contents immutably.
    pub fn borrow(&self) -> ThreadSafeReadGuard<'_, T> {
        ThreadSafeReadGuard {
            #[cfg(feature = "multithreading")]
            guard: RwLockReadGuard::map(self.inner.read(), |x| x),
            #[cfg(not(feature = "multithreading"))]
            guard: self.inner.borrow(),
        }
    }

    /// Borrow the contents mutably.
    pub fn borrow_mut<'gc>(&self, _gc: &Mutation<'gc>) -> ThreadSafeWriteGuard<'_, T> {
        ThreadSafeWriteGuard {
            #[cfg(feature = "multithreading")]
            guard: RwLockWriteGuard::map(self.inner.write(), |x| x),
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
            self.inner.try_read().map(|guard| ThreadSafeReadGuard {
                guard: RwLockReadGuard::map(guard, |x| x),
            })
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
    pub fn try_borrow_mut<'gc>(&self, _gc: &Mutation<'gc>) -> Option<ThreadSafeWriteGuard<'_, T>> {
        #[cfg(feature = "multithreading")]
        {
            let _ = _gc;
            self.inner.try_write().map(|guard| ThreadSafeWriteGuard {
                guard: RwLockWriteGuard::map(guard, |x| x),
            })
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

// The lock itself is Send and Sync if the inner type is Send
unsafe impl<T: Send> Send for ThreadSafeLock<T> {}
unsafe impl<T: Send> Sync for ThreadSafeLock<T> {}

#[cfg(feature = "multithreaded-gc")]
thread_local! {
    /// Found cross-arena references during the current marking phase.
    static FOUND_CROSS_ARENA_REFS: RefCell<Vec<(u64, usize)>> = const { RefCell::new(Vec::new()) };
    /// The thread ID of the arena currently being traced.
    static CURRENTLY_TRACING_THREAD_ID: Cell<Option<u64>> = const { Cell::new(None) };
}

#[cfg(feature = "multithreaded-gc")]
/// Set the thread ID of the arena currently being traced.
pub fn set_currently_tracing(thread_id: Option<u64>) {
    CURRENTLY_TRACING_THREAD_ID.with(|id| {
        id.set(thread_id);
    });
}

#[cfg(feature = "multithreaded-gc")]
/// Get the thread ID of the arena currently being traced.
pub fn get_currently_tracing() -> Option<u64> {
    CURRENTLY_TRACING_THREAD_ID.get()
}

#[cfg(feature = "multithreaded-gc")]
/// Take all found cross-arena references and clear the local list.
pub fn take_found_cross_arena_refs() -> Vec<(u64, usize)> {
    FOUND_CROSS_ARENA_REFS.with(|refs| {
        let mut r = refs.borrow_mut();
        mem::take(&mut *r)
    })
}

#[cfg(feature = "multithreaded-gc")]
/// Record a cross-arena reference found during marking.
pub fn record_cross_arena_ref(target_thread_id: u64, ptr: usize) {
    FOUND_CROSS_ARENA_REFS.with(|refs| {
        refs.borrow_mut().push((target_thread_id, ptr));
    });
}

#[cfg(feature = "multithreaded-gc")]
/// Clear tracing-related thread-local state.
pub fn clear_tracing_state() {
    CURRENTLY_TRACING_THREAD_ID.with(|id| {
        id.set(None);
    });
    FOUND_CROSS_ARENA_REFS.with(|refs| {
        refs.borrow_mut().clear();
    });
    CURRENT_ARENA_HANDLE.with(|h| {
        *h.borrow_mut() = None;
    });
}

#[cfg(feature = "multithreaded-gc")]
thread_local! {
    /// The ArenaHandle for the current thread.
    static CURRENT_ARENA_HANDLE: RefCell<Option<ArenaHandle>> = const { RefCell::new(None) };
}

#[cfg(feature = "multithreaded-gc")]
/// GC commands sent from the coordinator to worker threads.
#[derive(Debug, Clone)]
pub enum GCCommand {
    /// Perform a full collection of the local arena.
    CollectAll,
    /// Mark specific objects in the local arena (for cross-arena resurrection).
    /// Stores opaque pointers (usize) to avoid dependency on ObjectPtr.
    MarkObjects(HashSet<usize>),
}

#[cfg(feature = "multithreaded-gc")]
/// Metadata about each thread's arena and its communication channel.
#[derive(Debug, Clone)]
pub struct ArenaHandle {
    pub thread_id: u64,
    pub allocation_counter: Arc<AtomicUsize>,
    pub gc_allocated_bytes: Arc<AtomicUsize>,
    pub external_allocated_bytes: Arc<AtomicUsize>,
    pub needs_collection: Arc<AtomicBool>,
    /// Command currently being processed by this thread.
    pub current_command: Arc<Mutex<Option<GCCommand>>>,
    /// Signal to wake up the thread when a command is available.
    pub command_signal: Arc<Condvar>,
    /// Signal to the coordinator that the command is finished.
    pub finish_signal: Arc<Condvar>,
}

#[cfg(feature = "multithreaded-gc")]
impl ArenaHandle {
    pub fn new(thread_id: u64) -> Self {
        Self {
            thread_id,
            allocation_counter: Arc::new(AtomicUsize::new(0)),
            gc_allocated_bytes: Arc::new(AtomicUsize::new(0)),
            external_allocated_bytes: Arc::new(AtomicUsize::new(0)),
            needs_collection: Arc::new(AtomicBool::new(false)),
            current_command: Arc::new(Mutex::new(None)),
            command_signal: Arc::new(Condvar::new()),
            finish_signal: Arc::new(Condvar::new()),
        }
    }

    /// Record an allocation and check if we've exceeded the threshold.
    pub fn record_allocation(&self, size: usize) {
        let current = self.allocation_counter.fetch_add(size, Ordering::Relaxed);
        if current + size > ALLOCATION_THRESHOLD {
            self.needs_collection.store(true, Ordering::Release);
        }
    }
}

#[cfg(feature = "multithreaded-gc")]
/// Allocation threshold in bytes that triggers a GC request for a thread-local arena.
pub const ALLOCATION_THRESHOLD: usize = 1024 * 1024; // 1MB per-thread trigger

#[cfg(feature = "multithreaded-gc")]
/// Set the ArenaHandle for the current thread.
pub fn set_current_arena_handle(handle: ArenaHandle) {
    CURRENT_ARENA_HANDLE.with(|h| {
        *h.borrow_mut() = Some(handle);
    });
}

#[cfg(feature = "multithreaded-gc")]
/// Record an allocation of the given size in the current thread's arena.
pub fn record_allocation(size: usize) {
    CURRENT_ARENA_HANDLE.with(|h| {
        if let Some(handle) = h.borrow().as_ref() {
            handle.record_allocation(size);
        }
    });
}

#[cfg(feature = "multithreaded-gc")]
/// Check if the current thread's arena has requested a collection.
pub fn is_current_arena_collection_requested() -> bool {
    CURRENT_ARENA_HANDLE.with(|h| {
        if let Some(handle) = h.borrow().as_ref() {
            handle.needs_collection.load(Ordering::Acquire)
        } else {
            false
        }
    })
}

#[cfg(feature = "multithreaded-gc")]
/// Reset the collection request flag for the current thread's arena.
pub fn reset_current_arena_collection_requested() {
    CURRENT_ARENA_HANDLE.with(|h| {
        if let Some(handle) = h.borrow().as_ref() {
            handle.needs_collection.store(false, Ordering::Release);
            handle.allocation_counter.store(0, Ordering::Release);
        }
    });
}

#[cfg(feature = "multithreaded-gc")]
/// Update the metrics for the current thread's arena.
pub fn update_current_arena_metrics(gc_bytes: usize, external_bytes: usize) {
    CURRENT_ARENA_HANDLE.with(|h| {
        if let Some(handle) = h.borrow().as_ref() {
            handle.gc_allocated_bytes.store(gc_bytes, Ordering::Relaxed);
            handle
                .external_allocated_bytes
                .store(external_bytes, Ordering::Relaxed);
        }
    });
}

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

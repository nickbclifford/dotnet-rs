//! Garbage collection utility types.
use gc_arena::{Collect, Collection, Mutation, barrier::Unlock};
use std::ops::{Deref, DerefMut};

#[cfg(feature = "multithreading")]
use crate::sync::{Arc, AtomicBool, AtomicUsize, Condvar, Mutex, Ordering};
#[cfg(feature = "multithreading")]
use parking_lot::{
    MappedRwLockReadGuard, MappedRwLockWriteGuard, RwLock, RwLockReadGuard, RwLockWriteGuard,
};
#[cfg(feature = "multithreading")]
use std::{
    cell::{Cell, RefCell},
    collections::{HashMap, HashSet},
    mem,
};

#[cfg(not(feature = "multithreading"))]
use gc_arena::lock::RefLock as RwLock;
#[cfg(not(feature = "multithreading"))]
use std::cell::{Ref as MappedRwLockReadGuard, RefMut as MappedRwLockWriteGuard};

/// A handle to the GC mutation context.
#[derive(Copy, Clone)]
pub struct GCHandle<'gc> {
    pub(crate) mutation: &'gc Mutation<'gc>,
    #[cfg(feature = "multithreading")]
    pub(crate) arena: &'gc ArenaHandleInner,
    #[cfg(feature = "memory-validation")]
    pub(crate) thread_id: crate::ArenaId,
}

impl<'gc> GCHandle<'gc> {
    pub fn new(
        mutation: &'gc Mutation<'gc>,
        #[cfg(feature = "multithreading")] arena: &'gc ArenaHandleInner,
        #[cfg(feature = "memory-validation")] thread_id: crate::ArenaId,
    ) -> Self {
        #[cfg(all(feature = "memory-validation", feature = "multithreading"))]
        {
            if arena.thread_id != thread_id {
                tracing::error!(
                    "GCHandle initialized with mismatched thread/arena: thread_id={}, arena.thread_id={}",
                    thread_id,
                    arena.thread_id
                );
            }
        }
        Self {
            mutation,
            #[cfg(feature = "multithreading")]
            arena,
            #[cfg(feature = "memory-validation")]
            thread_id,
        }
    }

    #[cfg(feature = "memory-validation")]
    fn validate_thread(&self) {
        let current_thread = crate::sync::get_current_thread_id();
        if self.thread_id != current_thread {
            tracing::error!(
                "GCHandle used on wrong thread (expected {:?}, got {:?})",
                self.thread_id,
                current_thread
            );
            panic!(
                "GCHandle used on wrong thread (expected {:?}, got {:?})",
                self.thread_id, current_thread
            );
        }
        #[cfg(feature = "multithreading")]
        {
            if self.arena.thread_id != self.thread_id {
                tracing::error!(
                    "GCHandle arena mismatch: handle.thread_id={}, arena.thread_id={}",
                    self.thread_id,
                    self.arena.thread_id
                );
                panic!(
                    "GCHandle arena mismatch: handle.thread_id={}, arena.thread_id={}",
                    self.thread_id, self.arena.thread_id
                );
            }
        }
    }

    /// Get the underlying mutation context.
    pub fn mutation(&self) -> &'gc Mutation<'gc> {
        #[cfg(feature = "memory-validation")]
        self.validate_thread();
        self.mutation
    }

    /// Record an allocation of the given size in the arena.
    pub fn record_allocation(&self, _size: usize) {
        #[cfg(feature = "memory-validation")]
        self.validate_thread();

        #[cfg(feature = "multithreading")]
        {
            self.arena.record_allocation(_size);
        }
    }
}

impl<'gc> Deref for GCHandle<'gc> {
    type Target = Mutation<'gc>;

    fn deref(&self) -> &Self::Target {
        #[cfg(feature = "memory-validation")]
        self.validate_thread();
        self.mutation
    }
}

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
    /// Returns `None` if any locks (read_unchecked or write) are currently held.
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

#[cfg(feature = "multithreading")]
static VALID_ARENAS: std::sync::LazyLock<RwLock<HashMap<crate::ArenaId, Arc<AtomicBool>>>> =
    std::sync::LazyLock::new(|| RwLock::new(HashMap::new()));

#[cfg(feature = "multithreading")]
pub fn register_arena(thread_id: crate::ArenaId, stw_in_progress: Arc<AtomicBool>) {
    VALID_ARENAS.write().insert(thread_id, stw_in_progress);
}

#[cfg(feature = "multithreading")]
pub fn unregister_arena(thread_id: crate::ArenaId) {
    VALID_ARENAS.write().remove(&thread_id);
}

#[cfg(feature = "multithreading")]
pub fn is_valid_cross_arena_ref(target_thread_id: crate::ArenaId) -> bool {
    VALID_ARENAS.read().contains_key(&target_thread_id)
}

#[cfg(feature = "multithreading")]
pub fn reset_arena_registry() {
    VALID_ARENAS.write().clear();
}

#[cfg(feature = "multithreading")]
pub fn set_stw_in_progress(arena_id: crate::ArenaId, in_progress: bool) {
    if let Some(flag) = VALID_ARENAS.read().get(&arena_id) {
        flag.store(in_progress, Ordering::Release);
    }
}

#[cfg(feature = "multithreading")]
pub fn is_stw_in_progress(arena_id: crate::ArenaId) -> bool {
    VALID_ARENAS
        .read()
        .get(&arena_id)
        .map(|flag| flag.load(Ordering::Acquire))
        .unwrap_or(false)
}
#[cfg(feature = "multithreading")]
thread_local! {
    /// Found cross-arena references during the current marking phase.
    static FOUND_CROSS_ARENA_REFS: RefCell<Vec<(crate::ArenaId, usize)>> = const { RefCell::new(Vec::new()) };
    /// The thread ID of the arena currently being traced.
    static CURRENTLY_TRACING_THREAD_ID: Cell<Option<crate::ArenaId>> = const { Cell::new(None) };
}

#[cfg(feature = "multithreading")]
/// Set the thread ID of the arena currently being traced.
pub fn set_currently_tracing(thread_id: Option<crate::ArenaId>) {
    CURRENTLY_TRACING_THREAD_ID.with(|id| {
        id.set(thread_id);
    });
}

#[cfg(feature = "multithreading")]
/// Get the thread ID of the arena currently being traced.
pub fn get_currently_tracing() -> Option<crate::ArenaId> {
    CURRENTLY_TRACING_THREAD_ID.get()
}

#[cfg(feature = "multithreading")]
/// Take all found cross-arena references and clear the local list.
pub fn take_found_cross_arena_refs() -> Vec<(crate::ArenaId, usize)> {
    FOUND_CROSS_ARENA_REFS.with(|refs| {
        let mut r = refs.borrow_mut();
        mem::take(&mut *r)
    })
}

#[cfg(feature = "multithreading")]
/// Record a cross-arena reference found during marking.
pub fn record_cross_arena_ref(target_thread_id: crate::ArenaId, ptr: usize) {
    if !is_valid_cross_arena_ref(target_thread_id) {
        panic!(
            "Dangling cross-arena reference detected: target_thread_id {:?} is no longer valid (thread exited?)",
            target_thread_id
        );
    }
    FOUND_CROSS_ARENA_REFS.with(|refs| {
        refs.borrow_mut().push((target_thread_id, ptr));
    });
}

#[cfg(feature = "multithreading")]
/// Clear tracing-related thread-local state.
pub fn clear_tracing_state() {
    CURRENTLY_TRACING_THREAD_ID.with(|id| {
        id.set(None);
    });
    FOUND_CROSS_ARENA_REFS.with(|refs| {
        refs.borrow_mut().clear();
    });
}

#[cfg(feature = "multithreading")]
/// GC commands sent from the coordinator to worker threads.
#[derive(Debug, Clone)]
pub enum GCCommand {
    /// Start the marking phase (clear roots, mark local roots).
    MarkAll,
    /// Mark specific objects in the local arena (for cross-arena resurrection).
    /// Stores opaque pointers (usize) to avoid dependency on ObjectPtr.
    MarkObjects(HashSet<usize>),
    /// Run finalizers for dead objects.
    Finalize,
    /// Finish collection (sweep).
    Sweep,
}

#[cfg(feature = "multithreading")]
/// Metadata about each thread's arena and its communication channel.
#[derive(Debug, Clone)]
pub struct ArenaHandle {
    inner: Arc<ArenaHandleInner>,
}

#[cfg(feature = "multithreading")]
#[derive(Debug)]
pub struct ArenaHandleInner {
    pub thread_id: crate::ArenaId,
    pub allocation_counter: AtomicUsize,
    #[cfg(feature = "memory-validation")]
    pub allocation_call_count: AtomicUsize,
    pub gc_allocated_bytes: AtomicUsize,
    pub external_allocated_bytes: AtomicUsize,
    pub needs_collection: AtomicBool,
    /// Command currently being processed by this thread.
    pub current_command: Mutex<Option<GCCommand>>,
    /// Signal to wake up the thread when a command is available.
    pub command_signal: Condvar,
    /// Signal to the coordinator that the command is finished.
    pub finish_signal: Condvar,
}

#[cfg(feature = "multithreading")]
impl ArenaHandleInner {
    /// Record an allocation and check if we've exceeded the threshold.
    pub fn record_allocation(&self, size: usize) {
        #[cfg(feature = "memory-validation")]
        self.allocation_call_count.fetch_add(1, Ordering::Relaxed);

        let current = self.allocation_counter.fetch_add(size, Ordering::Relaxed);
        if current + size > ALLOCATION_THRESHOLD {
            self.needs_collection.store(true, Ordering::Release);
        }
    }
}

#[cfg(feature = "multithreading")]
impl ArenaHandle {
    pub fn new(thread_id: crate::ArenaId) -> Self {
        Self {
            inner: Arc::new(ArenaHandleInner {
                thread_id,
                allocation_counter: AtomicUsize::new(0),
                #[cfg(feature = "memory-validation")]
                allocation_call_count: AtomicUsize::new(0),
                gc_allocated_bytes: AtomicUsize::new(0),
                external_allocated_bytes: AtomicUsize::new(0),
                needs_collection: AtomicBool::new(false),
                current_command: Mutex::new(None),
                command_signal: Condvar::new(),
                finish_signal: Condvar::new(),
            }),
        }
    }

    pub fn as_inner(&self) -> &ArenaHandleInner {
        &self.inner
    }

    pub fn thread_id(&self) -> crate::ArenaId {
        self.inner.thread_id
    }

    pub fn allocation_counter(&self) -> &AtomicUsize {
        &self.inner.allocation_counter
    }

    pub fn gc_allocated_bytes(&self) -> &AtomicUsize {
        &self.inner.gc_allocated_bytes
    }

    pub fn external_allocated_bytes(&self) -> &AtomicUsize {
        &self.inner.external_allocated_bytes
    }

    pub fn needs_collection(&self) -> &AtomicBool {
        &self.inner.needs_collection
    }

    pub fn current_command(&self) -> &Mutex<Option<GCCommand>> {
        &self.inner.current_command
    }

    pub fn command_signal(&self) -> &Condvar {
        &self.inner.command_signal
    }

    pub fn finish_signal(&self) -> &Condvar {
        &self.inner.finish_signal
    }

    /// Record an allocation and check if we've exceeded the threshold.
    pub fn record_allocation(&self, size: usize) {
        self.inner.record_allocation(size);
    }
}

#[cfg(feature = "multithreading")]
/// Allocation threshold in bytes that triggers a GC request for a thread-local arena.
pub const ALLOCATION_THRESHOLD: usize = 1024 * 1024; // 1MB per-thread trigger

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

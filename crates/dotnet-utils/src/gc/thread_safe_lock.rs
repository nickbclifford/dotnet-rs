use gc_arena::{Collect, Gc, Mutation, barrier::Unlock, collect::Trace};
use std::ops::{Deref, DerefMut};

#[cfg(feature = "multithreading")]
use std::cell::Cell;

#[cfg(feature = "multithreading")]
use parking_lot::{
    MappedRwLockReadGuard, MappedRwLockWriteGuard, RwLock, RwLockReadGuard, RwLockWriteGuard,
};

#[cfg(not(feature = "multithreading"))]
use gc_arena::lock::RefLock as RwLock;
#[cfg(not(feature = "multithreading"))]
use std::cell::{Ref as MappedRwLockReadGuard, RefMut as MappedRwLockWriteGuard};

#[cfg(feature = "multithreading")]
thread_local! {
    static ACTIVE_WRITE_GUARDS: Cell<usize> = const { Cell::new(0) };
}

/// Returns whether this thread currently holds a mutable `ThreadSafeLock` guard.
///
/// A managed thread in this state must not park at a GC safepoint: the collector
/// traces lock contents under a read lock, which requires every stopped thread to
/// have released its write guards first.
#[cfg(feature = "multithreading")]
pub fn has_active_thread_safe_write_guard() -> bool {
    ACTIVE_WRITE_GUARDS.get() != 0
}

#[cfg(feature = "multithreading")]
struct SafepointWriteGuard;

#[cfg(feature = "multithreading")]
impl SafepointWriteGuard {
    fn enter() -> Self {
        ACTIVE_WRITE_GUARDS.set(
            ACTIVE_WRITE_GUARDS
                .get()
                .checked_add(1)
                .expect("ThreadSafeLock write-guard depth overflow"),
        );
        Self
    }
}

#[cfg(feature = "multithreading")]
impl Drop for SafepointWriteGuard {
    fn drop(&mut self) {
        let depth = ACTIVE_WRITE_GUARDS.get();
        debug_assert!(depth > 0, "unbalanced ThreadSafeLock write guard");
        ACTIVE_WRITE_GUARDS.set(depth - 1);
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
}

impl<T: ?Sized> ThreadSafeLock<T> {
    /// Borrow the contents mutably, first triggering gc-arena's backward write barrier on the
    /// `Gc` that owns the lock.
    ///
    /// The barrier makes the collector re-trace the owner if it was already marked, so any
    /// `Gc` pointer stored through the guard is seen by incremental marking. A `&Mutation`
    /// alone is not a barrier, which is why this takes the owning `Gc`.
    pub fn borrow_mut<'gc>(this: Gc<'gc, Self>, mc: &Mutation<'gc>) -> ThreadSafeWriteGuard<'gc, T>
    where
        T: 'gc,
    {
        let lock: &'gc Self = Gc::write(mc, this);
        // SAFETY: F1.GcHandleRooted — `Gc::write` just triggered the backward barrier on the
        // lock's owning `Gc` in this callback, so storing new `Gc` children is allowed.
        unsafe { lock.borrow_mut_unbarriered(mc) }
    }

    /// Like [`ThreadSafeLock::borrow_mut`], but returns `None` instead of blocking or
    /// panicking when the lock is already borrowed. The barrier is triggered either way.
    pub fn try_borrow_mut<'gc>(
        this: Gc<'gc, Self>,
        mc: &Mutation<'gc>,
    ) -> Option<ThreadSafeWriteGuard<'gc, T>>
    where
        T: 'gc,
    {
        let lock: &'gc Self = Gc::write(mc, this);
        // SAFETY: F1.GcHandleRooted — barrier triggered on the owning `Gc` just above.
        unsafe { lock.try_borrow_mut_unbarriered(mc) }
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

    /// Borrow the contents mutably without triggering a write barrier.
    ///
    /// Prefer [`ThreadSafeLock::borrow_mut`], which barriers the owning `Gc`.
    ///
    /// # Safety
    ///
    /// Through the returned guard the caller must not make the contents adopt a new `Gc`
    /// pointer, unless `Gc::write` has already been triggered on the `Gc` that owns this lock
    /// during the current arena callback, or the written data is not traced by the collector
    /// (for example raw bytes that hold no GC references). Otherwise incremental marking can
    /// miss the new pointer and free a live object.
    pub unsafe fn borrow_mut_unbarriered<'gc>(
        &self,
        _gc: &Mutation<'gc>,
    ) -> ThreadSafeWriteGuard<'_, T> {
        ThreadSafeWriteGuard {
            guard: RwLockWriteGuard::map(self.inner.write(), |x| x),
            safepoint_guard: SafepointWriteGuard::enter(),
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

    /// Try to borrow the contents mutably without blocking and without triggering a write
    /// barrier.
    ///
    /// Returns `None` if any locks (read_unchecked or write) are currently held.
    ///
    /// # Safety
    ///
    /// Same contract as [`ThreadSafeLock::borrow_mut_unbarriered`].
    pub unsafe fn try_borrow_mut_unbarriered<'gc>(
        &self,
        _gc: &Mutation<'gc>,
    ) -> Option<ThreadSafeWriteGuard<'_, T>> {
        self.inner.try_write().map(|guard| ThreadSafeWriteGuard {
            guard: RwLockWriteGuard::map(guard, |x| x),
            safepoint_guard: SafepointWriteGuard::enter(),
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

    /// Borrow the contents mutably without triggering a write barrier.
    ///
    /// Prefer [`ThreadSafeLock::borrow_mut`], which barriers the owning `Gc`.
    ///
    /// # Safety
    ///
    /// Through the returned guard the caller must not make the contents adopt a new `Gc`
    /// pointer, unless `Gc::write` has already been triggered on the `Gc` that owns this lock
    /// during the current arena callback, or the written data is not traced by the collector
    /// (for example raw bytes that hold no GC references). Otherwise incremental marking can
    /// miss the new pointer and free a live object.
    pub unsafe fn borrow_mut_unbarriered<'gc>(
        &self,
        _gc: &Mutation<'gc>,
    ) -> ThreadSafeWriteGuard<'_, T> {
        ThreadSafeWriteGuard {
            // SAFETY: F1.GcHandleRooted — `_gc` proves we are inside a `mutate` callback, during
            // which gc-arena runs no collection step, so the tracer cannot hold a shared borrow
            // of this `RefLock`. That makes the `RefCell` borrow sound with respect to tracing;
            // whether new contents need a write barrier is the caller's obligation under this
            // function's `# Safety` contract.
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

    /// Try to borrow the contents mutably without blocking and without triggering a write
    /// barrier.
    ///
    /// Returns `None` if any locks (read_unchecked or write) are currently held.
    ///
    /// # Safety
    ///
    /// Same contract as [`ThreadSafeLock::borrow_mut_unbarriered`].
    pub unsafe fn try_borrow_mut_unbarriered<'gc>(
        &self,
        _gc: &Mutation<'gc>,
    ) -> Option<ThreadSafeWriteGuard<'_, T>> {
        // SAFETY: F1.GcHandleRooted — as in `borrow_mut_unbarriered`: `_gc` proves no collection
        // step is running, so the tracer holds no shared borrow of the `RefLock`. A held
        // mutator borrow makes `try_borrow_mut` return `None` instead of panicking.
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

// SAFETY: F5.TracesEveryGcRef — Both lock backends expose `T` to the tracer only through a shared
// borrow. The multithreaded backend requires the GC safepoint protocol to keep
// write-guard holders running until they release their guards, then verifies
// that invariant with `try_read` before tracing. The single-threaded `RefLock`
// implementation delegates to its `Collect` implementation.
unsafe impl<'gc, T: Collect<'gc> + 'gc> Collect<'gc> for ThreadSafeLock<T> {
    fn trace<Tr: Trace<'gc>>(&self, cc: &mut Tr) {
        #[cfg(feature = "multithreading")]
        {
            let guard = self
                .inner
                .try_read()
                .expect("ThreadSafeLock write guard remained active during stop-the-world tracing");
            guard.trace(cc);
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
    #[cfg(feature = "multithreading")]
    safepoint_guard: SafepointWriteGuard,
}

impl<'a, T: ?Sized> ThreadSafeWriteGuard<'a, T> {
    pub fn map<U: ?Sized, F>(this: Self, f: F) -> ThreadSafeWriteGuard<'a, U>
    where
        F: FnOnce(&mut T) -> &mut U,
    {
        let ThreadSafeWriteGuard {
            guard,
            #[cfg(feature = "multithreading")]
            safepoint_guard,
        } = this;
        ThreadSafeWriteGuard {
            #[cfg(feature = "multithreading")]
            guard: MappedRwLockWriteGuard::map(guard, f),
            #[cfg(not(feature = "multithreading"))]
            guard: std::cell::RefMut::map(guard, f),
            #[cfg(feature = "multithreading")]
            safepoint_guard,
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

#[cfg(test)]
mod tests {
    use super::*;
    use static_assertions::assert_impl_all;
    use static_assertions::assert_not_impl_all;
    use std::{cell::Cell, rc::Rc};

    #[cfg(feature = "multithreading")]
    assert_impl_all!(ThreadSafeLock<i32>: Send, Sync);
    #[cfg(feature = "multithreading")]
    assert_impl_all!(ThreadSafeLock<Cell<u8>>: Send);
    #[cfg(feature = "multithreading")]
    assert_not_impl_all!(ThreadSafeLock<Cell<u8>>: Sync);
    #[cfg(feature = "multithreading")]
    assert_not_impl_all!(ThreadSafeLock<Rc<u8>>: Send);
    #[cfg(feature = "multithreading")]
    assert_not_impl_all!(ThreadSafeLock<Rc<u8>>: Sync);
    #[cfg(feature = "multithreading")]
    assert_not_impl_all!(ThreadSafeWriteGuard<'static, i32>: Send);

    #[cfg(not(feature = "multithreading"))]
    assert_impl_all!(ThreadSafeLock<i32>: Send);
    #[cfg(not(feature = "multithreading"))]
    assert_not_impl_all!(ThreadSafeLock<i32>: Sync);
    #[cfg(not(feature = "multithreading"))]
    assert_impl_all!(ThreadSafeLock<Cell<u8>>: Send);
    #[cfg(not(feature = "multithreading"))]
    assert_not_impl_all!(ThreadSafeLock<Cell<u8>>: Sync);
    #[cfg(not(feature = "multithreading"))]
    assert_not_impl_all!(ThreadSafeLock<Rc<u8>>: Send);
    #[cfg(not(feature = "multithreading"))]
    assert_not_impl_all!(ThreadSafeLock<Rc<u8>>: Sync);

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

        type TestArena = Arena<Rootable![Gc<'_, ThreadSafeLock<i32>>]>;

        let mut arena = TestArena::new(|mc| Gc::new(mc, ThreadSafeLock::new(42)));
        arena.mutate_root(|mc, lock| {
            {
                let mut guard = ThreadSafeLock::borrow_mut(*lock, mc);
                *guard = 100;
            }
            let guard = lock.borrow();
            assert_eq!(*guard, 100);
        });
    }

    #[test]
    fn test_try_borrow() {
        use gc_arena::{Arena, Rootable};

        type TestArena = Arena<Rootable![Gc<'_, ThreadSafeLock<i32>>]>;

        let mut arena = TestArena::new(|mc| Gc::new(mc, ThreadSafeLock::new(42)));
        arena.mutate_root(|mc, lock| {
            let _writer = ThreadSafeLock::borrow_mut(*lock, mc);
            // Should fail to get reader while writer is active
            assert!(lock.try_borrow().is_none());
        });
    }

    #[cfg(feature = "multithreading")]
    #[test]
    fn write_guards_track_safepoint_exclusion_across_mapping() {
        use gc_arena::{Arena, Rootable};

        type TestArena = Arena<Rootable![Gc<'_, ThreadSafeLock<(i32, i32)>>]>;

        assert!(!has_active_thread_safe_write_guard());
        let mut arena = TestArena::new(|mc| Gc::new(mc, ThreadSafeLock::new((1, 2))));
        arena.mutate_root(|mc, lock| {
            let guard = ThreadSafeLock::borrow_mut(*lock, mc);
            assert!(has_active_thread_safe_write_guard());

            let mut mapped = ThreadSafeWriteGuard::map(guard, |pair| &mut pair.1);
            assert!(has_active_thread_safe_write_guard());
            *mapped = 3;
            drop(mapped);

            assert!(!has_active_thread_safe_write_guard());
            assert_eq!(*lock.borrow(), (1, 3));
        });
    }

    #[test]
    fn borrow_mut_barriers_new_gc_children_of_marked_parent() {
        use gc_arena::{Arena, Gc, Rootable, static_collect};
        use std::rc::Rc;

        struct Flag(Rc<Cell<bool>>);
        impl Drop for Flag {
            fn drop(&mut self) {
                self.0.set(true);
            }
        }
        static_collect!(Flag);

        type Parent<'gc> = Gc<'gc, ThreadSafeLock<Option<Gc<'gc, Flag>>>>;
        type TestArena = Arena<Rootable![Parent<'_>]>;

        let dropped = Rc::new(Cell::new(false));
        let mut arena = TestArena::new(|mc| Gc::new(mc, ThreadSafeLock::new(None)));
        // Blacken the parent, as incremental marking between two mutator slices would.
        assert!(arena.finish_marking().is_some(), "marking completes");
        arena.mutate(|mc, parent| {
            let child = Gc::new(mc, Flag(dropped.clone()));
            *ThreadSafeLock::borrow_mut(*parent, mc) = Some(child);
        });
        arena.finish_cycle();
        assert!(
            !dropped.get(),
            "child stored through borrow_mut was swept: missing write barrier"
        );
    }
}

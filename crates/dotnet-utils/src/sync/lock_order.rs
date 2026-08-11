//! Lock-order marker types and typed lock wrappers for project-owned locks.
//!
//! These wrappers are additive over `dotnet_utils::sync::{Mutex, RwLock}` and are
//! intended for runtime-owned locks where we want explicit lock-level metadata.
//! The `define_lock_order_dag!` invocation below is authoritative for levels,
//! root acquisitions, and permitted nested-acquisition edges.
use std::{
    marker::PhantomData,
    ops::{Deref, DerefMut},
};

/// Marker trait for a level in the authoritative runtime lock-order DAG.
pub trait LockLevel {
    const NAME: &'static str;
}

/// Trait describing that a lock level can be acquired while `Prev` is held.
pub trait AcquireAfter<Prev: LockLevel>: LockLevel {}

macro_rules! define_lock_order_dag {
    (
        levels {
            $(
                $level:ident => $label:literal
            ),+ $(,)?
        }
        roots {
            $($root:ident),* $(,)?
        }
        edges {
            $(
                $next:ident after $prev:ident
            ),* $(,)?
        }
    ) => {
        /// Canonical lock levels for project-owned runtime locks.
        pub mod levels {
            use super::LockLevel;

            $(
                #[derive(Debug, Clone, Copy)]
                pub enum $level {}

                impl LockLevel for $level {
                    const NAME: &'static str = $label;
                }
            )+
        }

        // Keep root acquisition explicit: omission from `roots` is an enforced constraint.
        $(
            impl AcquireAfter<levels::Unlocked> for levels::$root {}
        )*

        $(
            impl AcquireAfter<levels::$prev> for levels::$next {}
        )*
    };
}

define_lock_order_dag! {
    levels {
        Unlocked => "Unlocked",
        CollectionLock => "GCCoordinator::collection_lock",
        GcCoordination => "ThreadManager::gc_coordination",
        ThreadRegistry => "ThreadManager::threads",
        CoordinatorArenas => "GCCoordinator::arenas",
        ArenaCurrentCommand => "ArenaHandle::current_command",
        CrossArenaRefs => "GCCoordinator::cross_arena_refs",
        StaticInitMutex => "StaticStorage::init_mutex",
        StaticWaitGraph => "StaticStorageManager::wait_graph",
        SyncBlocks => "SyncBlockManager::blocks",
        SyncNextIndex => "SyncBlockManager::next_index",
        SyncState => "SyncBlock::state"
    }
    // Only these levels implement `AcquireAfter<Unlocked>`, enabling the unqualified
    // `lock`/`read`/`write` entry points. `Unlocked` is a sentinel; `SyncNextIndex` is
    // deliberately omitted and must be acquired after `SyncBlocks`.
    roots {
        CollectionLock,
        GcCoordination,
        ThreadRegistry,
        CoordinatorArenas,
        ArenaCurrentCommand,
        CrossArenaRefs,
        StaticInitMutex,
        StaticWaitGraph,
        SyncBlocks,
        SyncState,
    }
    edges {
        GcCoordination after CollectionLock,
        ThreadRegistry after GcCoordination,
        ThreadRegistry after CollectionLock,
        CoordinatorArenas after CollectionLock,
        ArenaCurrentCommand after CoordinatorArenas,
        CrossArenaRefs after CollectionLock,
        StaticWaitGraph after StaticInitMutex,
        SyncNextIndex after SyncBlocks
        // SyncState intentionally has no in-edges. Audit note:
        // - SyncBlock::state is acquired only inside `sync/threaded.rs` SyncBlock methods.
        // - SyncBlockManager acquires `SyncBlocks`/`SyncNextIndex` only while allocating/looking up
        //   Arc<SyncBlock>, and those guards are dropped before any SyncBlock method can run.
        // Therefore `SyncState` is explicitly root-acquirable and intentionally has no in-edge.
    }
}

/// Token proving a given lock level is currently held.
#[derive(Debug, Clone, Copy)]
pub struct HeldLockLevel<L: LockLevel> {
    _marker: PhantomData<fn() -> L>,
}

impl<L: LockLevel> HeldLockLevel<L> {
    const fn new() -> Self {
        Self {
            _marker: PhantomData,
        }
    }
}

/// Root lock-level token (no lock held).
pub const fn unlocked() -> HeldLockLevel<levels::Unlocked> {
    HeldLockLevel::new()
}

/// Mutex wrapper carrying lock-level metadata.
#[derive(Debug)]
pub struct OrderedMutex<L: LockLevel, T> {
    inner: super::Mutex<T>,
    _level: PhantomData<fn() -> L>,
}

impl<L: LockLevel, T> OrderedMutex<L, T> {
    pub fn new(value: T) -> Self {
        Self {
            inner: super::Mutex::new(value),
            _level: PhantomData,
        }
    }

    pub fn lock(&self) -> OrderedMutexGuard<'_, L, T>
    where
        L: AcquireAfter<levels::Unlocked>,
    {
        self.lock_after(unlocked())
    }

    pub fn lock_after<Prev: LockLevel>(
        &self,
        _held: HeldLockLevel<Prev>,
    ) -> OrderedMutexGuard<'_, L, T>
    where
        L: AcquireAfter<Prev>,
    {
        OrderedMutexGuard {
            guard: self.inner.lock(),
            _level: PhantomData,
        }
    }

    pub fn try_lock(&self) -> Option<OrderedMutexGuard<'_, L, T>>
    where
        L: AcquireAfter<levels::Unlocked>,
    {
        self.try_lock_after(unlocked())
    }

    pub fn try_lock_after<Prev: LockLevel>(
        &self,
        _held: HeldLockLevel<Prev>,
    ) -> Option<OrderedMutexGuard<'_, L, T>>
    where
        L: AcquireAfter<Prev>,
    {
        self.inner.try_lock().map(|guard| OrderedMutexGuard {
            guard,
            _level: PhantomData,
        })
    }

    pub fn get_mut(&mut self) -> &mut T {
        self.inner.get_mut()
    }
}

impl<L: LockLevel, T: Default> Default for OrderedMutex<L, T> {
    fn default() -> Self {
        Self::new(T::default())
    }
}

/// Guard for an [`OrderedMutex`].
pub struct OrderedMutexGuard<'a, L: LockLevel, T> {
    guard: super::MutexGuard<'a, T>,
    _level: PhantomData<fn() -> L>,
}

impl<'a, L: LockLevel, T> OrderedMutexGuard<'a, L, T> {
    pub fn held_level(&self) -> HeldLockLevel<L> {
        HeldLockLevel::new()
    }

    pub fn raw(&self) -> &super::MutexGuard<'a, T> {
        &self.guard
    }

    pub fn raw_mut(&mut self) -> &mut super::MutexGuard<'a, T> {
        &mut self.guard
    }
}

impl<L: LockLevel, T> Deref for OrderedMutexGuard<'_, L, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.guard
    }
}

impl<L: LockLevel, T> DerefMut for OrderedMutexGuard<'_, L, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.guard
    }
}

/// RwLock wrapper carrying lock-level metadata.
#[derive(Debug)]
pub struct OrderedRwLock<L: LockLevel, T> {
    inner: super::RwLock<T>,
    _level: PhantomData<fn() -> L>,
}

impl<L: LockLevel, T> OrderedRwLock<L, T> {
    pub fn new(value: T) -> Self {
        Self {
            inner: super::RwLock::new(value),
            _level: PhantomData,
        }
    }

    pub fn read(&self) -> OrderedRwLockReadGuard<'_, L, T>
    where
        L: AcquireAfter<levels::Unlocked>,
    {
        self.read_after(unlocked())
    }

    pub fn read_after<Prev: LockLevel>(
        &self,
        _held: HeldLockLevel<Prev>,
    ) -> OrderedRwLockReadGuard<'_, L, T>
    where
        L: AcquireAfter<Prev>,
    {
        OrderedRwLockReadGuard {
            guard: self.inner.read(),
            _level: PhantomData,
        }
    }

    pub fn try_read(&self) -> Option<OrderedRwLockReadGuard<'_, L, T>>
    where
        L: AcquireAfter<levels::Unlocked>,
    {
        self.try_read_after(unlocked())
    }

    pub fn try_read_after<Prev: LockLevel>(
        &self,
        _held: HeldLockLevel<Prev>,
    ) -> Option<OrderedRwLockReadGuard<'_, L, T>>
    where
        L: AcquireAfter<Prev>,
    {
        self.inner.try_read().map(|guard| OrderedRwLockReadGuard {
            guard,
            _level: PhantomData,
        })
    }

    pub fn write(&self) -> OrderedRwLockWriteGuard<'_, L, T>
    where
        L: AcquireAfter<levels::Unlocked>,
    {
        self.write_after(unlocked())
    }

    pub fn write_after<Prev: LockLevel>(
        &self,
        _held: HeldLockLevel<Prev>,
    ) -> OrderedRwLockWriteGuard<'_, L, T>
    where
        L: AcquireAfter<Prev>,
    {
        OrderedRwLockWriteGuard {
            guard: self.inner.write(),
            _level: PhantomData,
        }
    }

    pub fn try_write(&self) -> Option<OrderedRwLockWriteGuard<'_, L, T>>
    where
        L: AcquireAfter<levels::Unlocked>,
    {
        self.try_write_after(unlocked())
    }

    pub fn try_write_after<Prev: LockLevel>(
        &self,
        _held: HeldLockLevel<Prev>,
    ) -> Option<OrderedRwLockWriteGuard<'_, L, T>>
    where
        L: AcquireAfter<Prev>,
    {
        self.inner.try_write().map(|guard| OrderedRwLockWriteGuard {
            guard,
            _level: PhantomData,
        })
    }

    pub fn get_mut(&mut self) -> &mut T {
        self.inner.get_mut()
    }

    /// Get a pointer to the inner data.
    ///
    /// # Safety
    ///
    /// Caller must guarantee that the returned pointer is not used in a way
    /// that violates the lock's aliasing requirements.
    pub unsafe fn data_ptr(&self) -> *mut T {
        #[cfg(feature = "multithreading")]
        {
            self.inner.data_ptr()
        }
        #[cfg(not(feature = "multithreading"))]
        {
            // SAFETY: F8.LockOrderRespected — Forwarding contract to underlying lock implementation.
            unsafe { self.inner.data_ptr() }
        }
    }
}

impl<L: LockLevel, T: Default> Default for OrderedRwLock<L, T> {
    fn default() -> Self {
        Self::new(T::default())
    }
}

/// Read guard for an [`OrderedRwLock`].
pub struct OrderedRwLockReadGuard<'a, L: LockLevel, T> {
    guard: super::RwLockReadGuard<'a, T>,
    _level: PhantomData<fn() -> L>,
}

impl<'a, L: LockLevel, T> OrderedRwLockReadGuard<'a, L, T> {
    pub fn held_level(&self) -> HeldLockLevel<L> {
        HeldLockLevel::new()
    }

    pub fn raw(&self) -> &super::RwLockReadGuard<'a, T> {
        &self.guard
    }
}

impl<L: LockLevel, T> Deref for OrderedRwLockReadGuard<'_, L, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.guard
    }
}

/// Write guard for an [`OrderedRwLock`].
pub struct OrderedRwLockWriteGuard<'a, L: LockLevel, T> {
    guard: super::RwLockWriteGuard<'a, T>,
    _level: PhantomData<fn() -> L>,
}

impl<'a, L: LockLevel, T> OrderedRwLockWriteGuard<'a, L, T> {
    pub fn held_level(&self) -> HeldLockLevel<L> {
        HeldLockLevel::new()
    }

    pub fn raw(&self) -> &super::RwLockWriteGuard<'a, T> {
        &self.guard
    }

    pub fn raw_mut(&mut self) -> &mut super::RwLockWriteGuard<'a, T> {
        &mut self.guard
    }
}

impl<L: LockLevel, T> Deref for OrderedRwLockWriteGuard<'_, L, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.guard
    }
}

impl<L: LockLevel, T> DerefMut for OrderedRwLockWriteGuard<'_, L, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.guard
    }
}

#[cfg(test)]
mod tests {
    use super::{AcquireAfter, LockLevel, OrderedMutex, OrderedRwLock, levels};
    use static_assertions::assert_not_impl_all;

    // Guard the trait-level forbidden inversions documented in
    // `docs/THREADING_AND_SYNCHRONIZATION.md`.
    assert_not_impl_all!(levels::CollectionLock: AcquireAfter<levels::GcCoordination>);
    assert_not_impl_all!(levels::GcCoordination: AcquireAfter<levels::ThreadRegistry>);
    assert_not_impl_all!(levels::CollectionLock: AcquireAfter<levels::CoordinatorArenas>);
    assert_not_impl_all!(levels::CollectionLock: AcquireAfter<levels::ArenaCurrentCommand>);
    assert_not_impl_all!(levels::CoordinatorArenas: AcquireAfter<levels::ArenaCurrentCommand>);
    assert_not_impl_all!(levels::StaticInitMutex: AcquireAfter<levels::StaticWaitGraph>);
    assert_not_impl_all!(levels::SyncBlocks: AcquireAfter<levels::SyncNextIndex>);
    // `SyncNextIndex` is the sole concrete lock level intentionally excluded from `roots`.
    assert_not_impl_all!(levels::SyncNextIndex: AcquireAfter<levels::Unlocked>);

    fn assert_can_acquire_after<L, P>()
    where
        L: super::AcquireAfter<P>,
        L: super::LockLevel,
        P: super::LockLevel,
    {
    }

    #[test]
    fn level_names_match_canonical_lock_order_table() {
        assert_eq!(
            [
                levels::Unlocked::NAME,
                levels::CollectionLock::NAME,
                levels::GcCoordination::NAME,
                levels::ThreadRegistry::NAME,
                levels::CoordinatorArenas::NAME,
                levels::ArenaCurrentCommand::NAME,
                levels::CrossArenaRefs::NAME,
                levels::StaticInitMutex::NAME,
                levels::StaticWaitGraph::NAME,
                levels::SyncBlocks::NAME,
                levels::SyncNextIndex::NAME,
                levels::SyncState::NAME,
            ],
            [
                "Unlocked",
                "GCCoordinator::collection_lock",
                "ThreadManager::gc_coordination",
                "ThreadManager::threads",
                "GCCoordinator::arenas",
                "ArenaHandle::current_command",
                "GCCoordinator::cross_arena_refs",
                "StaticStorage::init_mutex",
                "StaticStorageManager::wait_graph",
                "SyncBlockManager::blocks",
                "SyncBlockManager::next_index",
                "SyncBlock::state",
            ],
        );
    }

    #[test]
    fn lock_level_edges_cover_runtime_chains() {
        assert_can_acquire_after::<levels::GcCoordination, levels::CollectionLock>();
        assert_can_acquire_after::<levels::ThreadRegistry, levels::GcCoordination>();
        assert_can_acquire_after::<levels::ThreadRegistry, levels::CollectionLock>();
        assert_can_acquire_after::<levels::CoordinatorArenas, levels::CollectionLock>();
        assert_can_acquire_after::<levels::ArenaCurrentCommand, levels::CoordinatorArenas>();
        assert_can_acquire_after::<levels::CrossArenaRefs, levels::CollectionLock>();
        assert_can_acquire_after::<levels::StaticWaitGraph, levels::StaticInitMutex>();
        assert_can_acquire_after::<levels::SyncNextIndex, levels::SyncBlocks>();
    }

    #[test]
    fn ordered_mutex_supports_nested_acquisition_tokens() {
        let collection = OrderedMutex::<levels::CollectionLock, usize>::new(1);
        let gc_coordination = OrderedMutex::<levels::GcCoordination, usize>::new(2);

        let mut collection_guard = collection.lock();
        *collection_guard += 1;

        let mut gc_guard = gc_coordination.lock_after(collection_guard.held_level());
        *gc_guard += 1;

        assert_eq!(*collection_guard, 2);
        assert_eq!(*gc_guard, 3);
    }

    #[test]
    fn ordered_mutex_exposes_raw_guard_for_condvar_interop() {
        let lock = OrderedMutex::<levels::SyncState, usize>::new(7);
        let mut guard = lock.lock();

        fn takes_raw_guard(_guard: &mut super::super::MutexGuard<'_, usize>) {}

        takes_raw_guard(guard.raw_mut());
    }

    #[test]
    fn ordered_rwlock_supports_read_and_write() {
        let lock = OrderedRwLock::<levels::StaticInitMutex, usize>::new(10);

        let read = lock.read();
        assert_eq!(*read, 10);
        drop(read);

        let mut write = lock.write();
        *write += 5;
        drop(write);

        assert_eq!(*lock.read(), 15);
    }
}

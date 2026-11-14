//! GC-traced, arena-local bookkeeping for managed `System.Threading.Thread` objects.
//!
//! This deliberately lives apart from [`super::ThreadManager`]: the manager is shared between
//! arenas and must never retain arena-branded GC references. These records belong to the parent
//! executor arenas and keep thread objects, delegates and start arguments alive while a worker
//! may use them. A completion flag removes these roots during the next trace or record edit.
//! Unstarted thread records remain rooted until arena teardown.

use dotnet_utils::sync::AtomicI32;
use dotnet_value::object::ObjectRef;
use gc_arena::{Collect, collect::Trace};
use std::{
    cell::{Cell, RefCell, RefMut},
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

#[cfg(not(loom))]
static NEXT_MANAGED_ID: AtomicI32 = AtomicI32::new(1);

#[cfg(loom)]
loom::lazy_static! {
    static ref NEXT_MANAGED_ID: AtomicI32 = AtomicI32::new(1);
}

pub(crate) fn next_managed_id() -> i32 {
    allocate_managed_id(&NEXT_MANAGED_ID)
}

fn allocate_managed_id(counter: &AtomicI32) -> i32 {
    counter
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |id| id.checked_add(1))
        .expect("managed thread ID exhaustion")
}

#[cfg(feature = "multithreading")]
pub(crate) struct WorkerStart {
    pub(crate) thread: dotnet_value::object::ObjectPtr,
    pub(crate) delegate: dotnet_value::object::ObjectPtr,
    pub(crate) argument: Option<dotnet_value::object::ObjectPtr>,
    pub(crate) id: i32,
    pub(crate) retained: Arc<AtomicBool>,
}

/// Per-managed-Thread roots retained by a creating or starting executor arena.
pub(crate) struct ManagedThreadRecord<'gc> {
    /// The managed delegate supplied to `Thread(ThreadStart)`.
    pub(crate) thread_start: ObjectRef<'gc>,
    pub(crate) start_argument: ObjectRef<'gc>,
    pub(crate) retained: Arc<AtomicBool>,
}

impl<'gc> ManagedThreadRecord<'gc> {
    pub(crate) fn new(thread_start: ObjectRef<'gc>, retained: Arc<AtomicBool>) -> Self {
        Self {
            thread_start,
            start_argument: ObjectRef(None),
            retained,
        }
    }
}

/// GC-bearing managed-Thread records owned by one executor arena.
///
/// The keys are allocated managed `System.Threading.Thread` objects. The values retain their
/// associated delegates and start arguments while the worker is running.
pub(crate) struct ManagedThreadLocalState<'gc> {
    records: RefCell<HashMap<ObjectRef<'gc>, ManagedThreadRecord<'gc>>>,
    current: RefCell<Option<ObjectRef<'gc>>>,
    current_id: Cell<i32>,
    #[cfg(not(feature = "multithreading"))]
    background: Cell<bool>,
}

impl<'gc> ManagedThreadLocalState<'gc> {
    pub(crate) fn new() -> Self {
        Self {
            records: RefCell::new(HashMap::new()),
            current: RefCell::new(None),
            current_id: Cell::new(next_managed_id()),
            #[cfg(not(feature = "multithreading"))]
            background: Cell::new(false),
        }
    }

    pub(crate) fn current(&self) -> Option<ObjectRef<'gc>> {
        *self.current.borrow()
    }

    pub(crate) fn set_current(&self, thread: ObjectRef<'gc>) {
        *self.current.borrow_mut() = Some(thread);
    }

    #[cfg(feature = "multithreading")]
    pub(crate) fn clear_current(&self) {
        *self.current.borrow_mut() = None;
    }

    pub(crate) fn current_id(&self) -> i32 {
        self.current_id.get()
    }

    #[cfg(feature = "multithreading")]
    pub(crate) fn set_current_id(&self, id: i32) {
        self.current_id.set(id);
    }

    #[cfg(not(feature = "multithreading"))]
    pub(crate) fn background(&self) -> bool {
        self.background.get()
    }

    #[cfg(not(feature = "multithreading"))]
    pub(crate) fn set_background(&self, value: bool) {
        self.background.set(value);
    }

    pub(crate) fn records_write(
        &self,
    ) -> RefMut<'_, HashMap<ObjectRef<'gc>, ManagedThreadRecord<'gc>>> {
        let mut records = self.records.borrow_mut();
        records.retain(|_, record| record.retained.load(Ordering::Acquire));
        records
    }
}

impl<'gc> Default for ManagedThreadLocalState<'gc> {
    fn default() -> Self {
        Self::new()
    }
}

// SAFETY: F5.TracesEveryGcRef — the current Thread and every retained record's Thread key,
// delegate and start argument are traced. Completed records are removed before tracing so no
// untraced `Gc` remains in this root. IDs and retention flags contain no GC-managed references.
unsafe impl<'gc> Collect<'gc> for ManagedThreadLocalState<'gc> {
    fn trace<Tr: Trace<'gc>>(&self, cc: &mut Tr) {
        self.current.borrow().trace(cc);
        let mut records = self.records.borrow_mut();
        records.retain(|_, record| record.retained.load(Ordering::Acquire));
        for (thread, record) in records.iter() {
            thread.trace(cc);
            record.thread_start.trace(cc);
            record.start_argument.trace(cc);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(not(loom))]
    #[test]
    fn managed_ids_are_monotonic_and_do_not_wrap() {
        let counter = AtomicI32::new(i32::MAX - 1);
        assert_eq!(allocate_managed_id(&counter), i32::MAX - 1);
        assert!(std::panic::catch_unwind(|| allocate_managed_id(&counter)).is_err());
        assert_eq!(counter.load(Ordering::Relaxed), i32::MAX);
    }

    #[cfg(loom)]
    #[test]
    fn managed_ids_are_unique_under_loom() {
        loom::model(|| {
            let counter = loom::sync::Arc::new(AtomicI32::new(1));
            let other = counter.clone();
            let worker = loom::thread::spawn(move || allocate_managed_id(&other));
            let current = allocate_managed_id(&counter);
            let other = worker.join().unwrap();
            assert_eq!(current.min(other), 1);
            assert_eq!(current.max(other), 2);
        });
    }

    #[test]
    fn records_are_parent_local_and_start_unstarted() {
        let state = ManagedThreadLocalState::new();
        let thread = ObjectRef(None);
        let thread_start = ObjectRef(None);

        state.records_write().insert(
            thread,
            ManagedThreadRecord::new(thread_start, Arc::new(AtomicBool::new(true))),
        );

        let records = state.records.borrow();
        let record = records.get(&thread).expect("registered Thread record");
        assert_eq!(record.thread_start, thread_start);
    }
}

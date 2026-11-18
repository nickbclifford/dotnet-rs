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
    #[cfg(not(feature = "multithreading"))]
    finalizer_active: Cell<bool>,
    #[cfg(not(feature = "multithreading"))]
    finalizer_current: RefCell<Option<ObjectRef<'gc>>>,
    #[cfg(not(feature = "multithreading"))]
    finalizer_id: Cell<i32>,
    #[cfg(not(feature = "multithreading"))]
    finalizer_background: Cell<bool>,
}

impl<'gc> ManagedThreadLocalState<'gc> {
    pub(crate) fn new() -> Self {
        Self {
            records: RefCell::new(HashMap::new()),
            current: RefCell::new(None),
            current_id: Cell::new(next_managed_id()),
            #[cfg(not(feature = "multithreading"))]
            background: Cell::new(false),
            #[cfg(not(feature = "multithreading"))]
            finalizer_active: Cell::new(false),
            #[cfg(not(feature = "multithreading"))]
            finalizer_current: RefCell::new(None),
            #[cfg(not(feature = "multithreading"))]
            finalizer_id: Cell::new(next_managed_id()),
            #[cfg(not(feature = "multithreading"))]
            finalizer_background: Cell::new(true),
        }
    }

    pub(crate) fn current(&self) -> Option<ObjectRef<'gc>> {
        #[cfg(not(feature = "multithreading"))]
        if self.finalizer_active.get() {
            return *self.finalizer_current.borrow();
        }
        *self.current.borrow()
    }

    pub(crate) fn set_current(&self, thread: ObjectRef<'gc>) {
        #[cfg(not(feature = "multithreading"))]
        if self.finalizer_active.get() {
            *self.finalizer_current.borrow_mut() = Some(thread);
            return;
        }
        *self.current.borrow_mut() = Some(thread);
    }

    #[cfg(feature = "multithreading")]
    pub(crate) fn clear_current(&self) {
        *self.current.borrow_mut() = None;
    }

    pub(crate) fn current_id(&self) -> i32 {
        #[cfg(not(feature = "multithreading"))]
        if self.finalizer_active.get() {
            return self.finalizer_id.get();
        }
        self.current_id.get()
    }

    #[cfg(feature = "multithreading")]
    pub(crate) fn set_current_id(&self, id: i32) {
        self.current_id.set(id);
    }

    #[cfg(not(feature = "multithreading"))]
    pub(crate) fn set_finalizer_active(&self, active: bool) {
        self.finalizer_active.set(active);
    }

    #[cfg(not(feature = "multithreading"))]
    pub(crate) fn id_for(&self, thread: ObjectRef<'gc>) -> Option<i32> {
        if *self.current.borrow() == Some(thread) {
            Some(self.current_id.get())
        } else if *self.finalizer_current.borrow() == Some(thread) {
            Some(self.finalizer_id.get())
        } else {
            None
        }
    }

    #[cfg(not(feature = "multithreading"))]
    pub(crate) fn background_for(
        &self,
        thread: ObjectRef<'gc>,
        value: Option<bool>,
    ) -> Option<bool> {
        let flag = if *self.current.borrow() == Some(thread) {
            &self.background
        } else if *self.finalizer_current.borrow() == Some(thread) {
            &self.finalizer_background
        } else {
            return None;
        };
        if let Some(value) = value {
            flag.set(value);
        }
        Some(flag.get())
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

// SAFETY: F5.TracesEveryGcRef — both segment Threads and every retained record's Thread key,
// delegate and start argument are traced. Completed records are removed before tracing so no
// untraced `Gc` remains in this root. IDs and retention flags contain no GC-managed references.
unsafe impl<'gc> Collect<'gc> for ManagedThreadLocalState<'gc> {
    fn trace<Tr: Trace<'gc>>(&self, cc: &mut Tr) {
        self.current.borrow().trace(cc);
        #[cfg(not(feature = "multithreading"))]
        self.finalizer_current.borrow().trace(cc);
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
    fn records_trace_parent_local_roots_and_prune_completed_workers() {
        use crate::{
            state::SharedGlobalState,
            sync::Arc as VmArc,
            test_utils::{new_test_arena, test_gc_handle},
        };
        use dotnet_assemblies::AssemblyLoader;
        use dotnet_value::{object::HeapStorage, string::CLRString};

        #[allow(
            clippy::arc_with_non_send_sync,
            reason = "the no-MT test loader stays inside its single-threaded arenas"
        )]
        let shared = VmArc::new(SharedGlobalState::new(VmArc::new(
            AssemblyLoader::new_bare("managed-thread-records".into()).unwrap(),
        )));
        let mut parent = new_test_arena(&shared);
        let mut other_parent = new_test_arena(&shared);
        let unstarted_retained = Arc::new(AtomicBool::new(true));
        let parent_retained = Arc::new(AtomicBool::new(true));
        let other_retained = Arc::new(AtomicBool::new(true));

        parent.mutate_root(|gc, engine| {
            // SAFETY: F1.GcHandleRooted — the engine is this arena's live root for the whole mutation.
            let handle = unsafe { test_gc_handle(gc, engine) };
            let unstarted_thread = ObjectRef::new(
                handle,
                HeapStorage::Str(CLRString::from("unstarted thread")),
            );
            let unstarted_delegate = ObjectRef::new(
                handle,
                HeapStorage::Str(CLRString::from("unstarted delegate")),
            );
            let thread = ObjectRef::new(handle, HeapStorage::Str(CLRString::from("parent thread")));
            let delegate =
                ObjectRef::new(handle, HeapStorage::Str(CLRString::from("parent delegate")));
            let argument =
                ObjectRef::new(handle, HeapStorage::Str(CLRString::from("parent argument")));
            let state = &engine.stack.local.managed_threads;
            state.records_write().insert(
                unstarted_thread,
                ManagedThreadRecord::new(unstarted_delegate, unstarted_retained.clone()),
            );
            let mut record = ManagedThreadRecord::new(delegate, parent_retained.clone());
            record.start_argument = argument;
            state.records_write().insert(thread, record);

            let records = state.records.borrow();
            assert_eq!(records.len(), 2);
            let unstarted = records
                .get(&unstarted_thread)
                .expect("unstarted Thread record");
            assert_ne!(unstarted.thread_start, ObjectRef(None));
            assert_eq!(unstarted.start_argument, ObjectRef(None));
            let record = records.get(&thread).expect("parent Thread record");
            assert_ne!(thread, ObjectRef(None));
            assert_ne!(record.thread_start, ObjectRef(None));
            assert_ne!(record.start_argument, ObjectRef(None));
            assert!(matches!(
                record.thread_start.0.unwrap().borrow().storage,
                HeapStorage::Str(ref text) if text.as_string() == "parent delegate"
            ));
            assert!(matches!(
                record.start_argument.0.unwrap().borrow().storage,
                HeapStorage::Str(ref text) if text.as_string() == "parent argument"
            ));
        });

        other_parent.mutate_root(|gc, engine| {
            // SAFETY: F1.GcHandleRooted — the engine is this arena's live root for the whole mutation.
            let handle = unsafe { test_gc_handle(gc, engine) };
            let thread = ObjectRef::new(handle, HeapStorage::Str(CLRString::from("other thread")));
            let delegate =
                ObjectRef::new(handle, HeapStorage::Str(CLRString::from("other delegate")));
            let argument =
                ObjectRef::new(handle, HeapStorage::Str(CLRString::from("other argument")));
            let state = &engine.stack.local.managed_threads;
            let mut record = ManagedThreadRecord::new(delegate, other_retained.clone());
            record.start_argument = argument;
            state.records_write().insert(thread, record);
            assert_eq!(state.records.borrow().len(), 1);

            // Completing one parent's worker prunes only that parent's record.
            parent_retained.store(false, Ordering::Release);
            assert!(state.records_write().contains_key(&thread));
            other_retained.store(false, Ordering::Release);
            assert!(state.records_write().is_empty());
        });

        parent.mutate_root(|_, engine| {
            let records = engine.stack.local.managed_threads.records_write();
            assert_eq!(
                records.len(),
                1,
                "the parent state prunes only completed workers"
            );
            let (thread, record) = records
                .iter()
                .next()
                .expect("unstarted Thread record remains");
            assert_ne!(*thread, ObjectRef(None));
            assert_ne!(record.thread_start, ObjectRef(None));
            assert_eq!(record.start_argument, ObjectRef(None));
            assert!(
                record.retained.load(Ordering::Acquire),
                "unstarted Thread roots remain until arena teardown"
            );
        });
    }
}

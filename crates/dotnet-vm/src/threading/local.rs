//! GC-traced, arena-local bookkeeping for managed `System.Threading.Thread` objects.
//!
//! This deliberately lives apart from [`super::ThreadManager`]: the manager is shared between
//! arenas and must never retain arena-branded GC references. These records belong to the parent
//! executor arena that created the managed `Thread` and keep its `ThreadStart` delegate alive
//! through `Start`/`Join`.

use crate::ExecutorResult;
use dotnet_value::object::ObjectRef;
use gc_arena::{Collect, collect::Trace};
use std::{
    cell::{Ref, RefCell, RefMut},
    collections::HashMap,
    thread::JoinHandle,
};

/// Per-managed-Thread state retained by the parent executor arena.
pub(crate) struct ManagedThreadRecord<'gc> {
    /// The managed delegate supplied to `Thread(ThreadStart)`.
    pub(crate) thread_start: ObjectRef<'gc>,
    /// The native worker created by `Start`, if it has been started, returning its VM outcome.
    ///
    /// A `JoinHandle` contains no GC references and is intentionally not traced.
    pub(crate) join_handle: Option<JoinHandle<ExecutorResult>>,
}

impl<'gc> ManagedThreadRecord<'gc> {
    pub(crate) fn new(thread_start: ObjectRef<'gc>) -> Self {
        Self {
            thread_start,
            join_handle: None,
        }
    }
}

/// GC-bearing managed-Thread records owned by one executor arena.
///
/// The keys are allocated managed `System.Threading.Thread` objects. The values retain their
/// associated `ThreadStart` delegates until the record is removed after `Join`.
pub(crate) struct ManagedThreadLocalState<'gc> {
    records: RefCell<HashMap<ObjectRef<'gc>, ManagedThreadRecord<'gc>>>,
}

impl<'gc> ManagedThreadLocalState<'gc> {
    pub(crate) fn new() -> Self {
        Self {
            records: RefCell::new(HashMap::new()),
        }
    }

    pub(crate) fn records_read(
        &self,
    ) -> Ref<'_, HashMap<ObjectRef<'gc>, ManagedThreadRecord<'gc>>> {
        self.records.borrow()
    }

    pub(crate) fn records_write(
        &self,
    ) -> RefMut<'_, HashMap<ObjectRef<'gc>, ManagedThreadRecord<'gc>>> {
        self.records.borrow_mut()
    }
}

impl<'gc> Default for ManagedThreadLocalState<'gc> {
    fn default() -> Self {
        Self::new()
    }
}

// SAFETY: F5.TracesEveryGcRef — each managed Thread key and its retained ThreadStart delegate
// is traced. `JoinHandle` contains no GC-managed references and is intentionally omitted.
unsafe impl<'gc> Collect<'gc> for ManagedThreadLocalState<'gc> {
    fn trace<Tr: Trace<'gc>>(&self, cc: &mut Tr) {
        for (thread, record) in self.records.borrow().iter() {
            thread.trace(cc);
            record.thread_start.trace(cc);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_are_parent_local_and_start_unstarted() {
        let state = ManagedThreadLocalState::new();
        let thread = ObjectRef(None);
        let thread_start = ObjectRef(None);

        state
            .records_write()
            .insert(thread, ManagedThreadRecord::new(thread_start));

        let records = state.records_read();
        let record = records.get(&thread).expect("registered Thread record");
        assert_eq!(record.thread_start, thread_start);
        assert!(record.join_handle.is_none());
    }
}

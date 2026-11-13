//! Cross-arena finalizer handoff. Entries remain rooted until execution finishes.
use dotnet_utils::sync::{Mutex, get_current_thread_id};
use dotnet_value::object::{ObjectPtr, ObjectRef};
use gc_arena::{Collect, Gc, collect::Trace};

#[derive(Default)]
pub(crate) struct Finalizers {
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    pending: Vec<ObjectPtr>,
    active: Option<ObjectPtr>,
    registrations: Vec<(ObjectPtr, bool)>,
}

impl Finalizers {
    pub(crate) fn enqueue(&self, object: ObjectRef<'_>) {
        self.state
            .lock()
            .pending
            .push(object.as_ptr().expect("finalizer object"));
    }

    pub(crate) fn is_pending(&self) -> bool {
        let state = self.state.lock();
        state.active.is_some() || !state.pending.is_empty()
    }

    pub(crate) fn claim(&self) -> Option<ObjectPtr> {
        let mut state = self.state.lock();
        debug_assert!(state.active.is_none());
        let object = state.pending.pop()?;
        state.active = Some(object);
        Some(object)
    }

    pub(crate) fn complete(&self) {
        self.state.lock().active = None;
    }

    pub(crate) fn set_registered(&self, object: ObjectRef<'_>, registered: bool) {
        let ptr = object.as_ptr().expect("finalizer object");
        let mut state = self.state.lock();
        state.registrations.push((ptr, registered));
    }

    /// Drain weak registrations on their owner's finalization pass, before sweep.
    pub(crate) fn take_registrations(&self) -> Vec<(ObjectPtr, bool)> {
        let owner = get_current_thread_id();
        let mut state = self.state.lock();
        let mut result = Vec::new();
        state.registrations.retain(|(ptr, registered)| {
            if ptr.owner_id() == owner {
                result.push((*ptr, *registered));
                false
            } else {
                true
            }
        });
        result
    }
}

// SAFETY: F1.FinalizerPublication — every owner traces its queued and active objects while mutators are parked.
// The collector's final root barrier sees publications made during incremental marking, as for
// the shared reflection registry. Registrations are weak and are consumed before owner sweep.
unsafe impl<'gc> Collect<'gc> for Finalizers {
    fn trace<Tr: Trace<'gc>>(&self, cc: &mut Tr) {
        let owner = get_current_thread_id();
        let state = self.state.lock();
        for ptr in state.pending.iter().chain(state.active.iter()) {
            if ptr.owner_id() == owner {
                // SAFETY: F1.FinalizerPublication — STW keeps the owner alive and the handle stays within tracing.
                ObjectRef(Some(unsafe { Gc::from_ptr(ptr.as_ptr().cast()) })).trace(cc);
            }
        }
    }
}

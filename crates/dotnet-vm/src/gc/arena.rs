use crate::stack::GCArena;
use dotnet_utils::gc::ArenaIdentity;
use std::{
    cell::{Cell, RefCell},
    marker::PhantomData,
    rc::Rc,
};

struct Slot {
    identity: Option<ArenaIdentity>,
    arena: Option<Box<GCArena>>,
}

impl Drop for Slot {
    fn drop(&mut self) {
        // INVARIANT: F1.ExecutorArenaRetained — TLS destruction is not evidence
        // of a completed drain (the Executor may have been forgotten).
        if let Some(arena) = self.arena.take() {
            std::mem::forget(arena);
        }
    }
}

thread_local! {
    static THREAD_ARENA: RefCell<Option<Slot>> = const { RefCell::new(None) };
}

/// Reserves this thread independently of the Box. `None` is an initializing
/// reservation, not a free slot; it is bound once to the exact registration.
/// Extraction never releases it.
pub(crate) struct ArenaOwner {
    identity: Cell<Option<ArenaIdentity>>,
    _affinity: PhantomData<Rc<()>>,
}

impl ArenaOwner {
    pub(crate) fn reserve() -> Self {
        THREAD_ARENA.with(|cell| {
            let mut slot = cell.borrow_mut();
            assert!(
                slot.is_none(),
                "Executor::new: this thread already owns a live Executor arena; drop it first"
            );
            *slot = Some(Slot {
                identity: None,
                arena: None,
            });
            Self {
                identity: Cell::new(None),
                _affinity: PhantomData,
            }
        })
    }

    pub(crate) fn install(&self, arena: Box<GCArena>) {
        THREAD_ARENA.with(|cell| {
            let mut slot = cell.borrow_mut();
            let slot = slot.as_mut().expect("reserved arena slot");
            assert_eq!(slot.identity, self.identity.get());
            assert!(slot.identity.is_none());
            assert!(slot.arena.is_none());
            slot.arena = Some(arena);
        });
    }

    pub(crate) fn bind_identity(&self, identity: ArenaIdentity) {
        THREAD_ARENA.with(|cell| {
            let mut slot = cell.borrow_mut();
            let slot = slot.as_mut().expect("reserved arena slot");
            assert_eq!(slot.identity, self.identity.get());
            assert!(slot.identity.is_none());
            slot.identity = Some(identity);
            self.identity.set(Some(identity));
        });
    }

    pub(crate) fn take(&self) -> Option<Box<GCArena>> {
        THREAD_ARENA.with(|cell| {
            cell.borrow_mut()
                .as_mut()
                .filter(|s| s.identity == self.identity.get())?
                .arena
                .take()
        })
    }

    pub(crate) fn with<R>(&self, f: impl FnOnce(Option<&mut GCArena>) -> R) -> R {
        THREAD_ARENA.with(|cell| {
            let mut slot = cell.borrow_mut();
            let arena = slot
                .as_mut()
                .filter(|s| s.identity == self.identity.get())
                .and_then(|s| s.arena.as_deref_mut());
            f(arena)
        })
    }
}

impl Drop for ArenaOwner {
    fn drop(&mut self) {
        THREAD_ARENA.with(|cell| {
            let mut slot = cell.borrow_mut();
            if slot
                .as_ref()
                .is_some_and(|s| s.identity == self.identity.get())
            {
                // INVARIANT: F1.ExecutorArenaRetained — a construction unwind must
                // not free storage whose drain has not completed.
                if let Some(arena) = slot.take().and_then(|mut s| s.arena.take()) {
                    std::mem::forget(arena);
                }
            }
        });
    }
}

/// Internal safepoint dispatch only; callers cannot replace or extract storage.
pub(crate) fn with_current<R>(f: impl FnOnce(Option<&mut GCArena>) -> R) -> R {
    THREAD_ARENA.with(|cell| {
        let mut slot = cell.borrow_mut();
        f(slot.as_mut().and_then(|s| s.arena.as_deref_mut()))
    })
}

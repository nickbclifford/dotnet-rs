//! VM-wide GC handle slots. Mutators access slots only while their arena is active;
//! collection accesses them after all mutators have parked.
use dotnet_utils::{
    gc::GCHandleType,
    sync::{Mutex, get_current_thread_id},
};
use dotnet_value::object::{ObjectPtr, ObjectRef};
use gc_arena::{Collect, Gc, collect::Trace};
use std::collections::HashSet;

type HandleSlot = Option<(Option<ObjectPtr>, GCHandleType)>;

#[derive(Default)]
pub struct SharedGcHandles {
    slots: Mutex<Vec<HandleSlot>>,
}

impl SharedGcHandles {
    pub fn len(&self) -> usize {
        self.slots.lock().len()
    }

    pub fn is_empty(&self) -> bool {
        self.slots.lock().is_empty()
    }

    pub fn alloc(&self, object: ObjectRef<'_>, kind: GCHandleType) -> usize {
        let mut slots = self.slots.lock();
        let entry = Some((object.as_ptr(), kind));
        if let Some(index) = slots.iter().position(Option::is_none) {
            slots[index] = entry;
            index
        } else {
            slots.push(entry);
            slots.len() - 1
        }
    }

    /// # Safety
    /// The caller must be in an active arena mutation, use that mutation's brand for `'gc`,
    /// and root the result before a safepoint. The result must not outlive that mutation.
    pub unsafe fn get<'gc>(&self, index: usize) -> Option<(ObjectRef<'gc>, GCHandleType)> {
        self.slots
            .lock()
            .get(index)
            .copied()
            .flatten()
            .map(|(ptr, kind)| {
                // SAFETY: The slot protects its pointer until this read; the caller keeps the
                // mutator active and roots the result before any safepoint.
                (
                    ObjectRef(ptr.map(|p| unsafe { Gc::from_ptr(p.as_ptr().cast()) })),
                    kind,
                )
            })
    }

    pub fn set(&self, index: usize, object: ObjectRef<'_>) {
        if let Some(Some((ptr, _))) = self.slots.lock().get_mut(index) {
            *ptr = object.as_ptr();
        }
    }

    pub fn free(&self, index: usize) {
        if let Some(slot) = self.slots.lock().get_mut(index) {
            *slot = None;
        }
    }

    /// # Safety
    /// The current thread must be collecting its own arena with `fc`, all other mutators
    /// must be parked, and every slot target must remain allocated until weak clearing ends.
    pub unsafe fn clear_dead<'gc>(
        &self,
        fc: &'gc gc_arena::Finalization<'gc>,
        kind: GCHandleType,
        resurrected: &HashSet<usize>,
    ) {
        let owner = get_current_thread_id();
        for (ptr, entry_kind) in self.slots.lock().iter_mut().flatten() {
            if *entry_kind == kind
                && let Some(p) = *ptr
                && p.owner_id() == owner
            {
                // SAFETY: STW retains the owner; only its collector reads its colors.
                let object: ObjectRef<'gc> =
                    ObjectRef(Some(unsafe { Gc::from_ptr(p.as_ptr().cast()) }));
                let gc = object.0.unwrap();
                if Gc::is_dead(fc, gc) && !resurrected.contains(&Gc::as_ptr(gc).addr()) {
                    *ptr = None;
                }
            }
        }
    }
}

// SAFETY: F5.TracesEveryGcRef — strong slots are traced by their owner. Like the
// shared reflection registry, MarkAll applies the root barrier after all mutators
// park, so publications before that point are seen even if marking had begun.
unsafe impl<'gc> Collect<'gc> for SharedGcHandles {
    fn trace<Tr: Trace<'gc>>(&self, cc: &mut Tr) {
        let owner = get_current_thread_id();
        for (ptr, kind) in self.slots.lock().iter().flatten() {
            if matches!(kind, GCHandleType::Normal | GCHandleType::Pinned)
                && let Some(p) = ptr
                && p.owner_id() == owner
            {
                // SAFETY: STW keeps owners alive and this temporary handle cannot escape.
                ObjectRef(Some(unsafe { Gc::from_ptr(p.as_ptr().cast()) })).trace(cc);
            }
        }
    }
}

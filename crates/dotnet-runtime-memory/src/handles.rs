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
    dependents: Mutex<Dependents>,
}

#[derive(Default)]
struct Dependents {
    slots: Vec<Option<(Option<ObjectPtr>, Option<ObjectPtr>)>>,
    active: HashSet<usize>,
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
                // SAFETY: F1.GcHandleRooted — the slot protects its pointer until this read; the caller keeps the
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

    pub fn alloc_dependent(&self, key: ObjectRef<'_>, value: ObjectRef<'_>) -> usize {
        let mut table = self.dependents.lock();
        let entry = Some((key.as_ptr(), value.as_ptr()));
        if let Some(index) = table.slots.iter().position(Option::is_none) {
            table.slots[index] = entry;
            index
        } else {
            table.slots.push(entry);
            table.slots.len() - 1
        }
    }

    /// # Safety
    /// The caller must be in an active arena mutation, use its brand for `'gc`, and
    /// root both results before a safepoint. Neither result may outlive that mutation.
    pub unsafe fn get_dependent<'gc>(
        &self,
        index: usize,
    ) -> Option<(ObjectRef<'gc>, ObjectRef<'gc>)> {
        self.dependents
            .lock()
            .slots
            .get(index)
            .copied()
            .flatten()
            .map(|(key, value)| {
                // The active mutation prevents collection until the caller roots the results.
                let convert = |ptr: Option<ObjectPtr>| {
                    ObjectRef(ptr.map(|p| {
                        // SAFETY: F1.GcHandleRooted — the active mutation keeps the pointer live.
                        unsafe { Gc::from_ptr(p.as_ptr().cast()) }
                    }))
                };
                (convert(key), convert(value))
            })
    }

    pub fn set_dependent(&self, index: usize, value: ObjectRef<'_>) {
        if let Some(Some((_, dependent))) = self.dependents.lock().slots.get_mut(index) {
            *dependent = value.as_ptr();
        }
    }

    pub fn clear_dependent(&self, index: usize) {
        let mut table = self.dependents.lock();
        if let Some(Some(entry)) = table.slots.get_mut(index) {
            *entry = (None, None);
        }
        table.active.remove(&index);
    }

    pub fn free_dependent(&self, index: usize) {
        let mut table = self.dependents.lock();
        if let Some(slot) = table.slots.get_mut(index) {
            *slot = None;
        }
        table.active.remove(&index);
    }

    pub fn reset_dependents(&self) {
        self.dependents.lock().active.clear();
    }

    /// # Safety
    /// The caller must collect its own arena; all other mutators must be parked.
    pub unsafe fn activate_dependents<'gc>(&self, fc: &'gc gc_arena::Finalization<'gc>) -> bool {
        let owner = get_current_thread_id();
        let mut table = self.dependents.lock();
        let mut changed = false;
        for index in 0..table.slots.len() {
            if let Some((Some(key), _)) = table.slots[index]
                && key.owner_id() == owner
            {
                // SAFETY: F1.StwParked; F6.NoEscapeAcrossArena — only the key's owner reads its mark color during collection.
                let object: ObjectRef<'gc> =
                    ObjectRef(Some(unsafe { Gc::from_ptr(key.as_ptr().cast()) }));
                let gc = object.0.unwrap();
                if !Gc::is_dead(fc, gc) {
                    changed |= table.active.insert(index);
                }
            }
        }
        changed
    }

    /// # Safety
    /// The caller must collect its own arena before any arena sweeps.
    pub unsafe fn clear_dead_dependents<'gc>(&self, fc: &'gc gc_arena::Finalization<'gc>) {
        let owner = get_current_thread_id();
        let mut table = self.dependents.lock();
        for entry in table.slots.iter_mut().flatten() {
            if entry.0.is_none() {
                entry.1 = None;
            } else if let (Some(key), _) = *entry
                && key.owner_id() == owner
            {
                // SAFETY: F1.StwParked; F6.NoEscapeAcrossArena — only the key's owner reads its mark color during collection.
                let object: ObjectRef<'gc> =
                    ObjectRef(Some(unsafe { Gc::from_ptr(key.as_ptr().cast()) }));
                let gc = object.0.unwrap();
                if Gc::is_dead(fc, gc) {
                    *entry = (None, None);
                }
            }
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
                // SAFETY: F1.StwParked; F6.NoEscapeAcrossArena — STW retains the owner; only its collector reads its colors.
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
// Dependent activations are followed by a root barrier on every value owner.
unsafe impl<'gc> Collect<'gc> for SharedGcHandles {
    fn trace<Tr: Trace<'gc>>(&self, cc: &mut Tr) {
        let owner = get_current_thread_id();
        let table = self.dependents.lock();
        for index in &table.active {
            if let Some(Some((_, Some(value)))) = table.slots.get(*index)
                && value.owner_id() == owner
            {
                // SAFETY: F1.StwParked; F6.NoEscapeAcrossArena — STW retains the value's owner throughout tracing.
                ObjectRef(Some(unsafe { Gc::from_ptr(value.as_ptr().cast()) })).trace(cc);
            }
        }
        drop(table);
        for (ptr, kind) in self.slots.lock().iter().flatten() {
            if matches!(kind, GCHandleType::Normal | GCHandleType::Pinned)
                && let Some(p) = ptr
                && p.owner_id() == owner
            {
                // SAFETY: F1.StwParked; F6.NoEscapeAcrossArena — STW keeps owners alive and this temporary handle cannot escape.
                ObjectRef(Some(unsafe { Gc::from_ptr(p.as_ptr().cast()) })).trace(cc);
            }
        }
    }
}

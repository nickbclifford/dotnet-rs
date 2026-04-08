use crate::host::MemorySharedStateHost;
use dotnet_utils::gc::GCHandleType;
use dotnet_value::object::{HeapStorage, ObjectRef};
use gc_arena::{Collect, Gc, collect::Trace};
use slab::Slab;
use smallvec::SmallVec;
use std::{
    cell::{Cell, RefCell},
    collections::{HashMap, HashSet},
};

#[cfg(feature = "heap-diagnostics")]
use std::collections::BTreeMap;

#[cfg(feature = "multithreading")]
use dotnet_value::object::ObjectPtr;

const SMALL_QUEUE_CAPACITY: usize = 8;
#[cfg(not(feature = "heap-diagnostics"))]
const REGISTRY_BUCKET_SHIFT: usize = 6;

type FinalizationQueue<'gc> = SmallVec<[ObjectRef<'gc>; SMALL_QUEUE_CAPACITY]>;
type PinnedObjects<'gc> = SmallVec<[ObjectRef<'gc>; SMALL_QUEUE_CAPACITY]>;

pub struct HeapManager<'gc> {
    pub finalization_queue: RefCell<FinalizationQueue<'gc>>,
    pub pending_finalization: RefCell<FinalizationQueue<'gc>>,
    pub pinned_objects: RefCell<PinnedObjects<'gc>>,
    pub gchandles: RefCell<Vec<Option<(ObjectRef<'gc>, GCHandleType)>>>,
    pub processing_finalizer: Cell<bool>,
    pub needs_full_collect: Cell<bool>,
    /// Roots for objects in this arena that are referenced by other arenas.
    /// This is populated during the coordinated GC marking phase.
    #[cfg(feature = "multithreading")]
    pub cross_arena_roots: RefCell<HashSet<ObjectPtr>>,
    /// Untraced handles to every heap object in this arena (diagnostics mode).
    /// Used by the tracer during debugging and for conservative stack scanning.
    #[cfg(feature = "heap-diagnostics")]
    pub _all_objs: RefCell<BTreeMap<usize, ObjectRef<'gc>>>,
    #[cfg(not(feature = "heap-diagnostics"))]
    object_registry: RefCell<ObjectRegistry<'gc>>,
}

impl<'gc> HeapManager<'gc> {
    pub fn new() -> Self {
        Self {
            finalization_queue: RefCell::new(SmallVec::new()),
            pending_finalization: RefCell::new(SmallVec::new()),
            pinned_objects: RefCell::new(SmallVec::new()),
            gchandles: RefCell::new(Vec::new()),
            processing_finalizer: Cell::new(false),
            needs_full_collect: Cell::new(false),
            #[cfg(feature = "multithreading")]
            cross_arena_roots: RefCell::new(HashSet::new()),
            #[cfg(feature = "heap-diagnostics")]
            _all_objs: RefCell::new(BTreeMap::new()),
            #[cfg(not(feature = "heap-diagnostics"))]
            object_registry: RefCell::new(ObjectRegistry::new()),
        }
    }

    #[inline]
    pub fn register_object(&self, instance: ObjectRef<'gc>) {
        let Some(ptr) = instance.0 else {
            return;
        };
        let addr = Gc::as_ptr(ptr) as usize;

        #[cfg(feature = "heap-diagnostics")]
        {
            self._all_objs.borrow_mut().insert(addr, instance);
        }
        #[cfg(not(feature = "heap-diagnostics"))]
        {
            if let Some(size) = object_size(instance) {
                self.object_registry
                    .borrow_mut()
                    .insert(addr, size, instance);
            }
        }
    }

    #[inline]
    pub fn pin_object(&self, object: ObjectRef<'gc>) {
        let mut pinned = self.pinned_objects.borrow_mut();
        if !pinned.contains(&object) {
            pinned.push(object);
        }
    }

    #[inline]
    pub fn unpin_object(&self, object: ObjectRef<'gc>) {
        let mut pinned = self.pinned_objects.borrow_mut();
        if let Some(i) = pinned.iter().position(|o| *o == object) {
            pinned.swap_remove(i);
        }
    }

    #[inline]
    pub fn live_object_count(&self) -> usize {
        #[cfg(feature = "heap-diagnostics")]
        {
            self._all_objs.borrow().len()
        }
        #[cfg(not(feature = "heap-diagnostics"))]
        {
            self.object_registry.borrow().len()
        }
    }

    #[inline]
    pub fn snapshot_objects(&self) -> Vec<ObjectRef<'gc>> {
        #[cfg(feature = "heap-diagnostics")]
        {
            self._all_objs.borrow().values().copied().collect()
        }
        #[cfg(not(feature = "heap-diagnostics"))]
        {
            Vec::new()
        }
    }

    pub fn find_object(&self, ptr: usize) -> Option<ObjectRef<'gc>> {
        #[cfg(feature = "heap-diagnostics")]
        {
            let all_objs = self._all_objs.borrow();
            let (&start, &obj) = all_objs.range(..=ptr).next_back()?;
            let size = object_size(obj)?;

            if ptr < start + size { Some(obj) } else { None }
        }
        #[cfg(not(feature = "heap-diagnostics"))]
        {
            self.object_registry.borrow().find(ptr)
        }
    }

    pub fn has_pending_finalizers(&self) -> bool {
        !self.pending_finalization.borrow().is_empty()
    }

    pub fn finalize_check(
        &self,
        fc: &'gc gc_arena::Finalization<'gc>,
        shared: &impl MemorySharedStateHost,
        indent: usize,
    ) {
        let mut queue = self.finalization_queue.borrow_mut();
        let mut handles = self.gchandles.borrow_mut();
        let mut resurrected = HashSet::new();

        let mut zero_out_handles = |for_type: GCHandleType, resurrected: &HashSet<usize>| {
            for (obj_ref, handle_type) in handles.iter_mut().flatten() {
                if *handle_type == for_type
                    && let ObjectRef(Some(ptr)) = obj_ref
                    && Gc::is_dead(fc, *ptr)
                {
                    if for_type == GCHandleType::WeakTrackResurrection
                        && resurrected.contains(&(Gc::as_ptr(*ptr) as usize))
                    {
                        continue;
                    }
                    *obj_ref = ObjectRef(None);
                }
            }
        };

        let mut to_finalize = Vec::new();
        if !queue.is_empty() {
            #[cfg(feature = "memory-validation")]
            {
                let mut seen = HashSet::new();
                for obj in queue.iter() {
                    let ptr = obj.0.expect("object in finalization queue is null");
                    let addr = Gc::as_ptr(ptr) as usize;
                    if !seen.insert(addr) {
                        panic!(
                            "Duplicate object in finalization queue at address {:#x}",
                            addr
                        );
                    }

                    let inner = ptr.borrow();
                    match &inner.storage {
                        HeapStorage::Obj(o) => {
                            let has_finalizer = o.description.static_initializer().is_some()
                                || o.description
                                    .definition()
                                    .methods
                                    .iter()
                                    .any(|m| m.name == "Finalize");
                            if !has_finalizer {
                                panic!(
                                    "Object without finalizer in finalization queue: {:?}",
                                    o.description
                                );
                            }
                        }
                        _ => {
                            panic!("Non-object in finalization queue");
                        }
                    }
                }
            }

            let mut i = 0;
            while i < queue.len() {
                let obj = queue[i];
                let ptr = obj.0.expect("object in finalization queue is null");

                let is_dead = Gc::is_dead(fc, ptr);

                let is_suppressed = match &ptr.borrow().storage {
                    HeapStorage::Obj(o) => o.finalizer_suppressed,
                    _ => false,
                };

                if is_suppressed {
                    queue.swap_remove(i);
                    continue;
                }

                if is_dead {
                    to_finalize.push(queue.swap_remove(i));
                } else {
                    i += 1;
                }
            }
        }

        // 1. Zero out Weak handles for dead objects.
        // This MUST be done before resurrection because "short" weak handles
        // are cleared as soon as the object is no longer strongly reachable.
        zero_out_handles(GCHandleType::Weak, &resurrected);

        if !to_finalize.is_empty() {
            let mut pending = self.pending_finalization.borrow_mut();
            for obj in to_finalize {
                let ptr = obj.0.unwrap();
                pending.push(obj);
                if resurrected.insert(Gc::as_ptr(ptr) as usize) {
                    // Trace resurrection event
                    if shared.tracer_enabled_relaxed() {
                        let obj_type_name = match &ptr.borrow().storage {
                            HeapStorage::Obj(o) => format!("{:?}", o.description),
                            HeapStorage::Vec(_) => "Vector".to_string(),
                            HeapStorage::Str(_) => "String".to_string(),
                            HeapStorage::Boxed(_) => "Boxed".to_string(),
                        };
                        let addr = Gc::as_ptr(ptr) as usize;
                        shared.trace_gc_resurrection(indent, &obj_type_name, addr);
                    }
                    Gc::resurrect(fc, ptr);
                    ptr.borrow().storage.resurrect(fc, &mut resurrected, 0);
                }
            }
        }

        // 2. Zero out WeakTrackResurrection handles
        // These are only cleared if the object was NOT resurrected.
        zero_out_handles(GCHandleType::WeakTrackResurrection, &resurrected);

        // 3. Prune dead objects from the debugging harness
        #[cfg(feature = "heap-diagnostics")]
        self._all_objs.borrow_mut().retain(|addr, obj| match obj.0 {
            Some(ptr) => !Gc::is_dead(fc, ptr) || resurrected.contains(addr),
            None => false,
        });
        #[cfg(not(feature = "heap-diagnostics"))]
        self.object_registry
            .borrow_mut()
            .prune_dead(fc, &resurrected);
    }
}

impl<'gc> Default for HeapManager<'gc> {
    fn default() -> Self {
        Self::new()
    }
}

fn object_size<'gc>(obj: ObjectRef<'gc>) -> Option<usize> {
    let inner_gc = obj.0?;
    let inner = inner_gc.borrow();
    Some(match &inner.storage {
        HeapStorage::Obj(o) => o.size_bytes(),
        HeapStorage::Vec(v) => v.size_bytes(),
        HeapStorage::Str(s) => s.size_bytes(),
        HeapStorage::Boxed(b) => b.size_bytes(),
    })
}

#[cfg(not(feature = "heap-diagnostics"))]
struct RegistryEntry<'gc> {
    start: usize,
    size: usize,
    obj: ObjectRef<'gc>,
}

#[cfg(not(feature = "heap-diagnostics"))]
struct ObjectRegistry<'gc> {
    entries: Slab<RegistryEntry<'gc>>,
    start_to_id: HashMap<usize, usize>,
    buckets: HashMap<usize, SmallVec<[usize; 2]>>,
}

#[cfg(not(feature = "heap-diagnostics"))]
impl<'gc> ObjectRegistry<'gc> {
    fn new() -> Self {
        Self {
            entries: Slab::new(),
            start_to_id: HashMap::new(),
            buckets: HashMap::new(),
        }
    }

    fn len(&self) -> usize {
        self.entries.len()
    }

    fn insert(&mut self, start: usize, size: usize, obj: ObjectRef<'gc>) {
        if size == 0 {
            return;
        }

        if let Some(existing) = self.start_to_id.remove(&start) {
            self.remove_entry(existing);
        }

        let id = self.entries.insert(RegistryEntry { start, size, obj });
        self.start_to_id.insert(start, id);

        for bucket in bucket_range(start, size) {
            let ids = self.buckets.entry(bucket).or_default();
            if !ids.contains(&id) {
                ids.push(id);
            }
        }
    }

    fn find(&self, ptr: usize) -> Option<ObjectRef<'gc>> {
        if let Some(id) = self.start_to_id.get(&ptr).copied()
            && let Some(entry) = self.entries.get(id)
        {
            return Some(entry.obj);
        }

        let bucket = ptr >> REGISTRY_BUCKET_SHIFT;
        let ids = self.buckets.get(&bucket)?;
        for id in ids {
            let Some(entry) = self.entries.get(*id) else {
                continue;
            };
            if ptr >= entry.start && ptr < entry.start.saturating_add(entry.size) {
                return Some(entry.obj);
            }
        }

        None
    }

    fn prune_dead(&mut self, fc: &'gc gc_arena::Finalization<'gc>, resurrected: &HashSet<usize>) {
        let mut to_remove = Vec::new();
        for (id, entry) in &self.entries {
            let remove = match entry.obj.0 {
                Some(ptr) => Gc::is_dead(fc, ptr) && !resurrected.contains(&entry.start),
                None => true,
            };
            if remove {
                to_remove.push(id);
            }
        }

        for id in to_remove {
            self.remove_entry(id);
        }
    }

    fn remove_entry(&mut self, id: usize) {
        if !self.entries.contains(id) {
            return;
        }

        let entry = self.entries.remove(id);
        self.start_to_id.remove(&entry.start);

        for bucket in bucket_range(entry.start, entry.size) {
            let mut clear_bucket = false;
            if let Some(ids) = self.buckets.get_mut(&bucket) {
                if let Some(pos) = ids.iter().position(|existing| *existing == id) {
                    ids.swap_remove(pos);
                }
                clear_bucket = ids.is_empty();
            }
            if clear_bucket {
                self.buckets.remove(&bucket);
            }
        }
    }
}

#[cfg(not(feature = "heap-diagnostics"))]
fn bucket_range(start: usize, size: usize) -> std::ops::RangeInclusive<usize> {
    let end = start.saturating_add(size.saturating_sub(1));
    (start >> REGISTRY_BUCKET_SHIFT)..=(end >> REGISTRY_BUCKET_SHIFT)
}

// SAFETY: `HeapManager::trace` correctly traces GC-managed object references based on their
// handle type. Normal and Pinned handles trace their objects to keep them alive. Weak handles
// do not trace to allow collection. The finalization queues and cross-arena roots are also
// traced. All other fields are non-GC metadata or control state that does not need tracing.
unsafe impl<'gc> Collect<'gc> for HeapManager<'gc> {
    fn trace<Tr: Trace<'gc>>(&self, cc: &mut Tr) {
        // Normal and Pinned handles keep objects alive.
        // Weak handles DO NOT trace.
        for entry in self.gchandles.borrow().iter().flatten() {
            match entry.1 {
                GCHandleType::Normal | GCHandleType::Pinned => {
                    entry.0.trace(cc);
                }
                GCHandleType::Weak | GCHandleType::WeakTrackResurrection => {
                    // Weak handles don't trace
                }
            }
        }

        for obj in self.pinned_objects.borrow().iter() {
            obj.trace(cc);
        }
        #[cfg(feature = "multithreading")]
        for ptr in self.cross_arena_roots.borrow().iter() {
            unsafe {
                Gc::from_ptr(ptr.as_ptr()).trace(cc);
            }
        }
        self.pending_finalization.borrow().trace(cc);
    }
}

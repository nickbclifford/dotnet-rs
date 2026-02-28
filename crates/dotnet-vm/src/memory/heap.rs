use crate::{state::SharedGlobalState, sync::Ordering};
use dotnet_utils::gc::GCHandleType;
use dotnet_value::object::{HeapStorage, ObjectRef};
use gc_arena::{Collect, Collection, Gc};
use std::{
    cell::{Cell, RefCell},
    collections::{BTreeMap, HashSet},
};

#[cfg(feature = "multithreading")]
use dotnet_value::object::ObjectPtr;

pub struct HeapManager<'gc> {
    pub finalization_queue: RefCell<Vec<ObjectRef<'gc>>>,
    pub pending_finalization: RefCell<Vec<ObjectRef<'gc>>>,
    pub pinned_objects: RefCell<HashSet<ObjectRef<'gc>>>,
    pub gchandles: RefCell<Vec<Option<(ObjectRef<'gc>, GCHandleType)>>>,
    pub processing_finalizer: Cell<bool>,
    pub needs_full_collect: Cell<bool>,
    /// Roots for objects in this arena that are referenced by other arenas.
    /// This is populated during the coordinated GC marking phase.
    #[cfg(feature = "multithreading")]
    pub cross_arena_roots: RefCell<HashSet<ObjectPtr>>,
    /// Untraced handles to every heap object in this arena.
    /// Used by the tracer during debugging and for conservative stack scanning.
    pub(crate) _all_objs: RefCell<BTreeMap<usize, ObjectRef<'gc>>>,
}

impl<'gc> HeapManager<'gc> {
    pub fn find_object(&self, ptr: usize) -> Option<ObjectRef<'gc>> {
        let all_objs = self._all_objs.borrow();
        let (&start, &obj) = all_objs.range(..=ptr).next_back()?;

        let size = {
            let inner_gc = obj.0.unwrap();
            let inner = inner_gc.borrow();
            match &inner.storage {
                HeapStorage::Obj(o) => o.size_bytes(),
                HeapStorage::Vec(v) => v.size_bytes(),
                HeapStorage::Str(s) => s.size_bytes(),
                HeapStorage::Boxed(b) => b.size_bytes(),
            }
        };

        if ptr < start + size { Some(obj) } else { None }
    }

    pub fn has_pending_finalizers(&self) -> bool {
        !self.pending_finalization.borrow().is_empty()
    }

    pub fn finalize_check(
        &self,
        fc: &'gc gc_arena::Finalization<'gc>,
        shared: &SharedGlobalState<'_>,
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

            let mut to_finalize = Vec::new();
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

            if !to_finalize.is_empty() {
                let mut pending = self.pending_finalization.borrow_mut();
                for obj in to_finalize {
                    let ptr = obj.0.unwrap();
                    pending.push(obj);
                    if resurrected.insert(Gc::as_ptr(ptr) as usize) {
                        // Trace resurrection event
                        if shared.tracer_enabled.load(Ordering::Relaxed) {
                            let obj_type_name = match &ptr.borrow().storage {
                                HeapStorage::Obj(o) => format!("{:?}", o.description),
                                HeapStorage::Vec(_) => "Vector".to_string(),
                                HeapStorage::Str(_) => "String".to_string(),
                                HeapStorage::Boxed(_) => "Boxed".to_string(),
                            };
                            let addr = Gc::as_ptr(ptr) as usize;
                            shared
                                .tracer
                                .trace_gc_resurrection(indent, &obj_type_name, addr);
                        }
                        Gc::resurrect(fc, ptr);
                        ptr.borrow().storage.resurrect(fc, &mut resurrected, 0);
                    }
                }
            }
        }

        // 1. Zero out Weak handles for dead objects
        zero_out_handles(GCHandleType::Weak, &resurrected);

        // 2. Zero out WeakTrackResurrection handles
        zero_out_handles(GCHandleType::WeakTrackResurrection, &resurrected);

        // 3. Prune dead objects from the debugging harness
        self._all_objs.borrow_mut().retain(|addr, obj| match obj.0 {
            Some(ptr) => !Gc::is_dead(fc, ptr) || resurrected.contains(addr),
            None => false,
        });
    }
}

// SAFETY: `HeapManager::trace` correctly traces GC-managed object references based on their
// handle type. Normal and Pinned handles trace their objects to keep them alive. Weak handles
// do not trace to allow collection. The finalization queues and cross-arena roots are also
// traced. All other fields are non-GC metadata or control state that does not need tracing.
unsafe impl<'gc> Collect for HeapManager<'gc> {
    fn trace(&self, cc: &Collection) {
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

use crate::{state::SharedGlobalState, sync::Ordering};
use dotnet_utils::gc::GCHandleType;
use dotnet_value::object::{HeapStorage, ObjectRef};
use gc_arena::{Collect, Collection, Gc};
use std::{
    cell::{Cell, RefCell},
    collections::{BTreeMap, HashSet},
};
#[cfg(feature = "multithreaded-gc")]
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
    #[cfg(feature = "multithreaded-gc")]
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

        if ptr < start + size {
            Some(obj)
        } else {
            None
        }
    }

    pub fn finalize_check(
        &self,
        fc: &gc_arena::Finalization<'gc>,
        shared: &SharedGlobalState<'_>,
        indent: usize,
    ) {
        let mut queue = self.finalization_queue.borrow_mut();
        let mut handles = self.gchandles.borrow_mut();
        let mut resurrected = HashSet::new();

        let mut zero_out_handles = |for_type: GCHandleType, resurrected: &HashSet<usize>| {
            for (obj_ref, handle_type) in handles.iter_mut().flatten() {
                if *handle_type == for_type {
                    if let ObjectRef(Some(ptr)) = obj_ref {
                        if Gc::is_dead(fc, *ptr) {
                            if for_type == GCHandleType::WeakTrackResurrection
                                && resurrected.contains(&(Gc::as_ptr(*ptr) as usize))
                            {
                                continue;
                            }
                            *obj_ref = ObjectRef(None);
                        }
                    }
                }
            }
        };

        if !queue.is_empty() {
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
                            shared.tracer.lock().trace_gc_resurrection(
                                indent,
                                &obj_type_name,
                                addr,
                            );
                        }
                        Gc::resurrect(fc, ptr);
                        ptr.borrow().storage.resurrect(fc, &mut resurrected);
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
        #[cfg(feature = "multithreaded-gc")]
        for ptr in self.cross_arena_roots.borrow().iter() {
            unsafe {
                Gc::from_ptr(ptr.as_ptr()).trace(cc);
            }
        }
        self.pending_finalization.borrow().trace(cc);
    }
}

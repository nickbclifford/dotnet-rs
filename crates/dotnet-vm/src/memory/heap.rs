use dotnet_utils::gc::GCHandleType;
use dotnet_value::object::{HeapStorage, ObjectRef};
#[cfg(feature = "multithreaded-gc")]
use dotnet_value::object::ObjectPtr;
use gc_arena::{Collect, Collection};
#[cfg(feature = "multithreaded-gc")]
use gc_arena::Gc;
use std::{
    cell::{Cell, RefCell},
    collections::{BTreeMap, HashSet},
};

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

use crate::{
    object::types::{Object, Vector},
    string::CLRString,
};
use gc_arena::{Collect, collect::Trace};
use std::collections::HashSet;

#[derive(Clone, Debug, PartialEq)]
pub enum HeapStorage<'gc> {
    Vec(Vector<'gc>),
    Obj(Object<'gc>),
    Str(CLRString),
    Boxed(Object<'gc>),
}

// SAFETY: HeapStorage is an enum where each variant either implements Collect or
// contains no GC references (like CLRString). We manually trace each variant.
unsafe impl<'gc> Collect<'gc> for HeapStorage<'gc> {
    fn trace<Tr: Trace<'gc>>(&self, cc: &mut Tr) {
        match self {
            Self::Vec(v) => v.trace(cc),
            Self::Obj(o) => o.trace(cc),
            Self::Boxed(v) => v.trace(cc),
            Self::Str(_) => {}
        }
    }
}

impl<'gc> HeapStorage<'gc> {
    pub fn size_bytes(&self) -> usize {
        match self {
            HeapStorage::Vec(v) => v.size_bytes(),
            HeapStorage::Obj(o) => o.size_bytes(),
            HeapStorage::Str(s) => s.size_bytes(),
            HeapStorage::Boxed(o) => o.size_bytes(),
        }
    }

    pub fn validate_resurrection_invariants(&self) {
        match self {
            HeapStorage::Vec(v) => v.validate_resurrection_invariants(),
            HeapStorage::Obj(o) => o.validate_resurrection_invariants(),
            HeapStorage::Str(_) => {}
            HeapStorage::Boxed(o) => o.validate_resurrection_invariants(),
        }
    }

    pub fn resurrect(
        &self,
        fc: &'gc gc_arena::Finalization<'gc>,
        visited: &mut HashSet<usize>,
        depth: usize,
    ) {
        match self {
            HeapStorage::Vec(v) => v.resurrect(fc, visited, depth),
            HeapStorage::Obj(o) => o.resurrect(fc, visited, depth),
            HeapStorage::Boxed(o) => o.resurrect(fc, visited, depth),
            HeapStorage::Str(_) => {}
        }
    }
    pub fn as_obj(&self) -> Option<&Object<'gc>> {
        match self {
            HeapStorage::Obj(o) => Some(o),
            _ => None,
        }
    }
    pub fn as_obj_mut(&mut self) -> Option<&mut Object<'gc>> {
        match self {
            HeapStorage::Obj(o) => Some(o),
            _ => None,
        }
    }

    pub fn with_data<T>(&self, f: impl FnOnce(&[u8]) -> T) -> T {
        match self {
            HeapStorage::Vec(v) => f(&v.storage),
            HeapStorage::Str(s) => unsafe {
                let bytes = std::slice::from_raw_parts(s.as_ptr() as *const u8, s.len() * 2);
                f(bytes)
            },
            HeapStorage::Obj(o) => o.instance_storage.with_data(f),
            HeapStorage::Boxed(o) => o.instance_storage.with_data(f),
        }
    }

    pub fn with_data_mut<T>(&mut self, f: impl FnOnce(&mut [u8]) -> T) -> T {
        match self {
            HeapStorage::Vec(v) => f(&mut v.storage),
            HeapStorage::Str(_) => panic!("Cannot modify CLRString directly"),
            HeapStorage::Obj(o) => o.instance_storage.with_data_mut(f),
            HeapStorage::Boxed(o) => o.instance_storage.with_data_mut(f),
        }
    }

    /// Returns a pointer to the raw data without acquiring a lock.
    ///
    /// # Safety
    /// The caller must ensure that the lock is held elsewhere (e.g. during STW GC)
    /// or that the data is otherwise stable and no writers are active.
    pub unsafe fn raw_data_ptr(&self) -> *mut u8 {
        match self {
            HeapStorage::Vec(v) => unsafe { v.raw_data_ptr() },
            HeapStorage::Str(s) => s.as_ptr() as *mut u8,
            HeapStorage::Obj(o) => unsafe { o.instance_storage.raw_data_ptr() },
            HeapStorage::Boxed(o) => unsafe { o.instance_storage.raw_data_ptr() },
        }
    }
}

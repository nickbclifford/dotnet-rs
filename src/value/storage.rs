use crate::value::{
    read_gc_ptr,
    layout::{FieldLayoutManager, HasLayout},
    Context, ObjectRef, TypeDescription,
};
use gc_arena::{Collect, Collection};
use std::marker::PhantomData;

#[derive(Clone, Debug, PartialEq)]
pub struct FieldStorage<'gc> {
    layout: FieldLayoutManager,
    storage: Vec<u8>,
    _contains_gc: PhantomData<&'gc ()>,
}

unsafe impl Collect for FieldStorage<'_> {
    #[inline]
    fn trace(&self, cc: &Collection) {
        for field in self.layout.fields.values() {
            if field.layout.is_gc_ptr() {
                if let ObjectRef(Some(gc)) = read_gc_ptr(&self.storage[field.position..]) {
                    gc.trace(cc);
                }
            }
        }
    }
}

impl FieldStorage<'_> {
    pub fn instance_fields(description: TypeDescription, context: Context) -> Self {
        let layout = FieldLayoutManager::instance_fields(description, context);
        Self {
            storage: vec![0; layout.size()],
            layout,
            _contains_gc: PhantomData,
        }
    }

    pub fn static_fields(description: TypeDescription, context: Context) -> Self {
        let layout = FieldLayoutManager::static_fields(description, context);
        Self {
            storage: vec![0; layout.size()],
            layout,
            _contains_gc: PhantomData,
        }
    }
}

// TODO: static type storage

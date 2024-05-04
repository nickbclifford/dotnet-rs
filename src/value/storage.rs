use std::collections::HashMap;
use std::marker::PhantomData;

use gc_arena::{Collect, Collection};

use crate::value::{
    layout::{FieldLayoutManager, HasLayout},
    read_gc_ptr, Context, MethodDescription, ObjectRef, TypeDescription,
};

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

    pub fn get(&self) -> &[u8] {
        &self.storage
    }

    pub fn get_field(&self, field: &str) -> &[u8] {
        match self.layout.fields.get(field) {
            None => panic!("field {} not found", field),
            Some(l) => &self.storage[l.as_range()],
        }
    }

    pub fn get_field_mut(&mut self, field: &str) -> &mut [u8] {
        match self.layout.fields.get(field) {
            None => panic!("field {} not found", field),
            Some(l) => &mut self.storage[l.as_range()],
        }
    }
}

#[derive(Clone, Debug, Collect)]
#[collect(no_drop)]
pub struct StaticStorage<'gc> {
    initialized: bool,
    storage: FieldStorage<'gc>,
}

#[derive(Clone, Debug, Collect)]
#[collect(no_drop)]
pub struct StaticStorageManager<'gc> {
    types: HashMap<TypeDescription, StaticStorage<'gc>>,
}
impl<'gc> StaticStorageManager<'gc> {
    pub fn new() -> Self {
        Self {
            types: HashMap::new(),
        }
    }

    pub fn get(&self, description: TypeDescription) -> &FieldStorage<'gc> {
        &self
            .types
            .get(&description)
            .expect("missing type in static storage")
            .storage
    }

    pub fn get_mut(&mut self, description: TypeDescription) -> &mut FieldStorage<'gc> {
        &mut self
            .types
            .get_mut(&description)
            .expect("missing type in static storage")
            .storage
    }

    #[must_use]
    pub fn init(
        &mut self,
        description: TypeDescription,
        context: Context,
    ) -> Option<MethodDescription> {
        if !self.types.contains_key(&description) {
            self.types.insert(
                description,
                StaticStorage {
                    initialized: false,
                    storage: FieldStorage::static_fields(description, context),
                },
            );
        }

        match description.static_initializer() {
            None => None,
            Some(m) => {
                let mut t = self
                    .types
                    .get_mut(&description)
                    .expect("missing type in static storage");
                if t.initialized {
                    None
                } else {
                    t.initialized = true;
                    Some(m)
                }
            }
        }
    }
}

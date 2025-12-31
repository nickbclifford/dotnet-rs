use crate::{
    utils::DebugStr,
    value::{
        layout::{FieldLayoutManager, HasLayout},
        Context, MethodDescription, ObjectRef, TypeDescription,
    },
};

use gc_arena::{Collect, Collection};
use std::{
    collections::HashMap,
    fmt::{Debug, Formatter},
    marker::PhantomData,
    ops::Range,
};

#[derive(Clone, PartialEq)]
pub struct FieldStorage<'gc> {
    layout: FieldLayoutManager,
    storage: Vec<u8>,
    _contains_gc: PhantomData<&'gc ()>,
}

impl Debug for FieldStorage<'_> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        let mut fs: Vec<(_, _)> = self
            .layout
            .fields
            .iter()
            .map(|(k, v)| {
                let data = &self.storage[v.as_range()];
                let data_rep = if v.layout.is_gc_ptr() {
                    format!("{:?}", ObjectRef::read(data))
                } else {
                    let bytes: Vec<_> = data.iter().map(|b| format!("{:02x}", b)).collect();
                    bytes.join(" ")
                };

                (
                    v.position,
                    DebugStr(format!("{} {}: {}", v.layout.type_tag(), k, data_rep)),
                )
            })
            .collect();

        fs.sort_by_key(|(p, _)| *p);

        f.debug_list()
            .entries(fs.into_iter().map(|(_, r)| r))
            .finish()
    }
}

unsafe impl Collect for FieldStorage<'_> {
    #[inline]
    fn trace(&self, cc: &Collection) {
        for field in self.layout.fields.values() {
            if field.layout.is_gc_ptr() {
                ObjectRef::read(&self.storage[field.position..]).trace(cc);
            }
        }
    }
}

impl FieldStorage<'_> {
    pub fn new(layout: FieldLayoutManager) -> Self {
        Self {
            storage: vec![0; layout.size()],
            layout,
            _contains_gc: PhantomData,
        }
    }

    pub fn instance_fields(description: TypeDescription, context: Context) -> Self {
        Self::new(FieldLayoutManager::instance_fields(description, context))
    }

    pub fn static_fields(description: TypeDescription, context: Context) -> Self {
        Self::new(FieldLayoutManager::static_fields(description, context))
    }

    pub fn get(&self) -> &[u8] {
        &self.storage
    }

    pub fn get_mut(&mut self) -> &mut [u8] {
        &mut self.storage
    }

    fn get_field_range(&self, field: &str) -> Range<usize> {
        match self.layout.fields.get(field) {
            None => panic!("field {} not found", field),
            Some(l) => l.as_range(),
        }
    }

    pub fn get_field(&self, field: &str) -> &[u8] {
        &self.storage[self.get_field_range(field)]
    }

    pub fn get_field_mut(&mut self, field: &str) -> &mut [u8] {
        let r = self.get_field_range(field);
        &mut self.storage[r]
    }
}

#[derive(Clone, Collect)]
#[collect(no_drop)]
pub struct StaticStorage<'gc> {
    initialized: bool,
    storage: FieldStorage<'gc>,
}
impl Debug for StaticStorage<'_> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        if self.initialized {
            Debug::fmt(&self.storage, f)
        } else {
            write!(f, "uninitialized")
        }
    }
}

#[derive(Clone, Collect)]
#[collect(no_drop)]
pub struct StaticStorageManager<'gc> {
    types: HashMap<TypeDescription, StaticStorage<'gc>>,
}
impl Debug for StaticStorageManager<'_> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_map()
            .entries(self.types.iter().map(|(k, v)| (DebugStr(k.type_name()), v)))
            .finish()
    }
}

impl<'gc> StaticStorageManager<'gc> {
    pub fn new() -> Self {
        Self {
            types: HashMap::new(),
        }
    }
}

impl<'gc> Default for StaticStorageManager<'gc> {
    fn default() -> Self {
        Self::new()
    }
}

impl<'gc> StaticStorageManager<'gc> {
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
        self.types
            .entry(description)
            .or_insert_with(|| StaticStorage {
                initialized: false,
                storage: FieldStorage::static_fields(description, context),
            });

        match description.static_initializer() {
            None => None,
            Some(m) => {
                let t = self
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

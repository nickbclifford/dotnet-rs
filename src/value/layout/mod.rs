use crate::{
    types::TypeDescription,
    utils::sync::Arc,
    value::object::ObjectRef,
};
use enum_dispatch::enum_dispatch;
use gc_arena::{Collect, Collection};
use std::{
    collections::{HashMap, HashSet},
    ops::Range,
};

#[enum_dispatch]
pub trait HasLayout {
    fn size(&self) -> usize;
}

#[enum_dispatch(HasLayout)]
#[derive(Clone, Debug, PartialEq)]
pub enum LayoutManager {
    FieldLayoutManager,
    ArrayLayoutManager,
    Scalar,
}

impl LayoutManager {
    pub fn is_gc_ptr(&self) -> bool {
        matches!(
            self,
            LayoutManager::Scalar(Scalar::ObjectRef) | LayoutManager::Scalar(Scalar::ManagedPtr)
        )
    }

    pub fn is_or_contains_refs(&self) -> bool {
        match self {
            LayoutManager::FieldLayoutManager(f) => {
                f.fields.values().any(|f| f.layout.is_or_contains_refs())
            }
            LayoutManager::ArrayLayoutManager(a) => a.element_layout.is_or_contains_refs(),
            LayoutManager::Scalar(Scalar::ObjectRef)
            | LayoutManager::Scalar(Scalar::ManagedPtr) => true,
            _ => false,
        }
    }

    pub fn type_tag(&self) -> &'static str {
        match &self {
            LayoutManager::FieldLayoutManager(_) => "struct",
            LayoutManager::ArrayLayoutManager(_) => "arr",
            LayoutManager::Scalar(s) => match s {
                Scalar::ObjectRef => "obj",
                Scalar::ManagedPtr => "ptr",
                Scalar::Int8 => "i8",
                Scalar::Int16 => "i16",
                Scalar::Int32 => "i32",
                Scalar::Int64 => "i64",
                Scalar::NativeInt => "ptr",
                Scalar::Float32 => "f32",
                Scalar::Float64 => "f64",
            },
        }
    }

    pub fn alignment(&self) -> usize {
        match self {
            LayoutManager::FieldLayoutManager(f) => f.alignment,
            LayoutManager::ArrayLayoutManager(a) => a.element_layout.alignment(),
            LayoutManager::Scalar(s) => s.alignment(),
        }
    }

    pub fn trace(&self, storage: &[u8], cc: &Collection) {
        match self {
            LayoutManager::Scalar(Scalar::ObjectRef) => {
                ObjectRef::read(storage).trace(cc);
            }
            LayoutManager::Scalar(Scalar::ManagedPtr) => {
                // NOTE: ManagedPtr in memory is now pointer-sized (8 bytes).
                // The GC metadata (owner handle) is stored in the Object's side-table.
                // Tracing of managed pointer owners is handled by Object::trace,
                // which has access to the side-table. Here we do nothing since
                // the raw pointer value doesn't need tracing.
            }
            LayoutManager::FieldLayoutManager(f) => {
                for field in f.fields.values() {
                    field.layout.trace(&storage[field.position..], cc);
                }
            }
            LayoutManager::ArrayLayoutManager(a) => {
                let elem_size = a.element_layout.size();
                for i in 0..a.length {
                    a.element_layout.trace(&storage[i * elem_size..], cc);
                }
            }
            _ => {}
        }
    }

    pub fn resurrect<'gc>(
        &self,
        storage: &[u8],
        fc: &gc_arena::Finalization<'gc>,
        visited: &mut HashSet<usize>,
    ) {
        match self {
            LayoutManager::Scalar(Scalar::ObjectRef) => {
                ObjectRef::read(storage).resurrect(fc, visited);
            }
            LayoutManager::Scalar(Scalar::ManagedPtr) => {
                // NOTE: ManagedPtr resurrection is handled by Object::resurrect
                // which has access to the side-table containing the owner handles.
            }
            LayoutManager::FieldLayoutManager(f) => {
                for field in f.fields.values() {
                    field
                        .layout
                        .resurrect(&storage[field.position..], fc, visited);
                }
            }
            LayoutManager::ArrayLayoutManager(a) => {
                let elem_size = a.element_layout.size();
                for i in 0..a.length {
                    a.element_layout
                        .resurrect(&storage[i * elem_size..], fc, visited);
                }
            }
            _ => {}
        }
    }
}

pub(crate) fn align_up(value: usize, align: usize) -> usize {
    let misalignment = value % align;
    if misalignment == 0 {
        value
    } else {
        value + align - misalignment
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct FieldLayout {
    pub position: usize,
    pub layout: Arc<LayoutManager>,
}

impl FieldLayout {
    pub fn as_range(&self) -> Range<usize> {
        self.position..self.position + self.layout.size()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct FieldKey {
    pub owner: TypeDescription,
    pub name: String,
}

impl std::fmt::Display for FieldKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.name)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct FieldLayoutManager {
    pub fields: HashMap<FieldKey, FieldLayout>,
    pub total_size: usize,
    pub alignment: usize,
}

impl HasLayout for FieldLayoutManager {
    fn size(&self) -> usize {
        self.total_size
    }
}

impl FieldLayoutManager {
    pub fn trace(&self, storage: &[u8], cc: &Collection) {
        for field in self.fields.values() {
            field.layout.trace(&storage[field.position..], cc);
        }
    }

    pub fn resurrect<'gc>(
        &self,
        storage: &[u8],
        fc: &gc_arena::Finalization<'gc>,
        visited: &mut HashSet<usize>,
    ) {
        for field in self.fields.values() {
            field
                .layout
                .resurrect(&storage[field.position..], fc, visited);
        }
    }

    /// Get a field's layout by owner and name
    pub fn get_field(&self, owner: TypeDescription, name: &str) -> Option<&FieldLayout> {
        self.fields.get(&FieldKey {
            owner,
            name: name.to_string(),
        })
    }

    /// Get a field's layout by name, searching through all types
    /// This is a fallback for cases where we don't know the exact owner
    pub fn get_field_by_name(&self, name: &str) -> Option<&FieldLayout> {
        self.fields
            .iter()
            .find(|(k, _)| k.name == name)
            .map(|(_, v)| v)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ArrayLayoutManager {
    pub element_layout: Arc<LayoutManager>,
    pub length: usize,
}

impl HasLayout for ArrayLayoutManager {
    fn size(&self) -> usize {
        self.element_layout.size() * self.length
    }
}

impl ArrayLayoutManager {
    pub fn resurrect<'gc>(
        &self,
        storage: &[u8],
        fc: &gc_arena::Finalization<'gc>,
        visited: &mut HashSet<usize>,
    ) {
        let elem_size = self.element_layout.size();
        for i in 0..self.length {
            self.element_layout
                .resurrect(&storage[i * elem_size..], fc, visited);
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Scalar {
    ObjectRef,
    ManagedPtr,
    Int8,
    Int16,
    Int32,
    Int64,
    NativeInt,
    Float32,
    Float64,
}

impl HasLayout for Scalar {
    fn size(&self) -> usize {
        match self {
            Scalar::Int8 => 1,
            Scalar::Int16 => 2,
            Scalar::Int32 => 4,
            Scalar::Int64 => 8,
            Scalar::ObjectRef | Scalar::NativeInt => ObjectRef::SIZE,
            // ManagedPtr is pointer-sized in memory (metadata stored in side-table)
            Scalar::ManagedPtr => ObjectRef::SIZE,
            Scalar::Float32 => 4,
            Scalar::Float64 => 8,
        }
    }
}

impl Scalar {
    pub fn alignment(&self) -> usize {
        self.size()
    }
}

use crate::object::ObjectRef;
use bitvec::prelude::*;
use dotnet_types::TypeDescription;
use dotnet_utils::sync::Arc;
use gc_arena::{Collect, Collection};
use std::{
    collections::{HashMap, HashSet},
    ops::Range,
};

pub trait HasLayout {
    fn size(&self) -> usize;
}

#[derive(Clone, Debug, PartialEq)]
pub enum LayoutManager {
    Field(FieldLayoutManager),
    Array(ArrayLayoutManager),
    Scalar(Scalar),
}
impl HasLayout for LayoutManager {
    fn size(&self) -> usize {
        match self {
            LayoutManager::Field(f) => f.size(),
            LayoutManager::Array(a) => a.size(),
            LayoutManager::Scalar(s) => s.size(),
        }
    }
}
impl From<FieldLayoutManager> for LayoutManager {
    fn from(f: FieldLayoutManager) -> Self {
        Self::Field(f)
    }
}
impl From<ArrayLayoutManager> for LayoutManager {
    fn from(a: ArrayLayoutManager) -> Self {
        Self::Array(a)
    }
}
impl From<Scalar> for LayoutManager {
    fn from(s: Scalar) -> Self {
        Self::Scalar(s)
    }
}

impl LayoutManager {
    pub fn visit_managed_ptrs(&self, offset: usize, visitor: &mut dyn FnMut(usize)) {
        match self {
            Self::Scalar(Scalar::ManagedPtr) => visitor(offset),
            Self::Field(flm) => {
                for field in flm.fields.values() {
                    field
                        .layout
                        .visit_managed_ptrs(offset + field.position, visitor);
                }
            }
            Self::Array(alm) => {
                let elem_size = alm.element_layout.size();
                for i in 0..alm.length {
                    alm.element_layout
                        .visit_managed_ptrs(offset + i * elem_size, visitor);
                }
            }
            _ => {}
        }
    }

    pub fn is_gc_ptr(&self) -> bool {
        matches!(
            self,
            LayoutManager::Scalar(Scalar::ObjectRef) | LayoutManager::Scalar(Scalar::ManagedPtr)
        )
    }

    pub fn is_or_contains_refs(&self) -> bool {
        match self {
            LayoutManager::Field(f) => f.fields.values().any(|f| f.layout.is_or_contains_refs()),
            LayoutManager::Array(a) => a.element_layout.is_or_contains_refs(),
            LayoutManager::Scalar(Scalar::ObjectRef)
            | LayoutManager::Scalar(Scalar::ManagedPtr) => true,
            _ => false,
        }
    }

    pub fn type_tag(&self) -> &'static str {
        match &self {
            LayoutManager::Field(_) => "struct",
            LayoutManager::Array(_) => "arr",
            LayoutManager::Scalar(s) => match s {
                Scalar::ObjectRef => "obj",
                Scalar::ManagedPtr => "ptr",
                Scalar::Int8 | Scalar::UInt8 => "i8",
                Scalar::Int16 | Scalar::UInt16 => "i16",
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
            LayoutManager::Field(f) => f.alignment,
            LayoutManager::Array(a) => a.element_layout.alignment(),
            LayoutManager::Scalar(s) => s.alignment(),
        }
    }

    pub fn trace(&self, storage: &[u8], cc: &Collection) {
        match self {
            LayoutManager::Scalar(Scalar::ObjectRef) => {
                unsafe { ObjectRef::read_unchecked(storage) }.trace(cc);
            }
            LayoutManager::Scalar(Scalar::ManagedPtr) => {
                // ManagedPtr in memory is 16 bytes: (Owner ObjectRef, Offset).
                // We need to trace the owner at offset 0.
                let ptr_size = ObjectRef::SIZE;
                unsafe { ObjectRef::read_unchecked(&storage[0..ptr_size]) }.trace(cc);
            }
            LayoutManager::Field(f) => {
                f.trace(storage, cc);
            }
            LayoutManager::Array(a) => {
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
        fc: &'gc gc_arena::Finalization<'gc>,
        visited: &mut HashSet<usize>,
    ) {
        match self {
            LayoutManager::Scalar(Scalar::ObjectRef) => {
                unsafe { ObjectRef::read_branded(storage, fc) }.resurrect(fc, visited);
            }
            LayoutManager::Scalar(Scalar::ManagedPtr) => {
                // ManagedPtr in memory is 16 bytes: (Owner ObjectRef, Offset).
                // We need to resurrect the owner at offset 0.
                let ptr_size = size_of::<usize>();
                unsafe { ObjectRef::read_branded(&storage[0..ptr_size], fc) }
                    .resurrect(fc, visited);
            }
            LayoutManager::Field(f) => {
                for field in f.fields.values() {
                    field
                        .layout
                        .resurrect(&storage[field.position..], fc, visited);
                }
            }
            LayoutManager::Array(a) => {
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

pub const fn align_up(value: usize, align: usize) -> usize {
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

#[derive(Clone, Debug, PartialEq, Default)]
pub struct GcDesc {
    pub bitmap: BitVec<usize, Lsb0>,
}

impl GcDesc {
    pub fn set(&mut self, word_index: usize) {
        if word_index >= self.bitmap.len() {
            self.bitmap.resize(word_index + 1, false);
        }
        self.bitmap.set(word_index, true);
    }

    pub fn merge(&mut self, other: &GcDesc) {
        if other.bitmap.len() > self.bitmap.len() {
            self.bitmap.resize(other.bitmap.len(), false);
        }
        self.bitmap |= &other.bitmap;
    }

    pub fn trace(&self, storage: &[u8], cc: &Collection) {
        let ptr_size = size_of::<usize>();
        for word_index in self.bitmap.iter_ones() {
            let offset = word_index * ptr_size;

            // Safety: layout creation ensures this offset is valid and contains a pointer
            if offset + ptr_size <= storage.len() {
                // Use read_unchecked to handle potential unaligned access safely
                // and correctly reconstruct the ObjectRef.
                let ptr = unsafe { ObjectRef::read_unchecked(&storage[offset..]) };
                ptr.trace(cc);
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct FieldLayoutManager {
    pub fields: HashMap<FieldKey, FieldLayout>,
    pub total_size: usize,
    pub alignment: usize,
    pub gc_desc: GcDesc,
}

impl HasLayout for FieldLayoutManager {
    fn size(&self) -> usize {
        self.total_size
    }
}

impl FieldLayoutManager {
    pub fn visit_managed_ptrs(&self, offset: usize, visitor: &mut dyn FnMut(usize)) {
        for field in self.fields.values() {
            field
                .layout
                .visit_managed_ptrs(offset + field.position, visitor);
        }
    }

    pub fn trace(&self, storage: &[u8], cc: &Collection) {
        self.gc_desc.trace(storage, cc);
    }

    pub fn resurrect<'gc>(
        &self,
        storage: &[u8],
        fc: &'gc gc_arena::Finalization<'gc>,
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
        fc: &'gc gc_arena::Finalization<'gc>,
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
    UInt8,
    Int16,
    UInt16,
    Int32,
    Int64,
    NativeInt,
    Float32,
    Float64,
}

impl HasLayout for Scalar {
    fn size(&self) -> usize {
        self.size_const()
    }
}

impl Scalar {
    /// Const-compatible version of size calculation.
    /// Use this when you need compile-time size evaluation.
    pub const fn size_const(&self) -> usize {
        match self {
            Scalar::Int8 | Scalar::UInt8 => 1,
            Scalar::Int16 | Scalar::UInt16 => 2,
            Scalar::Int32 => 4,
            Scalar::Int64 => 8,
            Scalar::ObjectRef | Scalar::NativeInt => ObjectRef::SIZE,
            // ManagedPtr is stored as (ObjectRef, Offset) to eliminate side-tables
            Scalar::ManagedPtr => 2 * ObjectRef::SIZE,
            Scalar::Float32 => 4,
            Scalar::Float64 => 8,
        }
    }

    /// Const-compatible version of alignment calculation.
    pub const fn alignment_const(&self) -> usize {
        match self {
            Scalar::ManagedPtr => ObjectRef::SIZE,
            _ => self.size_const(),
        }
    }

    pub fn alignment(&self) -> usize {
        self.alignment_const()
    }
}

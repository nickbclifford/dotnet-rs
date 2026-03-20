use crate::{object::ObjectRef, pointer::ManagedPtr};
use bitvec::prelude::*;
use dotnet_types::TypeDescription;
use dotnet_utils::sync::Arc;
use gc_arena::{Collect, collect::Trace};
use std::{
    collections::{HashMap, HashSet},
    ops::Range,
};

pub trait HasLayout {
    fn size(&self) -> crate::ByteOffset;
}

#[derive(Clone, Debug, PartialEq)]
pub enum LayoutManager {
    Field(FieldLayoutManager),
    Array(ArrayLayoutManager),
    Scalar(Scalar),
}
impl HasLayout for LayoutManager {
    fn size(&self) -> crate::ByteOffset {
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
    pub fn visit_managed_ptrs(
        &self,
        offset: crate::ByteOffset,
        visitor: &mut dyn FnMut(crate::ByteOffset),
    ) {
        match self {
            Self::Scalar(Scalar::ManagedPtr) => visitor(offset),
            Self::Field(flm) => {
                for field in flm.fields.values() {
                    field
                        .layout
                        .visit_managed_ptrs(offset.checked_add(field.position).unwrap(), visitor);
                }
            }
            Self::Array(alm) => {
                let elem_size = alm.element_layout.size();
                for i in 0..alm.length {
                    alm.element_layout.visit_managed_ptrs(
                        offset
                            .checked_add(elem_size.checked_mul(i).unwrap())
                            .unwrap(),
                        visitor,
                    );
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

    pub fn has_managed_ptrs(&self) -> bool {
        match self {
            LayoutManager::Field(f) => f.has_ref_fields,
            LayoutManager::Array(a) => a.element_layout.has_managed_ptrs(),
            LayoutManager::Scalar(Scalar::ManagedPtr) => true,
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

    pub fn trace<'gc, Tr: Trace<'gc>>(&self, storage: &[u8], cc: &mut Tr) {
        match self {
            LayoutManager::Scalar(Scalar::ObjectRef) => {
                unsafe { ObjectRef::read_unchecked(storage) }.trace(cc);
            }
            LayoutManager::Scalar(Scalar::ManagedPtr) => {
                let info = unsafe { ManagedPtr::read_unchecked(storage) }
                    .expect("LayoutManager::trace: failed to read ManagedPtr");
                info.origin.trace(cc);
            }
            LayoutManager::Field(f) => {
                f.trace(storage, cc);
            }
            LayoutManager::Array(a) => {
                let elem_size = a.element_layout.size();
                for i in 0..a.length {
                    a.element_layout
                        .trace(&storage[elem_size.as_usize() * i..], cc);
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
        depth: usize,
    ) {
        match self {
            LayoutManager::Scalar(Scalar::ObjectRef) => {
                unsafe { ObjectRef::read_branded(storage, fc) }.resurrect(fc, visited, depth);
            }
            LayoutManager::Scalar(Scalar::ManagedPtr) => {
                let info = unsafe { ManagedPtr::read_branded(storage, fc) }
                    .expect("LayoutManager::resurrect: failed to read ManagedPtr");
                info.origin.resurrect(fc, visited, depth);
            }
            LayoutManager::Field(f) => {
                for field in f.fields.values() {
                    field.layout.resurrect(
                        &storage[field.position.as_usize()..],
                        fc,
                        visited,
                        depth,
                    );
                }
            }
            LayoutManager::Array(a) => {
                let elem_size = a.element_layout.size();
                for i in 0..a.length {
                    a.element_layout.resurrect(
                        &storage[elem_size.as_usize() * i..],
                        fc,
                        visited,
                        depth,
                    );
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
    pub position: crate::ByteOffset,
    pub layout: Arc<LayoutManager>,
}

impl FieldLayout {
    pub fn as_range(&self) -> Range<usize> {
        self.position.as_usize()..self.position.as_usize() + self.layout.size().as_usize()
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

    pub fn trace<'gc, Tr: Trace<'gc>>(&self, storage: &[u8], cc: &mut Tr) {
        let ptr_size = ObjectRef::SIZE;
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
    pub has_ref_fields: bool,
}

impl HasLayout for FieldLayoutManager {
    fn size(&self) -> crate::ByteOffset {
        crate::ByteOffset(self.total_size)
    }
}

impl FieldLayoutManager {
    pub fn visit_managed_ptrs(
        &self,
        offset: crate::ByteOffset,
        visitor: &mut dyn FnMut(crate::ByteOffset),
    ) {
        for field in self.fields.values() {
            field
                .layout
                .visit_managed_ptrs(offset + field.position, visitor);
        }
    }

    pub fn trace<'gc, Tr: Trace<'gc>>(&self, storage: &[u8], cc: &mut Tr) {
        // Trace regular ObjectRefs via the optimized GcDesc bitmap
        self.gc_desc.trace(storage, cc);

        // Trace ManagedPtrs (byrefs) if present.
        // We use visit_managed_ptrs to correctly find all ManagedPtrs, even in nested structs,
        // which avoids the previous bug where only top-level ManagedPtrs were traced.
        if self.has_ref_fields {
            self.visit_managed_ptrs(crate::ByteOffset(0), &mut |offset| {
                let off = offset.as_usize();
                if off + ManagedPtr::SIZE <= storage.len() {
                    // SAFETY: layout creation ensures these offsets contain valid ManagedPtrs.
                    // ManagedPtr::read_unchecked is tag-aware and safe to use during GC tracing.
                    let info = unsafe { ManagedPtr::read_unchecked(&storage[off..]) }
                        .expect("FieldLayoutManager::trace: failed to read ManagedPtr");
                    info.origin.trace(cc);
                }
            });
        }
    }

    pub fn resurrect<'gc>(
        &self,
        storage: &[u8],
        fc: &'gc gc_arena::Finalization<'gc>,
        visited: &mut HashSet<usize>,
        depth: usize,
    ) {
        for field in self.fields.values() {
            field
                .layout
                .resurrect(&storage[field.position.as_usize()..], fc, visited, depth);
        }
    }

    pub fn has_ref_fields(&self) -> bool {
        self.has_ref_fields
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
    fn size(&self) -> crate::ByteOffset {
        self.element_layout.size() * self.length
    }
}

impl ArrayLayoutManager {
    pub fn resurrect<'gc>(
        &self,
        storage: &[u8],
        fc: &'gc gc_arena::Finalization<'gc>,
        visited: &mut HashSet<usize>,
        depth: usize,
    ) {
        let elem_size = self.element_layout.size();
        for i in 0..self.length {
            self.element_layout.resurrect(
                &storage[(elem_size * i).as_usize()..],
                fc,
                visited,
                depth,
            );
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
    fn size(&self) -> crate::ByteOffset {
        crate::ByteOffset(self.size_const())
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
            // ManagedPtr is stored as (ObjectRef, Offset, Checksum) to eliminate side-tables
            Scalar::ManagedPtr => ManagedPtr::SIZE,
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

/// Trait for types that can be read/written to field storage
pub trait FieldType: Sized {
    const SCALAR: Scalar;
    fn read_from(bytes: &[u8]) -> Self;
    fn write_to(&self, bytes: &mut [u8]);
}

macro_rules! impl_scalar_field_type {
    ($type:ty, $scalar:expr) => {
        impl FieldType for $type {
            const SCALAR: Scalar = $scalar;
            fn read_from(bytes: &[u8]) -> Self {
                let size = std::mem::size_of::<Self>();
                let mut buf = [0u8; std::mem::size_of::<Self>()];
                buf.copy_from_slice(&bytes[..size]);
                <$type>::from_le_bytes(buf)
            }
            fn write_to(&self, bytes: &mut [u8]) {
                let size = std::mem::size_of::<Self>();
                bytes[..size].copy_from_slice(&self.to_le_bytes());
            }
        }
    };
}

impl_scalar_field_type!(i8, Scalar::Int8);
impl_scalar_field_type!(u8, Scalar::UInt8);
impl_scalar_field_type!(i16, Scalar::Int16);
impl_scalar_field_type!(u16, Scalar::UInt16);
impl_scalar_field_type!(i32, Scalar::Int32);
impl_scalar_field_type!(u32, Scalar::Int32); // u32 uses Int32 layout
impl_scalar_field_type!(i64, Scalar::Int64);
impl_scalar_field_type!(u64, Scalar::Int64); // u64 uses Int64 layout
impl_scalar_field_type!(f32, Scalar::Float32);
impl_scalar_field_type!(f64, Scalar::Float64);
impl_scalar_field_type!(isize, Scalar::NativeInt);
impl_scalar_field_type!(usize, Scalar::NativeInt);

impl<'gc> FieldType for ObjectRef<'gc> {
    const SCALAR: Scalar = Scalar::ObjectRef;
    fn read_from(bytes: &[u8]) -> Self {
        unsafe { ObjectRef::read_unchecked(bytes) }
    }
    fn write_to(&self, bytes: &mut [u8]) {
        self.write(bytes);
    }
}

impl<'gc> FieldType for ManagedPtr<'gc> {
    const SCALAR: Scalar = Scalar::ManagedPtr;
    fn read_from(bytes: &[u8]) -> Self {
        let info = unsafe { ManagedPtr::read_unchecked(bytes).unwrap() };
        ManagedPtr::from_info_full(info, dotnet_types::TypeDescription::NULL, false)
    }
    fn write_to(&self, bytes: &mut [u8]) {
        self.write(bytes);
    }
}

#[cfg(test)]
#[allow(clippy::mutable_key_type)]
mod tests {
    use super::*;

    #[test]
    fn test_field_layout_manager_recursion() {
        // Inner struct: { int i; ref int r; }
        // ManagedPtr is 24 bytes, Int32 is 4 bytes.
        // On 64-bit, alignment will be 8.
        let inner_layout = Arc::new(FieldLayoutManager {
            fields: {
                let mut m = HashMap::new();
                m.insert(
                    FieldKey {
                        owner: TypeDescription::NULL,
                        name: "i".to_string(),
                    },
                    FieldLayout {
                        position: crate::ByteOffset(0),
                        layout: Arc::new(LayoutManager::Scalar(Scalar::Int32)),
                    },
                );
                m.insert(
                    FieldKey {
                        owner: TypeDescription::NULL,
                        name: "r".to_string(),
                    },
                    FieldLayout {
                        position: crate::ByteOffset(8),
                        layout: Arc::new(LayoutManager::Scalar(Scalar::ManagedPtr)),
                    },
                );
                m
            },
            total_size: 8 + ManagedPtr::SIZE,
            alignment: 8,
            gc_desc: GcDesc::default(),
            has_ref_fields: true,
        });

        // Outer struct: { Inner inner; } at offset 8
        let outer_layout = FieldLayoutManager {
            fields: {
                let mut m = HashMap::new();
                m.insert(
                    FieldKey {
                        owner: TypeDescription::NULL,
                        name: "inner".to_string(),
                    },
                    FieldLayout {
                        position: crate::ByteOffset(8),
                        layout: Arc::new(LayoutManager::Field((*inner_layout).clone())),
                    },
                );
                m
            },
            total_size: 8 + 8 + ManagedPtr::SIZE,
            alignment: 8,
            gc_desc: GcDesc::default(),
            has_ref_fields: true,
        };

        // 1. Verify visit_managed_ptrs() finds the correct recursive logic
        let mut found_offsets = Vec::new();
        outer_layout.visit_managed_ptrs(crate::ByteOffset(0), &mut |off| {
            found_offsets.push(off.as_usize())
        });

        assert_eq!(found_offsets.len(), 1);
        assert_eq!(found_offsets[0], 16); // Correctly found 'r' at offset 16
    }
}

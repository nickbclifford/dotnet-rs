use crate::{
    StackValue, ValidationTag,
    layout::{ArrayLayoutManager, HasLayout, LayoutManager, Scalar},
    pointer::ManagedPtr,
    storage::FieldStorage,
};
use dotnet_types::{
    TypeDescription,
    generics::{ConcreteType, GenericLookup},
};
use dotnet_utils::DebugStr;
use gc_arena::{Collect, collect::Trace};
use std::{
    collections::HashSet,
    fmt::{self, Debug, Formatter},
    iter,
    marker::PhantomData,
    mem::size_of,
    sync::Arc,
};

#[cfg(feature = "fuzzing")]
use arbitrary::Arbitrary;

#[derive(Clone, Debug, PartialEq)]
pub enum ValueType<'gc> {
    Bool(bool),
    Char(u16),
    Int8(i8),
    UInt8(u8),
    Int16(i16),
    UInt16(u16),
    Int32(i32),
    UInt32(u32),
    Int64(i64),
    UInt64(u64),
    NativeInt(isize),
    NativeUInt(usize),
    Pointer(ManagedPtr<'gc>),
    Float32(f32),
    Float64(f64),
    TypedRef(ManagedPtr<'gc>, Arc<TypeDescription>),
    Struct(Object<'gc>),
}

// SAFETY: ValueType is an enum where only Pointer and Struct variants contain
// potential GC references. We manually trace these variants.
unsafe impl<'gc> Collect<'gc> for ValueType<'gc> {
    fn trace<Tr: Trace<'gc>>(&self, cc: &mut Tr) {
        match self {
            Self::Pointer(p) => p.trace(cc),
            Self::TypedRef(p, _) => p.trace(cc),
            Self::Struct(o) => o.trace(cc),
            _ => {}
        }
    }
}

impl<'gc> ValueType<'gc> {
    pub fn size_bytes(&self) -> usize {
        match self {
            ValueType::Bool(_) => 1,
            ValueType::Char(_) => 2,
            ValueType::Int8(_) | ValueType::UInt8(_) => 1,
            ValueType::Int16(_) | ValueType::UInt16(_) => 2,
            ValueType::Int32(_) | ValueType::UInt32(_) => 4,
            ValueType::Int64(_) | ValueType::UInt64(_) => 8,
            ValueType::NativeInt(_) | ValueType::NativeUInt(_) => size_of::<usize>(),
            ValueType::Float32(_) => 4,
            ValueType::Float64(_) => 8,
            ValueType::Pointer(_) => 16,
            ValueType::TypedRef(_, _) => 16,
            ValueType::Struct(o) => o.size_bytes(),
        }
    }

    pub fn validate_resurrection_invariants(&self) {
        match self {
            ValueType::Pointer(p) => p.validate_magic(),
            ValueType::TypedRef(p, _) => p.validate_magic(),
            ValueType::Struct(o) => o.validate_resurrection_invariants(),
            _ => {}
        }
    }

    pub fn resurrect(
        &self,
        fc: &'gc gc_arena::Finalization<'gc>,
        visited: &mut HashSet<usize>,
        depth: usize,
    ) {
        match self {
            ValueType::Pointer(p) => p.resurrect(fc, visited, depth),
            ValueType::TypedRef(p, _) => p.resurrect(fc, visited, depth),
            ValueType::Struct(o) => o.resurrect(fc, visited, depth),
            _ => {}
        }
    }
}

#[derive(Debug)]
pub enum CTSValue<'gc> {
    Value(ValueType<'gc>),
    Ref(super::ObjectRef<'gc>),
}

impl<'gc> CTSValue<'gc> {
    pub fn into_stack(self) -> StackValue<'gc> {
        use CTSValue::*;
        use ValueType::*;
        match self {
            Value(Bool(b)) => StackValue::Int32(b as i32),
            Value(Char(c)) => StackValue::Int32(c as i32),
            Value(Int8(i)) => StackValue::Int32(i as i32),
            Value(UInt8(i)) => StackValue::Int32(i as i32),
            Value(Int16(i)) => StackValue::Int32(i as i32),
            Value(UInt16(i)) => StackValue::Int32(i as i32),
            Value(Int32(i)) => StackValue::Int32(i),
            Value(UInt32(i)) => StackValue::Int32(i as i32),
            Value(Int64(i)) => StackValue::Int64(i),
            Value(UInt64(i)) => StackValue::Int64(i as i64),
            Value(NativeInt(i)) => StackValue::NativeInt(i),
            Value(NativeUInt(i)) => StackValue::NativeInt(i as isize),
            Value(Pointer(p)) => StackValue::ManagedPtr(p),
            Value(Float32(f)) => StackValue::NativeFloat(f as f64),
            Value(Float64(f)) => StackValue::NativeFloat(f),
            Value(TypedRef(p, t)) => StackValue::TypedRef(p, t),
            Value(Struct(s)) => StackValue::ValueType(s),
            Ref(o) => StackValue::ObjectRef(o),
        }
    }

    pub fn write(&self, dest: &mut [u8]) {
        use ValueType::*;
        match self {
            CTSValue::Value(v) => match v {
                Bool(b) => dest.copy_from_slice(&[*b as u8]),
                Char(c) => dest.copy_from_slice(&c.to_ne_bytes()),
                Int8(i) => dest.copy_from_slice(&i.to_ne_bytes()),
                UInt8(i) => dest.copy_from_slice(&i.to_ne_bytes()),
                Int16(i) => dest.copy_from_slice(&i.to_ne_bytes()),
                UInt16(i) => dest.copy_from_slice(&i.to_ne_bytes()),
                Int32(i) => dest.copy_from_slice(&i.to_ne_bytes()),
                UInt32(i) => dest.copy_from_slice(&i.to_ne_bytes()),
                Int64(i) => dest.copy_from_slice(&i.to_ne_bytes()),
                UInt64(i) => dest.copy_from_slice(&i.to_ne_bytes()),
                NativeInt(i) => dest.copy_from_slice(&i.to_ne_bytes()),
                NativeUInt(i) => dest.copy_from_slice(&i.to_ne_bytes()),
                Pointer(p) => {
                    // Write full ManagedPtr (16 bytes: Owner + Pointer)
                    p.write(dest);
                }
                Float32(f) => dest.copy_from_slice(&f.to_ne_bytes()),
                Float64(f) => dest.copy_from_slice(&f.to_ne_bytes()),
                TypedRef(p, t) => {
                    let addr = unsafe { p.with_data(0, |data| data.as_ptr() as usize) };
                    let type_ptr = Arc::as_ptr(t) as usize;
                    dest[0..8].copy_from_slice(&addr.to_ne_bytes());
                    dest[8..16].copy_from_slice(&type_ptr.to_ne_bytes());
                }
                Struct(o) => o.instance_storage.with_data(|d| dest.copy_from_slice(d)),
            },
            CTSValue::Ref(o) => o.write(dest),
        }
    }
}

#[cfg(any(feature = "memory-validation", debug_assertions))]
pub(crate) const VECTOR_MAGIC: u64 = 0x5AFE_7EC7_0B00_0000;

// Manual implementation of Clone and PartialEq to handle ThreadSafeLock
pub struct Vector<'gc> {
    pub(crate) magic: ValidationTag,
    pub element: ConcreteType,
    pub layout: ArrayLayoutManager,
    pub(crate) storage: Vec<u8>,
    pub dims: Box<[usize]>,
    pub(crate) _contains_gc: PhantomData<fn(&'gc ()) -> &'gc ()>,
}

impl<'gc> Clone for Vector<'gc> {
    fn clone(&self) -> Self {
        self.validate_magic();
        Self {
            magic: ValidationTag::new(VECTOR_MAGIC),
            element: self.element.clone(),
            layout: self.layout.clone(),
            storage: self.storage.clone(),
            dims: self.dims.clone(),
            _contains_gc: PhantomData,
        }
    }
}

impl<'gc> PartialEq for Vector<'gc> {
    fn eq(&self, other: &Self) -> bool {
        self.validate_magic();
        other.validate_magic();
        self.element == other.element
            && self.layout == other.layout
            && self.storage == other.storage
            && self.dims == other.dims
    }
}

// SAFETY: Vector contains raw byte storage that may hold GC pointers (ObjectRef).
// We use the layout manager to identify and trace any such pointers.
unsafe impl<'gc> Collect<'gc> for Vector<'gc> {
    #[inline]
    fn trace<Tr: Trace<'gc>>(&self, cc: &mut Tr) {
        self.validate_magic();
        let element = &self.layout.element_layout;
        match element.as_ref() {
            LayoutManager::Scalar(Scalar::ObjectRef) => {
                for i in 0..self.layout.length {
                    unsafe {
                        super::ObjectRef::read_unchecked(
                            &self.storage[(element.size() * i).as_usize()..],
                        )
                    }
                    .trace(cc);
                }
            }
            _ => {
                for i in 0..self.layout.length {
                    LayoutManager::trace(
                        element,
                        &self.storage[(element.size() * i).as_usize()..],
                        cc,
                    );
                }
            }
        }
    }
}

impl<'gc> Vector<'gc> {
    pub fn new(
        element: ConcreteType,
        layout: ArrayLayoutManager,
        storage: Vec<u8>,
        dims: Vec<usize>,
    ) -> Self {
        Self {
            magic: ValidationTag::new(VECTOR_MAGIC),
            element,
            layout,
            storage,
            dims: dims.into_boxed_slice(),
            _contains_gc: PhantomData,
        }
    }

    fn validate_magic(&self) {
        self.magic.validate(VECTOR_MAGIC, "Vector");
    }

    pub fn size_bytes(&self) -> usize {
        self.validate_magic();
        size_of::<Vector>() + self.storage.len() + (self.dims.len() * size_of::<usize>())
    }

    pub fn validate_resurrection_invariants(&self) {
        self.validate_magic();
        let actual_size = self.storage.len();
        let expected_size = self.layout.size();
        if actual_size != expected_size.as_usize() {
            panic!(
                "Vector storage size mismatch during resurrection: actual {}, expected {}",
                actual_size, expected_size
            );
        }
    }

    pub fn resurrect(
        &self,
        fc: &'gc gc_arena::Finalization<'gc>,
        visited: &mut HashSet<usize>,
        depth: usize,
    ) {
        self.validate_magic();
        self.layout.resurrect(&self.storage, fc, visited, depth);
    }

    pub fn get(&self) -> &[u8] {
        self.validate_magic();
        &self.storage
    }

    pub fn get_mut(&mut self) -> &mut [u8] {
        self.validate_magic();
        &mut self.storage
    }

    /// Returns a pointer to the raw data without acquiring a lock.
    ///
    /// # Safety
    /// The caller must ensure that the lock is held elsewhere (e.g. during STW GC)
    /// or that the data is otherwise stable and no writers are active.
    pub unsafe fn raw_data_ptr(&self) -> *mut u8 {
        self.storage.as_ptr() as *mut u8
    }
}

impl Debug for Vector<'_> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_list()
            .entries(
                iter::once(format!(
                    "vector of {:?} (length {})",
                    self.element, self.layout.length
                ))
                .chain(
                    self.storage
                        .chunks(self.layout.element_layout.size().as_usize())
                        .map(match self.layout.element_layout.as_ref() {
                            LayoutManager::Scalar(Scalar::ObjectRef) => |chunk: &[u8]| {
                                format!("{:?}", unsafe { super::ObjectRef::read_unchecked(chunk) })
                            },
                            LayoutManager::Scalar(Scalar::ManagedPtr) => {
                                |chunk: &[u8]| {
                                    // ManagedPtr is now printed via its Debug impl if we could read it.
                                    // But read_unchecked returns ObjectRef.
                                    // We need ManagedPtr::read_from_bytes.
                                    // For now just print bytes to avoid issues.
                                    let bytes: Vec<_> =
                                        chunk.iter().map(|b| format!("{:02x}", b)).collect();
                                    format!("ptr({})", bytes.join(" "))
                                }
                            }
                            _ => |chunk: &[u8]| {
                                let bytes: Vec<_> =
                                    chunk.iter().map(|b| format!("{:02x}", b)).collect();
                                bytes.join(" ")
                            },
                        }),
                )
                .map(DebugStr),
            )
            .finish()
    }
}

pub struct Object<'gc> {
    pub description: TypeDescription,
    pub generics: GenericLookup,
    pub instance_storage: FieldStorage,
    pub finalizer_suppressed: bool,
    /// Sync block index for System.Threading.Monitor support.
    /// None means no sync block allocated yet (lazy allocation).
    pub sync_block_index: Option<usize>,
    pub _phantom: PhantomData<&'gc ()>,
}

#[cfg(feature = "fuzzing")]
impl<'a, 'gc> Arbitrary<'a> for Object<'gc> {
    fn arbitrary(u: &mut arbitrary::Unstructured<'a>) -> arbitrary::Result<Self> {
        Ok(Self {
            description: u.arbitrary()?,
            generics: u.arbitrary()?,
            instance_storage: FieldStorage::new(
                Arc::new(crate::layout::FieldLayoutManager {
                    fields: std::collections::HashMap::new(),
                    total_size: 0,
                    alignment: 1,
                    gc_desc: crate::layout::GcDesc::default(),
                    has_ref_fields: false,
                }),
                vec![],
            ),
            finalizer_suppressed: u.arbitrary()?,
            sync_block_index: u.arbitrary()?,
            _phantom: PhantomData,
        })
    }
}

impl<'gc> Clone for Object<'gc> {
    fn clone(&self) -> Self {
        Self {
            description: self.description,
            generics: self.generics.clone(),
            instance_storage: self.instance_storage.clone(),
            finalizer_suppressed: self.finalizer_suppressed,
            sync_block_index: self.sync_block_index,
            _phantom: PhantomData,
        }
    }
}

impl<'gc> PartialEq for Object<'gc> {
    fn eq(&self, other: &Self) -> bool {
        self.description == other.description
            && self.generics == other.generics
            && self.instance_storage == other.instance_storage
            && self.finalizer_suppressed == other.finalizer_suppressed
            && self.sync_block_index == other.sync_block_index
    }
}

impl<'gc> Eq for Object<'gc> {}

// SAFETY: Object contains field storage that may hold GC pointers.
// We use the layout manager associated with the type description to trace fields.
unsafe impl<'gc> Collect<'gc> for Object<'gc> {
    fn trace<Tr: Trace<'gc>>(&self, cc: &mut Tr) {
        self.instance_storage.trace(cc);
        // ManagedPtr fields are self-contained and traced via their layout.
    }
}

impl<'gc> Object<'gc> {
    pub fn size_bytes(&self) -> usize {
        size_of::<Object>() + unsafe { self.instance_storage.raw_data_unsynchronized().len() }
    }

    /// Safely accesses the object's data as a byte slice.
    pub fn with_data<T>(&self, f: impl FnOnce(&[u8]) -> T) -> T {
        self.instance_storage.with_data(f)
    }

    pub fn validate_resurrection_invariants(&self) {
        let actual_size = unsafe { self.instance_storage.raw_data_unsynchronized().len() };
        let expected_size = self.instance_storage.layout().total_size;
        if actual_size != expected_size {
            panic!(
                "Object storage size mismatch during resurrection: actual {}, expected {}",
                actual_size, expected_size
            );
        }
    }

    pub fn resurrect(
        &self,
        fc: &'gc gc_arena::Finalization<'gc>,
        visited: &mut HashSet<usize>,
        depth: usize,
    ) {
        self.instance_storage.resurrect(fc, visited, depth);
    }

    pub fn new(
        description: TypeDescription,
        generics: GenericLookup,
        instance_storage: FieldStorage,
    ) -> Self {
        Self {
            description,
            generics,
            instance_storage,
            finalizer_suppressed: false,
            sync_block_index: None,
            _phantom: PhantomData,
        }
    }
}

impl Debug for Object<'_> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_tuple(&self.description.type_name())
            .field(&self.instance_storage)
            .field(&DebugStr(format!(
                "stored at {:#?}",
                self.instance_storage.with_data(|d| d.as_ptr())
            )))
            .finish()
    }
}

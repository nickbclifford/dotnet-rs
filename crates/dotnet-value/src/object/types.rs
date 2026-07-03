use crate::{
    StackValue,
    cts_cli_conversion::CtsToCli,
    layout::{ArrayLayoutManager, HasLayout, LayoutManager, Scalar},
    pointer::ManagedPtr,
    storage::FieldStorage,
};

#[cfg(any(feature = "memory-validation", debug_assertions))]
use crate::ValidationTag;
use dotnet_types::{
    TypeDescription,
    generics::{ConcreteType, GenericLookup},
};
use dotnet_utils::DebugStr;
use gc_arena::{Collect, Mutation, collect::Trace};
use std::{
    collections::HashSet,
    fmt::{self, Debug, Formatter},
    iter,
    marker::PhantomData,
    mem::size_of,
    sync::{Arc, LazyLock},
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
        CtsToCli::widen(self)
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

static TRACE_GC_PTR_READ: LazyLock<bool> =
    LazyLock::new(|| std::env::var("DOTNET_TRACE_GC_PTR_READ").is_ok());

// Manual implementation of Clone and PartialEq to handle ThreadSafeLock
pub struct Vector<'gc> {
    #[cfg(any(feature = "memory-validation", debug_assertions))]
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
            #[cfg(any(feature = "memory-validation", debug_assertions))]
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
            #[cfg(any(feature = "memory-validation", debug_assertions))]
            magic: ValidationTag::new(VECTOR_MAGIC),
            element,
            layout,
            storage,
            dims: dims.into_boxed_slice(),
            _contains_gc: PhantomData,
        }
    }

    fn validate_magic(&self) {
        #[cfg(any(feature = "memory-validation", debug_assertions))]
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

    /// Iterates this vector's bytes as serialized [`super::ObjectRef`] elements.
    ///
    /// The iterator advances in `ObjectRef::SIZE` chunks and brands each element
    /// with the supplied GC mutation context.
    ///
    /// # Invariants
    ///
    /// This helper is only valid for vectors whose element layout is object
    /// references. Consume it inside the current `as_vector` borrow scope so the
    /// branded references do not outlive the active GC handle.
    pub fn object_ref_elements<'a>(
        &'a self,
        gc: &'a Mutation<'gc>,
    ) -> impl ExactSizeIterator<Item = super::ObjectRef<'gc>> + 'a {
        self.validate_magic();
        let chunks = self.storage.chunks_exact(super::ObjectRef::SIZE);
        debug_assert!(chunks.remainder().is_empty());
        chunks.map(move |chunk| unsafe { super::ObjectRef::read_branded(chunk, gc) })
    }

    /// Writes serialized object-reference elements into this vector in order.
    ///
    /// Uses `ObjectRef::SIZE` stride centrally so callers do not repeat manual
    /// offset arithmetic.
    ///
    /// # Panics
    ///
    /// Panics if `elements` contains more entries than this vector can store.
    pub fn write_object_ref_elements_mut(&mut self, elements: &[super::ObjectRef<'gc>]) {
        self.validate_magic();
        debug_assert_eq!(self.storage.len() % super::ObjectRef::SIZE, 0);
        let mut chunks = self.storage.chunks_exact_mut(super::ObjectRef::SIZE);
        let vector_len = self.layout.length;

        for element in elements {
            let chunk = chunks.next().unwrap_or_else(|| {
                panic!(
                    "write_object_ref_elements_mut received {} elements for vector length {}",
                    elements.len(),
                    vector_len
                )
            });
            element.write(chunk);
        }
    }

    /// Returns a pointer to the raw data without taking a field access guard.
    ///
    /// # Safety
    /// The caller must ensure synchronization is provided elsewhere (e.g. during STW GC)
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
    pub has_finalizer: bool,
    pub finalizer_suppressed: bool,
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
                    fields: hashbrown::HashMap::new(),
                    total_size: 0,
                    alignment: 1,
                    gc_desc: crate::layout::GcDesc::default(),
                    has_ref_fields: false,
                }),
                vec![],
            ),
            has_finalizer: u.arbitrary()?,
            finalizer_suppressed: u.arbitrary()?,
            _phantom: PhantomData,
        })
    }
}

impl<'gc> Clone for Object<'gc> {
    fn clone(&self) -> Self {
        Self {
            description: self.description.clone(),
            generics: self.generics.clone(),
            instance_storage: self.instance_storage.clone(),
            has_finalizer: self.has_finalizer,
            finalizer_suppressed: self.finalizer_suppressed,
            _phantom: PhantomData,
        }
    }
}

impl<'gc> PartialEq for Object<'gc> {
    fn eq(&self, other: &Self) -> bool {
        self.description == other.description
            && self.generics == other.generics
            && self.instance_storage == other.instance_storage
            && self.has_finalizer == other.has_finalizer
            && self.finalizer_suppressed == other.finalizer_suppressed
    }
}

impl<'gc> Eq for Object<'gc> {}

// SAFETY: Object contains field storage that may hold GC pointers.
// We use the layout manager associated with the type description to trace fields.
unsafe impl<'gc> Collect<'gc> for Object<'gc> {
    fn trace<Tr: Trace<'gc>>(&self, cc: &mut Tr) {
        if *TRACE_GC_PTR_READ {
            eprintln!(
                "[GC] tracing Object type={:?} generics={:?}",
                self.description, self.generics
            );
        }
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
        Self::new_with_finalizer_flag(description, generics, instance_storage, false)
    }

    pub fn new_with_finalizer_flag(
        description: TypeDescription,
        generics: GenericLookup,
        instance_storage: FieldStorage,
        has_finalizer: bool,
    ) -> Self {
        Self {
            description,
            generics,
            instance_storage,
            has_finalizer,
            finalizer_suppressed: false,
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

#[cfg(test)]
mod vector_object_ref_tests {
    use super::*;
    use crate::{
        layout::{ArrayLayoutManager, LayoutManager, Scalar},
        object::{HeapStorage, ObjectRef},
        test_helpers::with_test_gc_context,
    };
    use dotnet_types::{generics::ConcreteType, resolution::ResolutionS};
    use dotnetdll::prelude::BaseType;

    fn object_ref_vector<'gc>(length: usize, fill_byte: u8) -> Vector<'gc> {
        Vector::new(
            ConcreteType::new(ResolutionS::NULL, BaseType::Object),
            ArrayLayoutManager {
                element_layout: Arc::new(LayoutManager::Scalar(Scalar::ObjectRef)),
                length,
            },
            vec![fill_byte; length * ObjectRef::SIZE],
            vec![length],
        )
    }

    fn collect_object_ref_elements<'gc>(
        vector: &Vector<'gc>,
        gc: &Mutation<'gc>,
    ) -> Vec<ObjectRef<'gc>> {
        vector.object_ref_elements(gc).collect()
    }

    #[test]
    fn object_ref_elements_reads_pointer_sized_stride_with_gc_branding() {
        with_test_gc_context(|gc| {
            let first = ObjectRef::new(gc, HeapStorage::Str(crate::string::CLRString::from("a")));
            let second = ObjectRef::new(gc, HeapStorage::Str(crate::string::CLRString::from("b")));
            let mut vector = object_ref_vector(3, 0);

            vector.write_object_ref_elements_mut(&[first, ObjectRef(None), second]);

            let collected = collect_object_ref_elements(&vector, &gc);
            assert_eq!(collected, vec![first, ObjectRef(None), second]);
        });
    }

    #[test]
    fn write_object_ref_elements_mut_uses_pointer_stride_and_preserves_tail() {
        with_test_gc_context(|gc| {
            let first = ObjectRef::new(gc, HeapStorage::Str(crate::string::CLRString::from("x")));
            let second = ObjectRef::new(gc, HeapStorage::Str(crate::string::CLRString::from("y")));
            let mut vector = object_ref_vector(3, 0x01);

            vector.write_object_ref_elements_mut(&[first, second]);

            let mut iter = vector.object_ref_elements(&gc);
            assert_eq!(iter.next(), Some(first));
            assert_eq!(iter.next(), Some(second));

            let tail = &vector.get()[2 * ObjectRef::SIZE..3 * ObjectRef::SIZE];
            assert_eq!(tail, &[0x01; ObjectRef::SIZE]);
        });
    }
}

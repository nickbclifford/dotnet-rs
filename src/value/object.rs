use crate::{
    types::{TypeDescription, generics::ConcreteType},
    utils::DebugStr,
    value::{
        StackValue, layout::{ArrayLayoutManager, HasLayout, LayoutManager, Scalar},
        pointer::{ManagedPtr, UnmanagedPtr},
        storage::FieldStorage, string::CLRString,
    },
    vm::{GCHandle, context::ResolutionContext},
    vm_expect_stack,
};
use gc_arena::{Collect, Collection, Gc, lock::RefLock};
use std::{
    cmp::Ordering, collections::HashSet, fmt::{Debug, Formatter},
    hash::{Hash, Hasher},
    marker::PhantomData,
};

type ObjectInner<'gc> = RefLock<HeapStorage<'gc>>;
pub type ObjectPtr = *const ObjectInner<'static>;
pub type ObjectHandle<'gc> = Gc<'gc, ObjectInner<'gc>>;

#[derive(Copy, Clone)]
#[repr(transparent)]
pub struct ObjectRef<'gc>(pub Option<ObjectHandle<'gc>>);

unsafe impl<'gc> Collect for ObjectRef<'gc> {
    fn trace(&self, cc: &Collection) {
        if let Some(h) = self.0 {
            h.trace(cc);
        }
    }
}

//noinspection RsAssertEqual
// we assume this type is pointer-sized basically everywhere
// dependency-wise everything guarantees it; this is just a sanity check for the implementation
const _: () = assert!(ObjectRef::SIZE == size_of::<usize>());

impl PartialEq for ObjectRef<'_> {
    fn eq(&self, other: &Self) -> bool {
        match (self.0, other.0) {
            (Some(l), Some(r)) => Gc::ptr_eq(l, r),
            (None, None) => true,
            _ => false,
        }
    }
}

impl PartialOrd for ObjectRef<'_> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        match (self.0, other.0) {
            (Some(l), Some(r)) => Gc::as_ptr(l).partial_cmp(&Gc::as_ptr(r)),
            (None, None) => Some(Ordering::Equal),
            (None, _) => Some(Ordering::Less),
            (_, None) => Some(Ordering::Greater),
        }
    }
}

impl Eq for ObjectRef<'_> {}

impl Hash for ObjectRef<'_> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        match self.0 {
            Some(g) => Gc::as_ptr(g).hash(state),
            None => 0usize.hash(state),
        }
    }
}

impl<'gc> ObjectRef<'gc> {
    pub const SIZE: usize = size_of::<ObjectRef>();

    pub fn resurrect(&self, fc: &gc_arena::Finalization<'gc>, visited: &mut HashSet<usize>) {
        if let Some(handle) = self.0 {
            let ptr = Gc::as_ptr(handle) as usize;
            if visited.insert(ptr) {
                Gc::resurrect(fc, handle);
                handle.borrow().resurrect(fc, visited);
            }
        }
    }

    pub fn new(gc: GCHandle<'gc>, value: HeapStorage<'gc>) -> Self {
        Self(Some(Gc::new(gc, RefLock::new(value))))
    }

    pub fn read(source: &[u8]) -> Self {
        let mut ptr_bytes = [0u8; Self::SIZE];
        ptr_bytes.copy_from_slice(&source[0..Self::SIZE]);
        let ptr = usize::from_ne_bytes(ptr_bytes) as *const ObjectInner<'gc>;

        if ptr.is_null() {
            ObjectRef(None)
        } else {
            // SAFETY: since this came from Gc::as_ptr, we know it's valid
            // also, this will only ever be called inside the context of a GC mutation, so it's okay for 'gc to be unbounded
            ObjectRef(Some(unsafe { Gc::from_ptr(ptr) }))
        }
    }

    pub fn write(&self, dest: &mut [u8]) {
        let ptr: *const RefLock<_> = match self.0 {
            None => std::ptr::null(),
            Some(s) => Gc::as_ptr(s),
        };
        let ptr_bytes = (ptr as usize).to_ne_bytes();
        dest[..ptr_bytes.len()].copy_from_slice(&ptr_bytes);
    }

    pub fn expect_object_ref(self) -> Self {
        if self.0.is_none() {
            panic!("NullReferenceException");
        }
        self
    }

    pub fn as_object<T>(&self, op: impl FnOnce(&Object<'gc>) -> T) -> T {
        let ObjectRef(Some(o)) = &self else {
            panic!("NullReferenceException: called ObjectRef::as_object on NULL object reference")
        };
        let heap = o.borrow();
        let HeapStorage::Obj(instance) = &*heap else {
            panic!("called ObjectRef::as_object on non-object heap reference")
        };

        op(instance)
    }

    pub fn as_object_mut<T>(&self, gc: GCHandle<'gc>, op: impl FnOnce(&mut Object<'gc>) -> T) -> T {
        let ObjectRef(Some(o)) = &self else {
            panic!(
                "NullReferenceException: called ObjectRef::as_object_mut on NULL object reference"
            )
        };
        let mut heap = o.borrow_mut(gc);
        let HeapStorage::Obj(instance) = &mut *heap else {
            panic!("called ObjectRef::as_object_mut on non-object heap reference")
        };

        op(instance)
    }

    pub fn as_vector<T>(&self, op: impl FnOnce(&Vector<'gc>) -> T) -> T {
        let ObjectRef(Some(o)) = &self else {
            panic!("NullReferenceException: called ObjectRef::as_vector on NULL object reference")
        };
        let heap = o.borrow();
        let HeapStorage::Vec(instance) = &*heap else {
            panic!("called ObjectRef::as_vector on non-vector heap reference")
        };

        op(instance)
    }
}

impl Debug for ObjectRef<'_> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self.0 {
            None => f.write_str("NULL"),
            Some(gc) => {
                let handle = gc.borrow();
                let desc = match &*handle {
                    HeapStorage::Obj(o) => o.description.type_name(),
                    HeapStorage::Vec(v) => format!("{:?}[{}]", v.element, v.layout.length),
                    HeapStorage::Str(s) => format!("{:?}", s),
                    HeapStorage::Boxed(v) => format!("boxed {:?}", v),
                };
                write!(f, "{} @ {:#?}", desc, Gc::as_ptr(gc))
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum HeapStorage<'gc> {
    Vec(Vector<'gc>),
    Obj(Object<'gc>),
    Str(CLRString),
    Boxed(ValueType<'gc>),
}

unsafe impl<'gc> Collect for HeapStorage<'gc> {
    fn trace(&self, cc: &Collection) {
        match self {
            Self::Vec(v) => v.trace(cc),
            Self::Obj(o) => o.trace(cc),
            Self::Boxed(v) => v.trace(cc),
            Self::Str(_) => {}
        }
    }
}

impl<'gc> HeapStorage<'gc> {
    pub fn resurrect(&self, fc: &gc_arena::Finalization<'gc>, visited: &mut HashSet<usize>) {
        match self {
            HeapStorage::Vec(v) => v.resurrect(fc, visited),
            HeapStorage::Obj(o) => o.resurrect(fc, visited),
            HeapStorage::Boxed(v) => v.resurrect(fc, visited),
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
}

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
    TypedRef, // TODO
    Struct(Object<'gc>),
}

unsafe impl<'gc> Collect for ValueType<'gc> {
    fn trace(&self, cc: &Collection) {
        match self {
            Self::Pointer(p) => p.trace(cc),
            Self::Struct(o) => o.trace(cc),
            _ => {}
        }
    }
}

fn convert_num<T: TryFrom<i32> + TryFrom<isize> + TryFrom<usize>>(data: StackValue) -> T {
    match data {
        StackValue::Int32(i) => i
            .try_into()
            .unwrap_or_else(|_| panic!("failed to convert from i32")),
        StackValue::NativeInt(i) => i
            .try_into()
            .unwrap_or_else(|_| panic!("failed to convert from isize")),
        StackValue::UnmanagedPtr(UnmanagedPtr(p))
        | StackValue::ManagedPtr(ManagedPtr { value: p, .. }) => (p as usize)
            .try_into()
            .unwrap_or_else(|_| panic!("failed to convert from pointer")),
        other => panic!(
            "invalid stack value {:?} for conversion into {}",
            other,
            std::any::type_name::<T>()
        ),
    }
}

fn convert_i64<T: TryFrom<i64>>(data: StackValue) -> T
where
    T::Error: std::error::Error,
{
    match data {
        StackValue::Int64(i) => i.try_into().unwrap_or_else(|e| {
            panic!(
                "failed to convert from i64 to {} ({})",
                std::any::type_name::<T>(),
                e
            )
        }),
        other => panic!("invalid stack value {:?} for integer conversion", other),
    }
}

impl<'gc> ValueType<'gc> {
    pub fn resurrect(&self, fc: &gc_arena::Finalization<'gc>, visited: &mut HashSet<usize>) {
        match self {
            ValueType::Pointer(p) => p.resurrect(fc, visited),
            ValueType::Struct(o) => o.resurrect(fc, visited),
            _ => {}
        }
    }
    pub fn new(t: &ConcreteType, context: &ResolutionContext, data: StackValue<'gc>) -> Self {
        match CTSValue::new(t, context, data) {
            CTSValue::Value(v) => v,
            CTSValue::Ref(r) => {
                panic!(
                    "tried to instantiate value type, received object reference ({:?})",
                    r
                )
            }
        }
    }

    pub fn description(&self, context: &ResolutionContext) -> TypeDescription {
        let asms = &context.loader;
        match self {
            ValueType::Bool(_) => asms.corlib_type("System.Boolean"),
            ValueType::Char(_) => asms.corlib_type("System.Char"),
            ValueType::Int8(_) => asms.corlib_type("System.SByte"),
            ValueType::UInt8(_) => asms.corlib_type("System.Byte"),
            ValueType::Int16(_) => asms.corlib_type("System.Int16"),
            ValueType::UInt16(_) => asms.corlib_type("System.UInt16"),
            ValueType::Int32(_) => asms.corlib_type("System.Int32"),
            ValueType::UInt32(_) => asms.corlib_type("System.UInt32"),
            ValueType::Int64(_) => asms.corlib_type("System.Int64"),
            ValueType::UInt64(_) => asms.corlib_type("System.UInt64"),
            ValueType::NativeInt(_) => asms.corlib_type("System.IntPtr"),
            ValueType::NativeUInt(_) => asms.corlib_type("System.UIntPtr"),
            ValueType::Pointer(_) => asms.corlib_type("System.IntPtr"),
            ValueType::Float32(_) => asms.corlib_type("System.Single"),
            ValueType::Float64(_) => asms.corlib_type("System.Double"),
            ValueType::TypedRef => asms.corlib_type("System.TypedReference"),
            ValueType::Struct(s) => s.description,
        }
    }
}

macro_rules! from_bytes {
    ($t:ty, $data:expr) => {
        <$t>::from_ne_bytes($data.try_into().expect("source data was too small"))
    };
}

#[derive(Debug)]
pub enum CTSValue<'gc> {
    Value(ValueType<'gc>),
    Ref(ObjectRef<'gc>),
}

impl<'gc> CTSValue<'gc> {
    pub fn new(t: &ConcreteType, context: &ResolutionContext, data: StackValue<'gc>) -> Self {
        use crate::types::generics::GenericLookup;
        use crate::utils::decompose_type_source;
        use dotnetdll::prelude::{BaseType, ValueKind};
        use ValueType::*;
        let t = context.normalize_type(t.clone());
        match t.get() {
            BaseType::Boolean => Self::Value(Bool(convert_num::<u8>(data) != 0)),
            BaseType::Char => Self::Value(Char(convert_num(data))),
            BaseType::Int8 => Self::Value(Int8(convert_num(data))),
            BaseType::UInt8 => Self::Value(UInt8(convert_num(data))),
            BaseType::Int16 => Self::Value(Int16(convert_num(data))),
            BaseType::UInt16 => Self::Value(UInt16(convert_num(data))),
            BaseType::Int32 => Self::Value(Int32(convert_num(data))),
            BaseType::UInt32 => Self::Value(UInt32(convert_num(data))),
            BaseType::Int64 => Self::Value(Int64(convert_i64(data))),
            BaseType::UInt64 => Self::Value(UInt64(convert_i64(data))),
            BaseType::Float32 => Self::Value(Float32(match data {
                StackValue::NativeFloat(f) => f as f32,
                other => panic!("invalid stack value {:?} for float conversion", other),
            })),
            BaseType::Float64 => Self::Value(Float64(match data {
                StackValue::NativeFloat(f) => f,
                other => panic!("invalid stack value {:?} for float conversion", other),
            })),
            BaseType::IntPtr => Self::Value(NativeInt(convert_num(data))),
            BaseType::UIntPtr | BaseType::FunctionPointer(_) => {
                Self::Value(NativeUInt(convert_num(data)))
            }
            BaseType::ValuePointer(_, _) => {
                vm_expect_stack!(let ManagedPtr(p) = data);
                Self::Value(Pointer(p))
            }
            BaseType::Object | BaseType::String | BaseType::Vector(_, _) => {
                vm_expect_stack!(let ObjectRef(o) = data);
                Self::Ref(o)
            }
            BaseType::Type {
                value_kind: Some(ValueKind::Class),
                ..
            } => {
                vm_expect_stack!(let ObjectRef(o) = data);
                Self::Ref(o)
            }
            BaseType::Type {
                value_kind: None | Some(ValueKind::ValueType),
                source,
            } => {
                let (ut, type_generics) = decompose_type_source(source);
                let new_lookup = GenericLookup::new(type_generics);
                let new_ctx = context.with_generics(&new_lookup);
                let td = new_ctx.locate_type(ut);

                if let Some(e) = td.is_enum() {
                    let enum_type = new_ctx.make_concrete(e);
                    return CTSValue::new(&enum_type, context, data);
                }

                if td.type_name() == "System.TypedReference" {
                    return Self::Value(TypedRef);
                }

                vm_expect_stack!(let ValueType(o) = data);
                if !new_ctx.is_a(o.description, td) {
                    panic!(
                        "type mismatch: expected {:?}, found {:?}",
                        td, o.description
                    );
                }
                Self::Value(Struct(*o))
            }
            rest => panic!("tried to deserialize StackValue {:?}", rest),
        }
    }

    pub fn read(t: &ConcreteType, context: &ResolutionContext, data: &[u8]) -> Self {
        use crate::types::generics::GenericLookup;
        use crate::utils::decompose_type_source;
        use dotnetdll::prelude::{BaseType, ValueKind};
        use ValueType::*;
        let t = context.normalize_type(t.clone());
        match t.get() {
            BaseType::Boolean => Self::Value(Bool(data[0] != 0)),
            BaseType::Char => Self::Value(Char(from_bytes!(u16, data))),
            BaseType::Int8 => Self::Value(Int8(data[0] as i8)),
            BaseType::UInt8 => Self::Value(UInt8(data[0])),
            BaseType::Int16 => Self::Value(Int16(from_bytes!(i16, data))),
            BaseType::UInt16 => Self::Value(UInt16(from_bytes!(u16, data))),
            BaseType::Int32 => Self::Value(Int32(from_bytes!(i32, data))),
            BaseType::UInt32 => Self::Value(UInt32(from_bytes!(u32, data))),
            BaseType::Int64 => Self::Value(Int64(from_bytes!(i64, data))),
            BaseType::UInt64 => Self::Value(UInt64(from_bytes!(u64, data))),
            BaseType::Float32 => Self::Value(Float32(from_bytes!(f32, data))),
            BaseType::Float64 => Self::Value(Float64(from_bytes!(f64, data))),
            BaseType::IntPtr => Self::Value(NativeInt(from_bytes!(isize, data))),
            BaseType::UIntPtr | BaseType::FunctionPointer(_) => {
                Self::Value(NativeUInt(from_bytes!(usize, data)))
            }
            BaseType::ValuePointer(_, _) => Self::Value(Pointer(ManagedPtr::read(data))),
            BaseType::Object
            | BaseType::String
            | BaseType::Vector(_, _)
            | BaseType::Array(_, _) => Self::Ref(ObjectRef::read(data)),
            BaseType::Type {
                value_kind: Some(ValueKind::Class),
                ..
            } => Self::Ref(ObjectRef::read(data)),
            BaseType::Type {
                value_kind: None | Some(ValueKind::ValueType),
                source,
            } => {
                let (ut, type_generics) = decompose_type_source(source);
                let new_lookup = GenericLookup::new(type_generics);
                let new_ctx = context.with_generics(&new_lookup);
                let td = new_ctx.locate_type(ut);

                if let Some(e) = td.is_enum() {
                    let enum_type = new_ctx.make_concrete(e);
                    return CTSValue::read(&enum_type, context, data);
                }

                if td.type_name() == "System.TypedReference" {
                    return Self::Value(TypedRef);
                }

                let mut instance = Object::new(td, &new_ctx);
                instance.instance_storage.get_mut().copy_from_slice(data);
                Self::Value(Struct(instance))
            }
        }
    }

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
            Value(TypedRef) => todo!(),
            Value(Struct(s)) => StackValue::ValueType(Box::new(s)),
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
                Pointer(p) => p.write(dest),
                Float32(f) => dest.copy_from_slice(&f.to_ne_bytes()),
                Float64(f) => dest.copy_from_slice(&f.to_ne_bytes()),
                TypedRef => todo!("typedref implementation"),
                Struct(o) => dest.copy_from_slice(o.instance_storage.get()),
            },
            CTSValue::Ref(o) => o.write(dest),
        }
    }
}

#[derive(Clone, PartialEq)]
pub struct Vector<'gc> {
    pub element: ConcreteType,
    pub layout: ArrayLayoutManager,
    storage: Vec<u8>,
    _contains_gc: PhantomData<&'gc ()>, // TODO: variance rules?
}

unsafe impl Collect for Vector<'_> {
    #[inline]
    fn trace(&self, cc: &Collection) {
        let element = &self.layout.element_layout;
        match element.as_ref() {
            LayoutManager::Scalar(Scalar::ObjectRef) => {
                for i in 0..self.layout.length {
                    ObjectRef::read(&self.storage[(i * element.size())..]).trace(cc);
                }
            }
            LayoutManager::Scalar(Scalar::ManagedPtr) => {
                // Skip tracing ManagedPtr arrays to avoid unsafe transmute issues
                // ManagedPtr values should not be stored in arrays
            }
            _ => {
                for i in 0..self.layout.length {
                    LayoutManager::trace(element, &self.storage[(i * element.size())..], cc);
                }
            }
        }
    }
}

impl<'gc> Vector<'gc> {
    pub fn resurrect(&self, fc: &gc_arena::Finalization<'gc>, visited: &mut HashSet<usize>) {
        self.layout.resurrect(&self.storage, fc, visited);
    }
    pub fn new(element: ConcreteType, size: usize, context: &ResolutionContext) -> Self {
        let layout = ArrayLayoutManager::new(element.clone(), size, context);
        Self {
            storage: vec![0; layout.size()], // TODO: initialize properly
            layout,
            element,
            _contains_gc: PhantomData,
        }
    }

    pub fn get(&self) -> &[u8] {
        &self.storage
    }

    pub fn get_mut(&mut self) -> &mut [u8] {
        &mut self.storage
    }
}

impl Debug for Vector<'_> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_list()
            .entries(
                std::iter::once(format!(
                    "vector of {:?} (length {})",
                    self.element, self.layout.length
                ))
                .chain(self.storage.chunks(self.layout.element_layout.size()).map(
                    match self.layout.element_layout.as_ref() {
                        LayoutManager::Scalar(Scalar::ObjectRef) => {
                            |chunk: &[u8]| format!("{:?}", ObjectRef::read(chunk))
                        }
                        LayoutManager::Scalar(Scalar::ManagedPtr) => {
                            |chunk: &[u8]| {
                                // Skip reading ManagedPtr to avoid transmute issues
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
                    },
                ))
                .map(DebugStr),
            )
            .finish()
    }
}

#[derive(Clone, PartialEq)]
pub struct Object<'gc> {
    pub description: TypeDescription,
    pub instance_storage: FieldStorage<'gc>,
    pub finalizer_suppressed: bool,
}

unsafe impl<'gc> Collect for Object<'gc> {
    fn trace(&self, cc: &Collection) {
        self.instance_storage.trace(cc);
    }
}

impl<'gc> Object<'gc> {
    pub fn resurrect(&self, fc: &gc_arena::Finalization<'gc>, visited: &mut HashSet<usize>) {
        self.instance_storage.resurrect(fc, visited);
    }
    pub fn new(description: TypeDescription, context: &ResolutionContext) -> Self {
        Self {
            description,
            instance_storage: FieldStorage::instance_fields(description, context),
            finalizer_suppressed: false,
        }
    }
}

impl Debug for Object<'_> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple(&self.description.type_name())
            .field(&self.instance_storage)
            .field(&DebugStr(format!(
                "stored at {:#?}",
                self.instance_storage.get().as_ptr()
            )))
            .finish()
    }
}

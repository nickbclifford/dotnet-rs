use std::{
    cmp::Ordering,
    fmt::{Debug, Formatter},
    hash::{Hash, Hasher},
    marker::PhantomData,
    mem::size_of,
};

use dotnetdll::prelude::*;
use gc_arena::{lock::RefLock, unsafe_empty_collect, Collect, Collection, Gc};

use crate::{
    resolve::{Ancestor, Assemblies},
    utils::{decompose_type_source, DebugStr, ResolutionS},
    value::string::CLRString,
    vm::{intrinsics::expect_stack, GCHandle},
};

pub mod layout;
pub mod storage;
pub mod string;

use layout::*;
use storage::FieldStorage;

#[derive(Clone)]
pub struct Context<'a> {
    pub generics: &'a GenericLookup,
    pub assemblies: &'a Assemblies,
    pub resolution: ResolutionS,
}
impl<'a> Context<'a> {
    pub const fn with_generics(ctx: Context<'a>, generics: &'a GenericLookup) -> Context<'a> {
        Context { generics, ..ctx }
    }

    pub fn locate_type(&self, handle: UserType) -> TypeDescription {
        self.assemblies.locate_type(self.resolution, handle)
    }

    pub fn locate_method(
        &self,
        handle: UserMethod,
        generic_inst: &GenericLookup,
    ) -> MethodDescription {
        self.assemblies
            .locate_method(self.resolution, handle, generic_inst)
    }

    pub fn locate_field(&self, field: FieldSource) -> (FieldDescription, GenericLookup) {
        self.assemblies
            .locate_field(self.resolution, field, self.generics)
    }

    pub fn find_method_in_type(
        &self,
        parent_type: TypeDescription,
        name: &str,
        signature: &ManagedMethod<MethodType>,
    ) -> Option<MethodDescription> {
        self.assemblies
            .find_method_in_type(parent_type, name, signature)
    }

    pub fn get_ancestors(
        &self,
        child_type: TypeDescription,
    ) -> impl Iterator<Item = Ancestor<'a>> + 'a {
        self.assemblies.ancestors(child_type)
    }

    pub fn is_a(&self, value: TypeDescription, ancestor: TypeDescription) -> bool {
        self.get_ancestors(value).any(|(a, _)| a == ancestor)
    }

    pub fn get_heap_description(&self, object: ObjectHandle) -> TypeDescription {
        use HeapStorage::*;
        match &*object.as_ref().borrow() {
            Obj(o) => o.description,
            Vec(_) => self.assemblies.corlib_type("System.Array"),
            Str(_) => self.assemblies.corlib_type("System.String"),
            Boxed(v) => v.description(self),
        }
    }

    pub fn make_concrete<T: Clone + Into<MethodType>>(&self, t: &T) -> ConcreteType {
        self.generics.make_concrete(self.resolution, t.clone())
    }

    pub fn get_field_type(&self, field: FieldDescription) -> ConcreteType {
        let return_type = &field.field.return_type;
        if field.field.by_ref {
            let by_ref_t: MemberType = BaseType::pointer(return_type.clone()).into();
            self.make_concrete(&by_ref_t)
        } else {
            self.make_concrete(return_type)
        }
    }

    pub fn get_field_desc(&self, field: FieldDescription) -> TypeDescription {
        self.assemblies
            .find_concrete_type(self.get_field_type(field))
    }
}

#[derive(Clone, Debug, Collect, PartialEq)]
#[collect(no_drop)]
pub enum StackValue<'gc> {
    Int32(i32),
    Int64(i64),
    NativeInt(isize),
    NativeFloat(f64),
    ObjectRef(ObjectRef<'gc>),
    UnmanagedPtr(UnmanagedPtr),
    ManagedPtr(ManagedPtr),
    ValueType(Box<Object<'gc>>),
}
impl<'gc> StackValue<'gc> {
    pub fn unmanaged_ptr(ptr: *mut u8) -> Self {
        Self::UnmanagedPtr(UnmanagedPtr(ptr))
    }
    pub fn managed_ptr(ptr: *mut u8, target_type: TypeDescription) -> Self {
        Self::ManagedPtr(ManagedPtr {
            value: ptr,
            inner_type: target_type,
        })
    }
    pub fn null() -> Self {
        Self::ObjectRef(ObjectRef(None))
    }
    pub fn string(gc: GCHandle<'gc>, s: CLRString) -> Self {
        Self::ObjectRef(ObjectRef(Some(Gc::new(
            gc,
            RefLock::new(HeapStorage::Str(s)),
        ))))
    }

    pub fn data_location(&self) -> *const u8 {
        fn ref_to_ptr<T>(r: &T) -> *const u8 {
            (r as *const T) as *const u8
        }

        match self {
            Self::Int32(i) => ref_to_ptr(i),
            Self::Int64(i) => ref_to_ptr(i),
            Self::NativeInt(i) => ref_to_ptr(i),
            Self::NativeFloat(f) => ref_to_ptr(f),
            Self::ObjectRef(ObjectRef(o)) => ref_to_ptr(o),
            Self::UnmanagedPtr(UnmanagedPtr(u)) => ref_to_ptr(u),
            Self::ManagedPtr(m) => ref_to_ptr(&m.value),
            Self::ValueType(o) => o.instance_storage.get().as_ptr(),
        }
    }

    pub fn contains_type(&self, ctx: Context) -> TypeDescription {
        match self {
            Self::Int32(_) => ctx.assemblies.corlib_type("System.Int32"),
            Self::Int64(_) => ctx.assemblies.corlib_type("System.Int64"),
            Self::NativeInt(_) | Self::UnmanagedPtr(_) => {
                ctx.assemblies.corlib_type("System.IntPtr")
            }
            Self::NativeFloat(_) => ctx.assemblies.corlib_type("System.Double"),
            Self::ObjectRef(ObjectRef(Some(o))) => ctx.get_heap_description(*o),
            Self::ObjectRef(ObjectRef(None)) => ctx.assemblies.corlib_type("System.Object"),
            Self::ManagedPtr(m) => m.inner_type,
            Self::ValueType(o) => o.description,
        }
    }
}
impl Default for StackValue<'_> {
    fn default() -> Self {
        Self::null()
    }
}
impl PartialOrd for StackValue<'_> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        use StackValue::*;
        match (self, other) {
            (Int32(l), Int32(r)) => l.partial_cmp(r),
            (Int32(l), NativeInt(r)) => (*l as isize).partial_cmp(r),
            (Int64(l), Int64(r)) => l.partial_cmp(r),
            (NativeInt(l), Int32(r)) => l.partial_cmp(&(*r as isize)),
            (NativeInt(l), NativeInt(r)) => l.partial_cmp(r),
            (NativeFloat(l), NativeFloat(r)) => l.partial_cmp(r),
            (ManagedPtr(l), ManagedPtr(r)) => l.partial_cmp(r),
            _ => None,
        }
    }
}

#[derive(Copy, Clone, Debug, PartialEq, PartialOrd)]
pub struct UnmanagedPtr(pub *mut u8);
#[derive(Copy, Clone)]
pub struct ManagedPtr {
    pub value: *mut u8,
    pub inner_type: TypeDescription,
}
impl Debug for ManagedPtr {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "[{}] {:#?}", self.inner_type.type_name(), self.value)
    }
}
impl PartialEq for ManagedPtr {
    fn eq(&self, other: &Self) -> bool {
        self.value == other.value
    }
}
impl PartialOrd for ManagedPtr {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        self.value.partial_cmp(&other.value)
    }
}
impl ManagedPtr {
    pub fn map_value(self, transform: impl FnOnce(*mut u8) -> *mut u8) -> Self {
        ManagedPtr {
            value: transform(self.value),
            inner_type: self.inner_type,
        }
    }
}

unsafe_empty_collect!(UnmanagedPtr);
unsafe_empty_collect!(ManagedPtr);

type ObjectInner<'gc> = RefLock<HeapStorage<'gc>>;
pub type ObjectPtr = *const ObjectInner<'static>;
pub type ObjectHandle<'gc> = Gc<'gc, ObjectInner<'gc>>;

#[derive(Copy, Clone, Collect)]
#[collect(no_drop)]
#[repr(transparent)]
pub struct ObjectRef<'gc>(pub Option<ObjectHandle<'gc>>);

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
impl<'gc> ObjectRef<'gc> {
    pub const SIZE: usize = size_of::<ObjectRef>();

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
        dest.copy_from_slice(&ptr_bytes);
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

#[derive(Clone, Debug, Collect)]
#[collect(no_drop)]
pub enum HeapStorage<'gc> {
    Vec(Vector<'gc>),
    Obj(Object<'gc>),
    Str(CLRString),
    Boxed(ValueType<'gc>),
}

#[derive(Clone, Debug, Collect)]
#[collect(no_drop)]
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
    Float32(f32),
    Float64(f64),
    TypedRef, // TODO
    Struct(Object<'gc>),
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
        // StackValue::ObjectRef(ObjectRef(Some(o))) => (Gc::as_ptr(o) as usize)
        //     .try_into()
        //     .unwrap_or_else(|_| panic!("failed to convert from pointer")),
        // StackValue::ObjectRef(ObjectRef(None)) => 0usize
        //     .try_into()
        //     .unwrap_or_else(|_| panic!("failed to convert from pointer")),
        other => panic!(
            "invalid stack value {:?} for conversion into {}",
            other,
            std::any::type_name::<T>()
        ),
    }
}
fn convert_i64<T: TryFrom<i64>>(data: StackValue) -> T {
    match data {
        StackValue::Int64(i) => i
            .try_into()
            .unwrap_or_else(|_| panic!("failed to convert from i64")),
        other => panic!("invalid stack value {:?} for integer conversion", other),
    }
}

impl<'gc> ValueType<'gc> {
    pub fn new(t: &ConcreteType, context: &Context, data: StackValue<'gc>) -> Self {
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

    pub fn description(&self, context: &Context) -> TypeDescription {
        let asms = &context.assemblies;
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
    pub fn new(t: &ConcreteType, context: &Context, data: StackValue<'gc>) -> Self {
        use ValueType::*;
        match t.get() {
            BaseType::Type {
                value_kind: None | Some(ValueKind::ValueType),
                source,
            } => {
                let (ut, type_generics) = decompose_type_source(source);
                let new_lookup = GenericLookup::new(type_generics);
                let new_ctx = Context::with_generics(context.clone(), &new_lookup);
                let td = new_ctx.locate_type(ut);

                if let Some(e) = td.is_enum() {
                    let enum_type = new_ctx.make_concrete(e);
                    return CTSValue::new(&enum_type, context, data);
                }

                let v = match td.type_name().as_str() {
                    "System.Boolean" => Bool(convert_num::<u8>(data) != 0),
                    "System.Char" => Char(convert_num(data)),
                    "System.Single" => Float32({
                        expect_stack!(let NativeFloat(f) = data);
                        f as f32
                    }),
                    "System.Double" => Float64({
                        expect_stack!(let NativeFloat(f) = data);
                        f
                    }),
                    "System.SByte" => Int8(convert_num(data)),
                    "System.Int16" => Int16(convert_num(data)),
                    "System.Int32" => Int32(convert_num(data)),
                    "System.Int64" => Int64(convert_i64(data)),
                    "System.IntPtr" => NativeInt(convert_num(data)),
                    "System.TypedReference" => TypedRef,
                    "System.Byte" => UInt8(convert_num(data)),
                    "System.UInt16" => UInt16(convert_num(data)),
                    "System.UInt32" => UInt32(convert_num(data)),
                    "System.UInt64" => UInt64(convert_i64(data)),
                    "System.UIntPtr" => NativeUInt(convert_num(data)),
                    _ => {
                        expect_stack!(let ValueType(o) = data);
                        if !new_ctx.is_a(o.description, td) {
                            panic!(
                                "type mismatch: expected {:?}, found {:?}",
                                td, o.description
                            );
                        }
                        Struct(*o)
                    }
                };

                Self::Value(v)
            }
            BaseType::Type {
                value_kind: Some(ValueKind::Class),
                ..
            }
            | BaseType::Object
            | BaseType::Vector(_, _)
            | BaseType::String => {
                expect_stack!(let ObjectRef(o) = data);
                Self::Ref(o)
            }
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
            BaseType::UIntPtr | BaseType::ValuePointer(_, _) | BaseType::FunctionPointer(_) => {
                Self::Value(NativeUInt(convert_num(data)))
            }
            rest => todo!("tried to deserialize StackValue {:?}", rest),
        }
    }

    pub fn read(t: &ConcreteType, context: &Context, data: &[u8]) -> Self {
        use ValueType::*;
        match t.get() {
            BaseType::Type {
                value_kind: Some(ValueKind::ValueType),
                source,
            } => {
                let (ut, type_generics) = decompose_type_source(source);
                let new_lookup = GenericLookup::new(type_generics);
                let new_ctx = Context::with_generics(context.clone(), &new_lookup);
                let td = new_ctx.locate_type(ut);

                if let Some(e) = td.is_enum() {
                    let enum_type = new_ctx.make_concrete(e);
                    return CTSValue::read(&enum_type, context, data);
                }

                let v = match td.type_name().as_str() {
                    "System.Boolean" => Bool(data[0] != 0),
                    "System.Char" => Char(from_bytes!(u16, data)),
                    "System.Single" => Float32(from_bytes!(f32, data)),
                    "System.Double" => Float64(from_bytes!(f64, data)),
                    "System.SByte" => Int8(data[0] as i8),
                    "System.Int16" => Int16(from_bytes!(i16, data)),
                    "System.Int32" => Int32(from_bytes!(i32, data)),
                    "System.Int64" => Int64(from_bytes!(i64, data)),
                    "System.IntPtr" => NativeInt(from_bytes!(isize, data)),
                    "System.TypedReference" => TypedRef,
                    "System.Byte" => UInt8(data[0]),
                    "System.UInt16" => UInt16(from_bytes!(u16, data)),
                    "System.UInt32" => UInt32(from_bytes!(u32, data)),
                    "System.UInt64" => UInt64(from_bytes!(u64, data)),
                    "System.UIntPtr" => NativeUInt(from_bytes!(usize, data)),
                    _ => {
                        let mut instance = Object::new(td, new_ctx);
                        instance.instance_storage.get_mut().copy_from_slice(data);
                        Struct(instance)
                    }
                };

                Self::Value(v)
            }
            BaseType::Type { .. }
            | BaseType::Array(_, _)
            | BaseType::String
            | BaseType::Object
            | BaseType::Vector(_, _) => Self::Ref(ObjectRef::read(data)),
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
            BaseType::UIntPtr | BaseType::ValuePointer(_, _) | BaseType::FunctionPointer(_) => {
                Self::Value(NativeUInt(from_bytes!(usize, data)))
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
                Float32(f) => dest.copy_from_slice(&f.to_ne_bytes()),
                Float64(f) => dest.copy_from_slice(&f.to_ne_bytes()),
                TypedRef => dest.copy_from_slice(todo!()),
                Struct(o) => dest.copy_from_slice(o.instance_storage.get()),
            },
            CTSValue::Ref(o) => o.write(dest),
        }
    }
}

#[derive(Clone)]
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
        if element.is_gc_ptr() {
            for i in 0..self.layout.length {
                ObjectRef::read(&self.storage[(i * element.size())..]).trace(cc);
            }
        }
    }
}
impl<'gc> Vector<'gc> {
    pub fn new(element: ConcreteType, size: usize, context: Context) -> Self {
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
                    if self.layout.element_layout.is_gc_ptr() {
                        |chunk: &[u8]| format!("{:?}", ObjectRef::read(chunk))
                    } else {
                        |chunk: &[u8]| {
                            let bytes: Vec<_> =
                                chunk.iter().map(|b| format!("{:02x}", b)).collect();
                            bytes.join(" ")
                        }
                    },
                ))
                .map(DebugStr),
            )
            .finish()
    }
}

#[derive(Clone, Collect, PartialEq)]
#[collect(no_drop)]
pub struct Object<'gc> {
    pub description: TypeDescription,
    pub instance_storage: FieldStorage<'gc>,
}
impl<'gc> Object<'gc> {
    pub fn new(description: TypeDescription, context: Context) -> Self {
        Self {
            description,
            instance_storage: FieldStorage::instance_fields(description, context),
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

#[derive(Clone, Copy)]
pub struct TypeDescription {
    pub resolution: ResolutionS,
    pub definition: &'static TypeDefinition<'static>,
}
impl Debug for TypeDescription {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.definition.show(self.resolution))
    }
}
unsafe_empty_collect!(TypeDescription);
impl PartialEq for TypeDescription {
    fn eq(&self, other: &Self) -> bool {
        std::ptr::eq(self.resolution, other.resolution)
            && std::ptr::eq(self.definition, other.definition)
    }
}
impl Eq for TypeDescription {}
impl Hash for TypeDescription {
    fn hash<H: Hasher>(&self, state: &mut H) {
        (self.resolution as *const Resolution).hash(state);
        (self.definition as *const TypeDefinition).hash(state);
    }
}
impl TypeDescription {
    pub fn static_initializer(&self) -> Option<MethodDescription> {
        self.definition.methods.iter().find_map(|m| {
            if m.runtime_special_name
                && m.name == ".cctor"
                && !m.signature.instance
                && m.signature.parameters.is_empty()
            {
                Some(MethodDescription {
                    parent: *self,
                    method: m,
                })
            } else {
                None
            }
        })
    }

    pub fn type_name(&self) -> String {
        self.definition.nested_type_name(self.resolution)
    }

    pub fn is_enum(&self) -> Option<&MemberType> {
        match &self.definition.extends {
            Some(TypeSource::User(u))
                if matches!(u.type_name(self.resolution).as_str(), "System.Enum") =>
            {
                let inner = self.definition.fields.first()?;
                if inner.runtime_special_name && inner.name == "value__" {
                    Some(&inner.return_type)
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    pub fn is_value_type(&self, ctx: &Context) -> bool {
        for (a, _) in ctx.get_ancestors(*self) {
            if matches!(a.type_name().as_str(), "System.Enum" | "System.ValueType") {
                return true;
            }
        }
        false
    }
}

#[derive(Clone, Copy)]
pub struct MethodDescription {
    pub parent: TypeDescription,
    pub method: &'static Method<'static>,
}
impl MethodDescription {
    pub fn resolution(&self) -> ResolutionS {
        self.parent.resolution
    }
}
impl Debug for MethodDescription {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}",
            self.method.signature.show_with_name(
                self.resolution(),
                format!("{}::{}", self.parent.type_name(), self.method.name)
            )
        )
    }
}

#[derive(Clone, Copy)]
pub struct FieldDescription {
    pub parent: TypeDescription,
    pub field: &'static Field<'static>,
}
impl Debug for FieldDescription {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        if self.field.static_member {
            write!(f, "static ")?;
        }

        write!(
            f,
            "{} {}::{}",
            self.field.return_type.show(self.parent.resolution),
            self.parent.type_name(),
            self.field.name
        )?;

        Ok(())
    }
}

#[derive(Clone)]
pub struct ConcreteType {
    source: ResolutionS,
    base: Box<BaseType<Self>>,
}
impl ConcreteType {
    pub fn new(source: ResolutionS, base: BaseType<Self>) -> Self {
        ConcreteType {
            source,
            base: Box::new(base),
        }
    }

    pub fn get(&self) -> &BaseType<Self> {
        &*self.base
    }

    pub fn get_mut(&mut self) -> &mut BaseType<Self> {
        &mut *self.base
    }

    pub fn resolution(&self) -> ResolutionS {
        self.source
    }
}
impl Debug for ConcreteType {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.get().show(self.source))
    }
}
impl ResolvedDebug for ConcreteType {
    fn show(&self, _res: &Resolution) -> String {
        format!("{:?}", self)
    }
}
impl PartialEq for ConcreteType {
    fn eq(&self, other: &Self) -> bool {
        std::ptr::eq(self.source, other.source) && self.base == other.base
    }
}
impl Eq for ConcreteType {}
impl Hash for ConcreteType {
    fn hash<H: Hasher>(&self, state: &mut H) {
        (self.source as *const Resolution).hash(state);
        self.base.hash(state);
    }
}

#[derive(Clone, Default)]
pub struct GenericLookup {
    pub type_generics: Vec<ConcreteType>,
    pub method_generics: Vec<ConcreteType>,
}
impl GenericLookup {
    pub fn new(type_generics: Vec<ConcreteType>) -> Self {
        Self {
            type_generics,
            method_generics: vec![],
        }
    }

    pub fn make_concrete(&self, res: ResolutionS, t: impl Into<MethodType>) -> ConcreteType {
        match t.into() {
            MethodType::Base(b) => ConcreteType::new(res, b.map(|t| self.make_concrete(res, t))),
            MethodType::TypeGeneric(i) => self.type_generics[i].clone(),
            MethodType::MethodGeneric(i) => self.method_generics[i].clone(),
        }
    }
}
impl Debug for GenericLookup {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        struct GenericIndexFormatter(char, usize);
        impl Debug for GenericIndexFormatter {
            fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
                write!(f, "{}{}", self.0, self.1)
            }
        }

        f.debug_map()
            .entries(
                self.type_generics
                    .iter()
                    .enumerate()
                    .map(|(i, t)| (GenericIndexFormatter('T', i), t)),
            )
            .entries(
                self.method_generics
                    .iter()
                    .enumerate()
                    .map(|(i, t)| (GenericIndexFormatter('M', i), t)),
            )
            .finish()
    }
}

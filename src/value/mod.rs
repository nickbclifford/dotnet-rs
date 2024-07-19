use std::{
    cmp::Ordering,
    fmt::{Debug, Formatter},
    hash::{Hash, Hasher},
    marker::PhantomData,
    mem::size_of,
};

use dotnetdll::prelude::*;
use gc_arena::{lock::RefLock, unsafe_empty_collect, Collect, Collection, Gc};

use layout::*;
use storage::FieldStorage;

use crate::{resolve::Assemblies, utils::ResolutionS, value::string::CLRString, vm::GCHandle};

pub mod layout;
pub mod storage;
pub mod string;

#[derive(Clone)]
pub struct Context<'a> {
    pub generics: &'a GenericLookup,
    pub assemblies: &'a Assemblies,
    pub resolution: ResolutionS,
}
impl<'a> Context<'a> {
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

    pub fn locate_field(&self, field: FieldSource) -> FieldDescription {
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
    ) -> impl Iterator<Item = TypeDescription> + 'a {
        self.assemblies.ancestors(child_type)
    }

    pub fn is_a(&self, value: TypeDescription, ancestor: TypeDescription) -> bool {
        self.get_ancestors(value).any(|a| a == ancestor)
    }

    pub fn get_heap_description(&self, object: ObjectPtr) -> TypeDescription {
        match &*object.as_ref().borrow() {
            HeapStorage::Obj(o) => o.description,
            HeapStorage::Vec(_) => self.assemblies.corlib_type("System.Array"),
            HeapStorage::Str(_) => self.assemblies.corlib_type("System.String"),
            HeapStorage::Boxed(v) => v.description(self),
        }
    }

    pub fn make_concrete<T: Clone + Into<MethodType>>(&self, t: &T) -> ConcreteType {
        self.generics.make_concrete(self.resolution, t.clone())
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
    ValueType(Object<'gc>),
}
impl StackValue<'_> {
    pub fn unmanaged_ptr(ptr: *mut u8) -> Self {
        Self::UnmanagedPtr(UnmanagedPtr(ptr))
    }
    pub fn managed_ptr(ptr: *mut u8) -> Self {
        Self::ManagedPtr(ManagedPtr(ptr))
    }
    pub fn null() -> Self {
        Self::ObjectRef(ObjectRef(None))
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
            // important note: here we're returning a pointer to where these pointers are stored
            // NOT the pointers themselves, hence all the casting
            Self::ObjectRef(ObjectRef(o)) => (o as *const Option<_>) as *const u8,
            Self::UnmanagedPtr(UnmanagedPtr(u)) => (u as *const *mut _) as *const u8,
            Self::ManagedPtr(ManagedPtr(m)) => (m as *const *mut _) as *const u8,
            Self::ValueType(o) => o.instance_storage.get().as_ptr(),
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

// TODO: proper representations
#[derive(Copy, Clone, Debug, PartialEq, PartialOrd)]
#[repr(transparent)]
pub struct UnmanagedPtr(pub *mut u8);
#[derive(Copy, Clone, Debug, PartialEq, PartialOrd)]
#[repr(transparent)]
pub struct ManagedPtr(pub *mut u8);
unsafe_empty_collect!(UnmanagedPtr);
unsafe_empty_collect!(ManagedPtr);

pub type ObjectPtr<'gc> = Gc<'gc, RefLock<HeapStorage<'gc>>>;

#[derive(Copy, Clone, Collect)]
#[collect(no_drop)]
#[repr(transparent)]
pub struct ObjectRef<'gc>(pub Option<ObjectPtr<'gc>>);
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
    pub fn new(gc: GCHandle<'gc>, value: HeapStorage<'gc>) -> Self {
        Self(Some(Gc::new(gc, RefLock::new(value))))
    }
}
impl Debug for ObjectRef<'_> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self.0 {
            None => f.write_str("NULL"),
            Some(gc) => {
                let data = gc.borrow();
                write!(f, "heap {:?}", data)
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

fn convert_num<T: TryFrom<i32> + TryFrom<isize>>(data: StackValue) -> T {
    match data {
        StackValue::Int32(i) => i
            .try_into()
            .unwrap_or_else(|_| panic!("failed to convert from i32")),
        StackValue::NativeInt(i) => i
            .try_into()
            .unwrap_or_else(|_| panic!("failed to convert from isize")),
        other => panic!("invalid stack value {:?} for integer conversion", other),
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
            CTSValue::Ref(_) => {
                panic!("tried to instantiate value type, received object reference")
            }
        }
    }

    pub fn description(&self, context: &Context) -> TypeDescription {
        match self {
            ValueType::Bool(_) => context.assemblies.corlib_type("System.Boolean"),
            ValueType::Char(_) => context.assemblies.corlib_type("System.Char"),
            ValueType::Int8(_) => context.assemblies.corlib_type("System.SByte"),
            ValueType::UInt8(_) => context.assemblies.corlib_type("System.Byte"),
            ValueType::Int16(_) => context.assemblies.corlib_type("System.Int16"),
            ValueType::UInt16(_) => context.assemblies.corlib_type("System.UInt16"),
            ValueType::Int32(_) => context.assemblies.corlib_type("System.Int32"),
            ValueType::UInt32(_) => context.assemblies.corlib_type("System.UInt32"),
            ValueType::Int64(_) => context.assemblies.corlib_type("System.Int64"),
            ValueType::UInt64(_) => context.assemblies.corlib_type("System.UInt64"),
            ValueType::NativeInt(_) => context.assemblies.corlib_type("System.IntPtr"),
            ValueType::NativeUInt(_) => context.assemblies.corlib_type("System.UIntPtr"),
            ValueType::Float32(_) => context.assemblies.corlib_type("System.Single"),
            ValueType::Float64(_) => context.assemblies.corlib_type("System.Double"),
            ValueType::TypedRef => context.assemblies.corlib_type("System.TypedReference"),
            ValueType::Struct(s) => s.description,
        }
    }
}

macro_rules! from_bytes {
    ($t:ty, $data:expr) => {
        <$t>::from_ne_bytes($data.try_into().expect("source data was too small"))
    };
}

pub enum CTSValue<'gc> {
    Value(ValueType<'gc>),
    Ref(ObjectRef<'gc>),
}
impl<'gc> CTSValue<'gc> {
    pub fn new(t: &ConcreteType, context: &Context, data: StackValue<'gc>) -> Self {
        match t.get() {
            BaseType::Type {
                value_kind: None | Some(ValueKind::ValueType),
                source,
            } => Self::Value(match source {
                TypeSource::User(u) => {
                    let t = context.locate_type(*u);
                    match t.1.type_name().as_str() {
                        "System.Boolean" => ValueType::Bool(todo!()),
                        "System.Char" => ValueType::Char(todo!()),
                        "System.Single" => ValueType::Float32(match data {
                            StackValue::NativeFloat(f) => f as f32,
                            other => panic!("invalid stack value {:?} for float conversion", other),
                        }),
                        "System.Double" => ValueType::Float64(match data {
                            StackValue::NativeFloat(f) => f,
                            other => panic!("invalid stack value {:?} for float conversion", other),
                        }),
                        "System.SByte" => ValueType::Int8(convert_num(data)),
                        "System.Int16" => ValueType::Int16(convert_num(data)),
                        "System.Int32" => ValueType::Int32(convert_num(data)),
                        "System.Int64" => ValueType::Int64(convert_i64(data)),
                        "System.IntPtr" => ValueType::NativeInt(convert_num(data)),
                        "System.TypedReference" => ValueType::TypedRef,
                        "System.Byte" => ValueType::UInt8(convert_num(data)),
                        "System.UInt16" => ValueType::UInt16(convert_num(data)),
                        "System.UInt32" => ValueType::UInt32(convert_num(data)),
                        "System.UInt64" => ValueType::UInt64(convert_i64(data)),
                        "System.UIntPtr" => ValueType::NativeUInt(convert_num(data)),
                        _ => ValueType::Struct(todo!()),
                    }
                }
                TypeSource::Generic { .. } => {
                    todo!("resolve types with generics")
                }
            }),
            BaseType::Type {
                value_kind: Some(ValueKind::Class),
                ..
            } => match data {
                StackValue::ObjectRef(o) => {
                    // TODO: check declared type vs actual type?
                    Self::Ref(o)
                }
                other => panic!("invalid stack value {:?} for object ref conversion", other),
            },
            BaseType::Boolean => Self::Value(ValueType::Bool(convert_num::<u8>(data) != 0)),
            BaseType::Char => Self::Value(ValueType::Char(convert_num(data))),
            BaseType::Int8 => Self::Value(ValueType::Int8(convert_num(data))),
            BaseType::UInt8 => Self::Value(ValueType::UInt8(convert_num(data))),
            BaseType::Int16 => Self::Value(ValueType::Int16(convert_num(data))),
            BaseType::UInt16 => Self::Value(ValueType::UInt16(convert_num(data))),
            BaseType::Int32 => Self::Value(ValueType::Int32(convert_num(data))),
            BaseType::UInt32 => Self::Value(ValueType::UInt32(convert_num(data))),
            BaseType::Int64 => Self::Value(ValueType::Int64(convert_i64(data))),
            BaseType::UInt64 => Self::Value(ValueType::UInt64(convert_i64(data))),
            BaseType::Float32 => Self::Value(ValueType::Float32(match data {
                StackValue::NativeFloat(f) => f as f32,
                other => panic!("invalid stack value {:?} for float conversion", other),
            })),
            BaseType::Float64 => Self::Value(ValueType::Float64(match data {
                StackValue::NativeFloat(f) => f,
                other => panic!("invalid stack value {:?} for float conversion", other),
            })),
            BaseType::IntPtr => Self::Value(ValueType::NativeInt(convert_num(data))),
            BaseType::UIntPtr | BaseType::ValuePointer(_, _) | BaseType::FunctionPointer(_) => {
                Self::Value(ValueType::NativeUInt(convert_num(data)))
            }
            BaseType::Object => Self::Ref(match data {
                StackValue::ObjectRef(o) => o,
                other => panic!("expected object ref on stack, found {:?}", other),
            }),
            rest => todo!("tried to deserialize StackValue {:?}", rest),
        }
    }

    pub fn read(t: &ConcreteType, context: &Context, data: &[u8]) -> Self {
        match t.get() {
            BaseType::Type {
                value_kind: None | Some(ValueKind::ValueType),
                source,
            } => Self::Value(match source {
                TypeSource::User(u) => {
                    let t = context.locate_type(*u);
                    match t.1.type_name().as_str() {
                        "System.Boolean" => ValueType::Bool(data[0] != 0),
                        "System.Char" => ValueType::Char(from_bytes!(u16, data)),
                        "System.Single" => ValueType::Float32(from_bytes!(f32, data)),
                        "System.Double" => ValueType::Float64(from_bytes!(f64, data)),
                        "System.SByte" => ValueType::Int8(data[0] as i8),
                        "System.Int16" => ValueType::Int16(from_bytes!(i16, data)),
                        "System.Int32" => ValueType::Int32(from_bytes!(i32, data)),
                        "System.Int64" => ValueType::Int64(from_bytes!(i64, data)),
                        "System.IntPtr" => ValueType::NativeInt(from_bytes!(isize, data)),
                        "System.TypedReference" => ValueType::TypedRef,
                        "System.Byte" => ValueType::UInt8(data[0]),
                        "System.UInt16" => ValueType::UInt16(from_bytes!(u16, data)),
                        "System.UInt32" => ValueType::UInt32(from_bytes!(u32, data)),
                        "System.UInt64" => ValueType::UInt64(from_bytes!(u64, data)),
                        "System.UIntPtr" => ValueType::NativeUInt(from_bytes!(usize, data)),
                        _ => ValueType::Struct(todo!()),
                    }
                }
                TypeSource::Generic { .. } => {
                    todo!("resolve types with generics")
                }
            }),
            BaseType::Type {
                value_kind: Some(ValueKind::Class),
                ..
            }
            | BaseType::Array(_, _)
            | BaseType::String
            | BaseType::Object
            | BaseType::Vector(_, _) => Self::Ref(read_gc_ptr(data)),
            BaseType::Boolean => Self::Value(ValueType::Bool(data[0] != 0)),
            BaseType::Char => Self::Value(ValueType::Char(from_bytes!(u16, data))),
            BaseType::Int8 => Self::Value(ValueType::Int8(data[0] as i8)),
            BaseType::UInt8 => Self::Value(ValueType::UInt8(data[0])),
            BaseType::Int16 => Self::Value(ValueType::Int16(from_bytes!(i16, data))),
            BaseType::UInt16 => Self::Value(ValueType::UInt16(from_bytes!(u16, data))),
            BaseType::Int32 => Self::Value(ValueType::Int32(from_bytes!(i32, data))),
            BaseType::UInt32 => Self::Value(ValueType::UInt32(from_bytes!(u32, data))),
            BaseType::Int64 => Self::Value(ValueType::Int64(from_bytes!(i64, data))),
            BaseType::UInt64 => Self::Value(ValueType::UInt64(from_bytes!(u64, data))),
            BaseType::Float32 => Self::Value(ValueType::Float32(from_bytes!(f32, data))),
            BaseType::Float64 => Self::Value(ValueType::Float64(from_bytes!(f64, data))),
            BaseType::IntPtr => Self::Value(ValueType::NativeInt(from_bytes!(isize, data))),
            BaseType::UIntPtr | BaseType::ValuePointer(_, _) | BaseType::FunctionPointer(_) => {
                Self::Value(ValueType::NativeUInt(from_bytes!(usize, data)))
            }
        }
    }

    pub fn into_stack(self) -> StackValue<'gc> {
        match self {
            CTSValue::Value(ValueType::Bool(b)) => StackValue::Int32(b as i32),
            CTSValue::Value(ValueType::Char(c)) => StackValue::Int32(c as i32),
            CTSValue::Value(ValueType::Int8(i)) => StackValue::Int32(i as i32),
            CTSValue::Value(ValueType::UInt8(i)) => StackValue::Int32(i as i32),
            CTSValue::Value(ValueType::Int16(i)) => StackValue::Int32(i as i32),
            CTSValue::Value(ValueType::UInt16(i)) => StackValue::Int32(i as i32),
            CTSValue::Value(ValueType::Int32(i)) => StackValue::Int32(i),
            CTSValue::Value(ValueType::UInt32(i)) => StackValue::Int32(i as i32),
            CTSValue::Value(ValueType::Int64(i)) => StackValue::Int64(i),
            CTSValue::Value(ValueType::UInt64(i)) => StackValue::Int64(i as i64),
            CTSValue::Value(ValueType::NativeInt(i)) => StackValue::NativeInt(i),
            CTSValue::Value(ValueType::NativeUInt(i)) => StackValue::NativeInt(i as isize),
            CTSValue::Value(ValueType::Float32(f)) => StackValue::NativeFloat(f as f64),
            CTSValue::Value(ValueType::Float64(f)) => StackValue::NativeFloat(f),
            CTSValue::Value(ValueType::TypedRef) => todo!(),
            CTSValue::Value(ValueType::Struct(s)) => StackValue::ValueType(s),
            CTSValue::Ref(o) => StackValue::ObjectRef(o),
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
            CTSValue::Ref(o) => write_gc_ptr(*o, dest),
        }
    }
}

// explicit lifetimes needed to avoid elision assigning both the same lifetime
pub fn read_gc_ptr<'data, 'gc>(source: &'data [u8]) -> ObjectRef<'gc> {
    let mut ptr_bytes = [0u8; size_of::<ObjectRef>()];
    ptr_bytes.copy_from_slice(&source[0..size_of::<ObjectRef>()]);
    let ptr = usize::from_ne_bytes(ptr_bytes) as *const RefLock<_>;

    if ptr.is_null() {
        ObjectRef(None)
    } else {
        // SAFETY: since this came from Gc::as_ptr, we know it's valid
        // also, this will only ever be called inside the context of a GC mutation, so it's okay for 'gc to be unbounded
        ObjectRef(Some(unsafe { Gc::from_ptr(ptr) }))
    }
}
pub fn write_gc_ptr(ObjectRef(source): ObjectRef<'_>, dest: &mut [u8]) {
    let ptr: *const RefLock<_> = match source {
        None => std::ptr::null(),
        Some(s) => Gc::as_ptr(s),
    };
    let ptr_bytes = (ptr as usize).to_ne_bytes();
    dest.copy_from_slice(&ptr_bytes);
}

#[derive(Clone, Debug)]
pub struct Vector<'gc> {
    element: ConcreteType,
    layout: ArrayLayoutManager,
    storage: Vec<u8>,
    _contains_gc: PhantomData<&'gc ()>, // TODO: variance rules?
}
unsafe impl Collect for Vector<'_> {
    #[inline]
    fn trace(&self, cc: &Collection) {
        let element = &self.layout.element_layout;
        if element.is_gc_ptr() {
            for i in 0..self.layout.length {
                if let ObjectRef(Some(gc)) = read_gc_ptr(&self.storage[(i * element.size())..]) {
                    gc.trace(cc);
                }
            }
        }
    }
}
impl<'gc> Vector<'gc> {
    pub fn new(
        gc: GCHandle<'gc>,
        element: ConcreteType,
        size: usize,
        context: Context,
    ) -> Gc<'gc, Self> {
        let layout = ArrayLayoutManager::new(element.clone(), size, context);
        let value = Self {
            storage: vec![0; layout.size()], // TODO: initialize properly
            layout,
            element,
            _contains_gc: PhantomData,
        };
        Gc::new(gc, value)
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
        f.debug_struct("Object")
            .field("description", &self.description)
            .field("storage", &self.instance_storage.get())
            .finish()
    }
}

#[derive(Clone, Copy)]
pub struct TypeDescription(pub ResolutionS, pub &'static TypeDefinition<'static>);
impl Debug for TypeDescription {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.1.show(self.0))
    }
}
unsafe_empty_collect!(TypeDescription);
impl PartialEq for TypeDescription {
    fn eq(&self, other: &Self) -> bool {
        std::ptr::eq(self.0, other.0) && std::ptr::eq(self.1, other.1)
    }
}
impl Eq for TypeDescription {}
impl Hash for TypeDescription {
    fn hash<H: Hasher>(&self, state: &mut H) {
        (self.0 as *const Resolution).hash(state);
        (self.1 as *const TypeDefinition).hash(state);
    }
}
impl TypeDescription {
    pub fn static_initializer(&self) -> Option<MethodDescription> {
        self.1.methods.iter().find_map(|m| {
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
}

#[derive(Clone, Copy)]
pub struct MethodDescription {
    pub parent: TypeDescription,
    pub method: &'static Method<'static>,
}
impl MethodDescription {
    pub fn resolution(&self) -> ResolutionS {
        self.parent.0
    }
}
impl Debug for MethodDescription {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}",
            self.method.signature.show_with_name(
                self.resolution(),
                format!("{}::{}", self.parent.1.type_name(), self.method.name)
            )
        )
    }
}

#[derive(Clone, Debug, Copy)]
pub struct FieldDescription {
    pub parent: TypeDescription,
    pub field: &'static Field<'static>,
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

#[derive(Clone, Default)]
pub struct GenericLookup {
    pub type_generics: Vec<ConcreteType>,
    pub method_generics: Vec<ConcreteType>,
}
impl GenericLookup {
    pub fn make_concrete(&self, source: ResolutionS, t: impl Into<MethodType>) -> ConcreteType {
        match t.into() {
            MethodType::Base(b) => {
                ConcreteType::new(source, b.map(|t| self.make_concrete(source, t)))
            }
            MethodType::TypeGeneric(i) => self.type_generics[i].clone(),
            MethodType::MethodGeneric(i) => self.method_generics[i].clone(),
        }
    }

    pub fn instantiate_method(&self, parameters: Vec<ConcreteType>) -> Self {
        Self {
            method_generics: parameters,
            ..self.clone()
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

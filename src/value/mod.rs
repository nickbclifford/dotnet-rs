use std::{cmp::Ordering, marker::PhantomData, mem::size_of};
use std::hash::{Hash, Hasher};

use dotnetdll::prelude::*;
use gc_arena::{Collect, Collection, Gc, unsafe_empty_collect};

use layout::*;
use storage::FieldStorage;

use crate::utils::ResolutionS;
use crate::value::string::CLRString;
use crate::{resolve::Assemblies, vm::GCHandle};

mod layout;
pub mod string;
mod storage;

#[derive(Clone)]
pub struct Context<'a> {
    pub generics: &'a GenericLookup,
    pub assemblies: &'a Assemblies,
    pub resolution: ResolutionS,
}
impl Context<'_> {
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
    ) -> impl Iterator<Item = TypeDescription> {
        self.assemblies.ancestors(child_type)
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

#[derive(Copy, Clone, Debug, Collect)]
#[collect(no_drop)]
#[repr(transparent)]
pub struct ObjectRef<'gc>(pub Option<Gc<'gc, HeapStorage<'gc>>>);
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
        Self(Some(Gc::new(gc, value)))
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
    Char(char),
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

impl ValueType<'_> {
    pub fn new(t: &ConcreteType, context: &Context, data: StackValue) -> Self {
        match t.get() {
            BaseType::Type {
                value_kind: None | Some(ValueKind::ValueType),
                source,
            } => match source {
                TypeSource::User(u) => {
                    let t = context.locate_type(*u);
                    match t.1.type_name().as_str() {
                        "System.Boolean" => Self::Bool(todo!()),
                        "System.Char" => Self::Char(todo!()),
                        "System.Single" => Self::Float32(todo!()),
                        "System.Double" => Self::Float64(todo!()),
                        "System.SByte" => Self::Int8(convert_num(data)),
                        "System.Int16" => Self::Int16(convert_num(data)),
                        "System.Int32" => Self::Int32(convert_num(data)),
                        "System.Int64" => Self::Int64(todo!()),
                        "System.IntPtr" => Self::NativeInt(convert_num(data)),
                        "System.TypedReference" => Self::TypedRef,
                        "System.Byte" => Self::UInt8(convert_num(data)),
                        "System.UInt16" => Self::UInt16(convert_num(data)),
                        "System.UInt32" => Self::UInt32(convert_num(data)),
                        "System.UInt64" => Self::UInt64(todo!()),
                        "System.UIntPtr" => Self::NativeUInt(convert_num(data)),
                        _ => Self::Struct(todo!()),
                    }
                }
                TypeSource::Generic { .. } => {
                    todo!("resolve value types with generics")
                }
            },
            BaseType::Boolean => Self::Bool(todo!()),
            BaseType::Char => Self::Char(todo!()),
            BaseType::Int8 => Self::Int8(convert_num(data)),
            BaseType::UInt8 => Self::UInt8(convert_num(data)),
            BaseType::Int16 => Self::Int16(convert_num(data)),
            BaseType::UInt16 => Self::UInt16(convert_num(data)),
            BaseType::Int32 => Self::Int32(convert_num(data)),
            BaseType::UInt32 => Self::UInt32(convert_num(data)),
            BaseType::Int64 => Self::Int64(todo!()),
            BaseType::UInt64 => Self::UInt64(todo!()),
            BaseType::Float32 => Self::Float32(todo!()),
            BaseType::Float64 => Self::Float64(todo!()),
            BaseType::IntPtr => Self::NativeInt(convert_num(data)),
            BaseType::UIntPtr | BaseType::ValuePointer(_, _) | BaseType::FunctionPointer(_) => {
                Self::NativeUInt(convert_num(data))
            }
            rest => panic!("{:?} is not a value type", rest),
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

pub fn read_gc_ptr(source: &[u8]) -> ObjectRef<'_> {
    let mut ptr_bytes = [0u8; size_of::<ObjectRef>()];
    ptr_bytes.copy_from_slice(&source[0..size_of::<ObjectRef>()]);
    let ptr = usize::from_ne_bytes(ptr_bytes) as *const HeapStorage;

    // null pointer optimization ensures Option<Gc> has all zeroes for None
    // and the same layout as Gc for Some
    // thus, if the pointer is not null, we know it is a Some(Gc)
    if ptr.is_null() {
        ObjectRef(None)
    } else {
        // SAFETY: since this came from Gc::as_ptr, we know it's valid
        ObjectRef(Some(unsafe { Gc::from_ptr(ptr) }))
    }
}
pub fn write_gc_ptr(ObjectRef(source): ObjectRef<'_>, dest: &mut [u8]) {
    let ptr: *const HeapStorage = match source {
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
        if self.layout.element_layout.is_gc_ptr() {
            for i in 0..self.layout.length {
                if let ObjectRef(Some(gc)) = read_gc_ptr(&self.storage[i..]) {
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

#[derive(Clone, Debug, Collect, PartialEq)]
#[collect(no_drop)]
pub struct Object<'gc> {
    pub description: TypeDescription,
    instance_storage: FieldStorage<'gc>,
}
impl<'gc> Object<'gc> {
    pub fn new(gc: GCHandle<'gc>, description: TypeDescription, context: Context) -> Gc<'gc, Self> {
        let value = Self {
            description,
            instance_storage: FieldStorage::instance_fields(description, context)
        };
        Gc::new(gc, value)
    }
}

#[derive(Clone, Debug, Copy)]
pub struct TypeDescription(pub ResolutionS, pub &'static TypeDefinition<'static>);
unsafe_empty_collect!(TypeDescription);
impl PartialEq for TypeDescription {
    fn eq(&self, other: &Self) -> bool {
        std::ptr::eq(self.0, other.0) && std::ptr::eq(self.1, other.1)
    }
}
impl Hash for TypeDescription {
    fn hash<H: Hasher>(&self, state: &mut H) {
        (self.0 as *const Resolution).hash(state);
        (self.1 as *const TypeDefinition).hash(state);
    }
}

#[derive(Clone, Debug, Copy)]
pub struct MethodDescription {
    pub parent: TypeDescription,
    pub method: &'static Method<'static>,
}

#[derive(Clone, Debug)]
pub struct ConcreteType(Box<BaseType<ConcreteType>>);
impl ConcreteType {
    pub fn new(base: BaseType<Self>) -> Self {
        ConcreteType(Box::new(base))
    }

    pub fn get(&self) -> &BaseType<Self> {
        &*self.0
    }
}

#[derive(Clone, Debug, Default)]
pub struct GenericLookup {
    pub type_generics: Vec<ConcreteType>,
    pub method_generics: Vec<ConcreteType>,
}
impl GenericLookup {
    pub fn make_concrete(&self, t: impl Into<MethodType>) -> ConcreteType {
        match t.into() {
            MethodType::Base(b) => ConcreteType(Box::new(b.map(|t| self.make_concrete(t)))),
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

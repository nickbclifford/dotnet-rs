use std::{cmp::Ordering, marker::PhantomData, mem::size_of};

use dotnetdll::prelude::*;
use gc_arena::{unsafe_empty_collect, Collect, Collection, Gc};

use layout::*;

use crate::resolve::WithSource;
use crate::utils::ResolutionS;
use crate::value::string::CLRString;
use crate::{resolve::Assemblies, vm::GCHandle};

mod layout;
pub mod string;

#[derive(Clone)]
pub struct Context<'a> {
    pub generics: &'a GenericLookup,
    pub assemblies: &'a Assemblies,
    pub resolution: ResolutionS,
}
impl Context<'_> {
    pub fn locate_type(&self, handle: UserType) -> WithSource<TypeDescription> {
        self.assemblies.locate_type(self.resolution, handle)
    }

    pub fn locate_method(
        &self,
        handle: UserMethod,
        generic_inst: &GenericLookup,
    ) -> WithSource<MethodDescription> {
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
            .find_method_in_type(self.resolution, parent_type, name, signature)
    }

    pub fn get_ancestors(
        &self,
        child_type: TypeDescription,
    ) -> impl Iterator<Item = WithSource<TypeDescription>> {
        self.assemblies.ancestors(self.resolution, child_type)
    }
}

#[derive(Copy, Clone, Debug, Collect, PartialEq)]
#[collect(no_drop)]
pub enum StackValue<'gc> {
    Int32(i32),
    Int64(i64),
    NativeInt(isize),
    NativeFloat(f64),
    ObjectRef(ObjectRef<'gc>),
    UnmanagedPtr(UnmanagedPtr),
    ManagedPtr(ManagedPtr), // TODO: user-defined value type
                            // I.12.3.2.1
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
}

fn read_gc_ptr(source: &[u8]) -> ObjectRef<'_> {
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
fn write_gc_ptr(ObjectRef(source): ObjectRef<'_>, dest: &mut [u8]) {
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

#[derive(Clone, Debug)]
pub struct Object<'gc> {
    pub description: TypeDescription,
    field_layout: ClassLayoutManager,
    field_storage: Vec<u8>,
    _contains_gc: PhantomData<&'gc ()>, // ditto
}
unsafe impl Collect for Object<'_> {
    #[inline]
    fn trace(&self, cc: &Collection) {
        for field in self.field_layout.fields.values() {
            if field.layout.is_gc_ptr() {
                if let ObjectRef(Some(gc)) = read_gc_ptr(&self.field_storage[field.position..]) {
                    gc.trace(cc);
                }
            }
        }
    }
}
impl<'gc> Object<'gc> {
    pub fn new(gc: GCHandle<'gc>, description: TypeDescription, context: Context) -> Gc<'gc, Self> {
        let layout = ClassLayoutManager::new(description, context);
        let value = Self {
            description,
            field_storage: vec![0; layout.size()], // TODO: initialize properly
            field_layout: layout,
            _contains_gc: PhantomData,
        };
        Gc::new(gc, value)
    }
}

#[derive(Clone, Debug, Copy)]
pub struct TypeDescription(pub &'static TypeDefinition<'static>);
unsafe_empty_collect!(TypeDescription);

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

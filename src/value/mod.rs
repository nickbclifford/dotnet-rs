use dotnetdll::prelude::*;
use gc_arena::{Gc, Collect, unsafe_empty_collect};

#[derive(Clone, Debug, Collect)]
#[collect(no_drop)]
pub enum StackValue<'gc> {
    Int32(i32),
    Int64(i64),
    NativeInt(isize),
    NativeFloat(f64),
    ObjectRef(ObjectRef<'gc>),
    UnmanagedPtr(UnmanagedPtr),
    ManagedPtr(ManagedPtr)
    // TODO: user-defined value type
    // I.12.3.2.1
}
impl StackValue<'_> {
    pub fn unmanaged_ptr(ptr: *mut u8) -> Self {
        Self::UnmanagedPtr(UnmanagedPtr(ptr))
    }
    pub fn managed_ptr(ptr: *mut u8) -> Self {
        Self::ManagedPtr(ManagedPtr(ptr))
    }
}

// TODO: proper representations
#[derive(Clone, Debug)]
pub struct UnmanagedPtr(pub *mut u8);
#[derive(Clone, Debug)]
pub struct ManagedPtr(pub *mut u8);
unsafe_empty_collect!(UnmanagedPtr);
unsafe_empty_collect!(ManagedPtr);

pub type ObjectRef<'gc> = Option<Gc<'gc, ObjectKind<'gc>>>;

#[derive(Clone, Debug, Collect)]
#[collect(no_drop)]
pub enum ObjectKind<'gc> {
    Vec(Vector<'gc>),
    Obj(Object<'gc>)
}

#[derive(Clone, Debug, Collect)]
#[collect(no_drop)]
pub struct Vector<'gc> {
    elements: Vec<StackValue<'gc>>
}

#[derive(Clone, Debug, Collect)]
#[collect(no_drop)]
pub struct Object<'gc> {
    description: TypeDescription<'gc>
}

// the TypeDescription's data will longer than the garbage collector
// thus it's fine to pun the lifetimes
#[derive(Clone, Debug)]
pub struct TypeDescription<'data>(pub &'data TypeDefinition<'data>);
unsafe_empty_collect!(TypeDescription<'_>);

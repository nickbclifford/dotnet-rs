mod layout;

use dotnetdll::prelude::*;
use gc_arena::{unsafe_empty_collect, Collect, Collection, Gc};
use layout::*;
use std::marker::PhantomData;
use std::mem::size_of;

#[derive(Copy, Clone, Debug, Collect)]
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
        Self::ObjectRef(None)
    }
}
impl Default for StackValue<'_> {
    fn default() -> Self {
        Self::null()
    }
}

// TODO: proper representations
#[derive(Copy, Clone, Debug)]
#[repr(transparent)]
pub struct UnmanagedPtr(pub *mut u8);
#[derive(Copy, Clone, Debug)]
#[repr(transparent)]
pub struct ManagedPtr(pub *mut u8);
unsafe_empty_collect!(UnmanagedPtr);
unsafe_empty_collect!(ManagedPtr);

pub type ObjectRef<'gc> = Option<Gc<'gc, HeapStorage<'gc>>>;

#[derive(Clone, Debug, Collect)]
#[collect(no_drop)]
pub enum HeapStorage<'gc> {
    Vec(Vector<'gc>),
    Obj(Object<'gc>),
}

fn read_gc_ptr(source: &[u8]) -> ObjectRef<'_> {
    let mut ptr_bytes = [0u8; size_of::<ObjectRef>()];
    ptr_bytes.copy_from_slice(&source[0..size_of::<ObjectRef>()]);
    let ptr = usize::from_ne_bytes(ptr_bytes) as *const HeapStorage;

    // null pointer optimization ensures Option<Gc> has all zeroes for None
    // and the same layout as Gc for Some
    // thus, if the pointer is not null, we know it is a Some(Gc)
    if ptr.is_null() {
        None
    } else {
        // SAFETY: since this came from Gc::as_ptr, we know it's valid
        Some(unsafe { Gc::from_ptr(ptr) })
    }
}

#[derive(Clone, Debug)]
pub struct Vector<'gc> {
    element_type: MemberType,
    layout: ArrayLayoutManager,
    storage: Vec<u8>,
    _contains_gc: PhantomData<&'gc ()>, // TODO: variance rules?
}
unsafe impl Collect for Vector<'_> {
    #[inline]
    fn trace(&self, cc: &Collection) {
        if self.layout.element_layout.is_gc_ptr() {
            for i in 0..self.layout.length {
                if let Some(gc) = read_gc_ptr(&self.storage[i..]) {
                    gc.trace(cc);
                }
            }
        }
    }
}

#[derive(Clone, Debug)]
pub struct Object<'gc> {
    description: TypeDescription<'gc>,
    field_layout: ClassLayoutManager,
    field_storage: Vec<u8>,
}
unsafe impl Collect for Object<'_> {
    #[inline]
    fn trace(&self, cc: &Collection) {
        for field in self.field_layout.fields.values() {
            if field.layout.is_gc_ptr() {
                if let Some(gc) = read_gc_ptr(&self.field_storage[field.position..]) {
                    gc.trace(cc);
                }
            }
        }
    }
}

// the TypeDescription's data will longer than the garbage collector
// thus it's fine to pun the lifetimes
#[derive(Clone, Debug, Copy)]
pub struct TypeDescription<'data>(pub &'data TypeDefinition<'data>);
unsafe_empty_collect!(TypeDescription<'_>);

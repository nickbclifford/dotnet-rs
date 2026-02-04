use crate::{
    layout::{ArrayLayoutManager, HasLayout, LayoutManager, Scalar},
    pointer::ManagedPtr,
    storage::FieldStorage,
    string::CLRString,
    StackValue,
};
use dotnet_types::{generics::ConcreteType, TypeDescription};
use dotnet_utils::{
    gc::{GCHandle, ThreadSafeLock},
    sync::get_current_thread_id,
    DebugStr,
};
use gc_arena::{Collect, Collection, Gc};
use std::{
    cmp::Ordering,
    collections::HashSet,
    fmt::{self, Debug, Formatter},
    hash::{Hash, Hasher},
    iter,
    marker::PhantomData,
    ptr::{self, NonNull},
};

#[cfg(feature = "multithreaded-gc")]
use dotnet_utils::gc::{get_currently_tracing, record_allocation, record_cross_arena_ref};

#[cfg(any(feature = "memory-validation", debug_assertions))]
const OBJECT_MAGIC: u64 = 0x5AFE_0B1E_C700_0000;

#[derive(Collect, Debug)]
#[collect(no_drop)]
pub struct ObjectInner<'gc> {
    #[cfg(any(feature = "memory-validation", debug_assertions))]
    pub magic: u64,
    pub owner_id: u64,
    pub storage: HeapStorage<'gc>,
}

#[repr(transparent)]
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ObjectPtr(pub NonNull<ThreadSafeLock<ObjectInner<'static>>>);

unsafe impl Send for ObjectPtr {}
unsafe impl Sync for ObjectPtr {}

impl ObjectPtr {
    /// # Safety
    ///
    /// The pointer must be valid for the program lifetime and properly aligned.
    pub unsafe fn from_raw(ptr: *const ThreadSafeLock<ObjectInner<'static>>) -> Option<Self> {
        NonNull::new(ptr as *mut _).map(ObjectPtr)
    }

    pub fn as_ptr(&self) -> *const ThreadSafeLock<ObjectInner<'static>> {
        self.0.as_ptr()
    }
}

pub type ObjectHandle<'gc> = Gc<'gc, ThreadSafeLock<ObjectInner<'gc>>>;

#[derive(Copy, Clone)]
#[repr(transparent)]
pub struct ObjectRef<'gc>(pub Option<ObjectHandle<'gc>>);

unsafe impl<'gc> Collect for ObjectRef<'gc> {
    fn trace(&self, cc: &Collection) {
        if let Some(h) = self.0 {
            #[cfg(feature = "multithreaded-gc")]
            {
                // Check for cross-arena reference
                if let Some(tracing_id) = get_currently_tracing() {
                    // SAFETY: During stop-the-world GC, no other threads are running,
                    // so we can safely access the owner_id without acquiring the lock.
                    // This avoids potential deadlock if a thread was stopped while holding a write lock.
                    let owner_id = unsafe { (*h.as_ptr()).owner_id };
                    if owner_id != tracing_id {
                        // This is a reference to an object in another arena.
                        // Do not trace it here; instead, record it for coordinated resurrection.
                        // We cast to usize because utilities cannot depend on ObjectPtr.
                        let ptr = Gc::as_ptr(h) as usize;
                        record_cross_arena_ref(owner_id, ptr);
                        return;
                    }
                }
            }
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
                handle.borrow().storage.resurrect(fc, visited);
            }
        }
    }

    pub fn new(gc: GCHandle<'gc>, value: HeapStorage<'gc>) -> Self {
        let owner_id = get_current_thread_id();
        #[cfg(feature = "multithreaded-gc")]
        {
            let size = size_of::<ObjectInner>() + value.size_bytes();
            record_allocation(size);
        }

        Self(Some(Gc::new(
            gc,
            ThreadSafeLock::new(ObjectInner {
                #[cfg(any(feature = "memory-validation", debug_assertions))]
                magic: OBJECT_MAGIC,
                owner_id,
                storage: value,
            }),
        )))
    }

    /// Reads an ObjectRef from a byte slice without lifetime branding.
    ///
    /// # Safety
    /// - `source` must contain a valid, properly aligned `Gc` pointer (or null).
    /// - The caller must ensure the returned `ObjectRef` does not outlive the
    ///   arena generation it belongs to.
    pub unsafe fn read_unchecked(source: &[u8]) -> Self {
        let ptr_val = {
            // SAFETY: Use read_unaligned to avoid UB on unaligned access.
            // We assume the caller holds a lock (FieldStorage RwLock) to prevent tearing.
            (source.as_ptr() as *const usize).read_unaligned()
        };

        let ptr = ptr_val as *const ThreadSafeLock<ObjectInner<'gc>>;

        if ptr.is_null() {
            ObjectRef(None)
        } else {
            #[cfg(any(feature = "memory-validation", debug_assertions))]
            {
                if ptr_val % align_of::<ThreadSafeLock<ObjectInner<'static>>>() != 0 {
                    panic!("ObjectRef::read: Pointer {:#x} is not aligned", ptr_val);
                }

                // Verify magic number to ensure we are pointing to a valid object
                let inner = &*(*ptr).as_ptr();
                if inner.magic != OBJECT_MAGIC {
                    panic!(
                        "ObjectRef::read: Pointer {:#x} points to invalid object (bad magic: {:#x})",
                        ptr_val, inner.magic
                    );
                }
            }

            // SAFETY: The pointer was originally obtained via Gc::as_ptr and stored as bytes.
            // Since this is only called during VM execution where 'gc is valid and
            // the object is guaranteed to be alive (as it is traced by the caller),
            // it is safe to reconstruct the Gc pointer.
            // Note: We don't assert alignment of the Gc pointer itself here, but Gc ptrs are always aligned.
            ObjectRef(Some(Gc::from_ptr(ptr)))
        }
    }

    /// Reads an ObjectRef from a byte slice and brands it with the GC lifetime.
    ///
    /// # Safety
    /// - `source` must contain a valid `Gc` pointer.
    /// - The pointer must belong to the arena associated with `gc`.
    pub unsafe fn read_branded(source: &[u8], _gc: GCHandle<'gc>) -> Self {
        Self::read_unchecked(source)
    }

    pub fn write(&self, dest: &mut [u8]) {
        let ptr: *const ThreadSafeLock<ObjectInner<'_>> = match self.0 {
            None => ptr::null(),
            Some(s) => Gc::as_ptr(s),
        };
        unsafe {
            // SAFETY: Use write_unaligned to avoid UB. Caller holds lock.
            (dest.as_mut_ptr() as *mut usize).write_unaligned(ptr as usize);
        }
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
        let inner = o.borrow();
        let HeapStorage::Obj(instance) = &inner.storage else {
            let variant = match &inner.storage {
                HeapStorage::Vec(_) => "Vec",
                HeapStorage::Obj(_) => "Obj",
                HeapStorage::Str(_) => "Str",
                HeapStorage::Boxed(_) => "Boxed",
            };
            panic!(
                "called ObjectRef::as_object on non-object heap reference: variant={}",
                variant
            )
        };

        op(instance)
    }

    pub fn as_object_mut<T>(&self, gc: GCHandle<'gc>, op: impl FnOnce(&mut Object<'gc>) -> T) -> T {
        let ObjectRef(Some(o)) = &self else {
            panic!(
                "NullReferenceException: called ObjectRef::as_object_mut on NULL object reference"
            )
        };
        let mut inner = o.borrow_mut(gc);
        let HeapStorage::Obj(instance) = &mut inner.storage else {
            let variant = match &inner.storage {
                HeapStorage::Vec(_) => "Vec",
                HeapStorage::Obj(_) => "Obj",
                HeapStorage::Str(_) => "Str",
                HeapStorage::Boxed(_) => "Boxed",
            };
            panic!(
                "called ObjectRef::as_object_mut on non-object heap reference: variant={}",
                variant
            )
        };

        op(instance)
    }

    pub fn as_vector<T>(&self, op: impl FnOnce(&Vector<'gc>) -> T) -> T {
        let ObjectRef(Some(o)) = &self else {
            panic!("NullReferenceException: called ObjectRef::as_vector on NULL object reference")
        };
        let inner = o.borrow();
        let HeapStorage::Vec(instance) = &inner.storage else {
            panic!("called ObjectRef::as_vector on non-vector heap reference")
        };

        op(instance)
    }

    pub fn as_vector_mut<T>(&self, gc: GCHandle<'gc>, op: impl FnOnce(&mut Vector<'gc>) -> T) -> T {
        let ObjectRef(Some(o)) = &self else {
            panic!(
                "NullReferenceException: called ObjectRef::as_vector_mut on NULL object reference"
            )
        };
        let mut inner = o.borrow_mut(gc);
        let HeapStorage::Vec(instance) = &mut inner.storage else {
            panic!("called ObjectRef::as_vector_mut on non-vector heap reference")
        };

        op(instance)
    }

    pub fn as_heap_storage<T>(&self, op: impl FnOnce(&HeapStorage<'gc>) -> T) -> T {
        let ObjectRef(Some(o)) = &self else {
            panic!("NullReferenceException: called ObjectRef::as_heap_storage on NULL object reference")
        };
        let inner = o.borrow();
        op(&inner.storage)
    }
}

impl Debug for ObjectRef<'_> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self.0 {
            None => f.write_str("NULL"),
            Some(gc) => {
                let inner = gc.borrow();
                let desc = match &inner.storage {
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
    pub fn size_bytes(&self) -> usize {
        match self {
            HeapStorage::Vec(v) => v.size_bytes(),
            HeapStorage::Obj(o) => o.size_bytes(),
            HeapStorage::Str(s) => s.size_bytes(),
            HeapStorage::Boxed(v) => v.size_bytes(),
        }
    }

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

    pub fn as_ptr(&self) -> *const u8 {
        match self {
            HeapStorage::Vec(v) => v.get().as_ptr(),
            HeapStorage::Str(s) => s.as_ptr() as *const u8,
            HeapStorage::Obj(o) => o.instance_storage.get().as_ptr(),
            HeapStorage::Boxed(ValueType::Struct(o)) => {
                o.instance_storage.get().as_ptr()
            }
            _ => ptr::null(),
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

impl<'gc> ValueType<'gc> {
    pub fn size_bytes(&self) -> usize {
        match self {
            ValueType::Struct(o) => o.size_bytes(),
            _ => size_of::<ValueType>(),
        }
    }

    pub fn resurrect(&self, fc: &gc_arena::Finalization<'gc>, visited: &mut HashSet<usize>) {
        match self {
            ValueType::Pointer(p) => p.resurrect(fc, visited),
            ValueType::Struct(o) => o.resurrect(fc, visited),
            _ => {}
        }
    }
}

#[derive(Debug)]
pub enum CTSValue<'gc> {
    Value(ValueType<'gc>),
    Ref(ObjectRef<'gc>),
}

impl<'gc> CTSValue<'gc> {
    pub fn into_stack(self, _gc: GCHandle<'gc>) -> StackValue<'gc> {
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
                TypedRef => todo!("typedref implementation"),
                Struct(o) => dest.copy_from_slice(&o.instance_storage.get()),
            },
            CTSValue::Ref(o) => o.write(dest),
        }
    }
}

// Manual implementation of Clone and PartialEq to handle ThreadSafeLock
pub struct Vector<'gc> {
    pub element: ConcreteType,
    pub layout: ArrayLayoutManager,
    pub(crate) storage: Vec<u8>,
    pub dims: Box<[usize]>,
    pub(crate) _contains_gc: PhantomData<fn(&'gc ()) -> &'gc ()>,
}

impl<'gc> Clone for Vector<'gc> {
    fn clone(&self) -> Self {
        Self {
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
        self.element == other.element
            && self.layout == other.layout
            && self.storage == other.storage
            && self.dims == other.dims
    }
}

unsafe impl Collect for Vector<'_> {
    #[inline]
    fn trace(&self, cc: &Collection) {
        let element = &self.layout.element_layout;
        match element.as_ref() {
            LayoutManager::Scalar(Scalar::ObjectRef) => {
                for i in 0..self.layout.length {
                    unsafe { ObjectRef::read_unchecked(&self.storage[(i * element.size())..]) }
                        .trace(cc);
                }
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
    pub fn new(
        element: ConcreteType,
        layout: ArrayLayoutManager,
        storage: Vec<u8>,
        dims: Vec<usize>,
    ) -> Self {
        Self {
            element,
            layout,
            storage,
            dims: dims.into_boxed_slice(),
            _contains_gc: PhantomData,
        }
    }

    pub fn size_bytes(&self) -> usize {
        size_of::<Vector>() + self.storage.len() + (self.dims.len() * size_of::<usize>())
    }

    pub fn resurrect(&self, fc: &gc_arena::Finalization<'gc>, visited: &mut HashSet<usize>) {
        self.layout.resurrect(&self.storage, fc, visited);
    }

    pub fn get(&self) -> &[u8] {
        &self.storage
    }

    pub fn get_mut(&mut self) -> &mut [u8] {
        &mut self.storage
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
                .chain(self.storage.chunks(self.layout.element_layout.size()).map(
                    match self.layout.element_layout.as_ref() {
                        LayoutManager::Scalar(Scalar::ObjectRef) => |chunk: &[u8]| {
                            format!("{:?}", unsafe { ObjectRef::read_unchecked(chunk) })
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
                    },
                ))
                .map(DebugStr),
            )
            .finish()
    }
}

pub struct Object<'gc> {
    pub description: TypeDescription,
    pub instance_storage: FieldStorage,
    pub finalizer_suppressed: bool,
    /// Sync block index for System.Threading.Monitor support.
    /// None means no sync block allocated yet (lazy allocation).
    pub sync_block_index: Option<usize>,
    pub _phantom: PhantomData<&'gc ()>,
}

impl<'gc> Clone for Object<'gc> {
    fn clone(&self) -> Self {
        Self {
            description: self.description,
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
            && self.instance_storage == other.instance_storage
            && self.finalizer_suppressed == other.finalizer_suppressed
            && self.sync_block_index == other.sync_block_index
    }
}

unsafe impl<'gc> Collect for Object<'gc> {
    fn trace(&self, cc: &Collection) {
        self.instance_storage.trace(cc);
        // ManagedPtr fields are self-contained and traced via their layout.
    }
}

impl<'gc> Object<'gc> {
    pub fn size_bytes(&self) -> usize {
        size_of::<Object>() + self.instance_storage.get().len()
    }

    pub fn resurrect(&self, fc: &gc_arena::Finalization<'gc>, visited: &mut HashSet<usize>) {
        self.instance_storage.resurrect(fc, visited);
    }

    pub fn new(description: TypeDescription, instance_storage: FieldStorage) -> Self {
        Self {
            description,
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
                self.instance_storage.get().as_ptr()
            )))
            .finish()
    }
}

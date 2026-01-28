use crate::{
    layout::{ArrayLayoutManager, HasLayout, LayoutManager, Scalar},
    pointer::{ManagedPtr, ManagedPtrOwner},
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
use gc_arena::{Collect, Collection, Gc, Mutation};
use std::{
    cmp::Ordering,
    collections::{HashMap, HashSet},
    fmt::{self, Debug, Formatter},
    hash::{Hash, Hasher},
    iter,
    marker::PhantomData,
    ptr::{self, NonNull},
};

#[cfg(feature = "multithreaded-gc")]
use dotnet_utils::gc::{get_currently_tracing, record_allocation, record_cross_arena_ref};

/// Owner information for managed pointers stored in metadata.
/// We MUST store actual Gc handles, not raw pointers, because:
/// - GC can move objects (invalidating raw pointers)
/// - We need to trace through these for GC correctness
/// - Gc::from_ptr() on stale pointers causes undefined behavior
#[derive(Clone, Debug)]
pub enum MetadataOwner<'gc> {
    None,
    Heap(ObjectHandle<'gc>),
    Stack(NonNull<Object<'gc>>), // Stack pointers are traced separately, don't reconstruct
}

unsafe impl<'gc> Collect for MetadataOwner<'gc> {
    fn trace(&self, cc: &Collection) {
        if let MetadataOwner::Heap(h) = self {
            h.trace(cc);
        }
    }
}

/// Metadata for managed pointers stored in object fields.
/// Since managed pointers in memory are pointer-sized (8 bytes), we store
/// the GC and type metadata in a side-table indexed by byte offset.
#[derive(Clone, Debug)]
pub struct ManagedPtrMetadata<'gc> {
    pub inner_type: TypeDescription,
    pub owner: MetadataOwner<'gc>,
    pub pinned: bool,
}

unsafe impl<'gc> Collect for ManagedPtrMetadata<'gc> {
    fn trace(&self, cc: &Collection) {
        self.owner.trace(cc);
    }
}

impl<'gc> PartialEq for ManagedPtrMetadata<'gc> {
    fn eq(&self, other: &Self) -> bool {
        self.inner_type == other.inner_type
            && match (&self.owner, &other.owner) {
                (MetadataOwner::None, MetadataOwner::None) => true,
                (MetadataOwner::Heap(a), MetadataOwner::Heap(b)) => Gc::ptr_eq(*a, *b),
                (MetadataOwner::Stack(a), MetadataOwner::Stack(b)) => a.as_ptr() == b.as_ptr(),
                _ => false,
            }
            && self.pinned == other.pinned
    }
}

impl<'gc> ManagedPtrMetadata<'gc> {
    pub fn from_managed_ptr(m: &ManagedPtr<'gc>) -> Self {
        let owner = match m.owner {
            Some(ManagedPtrOwner::Heap(h)) => MetadataOwner::Heap(h),
            Some(ManagedPtrOwner::Stack(s)) => MetadataOwner::Stack(s),
            None => MetadataOwner::None,
        };
        Self {
            inner_type: m.inner_type,
            owner,
            pinned: m.pinned,
        }
    }

    pub fn recover_owner(&self) -> Option<ManagedPtrOwner<'gc>> {
        // Now that we store actual Gc handles instead of raw pointers,
        // recovery is safe and straightforward!
        match self.owner {
            MetadataOwner::Heap(h) => Some(ManagedPtrOwner::Heap(h)),
            MetadataOwner::Stack(s) => Some(ManagedPtrOwner::Stack(s)),
            MetadataOwner::None => None,
        }
    }
}

/// Side-table for managed pointer metadata, indexed by byte offset in storage.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct ManagedPtrSideTable<'gc> {
    pub metadata: HashMap<usize, ManagedPtrMetadata<'gc>>,
}

unsafe impl<'gc> Collect for ManagedPtrSideTable<'gc> {
    fn trace(&self, cc: &Collection) {
        for m in self.metadata.values() {
            m.trace(cc);
        }
    }
}

impl<'gc> ManagedPtrSideTable<'gc> {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, offset: usize, metadata: ManagedPtrMetadata<'gc>) {
        self.metadata.insert(offset, metadata);
    }

    pub fn get(&self, offset: usize) -> Option<&ManagedPtrMetadata<'gc>> {
        self.metadata.get(&offset)
    }

    pub fn remove(&mut self, offset: usize) {
        self.metadata.remove(&offset);
    }
}

#[derive(Collect, Debug)]
#[collect(no_drop)]
pub struct ObjectInner<'gc> {
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
                Pointer(p) => {
                    // Write only the pointer value (8 bytes) to memory.
                    // Metadata should be stored in Object's side-table separately.
                    p.write_ptr_only(dest);
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
    /// Side-table for managed pointer metadata (and dynamic roots).
    pub managed_ptr_metadata: ThreadSafeLock<ManagedPtrSideTable<'gc>>,
    pub(crate) _contains_gc: PhantomData<fn(&'gc ()) -> &'gc ()>,
}

impl<'gc> Clone for Vector<'gc> {
    fn clone(&self) -> Self {
        let metadata = self.managed_ptr_metadata.borrow().clone();
        Self {
            element: self.element.clone(),
            layout: self.layout.clone(),
            storage: self.storage.clone(),
            managed_ptr_metadata: ThreadSafeLock::new(metadata),
            _contains_gc: PhantomData,
        }
    }
}

impl<'gc> PartialEq for Vector<'gc> {
    fn eq(&self, other: &Self) -> bool {
        self.element == other.element
            && self.layout == other.layout
            && self.storage == other.storage
            && *self.managed_ptr_metadata.borrow() == *other.managed_ptr_metadata.borrow()
    }
}

unsafe impl Collect for Vector<'_> {
    #[inline]
    fn trace(&self, cc: &Collection) {
        // Trace managed pointer owners from side-table
        self.managed_ptr_metadata.trace(cc);

        let element = &self.layout.element_layout;
        match element.as_ref() {
            LayoutManager::Scalar(Scalar::ObjectRef) => {
                for i in 0..self.layout.length {
                    unsafe { ObjectRef::read_unchecked(&self.storage[(i * element.size())..]) }
                        .trace(cc);
                }
            }
            LayoutManager::Scalar(Scalar::ManagedPtr) => {
                // NOTE: Arrays cannot contain managed pointers in valid .NET code.
                // Managed pointers (&T) are only allowed as local variables, method parameters,
                // return values, and fields of ref structs (which can only exist on the stack).
                // ECMA-335 Part III §1.8.1.1 forbids managed pointers in arrays.
                // If we reach here, it's likely a bug in the bytecode or our verification.
                // Do nothing - there's no metadata to trace since arrays can't contain managed ptrs.
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
    pub fn new(element: ConcreteType, layout: ArrayLayoutManager, storage: Vec<u8>) -> Self {
        Self {
            element,
            layout,
            storage,
            managed_ptr_metadata: ThreadSafeLock::new(ManagedPtrSideTable::new()),
            _contains_gc: PhantomData,
        }
    }

    pub fn register_metadata(
        &self,
        offset: usize,
        metadata: ManagedPtrMetadata<'gc>,
        mc: &Mutation<'gc>,
    ) {
        let mut side_table = self.managed_ptr_metadata.borrow_mut(mc);
        side_table.insert(offset, metadata);
    }

    pub fn size_bytes(&self) -> usize {
        size_of::<Vector>() + self.storage.len()
    }

    pub fn resurrect(&self, fc: &gc_arena::Finalization<'gc>, visited: &mut HashSet<usize>) {
        self.layout.resurrect(&self.storage, fc, visited);

        // Resurrect managed pointer owners from side-table
        let side_table = self.managed_ptr_metadata.borrow();
        for metadata in side_table.metadata.values() {
            if let MetadataOwner::Heap(handle) = metadata.owner {
                let ptr = Gc::as_ptr(handle) as usize;
                if visited.insert(ptr) {
                    Gc::resurrect(fc, handle);
                    handle.borrow().storage.resurrect(fc, visited);
                }
            }
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
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_list()
            .entries(
                iter::once(format!(
                    "vector of {:?} (length {})",
                    self.element, self.layout.length
                ))
                .chain(self.storage.chunks(self.layout.element_layout.size()).map(
                    match self.layout.element_layout.as_ref() {
                        LayoutManager::Scalar(Scalar::ObjectRef) => {
                            |chunk: &[u8]| format!("{:?}", unsafe { ObjectRef::read_unchecked(chunk) })
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

pub struct Object<'gc> {
    pub description: TypeDescription,
    pub instance_storage: FieldStorage,
    pub finalizer_suppressed: bool,
    /// Sync block index for System.Threading.Monitor support.
    /// None means no sync block allocated yet (lazy allocation).
    pub sync_block_index: Option<usize>,
    /// Side-table for managed pointer metadata.
    /// Only allocated when the object contains managed pointer fields.
    pub managed_ptr_metadata: ThreadSafeLock<ManagedPtrSideTable<'gc>>,
    pub _phantom: PhantomData<&'gc ()>,
}

impl<'gc> Clone for Object<'gc> {
    fn clone(&self) -> Self {
        let metadata = self.managed_ptr_metadata.borrow().clone();
        Self {
            description: self.description,
            instance_storage: self.instance_storage.clone(),
            finalizer_suppressed: self.finalizer_suppressed,
            sync_block_index: self.sync_block_index,
            managed_ptr_metadata: ThreadSafeLock::new(metadata),
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
            && *self.managed_ptr_metadata.borrow() == *other.managed_ptr_metadata.borrow()
    }
}

unsafe impl<'gc> Collect for Object<'gc> {
    fn trace(&self, cc: &Collection) {
        self.instance_storage.trace(cc);

        // Trace managed pointer owners from side-table
        // Now that we store actual Gc handles, tracing is safe!
        self.managed_ptr_metadata.trace(cc);
    }
}

impl<'gc> Object<'gc> {
    pub fn size_bytes(&self) -> usize {
        size_of::<Object>() + self.instance_storage.get().len()
    }

    pub fn resurrect(&self, fc: &gc_arena::Finalization<'gc>, visited: &mut HashSet<usize>) {
        self.instance_storage.resurrect(fc, visited);

        // Resurrect managed pointer owners from side-table
        let side_table = self.managed_ptr_metadata.borrow();
        for metadata in side_table.metadata.values() {
            if let MetadataOwner::Heap(handle) = metadata.owner {
                let ptr = Gc::as_ptr(handle) as usize;
                if visited.insert(ptr) {
                    Gc::resurrect(fc, handle);
                    handle.borrow().storage.resurrect(fc, visited);
                }
            }
        }
    }
    pub fn new(description: TypeDescription, instance_storage: FieldStorage) -> Self {
        Self {
            description,
            instance_storage,
            finalizer_suppressed: false,
            sync_block_index: None,
            managed_ptr_metadata: ThreadSafeLock::new(ManagedPtrSideTable::new()),
            _phantom: PhantomData,
        }
    }

    /// Register a managed pointer's metadata in the side-table.
    pub fn register_managed_ptr(&self, offset: usize, m: &ManagedPtr<'gc>, mc: &Mutation<'gc>) {
        let mut side_table = self.managed_ptr_metadata.borrow_mut(mc);
        side_table.insert(offset, ManagedPtrMetadata::from_managed_ptr(m));
    }

    /// Register metadata directly in the side-table.
    pub fn register_metadata(
        &self,
        offset: usize,
        metadata: ManagedPtrMetadata<'gc>,
        mc: &Mutation<'gc>,
    ) {
        let mut side_table = self.managed_ptr_metadata.borrow_mut(mc);
        side_table.insert(offset, metadata);
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

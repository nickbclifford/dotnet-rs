use crate::{
    ArenaId, StackValue,
    layout::{ArrayLayoutManager, HasLayout, LayoutManager, Scalar},
    pointer::ManagedPtr,
    storage::FieldStorage,
    string::CLRString,
};
use dotnet_types::{
    TypeDescription,
    generics::{ConcreteType, GenericLookup},
};
use dotnet_utils::{
    DebugStr,
    gc::{GCHandle, ThreadSafeLock},
    sync::get_current_thread_id,
};
use gc_arena::{Collect, Collection, Gc, Mutation};
use std::{
    cmp::Ordering,
    collections::HashSet,
    fmt::{self, Debug, Formatter},
    hash::{Hash, Hasher},
    iter,
    marker::PhantomData,
    mem::size_of,
    ptr::NonNull,
    sync::Arc,
};

#[cfg(feature = "fuzzing")]
use arbitrary::Arbitrary;

#[cfg(feature = "multithreading")]
use dotnet_utils::gc::{get_currently_tracing, record_cross_arena_ref};

#[cfg(all(feature = "multithreading", feature = "memory-validation"))]
use dotnet_utils::gc::{is_stw_in_progress, is_valid_cross_arena_ref};

#[cfg(feature = "fuzzing")]
static OBJECT_REGISTRY: std::sync::LazyLock<dashmap::DashSet<usize>> =
    std::sync::LazyLock::new(dashmap::DashSet::new);

#[cfg(feature = "fuzzing")]
pub fn register_object_ptr(ptr: usize) {
    OBJECT_REGISTRY.insert(ptr);
}

#[cfg(feature = "fuzzing")]
pub fn is_valid_object_ptr(ptr: usize) -> bool {
    OBJECT_REGISTRY.contains(&ptr)
}

#[cfg(feature = "fuzzing")]
pub fn clear_object_registry() {
    OBJECT_REGISTRY.clear();
}

#[cfg(any(feature = "memory-validation", debug_assertions))]
pub const OBJECT_MAGIC: u64 = 0x5AFE_0B1E_C700_0000;

#[derive(Collect, Debug)]
#[collect(no_drop)]
#[repr(C)]
pub struct ObjectInner<'gc> {
    #[cfg(any(feature = "memory-validation", debug_assertions))]
    pub magic: u64,
    pub owner_id: ArenaId,
    pub storage: HeapStorage<'gc>,
}

impl<'gc> ObjectInner<'gc> {
    pub fn validate_arena_id(&self) {
        #[cfg(feature = "memory-validation")]
        {
            let current_id = get_current_thread_id();
            if self.owner_id != current_id && self.owner_id != ArenaId::INVALID {
                // In multithreading mode, this might be a valid cross-arena reference.
                // But it's unsafe to borrow it directly without coordination.
                #[cfg(not(feature = "multithreading"))]
                panic!(
                    "Arena mismatch: object owned by {:?}, but accessed by {:?}",
                    self.owner_id, current_id
                );

                #[cfg(feature = "multithreading")]
                {
                    if !is_valid_cross_arena_ref(self.owner_id) {
                        panic!(
                            "Dangling cross-arena reference: arena {:?} is no longer valid (thread exited?)",
                            self.owner_id
                        );
                    }

                    if is_stw_in_progress() && get_currently_tracing().is_none() {
                        panic!(
                            "Uncoordinated cross-arena access during STW GC: object owned by {:?}, accessed by {:?}",
                            self.owner_id, current_id
                        );
                    }
                }
            }
        }
    }

    pub fn validate_magic(&self) {
        #[cfg(any(feature = "memory-validation", debug_assertions))]
        {
            if self.magic != OBJECT_MAGIC {
                panic!("Object magic number corrupted: {:#x}", self.magic);
            }
        }
        self.validate_arena_id();
    }

    pub fn validate_resurrection_invariants(&self) {
        self.validate_magic();
        self.storage.validate_resurrection_invariants();
    }
}

#[repr(transparent)]
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ObjectPtr(NonNull<ThreadSafeLock<ObjectInner<'static>>>);

#[cfg(feature = "fuzzing")]
impl<'a> Arbitrary<'a> for ObjectPtr {
    fn arbitrary(u: &mut arbitrary::Unstructured<'a>) -> arbitrary::Result<Self> {
        let ptr_val: usize = u.arbitrary()?;
        Ok(unsafe { std::mem::transmute::<usize, ObjectPtr>(ptr_val) })
    }
}

// SAFETY: ObjectPtr is a transparent wrapper around a NonNull pointer to an ObjectInner.
// ObjectInner is managed by the VM and thread-safety is handled via ThreadSafeLock.
// This type is used primarily for cross-arena references where raw pointers are required.
unsafe impl Send for ObjectPtr {}
unsafe impl Sync for ObjectPtr {}

impl ObjectPtr {
    pub(crate) fn from_handle<'gc>(handle: ObjectHandle<'gc>) -> Self {
        ObjectPtr(NonNull::new(Gc::as_ptr(handle) as *mut _).unwrap())
    }

    /// # Safety
    ///
    /// The pointer must be valid for the program lifetime and properly aligned.
    pub unsafe fn from_raw(ptr: *const ThreadSafeLock<ObjectInner<'static>>) -> Option<Self> {
        NonNull::new(ptr as *mut _).map(ObjectPtr)
    }

    pub fn as_ptr(&self) -> *const ThreadSafeLock<ObjectInner<'static>> {
        self.0.as_ptr()
    }

    pub fn owner_id(&self) -> ArenaId {
        unsafe { (*self.0.as_ref().as_ptr()).owner_id }
    }

    pub fn with_data<T>(&self, f: impl FnOnce(&[u8]) -> T) -> T {
        let inner = unsafe { self.0.as_ref().borrow() };
        inner.storage.with_data(f)
    }

    pub fn with_data_mut<T>(&self, _gc: GCHandle<'_>, f: impl FnOnce(&mut [u8]) -> T) -> T {
        // SAFETY: We have GCHandle which proves we are in a mutation context.
        // Even if the object is in another arena, we can still write to its storage
        // because we hold the ThreadSafeLock.
        let mut inner = unsafe { self.0.as_ref().borrow_mut(_gc.mutation()) };
        inner.storage.with_data_mut(f)
    }

    pub fn as_heap_storage<T>(&self, f: impl FnOnce(&HeapStorage<'static>) -> T) -> T {
        let inner = unsafe { self.0.as_ref().borrow() };
        f(&inner.storage)
    }
}

pub type ObjectHandle<'gc> = Gc<'gc, ThreadSafeLock<ObjectInner<'gc>>>;

#[derive(Copy, Clone)]
#[repr(transparent)]
pub struct ObjectRef<'gc>(pub Option<ObjectHandle<'gc>>);

#[cfg(feature = "fuzzing")]
impl<'a, 'gc> Arbitrary<'a> for ObjectRef<'gc> {
    fn arbitrary(u: &mut arbitrary::Unstructured<'a>) -> arbitrary::Result<Self> {
        let ptr_val: usize = u.arbitrary()?;
        // SAFETY: Used only for serialization fuzzing. Do NOT dereference.
        Ok(unsafe { std::mem::transmute::<usize, ObjectRef<'_>>(ptr_val) })
    }
}

unsafe impl<'gc> Collect for ObjectRef<'gc> {
    fn trace(&self, cc: &Collection) {
        if let Some(h) = self.0 {
            #[cfg(feature = "multithreading")]
            {
                // Check for cross-arena reference
                if let Some(tracing_id) = get_currently_tracing() {
                    // SAFETY: During stop-the-world GC, no other threads are running,
                    // so we can safely access the owner_id without acquiring the lock.
                    // This avoids potential deadlock if a thread was stopped while holding a write lock.
                    let lock_ptr: *const ThreadSafeLock<ObjectInner> = Gc::as_ptr(h);
                    let owner_id = unsafe { (*(*lock_ptr).as_ptr()).owner_id };
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
// we assume this type is pointer-sized basically everywhere (for serialization/layout purposes)
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
    /// The serialized size when written to memory (always a single pointer/usize).
    /// This is the size used for layout calculations and array element size.
    pub const SIZE: usize = size_of::<usize>();

    /// Deprecated: Use SIZE instead. This constant was renamed for clarity.
    #[deprecated(note = "Use SIZE instead (now correctly represents serialized size)")]
    pub const SERIALIZED_SIZE: usize = size_of::<usize>();

    pub fn pointer(&self) -> Option<NonNull<u8>> {
        self.0.map(|h| {
            let inner = h.borrow();
            NonNull::new(unsafe { inner.storage.raw_data_ptr() })
                .expect("Object storage pointer is null")
        })
    }

    pub fn as_ptr(&self) -> Option<ObjectPtr> {
        self.0
            .map(|h| ObjectPtr::from_handle(h))
    }

    #[cfg(feature = "multithreading")]
    pub fn as_ptr_info(&self) -> Option<(ObjectPtr, ArenaId)> {
        self.0.map(|h| {
            let ptr = ObjectPtr::from_handle(h);
            let tid = ptr.owner_id();
            (ptr, tid)
        })
    }

    pub fn resurrect(
        &self,
        fc: &'gc gc_arena::Finalization<'gc>,
        visited: &mut HashSet<usize>,
        depth: usize,
    ) {
        if depth > 1000 {
            panic!(
                "Resurrection depth exceeded (possible infinite recursion in custom finalizers or corrupt graph)"
            );
        }
        if let Some(handle) = self.0 {
            let ptr = Gc::as_ptr(handle) as usize;
            if visited.insert(ptr) {
                let inner = handle.borrow();
                inner.validate_resurrection_invariants();
                drop(inner);

                Gc::resurrect(fc, handle);
                handle.borrow().storage.resurrect(fc, visited, depth + 1);
            }
        }
    }

    pub fn new(gc: GCHandle<'gc>, value: HeapStorage<'gc>) -> Self {
        let owner_id = get_current_thread_id();
        #[cfg(feature = "multithreading")]
        {
            let size = size_of::<ObjectInner>() + value.size_bytes();
            gc.record_allocation(size);
        }

        let h = Gc::new(
            &gc,
            ThreadSafeLock::new(ObjectInner {
                #[cfg(any(feature = "memory-validation", debug_assertions))]
                magic: OBJECT_MAGIC,
                owner_id,
                storage: value,
            }),
        );

        #[cfg(feature = "fuzzing")]
        register_object_ptr(Gc::as_ptr(h) as usize);

        Self(Some(h))
    }

    /// Reads an ObjectRef from a byte slice without lifetime branding.
    ///
    /// # Safety
    /// - `source` must contain a valid, properly aligned `Gc` pointer (or null).
    /// - The caller must ensure the returned `ObjectRef` does not outlive the
    ///   arena generation it belongs to.
    pub unsafe fn read_unchecked(source: &[u8]) -> Self {
        unsafe {
            let ptr_val = {
                // SAFETY: Use read_unaligned to avoid UB on unaligned access.
                // We assume the caller holds a lock (FieldStorage RwLock) to prevent tearing.
                (source.as_ptr() as *const usize).read_unaligned()
            };

            #[cfg(feature = "multithreading")]
            {
                let tag = ptr_val & 7;
                if tag == 5 {
                    // This is a CrossArenaObjectRef (Tag 5).
                    // Remove the tag to get the real pointer, then record it for coordinated GC.
                    let real_ptr = (ptr_val & !7) as *const ThreadSafeLock<ObjectInner<'static>>;
                    let owner_id = (*(*real_ptr).as_ptr()).owner_id;
                    record_cross_arena_ref(owner_id, real_ptr as usize);

                    // Continue with the untagged pointer to construct the ObjectRef
                    let ptr = real_ptr.cast::<ThreadSafeLock<ObjectInner<'gc>>>();

                    #[cfg(any(feature = "memory-validation", debug_assertions))]
                    {
                        use std::mem::align_of;
                        if !(real_ptr as usize)
                            .is_multiple_of(align_of::<ThreadSafeLock<ObjectInner<'static>>>())
                        {
                            panic!(
                                "ObjectRef::read: Pointer {:#x} is not aligned",
                                real_ptr as usize
                            );
                        }

                        // Verify magic number
                        let inner = &*(*ptr).as_ptr();
                        if inner.magic != OBJECT_MAGIC {
                            panic!(
                                "ObjectRef::read: CrossArena pointer {:#x} points to invalid object (bad magic: {:#x})",
                                real_ptr as usize, inner.magic
                            );
                        }
                    }

                    return ObjectRef(Some(Gc::from_ptr(ptr)));
                }
            }

            // If bit 0 is set and it's not Tag 5, it's a tagged pointer (Stack, Static, or other metadata).
            // These should not be traced as ObjectRefs.
            if ptr_val & 1 != 0 {
                return ObjectRef(None);
            }

            let ptr = ptr_val as *const ThreadSafeLock<ObjectInner<'gc>>;

            if ptr.is_null() {
                ObjectRef(None)
            } else {
                #[cfg(any(feature = "memory-validation", debug_assertions))]
                {
                    use std::mem::align_of;
                    if !ptr_val.is_multiple_of(align_of::<ThreadSafeLock<ObjectInner<'static>>>()) {
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
    }

    /// Reads an ObjectRef from a byte slice and brands it with the GC lifetime.
    ///
    /// # Safety
    /// - `source` must contain a valid `Gc` pointer.
    /// - The pointer must belong to the arena associated with `gc`.
    pub unsafe fn read_branded(source: &[u8], _gc: &Mutation<'gc>) -> Self {
        unsafe { Self::read_unchecked(source) }
    }

    pub fn write(&self, dest: &mut [u8]) {
        let ptr_val: usize = match self.0 {
            None => 0,
            Some(s) => {
                let ptr = Gc::as_ptr(s) as usize;
                #[cfg(feature = "multithreading")]
                {
                    // Tag cross-arena references with Tag 5
                    // SAFETY: We can safely access owner_id during write because:
                    // 1. The Gc<T> is alive (we hold a reference via self.0)
                    // 2. We use raw pointer access to avoid deadlocking with the write lock on the owner object.
                    //    The owner_id is immutable after object creation.
                    let current_id = get_current_thread_id();
                    let owner_id = unsafe { (*(*Gc::as_ptr(s)).as_ptr()).owner_id };
                    if owner_id != current_id && owner_id != ArenaId::INVALID {
                        // This is a cross-arena reference - tag it with Tag 5
                        // The pointer must have low bits clear (it's allocated by gc-arena)
                        debug_assert_eq!(ptr & 7, 0, "Pointer low bits should be clear");
                        ptr | 5
                    } else {
                        ptr
                    }
                }
                #[cfg(not(feature = "multithreading"))]
                ptr
            }
        };
        unsafe {
            // SAFETY: Use write_unaligned to avoid UB. Caller holds lock.
            (dest.as_mut_ptr() as *mut usize).write_unaligned(ptr_val);
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
        inner.validate_magic();
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
        let mut inner = o.borrow_mut(&gc);
        inner.validate_magic();
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
        inner.validate_magic();
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
        let mut inner = o.borrow_mut(&gc);
        inner.validate_magic();
        let HeapStorage::Vec(instance) = &mut inner.storage else {
            panic!("called ObjectRef::as_vector_mut on non-vector heap reference")
        };

        op(instance)
    }

    pub fn as_heap_storage<T>(&self, op: impl FnOnce(&HeapStorage<'gc>) -> T) -> T {
        let ObjectRef(Some(o)) = &self else {
            panic!(
                "NullReferenceException: called ObjectRef::as_heap_storage on NULL object reference"
            )
        };
        let inner = o.borrow();
        inner.validate_magic();
        op(&inner.storage)
    }

    /// Safely accesses the object's data as a byte slice.
    ///
    /// This method ensures that the object is locked for the duration of the access
    /// and provides a stable view of the underlying data.
    pub fn with_data<T>(&self, f: impl FnOnce(&[u8]) -> T) -> T {
        let ObjectRef(Some(o)) = &self else {
            panic!("NullReferenceException: called ObjectRef::with_data on NULL object reference")
        };
        let inner = o.borrow();
        inner.validate_magic();
        match &inner.storage {
            HeapStorage::Vec(v) => f(&v.storage),
            HeapStorage::Obj(o) => o.instance_storage.with_data(f),
            HeapStorage::Str(s) => unsafe {
                let bytes = std::slice::from_raw_parts(s.as_ptr() as *const u8, s.len() * 2);
                f(bytes)
            },
            HeapStorage::Boxed(o) => o.instance_storage.with_data(f),
        }
    }

    /// Safely accesses the object's data as a mutable byte slice.
    ///
    /// This method ensures that the object is locked for the duration of the access
    /// and provides a stable, mutable view of the underlying data.
    pub fn with_data_mut<T>(&self, gc: GCHandle<'gc>, f: impl FnOnce(&mut [u8]) -> T) -> T {
        let ObjectRef(Some(o)) = &self else {
            panic!(
                "NullReferenceException: called ObjectRef::with_data_mut on NULL object reference"
            )
        };
        let mut inner = o.borrow_mut(&gc);
        inner.validate_magic();
        match &mut inner.storage {
            HeapStorage::Vec(v) => f(&mut v.storage),
            HeapStorage::Obj(o) => o.instance_storage.with_data_mut(f),
            HeapStorage::Boxed(o) => o.instance_storage.with_data_mut(f),
            HeapStorage::Str(_) => {
                panic!("Strings are immutable and cannot be accessed via with_data_mut")
            }
        }
    }
}

impl Debug for ObjectRef<'_> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self.0 {
            None => f.write_str("NULL"),
            Some(gc) => {
                let inner = gc.borrow();
                inner.validate_magic();
                let desc = match &inner.storage {
                    HeapStorage::Obj(o) => o.description.type_name(),
                    HeapStorage::Vec(v) => format!("{:?}[{}]", v.element, v.layout.length),
                    HeapStorage::Str(s) => format!("{:?}", s),
                    HeapStorage::Boxed(o) => format!("boxed {}", o.description.type_name()),
                };
                write!(f, "{} @ {:#?}", desc, Gc::as_ptr(gc))
            }
        }
    }
}

impl<'gc> crate::ptr_common::PointerLike for ObjectRef<'gc> {
    #[inline]
    fn pointer(&self) -> Option<NonNull<u8>> {
        ObjectRef::pointer(self)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum HeapStorage<'gc> {
    Vec(Vector<'gc>),
    Obj(Object<'gc>),
    Str(CLRString),
    Boxed(Object<'gc>),
}

// SAFETY: HeapStorage is an enum where each variant either implements Collect or
// contains no GC references (like CLRString). We manually trace each variant.
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
            HeapStorage::Boxed(o) => o.size_bytes(),
        }
    }

    pub fn validate_resurrection_invariants(&self) {
        match self {
            HeapStorage::Vec(v) => v.validate_resurrection_invariants(),
            HeapStorage::Obj(o) => o.validate_resurrection_invariants(),
            HeapStorage::Str(_) => {}
            HeapStorage::Boxed(o) => o.validate_resurrection_invariants(),
        }
    }

    pub fn resurrect(
        &self,
        fc: &'gc gc_arena::Finalization<'gc>,
        visited: &mut HashSet<usize>,
        depth: usize,
    ) {
        match self {
            HeapStorage::Vec(v) => v.resurrect(fc, visited, depth),
            HeapStorage::Obj(o) => o.resurrect(fc, visited, depth),
            HeapStorage::Boxed(o) => o.resurrect(fc, visited, depth),
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

    pub fn with_data<T>(&self, f: impl FnOnce(&[u8]) -> T) -> T {
        match self {
            HeapStorage::Vec(v) => f(&v.storage),
            HeapStorage::Str(s) => unsafe {
                let bytes = std::slice::from_raw_parts(s.as_ptr() as *const u8, s.len() * 2);
                f(bytes)
            },
            HeapStorage::Obj(o) => o.instance_storage.with_data(f),
            HeapStorage::Boxed(o) => o.instance_storage.with_data(f),
        }
    }

    pub fn with_data_mut<T>(&mut self, f: impl FnOnce(&mut [u8]) -> T) -> T {
        match self {
            HeapStorage::Vec(v) => f(&mut v.storage),
            HeapStorage::Str(_) => panic!("Cannot modify CLRString directly"),
            HeapStorage::Obj(o) => o.instance_storage.with_data_mut(f),
            HeapStorage::Boxed(o) => o.instance_storage.with_data_mut(f),
        }
    }

    /// Returns a pointer to the raw data without acquiring a lock.
    ///
    /// # Safety
    /// The caller must ensure that the lock is held elsewhere (e.g. during STW GC)
    /// or that the data is otherwise stable and no writers are active.
    pub unsafe fn raw_data_ptr(&self) -> *mut u8 {
        match self {
            HeapStorage::Vec(v) => unsafe { v.raw_data_ptr() },
            HeapStorage::Str(s) => s.as_ptr() as *mut u8,
            HeapStorage::Obj(o) => unsafe { o.instance_storage.raw_data_ptr() },
            HeapStorage::Boxed(o) => unsafe { o.instance_storage.raw_data_ptr() },
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
    TypedRef(ManagedPtr<'gc>, Arc<TypeDescription>),
    Struct(Object<'gc>),
}

// SAFETY: ValueType is an enum where only Pointer and Struct variants contain
// potential GC references. We manually trace these variants.
unsafe impl<'gc> Collect for ValueType<'gc> {
    fn trace(&self, cc: &Collection) {
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
            Value(TypedRef(p, t)) => StackValue::TypedRef(p, t),
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
const VECTOR_MAGIC: u64 = 0x5AFE_7EC7_0B00_0000;

// Manual implementation of Clone and PartialEq to handle ThreadSafeLock
pub struct Vector<'gc> {
    #[cfg(any(feature = "memory-validation", debug_assertions))]
    magic: u64,
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
            magic: VECTOR_MAGIC,
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
unsafe impl Collect for Vector<'_> {
    #[inline]
    fn trace(&self, cc: &Collection) {
        self.validate_magic();
        let element = &self.layout.element_layout;
        match element.as_ref() {
            LayoutManager::Scalar(Scalar::ObjectRef) => {
                for i in 0..self.layout.length {
                    unsafe {
                        ObjectRef::read_unchecked(&self.storage[(element.size() * i).as_usize()..])
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
            magic: VECTOR_MAGIC,
            element,
            layout,
            storage,
            dims: dims.into_boxed_slice(),
            _contains_gc: PhantomData,
        }
    }

    fn validate_magic(&self) {
        #[cfg(any(feature = "memory-validation", debug_assertions))]
        {
            if self.magic != VECTOR_MAGIC {
                panic!("Vector magic number corrupted: {:#x}", self.magic);
            }
        }
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

    /// Returns a pointer to the raw data without acquiring a lock.
    ///
    /// # Safety
    /// The caller must ensure that the lock is held elsewhere (e.g. during STW GC)
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
    pub finalizer_suppressed: bool,
    /// Sync block index for System.Threading.Monitor support.
    /// None means no sync block allocated yet (lazy allocation).
    pub sync_block_index: Option<usize>,
    pub _phantom: PhantomData<&'gc ()>,
}

#[cfg(feature = "fuzzing")]
impl<'a, 'gc> Arbitrary<'a> for Object<'gc> {
    fn arbitrary(u: &mut arbitrary::Unstructured<'a>) -> arbitrary::Result<Self> {
        Ok(Self {
            description: u.arbitrary()?,
            generics: u.arbitrary()?,
            instance_storage: crate::storage::FieldStorage::new(
                std::sync::Arc::new(crate::layout::FieldLayoutManager {
                    fields: std::collections::HashMap::new(),
                    total_size: 0,
                    alignment: 1,
                    gc_desc: crate::layout::GcDesc::default(),
                    has_ref_fields: false,
                }),
                vec![],
            ),
            finalizer_suppressed: u.arbitrary()?,
            sync_block_index: u.arbitrary()?,
            _phantom: PhantomData,
        })
    }
}

impl<'gc> Clone for Object<'gc> {
    fn clone(&self) -> Self {
        Self {
            description: self.description,
            generics: self.generics.clone(),
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
            && self.generics == other.generics
            && self.instance_storage == other.instance_storage
            && self.finalizer_suppressed == other.finalizer_suppressed
            && self.sync_block_index == other.sync_block_index
    }
}

impl<'gc> Eq for Object<'gc> {}

// SAFETY: Object contains field storage that may hold GC pointers.
// We use the layout manager associated with the type description to trace fields.
unsafe impl<'gc> Collect for Object<'gc> {
    fn trace(&self, cc: &Collection) {
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
        Self {
            description,
            generics,
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
                self.instance_storage.with_data(|d| d.as_ptr())
            )))
            .finish()
    }
}

use crate::{ArenaId, ValidationTag, ptr_common::PointerLike};
use dotnet_utils::{
    gc::{GCHandle, ThreadSafeLock},
    sync::get_current_thread_id,
};
use gc_arena::{Collect, Gc, Mutation, collect::Trace};
use std::{
    cmp::Ordering,
    collections::HashSet,
    fmt::{self, Debug, Formatter},
    hash::{Hash, Hasher},
    mem::size_of,
    ptr::NonNull,
};

#[cfg(feature = "fuzzing")]
use arbitrary::Arbitrary;

#[cfg(feature = "multithreading")]
use dotnet_utils::gc::{get_currently_tracing, record_cross_arena_ref, try_acquire_lease};

#[cfg(all(feature = "multithreading", feature = "memory-validation"))]
use dotnet_utils::gc::{is_stw_in_progress, is_valid_cross_arena_ref};

pub mod heap_storage;
pub mod types;

pub use heap_storage::*;
pub use types::*;

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

pub const OBJECT_MAGIC: u64 = 0x5AFE_0B1E_C700_0000;

#[derive(Collect, Debug)]
#[collect(no_drop)]
#[repr(C)]
pub struct ObjectInner<'gc> {
    pub magic: ValidationTag,
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

                    if is_stw_in_progress(self.owner_id) && get_currently_tracing().is_none() {
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
        self.magic.validate(OBJECT_MAGIC, "ObjectInner");
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

unsafe impl<'gc> Collect<'gc> for ObjectRef<'gc> {
    fn trace<Tr: Trace<'gc>>(&self, cc: &mut Tr) {
        if let Some(h) = self.0 {
            #[cfg(feature = "multithreading")]
            {
                // Check for cross-arena reference
                if let Some(tracing_id) = get_currently_tracing() {
                    let lock_ptr: *const ThreadSafeLock<ObjectInner> = Gc::as_ptr(h);
                    // DANGER: lock_ptr might be dangling if the arena exited.
                    // But in STW, we assume it's either local or the other arena is also stopped.
                    // All threads MUST be at a safepoint during finish_marking.
                    let owner_id = unsafe { (*(*lock_ptr).as_ptr()).owner_id };
                    if owner_id != tracing_id {
                        let ptr = Gc::as_ptr(h) as usize;
                        if !record_cross_arena_ref(owner_id, ptr) {
                            return;
                        }
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

    pub fn pointer(&self) -> Option<NonNull<u8>> {
        self.0.map(|h| {
            let inner = h.borrow();
            NonNull::new(unsafe { inner.storage.raw_data_ptr() })
                .expect("Object storage pointer is null")
        })
    }

    pub fn as_ptr(&self) -> Option<ObjectPtr> {
        self.0.map(|h| ObjectPtr::from_handle(h))
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
                magic: ValidationTag::new(OBJECT_MAGIC),
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
                    // Recover the 16-bit ArenaId from the upper bits and the pointer from the lower 48 bits.
                    // This avoids dereferencing the pointer during deserialization/GC,
                    // which is critical for robustness against race conditions.
                    let owner_id_u16 = (ptr_val >> 48) as u16;
                    // Sign-extend from 16-bit to correctly handle ArenaId::INVALID (u64::MAX)
                    let owner_id = ArenaId::new(owner_id_u16 as i16 as i64 as u64);
                    let real_ptr_val = ptr_val & 0x0000FFFFFFFFFFF8;
                    let real_ptr = real_ptr_val as *const ThreadSafeLock<ObjectInner<'static>>;

                    // Acquire a lease to pin the arena while we reconstruct the Gc pointer.
                    // This ensures the memory remains valid during magic check and construction.
                    let lease = try_acquire_lease(owner_id);
                    if lease.is_none() {
                        return ObjectRef(None);
                    }
                    let _lease = lease;

                    // Also record it if we are tracing
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
                        inner
                            .magic
                            .validate(OBJECT_MAGIC, "ObjectRef::read (CrossArena)");
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
                    inner.magic.validate(OBJECT_MAGIC, "ObjectRef::read");
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
                    // Encode arena ownership with Tag 5 for every managed owner.
                    // This keeps ownership metadata intact when references flow through
                    // shared storage (e.g. statics) and are later read on another thread.
                    // SAFETY: We can safely access owner_id during write because:
                    // 1. The Gc<T> is alive (we hold a reference via self.0)
                    // 2. We use raw pointer access to avoid deadlocking with the write lock on the owner object.
                    //    The owner_id is immutable after object creation.
                    let lock_ptr = Gc::as_ptr(s);
                    // DANGER: speculative read of owner_id.
                    // Safe because owner_id is immutable after object creation.
                    let owner_id = unsafe { (*(*lock_ptr).as_ptr()).owner_id };
                    if owner_id != ArenaId::INVALID {
                        // Verify with a lease that the arena is still alive before
                        // we encode its ID into the tagged reference.
                        if let Some(_lease) = try_acquire_lease(owner_id) {
                            // Store pointer + owner arena id in a stable tagged form.
                            // Store the 16-byte ArenaId in the upper bits of the pointer.
                            // We assume 48-bit addressing (standard on current 64-bit OSs).
                            debug_assert_eq!(
                                ptr & 0xFFFF000000000000,
                                0,
                                "Pointer upper bits should be clear"
                            );
                            debug_assert_eq!(ptr & 7, 0, "Pointer low bits should be clear");
                            (ptr & 0x0000FFFFFFFFFFFF) | 5 | ((owner_id.as_u64() as usize) << 48)
                        } else {
                            // Arena exited; write as NULL.
                            0
                        }
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

impl<'gc> PointerLike for ObjectRef<'gc> {
    #[inline]
    fn pointer(&self) -> Option<NonNull<u8>> {
        ObjectRef::pointer(self)
    }
}

#[cfg(test)]
mod tests {
    #[allow(unused_imports)]
    use super::*;
    #[cfg(feature = "multithreading")]
    use dotnet_utils::ArenaId;
    #[cfg(feature = "multithreading")]
    use dotnet_utils::gc::{register_arena, unregister_arena};
    #[cfg(feature = "multithreading")]
    use dotnet_utils::sync::MANAGED_THREAD_ID;
    #[cfg(feature = "multithreading")]
    use std::sync::Arc;
    #[cfg(feature = "multithreading")]
    use std::sync::atomic::AtomicBool;
    #[cfg(feature = "multithreading")]
    use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};

    #[cfg(feature = "multithreading")]
    static NEXT_TEST_ARENA_ID: AtomicU64 = AtomicU64::new(2000);

    #[cfg(feature = "multithreading")]
    fn next_test_arena_id() -> ArenaId {
        ArenaId::new(NEXT_TEST_ARENA_ID.fetch_add(1, AtomicOrdering::Relaxed))
    }

    #[cfg(feature = "multithreading")]
    struct ManagedThreadIdGuard {
        previous: Option<ArenaId>,
    }

    #[cfg(feature = "multithreading")]
    impl ManagedThreadIdGuard {
        fn set(id: ArenaId) -> Self {
            let previous = MANAGED_THREAD_ID.with(|thread_id| {
                let prev = thread_id.get();
                thread_id.set(Some(id));
                prev
            });
            Self { previous }
        }
    }

    #[cfg(feature = "multithreading")]
    impl Drop for ManagedThreadIdGuard {
        fn drop(&mut self) {
            MANAGED_THREAD_ID.with(|thread_id| thread_id.set(self.previous));
        }
    }

    #[cfg(feature = "multithreading")]
    #[test]
    fn test_read_unchecked_cross_arena_invalid() {
        let arena_id = ArenaId::new(1000);
        unregister_arena(arena_id);

        // Construct a Tag 5 pointer for arena 1000.
        // Pointer value doesn't matter much since it shouldn't be dereferenced.
        let raw_ptr = 0x123456780000usize;
        let tagged_ptr = (raw_ptr & 0x0000FFFFFFFFFFFF) | 5 | ((arena_id.as_u64() as usize) << 48);

        let mut source = [0u8; 8];
        source.copy_from_slice(&tagged_ptr.to_ne_bytes());

        // SAFETY: tagged_ptr is a validly constructed Tag 5 pointer.
        let obj_ref = unsafe { ObjectRef::read_unchecked(&source) };

        assert!(
            obj_ref.0.is_none(),
            "Should return None for unregistered arena"
        );
    }

    #[cfg(feature = "multithreading")]
    #[test]
    fn test_read_unchecked_cross_arena_valid() {
        use gc_arena::{Arena, Rootable};
        struct Root;
        unsafe impl<'gc> gc_arena::Collect<'gc> for Root {
            fn trace<Tr: gc_arena::collect::Trace<'gc>>(&self, _cc: &mut Tr) {}
        }
        let arena = Arena::<Rootable![Root]>::new(|_mc| Root);
        arena.mutate(|mc, _root| {
            let arena_id = ArenaId::new(1001);
            register_arena(arena_id, Arc::new(AtomicBool::new(false)));

            // We need a pointer to something that looks like an ObjectInner to pass magic check.
            let inner = ObjectInner {
                magic: ValidationTag::new(OBJECT_MAGIC),
                owner_id: arena_id,
                storage: HeapStorage::Str(crate::string::CLRString::from("test")),
            };
            let lock = gc_arena::Gc::new(mc, ThreadSafeLock::new(inner));
            let lock_ptr = gc_arena::Gc::as_ptr(lock) as usize;

            let tagged_ptr =
                (lock_ptr & 0x0000FFFFFFFFFFFF) | 5 | ((arena_id.as_u64() as usize) << 48);

            let mut source = [0u8; 8];
            source.copy_from_slice(&tagged_ptr.to_ne_bytes());

            // SAFETY: lock is a valid GC pointer.
            let obj_ref = unsafe { ObjectRef::read_unchecked(&source) };

            assert!(
                obj_ref.0.is_some(),
                "Should return Some for registered arena"
            );

            unregister_arena(arena_id);
        });
    }

    #[cfg(feature = "multithreading")]
    #[test]
    fn test_write_encodes_owner_tag_even_on_owner_thread() {
        use gc_arena::{Arena, Rootable};

        struct Root;
        unsafe impl<'gc> gc_arena::Collect<'gc> for Root {
            fn trace<Tr: gc_arena::collect::Trace<'gc>>(&self, _cc: &mut Tr) {}
        }

        let arena_id = next_test_arena_id();
        let _thread_guard = ManagedThreadIdGuard::set(arena_id);
        register_arena(arena_id, Arc::new(AtomicBool::new(false)));

        let arena_handle_owner = dotnet_utils::gc::ArenaHandle::new(arena_id);
        let arena_handle = unsafe {
            std::mem::transmute::<
                &dotnet_utils::gc::ArenaHandleInner,
                &'static dotnet_utils::gc::ArenaHandleInner,
            >(arena_handle_owner.as_inner())
        };
        let arena = Arena::<Rootable![Root]>::new(|_mc| Root);
        arena.mutate(|mc, _root| {
            #[cfg(not(feature = "memory-validation"))]
            let handle = dotnet_utils::gc::GCHandle::new(mc, arena_handle);
            #[cfg(feature = "memory-validation")]
            let handle = dotnet_utils::gc::GCHandle::new(mc, arena_handle, arena_id);

            let obj = ObjectRef::new(
                handle,
                HeapStorage::Str(crate::string::CLRString::from("x")),
            );

            let mut bytes = [0u8; std::mem::size_of::<usize>()];
            obj.write(&mut bytes);
            let encoded = usize::from_ne_bytes(bytes);

            assert_eq!(encoded & 7, 5, "ObjectRef::write must persist owner tag");
            assert_eq!(
                (encoded >> 48) as u16 as u64,
                arena_id.as_u64() & 0xFFFF,
                "encoded arena id must match owner"
            );

            let roundtrip = unsafe { ObjectRef::read_unchecked(&bytes) };
            assert!(
                roundtrip.0.is_some(),
                "tagged reference should decode while owner arena is live"
            );
        });

        unregister_arena(arena_id);
    }
}

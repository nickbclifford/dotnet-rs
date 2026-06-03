use crate::layout::{FieldLayoutManager, FieldType, HasLayout};

#[cfg(any(feature = "memory-validation", debug_assertions))]
use crate::ValidationTag;
use dotnet_types::TypeDescription;
#[cfg(feature = "multithreading")]
use dotnet_utils::sync::{
    MappedRwLockReadGuard, MappedRwLockWriteGuard, RwLock, RwLockReadGuard, RwLockWriteGuard,
};
use dotnet_utils::{
    atomic::{Atomic, remove_atomic_locations_in_range},
    sync::{Arc, Ordering},
    validate_alignment,
};
use gc_arena::{Collect, collect::Trace};
#[cfg(not(feature = "multithreading"))]
use std::{
    cell::{Cell, UnsafeCell},
    ops::{Deref, DerefMut},
};
use std::{
    collections::HashSet,
    fmt::{self, Debug, Formatter},
    marker::PhantomData,
    sync::LazyLock,
};

#[cfg(any(feature = "memory-validation", debug_assertions))]
const FIELD_STORAGE_MAGIC: u64 = 0x5AFE_F1E1_D500_0000;

static TRACE_GC_PTR_READ: LazyLock<bool> =
    LazyLock::new(|| std::env::var("DOTNET_TRACE_GC_PTR_READ").is_ok());

#[cfg(feature = "multithreading")]
type FieldStorageData = RwLock<Vec<u8>>;
#[cfg(not(feature = "multithreading"))]
type FieldStorageData = UnsafeFieldStorageData;

#[cfg(feature = "multithreading")]
pub type FieldDataReadGuard<'a> = MappedRwLockReadGuard<'a, [u8]>;
#[cfg(feature = "multithreading")]
pub type FieldDataWriteGuard<'a> = MappedRwLockWriteGuard<'a, [u8]>;

#[cfg(not(feature = "multithreading"))]
/// Single-threaded backing storage for `FieldStorage`.
///
/// # Safety proof
///
/// `data` is placed in an `UnsafeCell` so `FieldStorage` can expose both
/// `&[u8]` and `&mut [u8]` through `&self` APIs without a lock.
/// To keep this sound, all accesses must go through `read_borrow` /
/// `write_borrow`, which enforce a runtime aliasing discipline:
///
/// - `borrow_state == -1` means one active mutable borrow
/// - `borrow_state >= 0` means that many active shared borrows
/// - reads are rejected while a writer is active
/// - writes are rejected while any read or write is active
///
/// The returned guards carry a token that updates `borrow_state` on drop,
/// so borrows are correctly released even during unwinding.
struct UnsafeFieldStorageData {
    data: UnsafeCell<Vec<u8>>,
    borrow_state: Cell<isize>,
}

#[cfg(not(feature = "multithreading"))]
impl UnsafeFieldStorageData {
    fn new(data: Vec<u8>) -> Self {
        Self {
            data: UnsafeCell::new(data),
            borrow_state: Cell::new(0),
        }
    }

    fn read_borrow(&self) -> LocalReadBorrow<'_> {
        let state = self.borrow_state.get();
        if state < 0 {
            panic!("FieldStorage read while mutable borrow is active");
        }
        self.borrow_state.set(
            state
                .checked_add(1)
                .expect("FieldStorage read borrow overflow"),
        );
        LocalReadBorrow {
            borrow_state: &self.borrow_state,
        }
    }

    fn write_borrow(&self) -> LocalWriteBorrow<'_> {
        let state = self.borrow_state.get();
        if state != 0 {
            panic!("FieldStorage mutable borrow while another borrow is active");
        }
        self.borrow_state.set(-1);
        LocalWriteBorrow {
            borrow_state: &self.borrow_state,
        }
    }

    unsafe fn data_ptr(&self) -> *mut Vec<u8> {
        self.data.get()
    }
}

#[cfg(not(feature = "multithreading"))]
struct LocalReadBorrow<'a> {
    borrow_state: &'a Cell<isize>,
}

#[cfg(not(feature = "multithreading"))]
impl Drop for LocalReadBorrow<'_> {
    fn drop(&mut self) {
        let state = self.borrow_state.get();
        debug_assert!(state > 0);
        self.borrow_state.set(state - 1);
    }
}

#[cfg(not(feature = "multithreading"))]
struct LocalWriteBorrow<'a> {
    borrow_state: &'a Cell<isize>,
}

#[cfg(not(feature = "multithreading"))]
impl Drop for LocalWriteBorrow<'_> {
    fn drop(&mut self) {
        let state = self.borrow_state.get();
        debug_assert_eq!(state, -1);
        self.borrow_state.set(0);
    }
}

#[cfg(not(feature = "multithreading"))]
pub struct FieldDataReadGuard<'a> {
    _borrow: LocalReadBorrow<'a>,
    data: &'a [u8],
}

#[cfg(not(feature = "multithreading"))]
impl Deref for FieldDataReadGuard<'_> {
    type Target = [u8];

    fn deref(&self) -> &Self::Target {
        self.data
    }
}

#[cfg(not(feature = "multithreading"))]
pub struct FieldDataWriteGuard<'a> {
    _borrow: LocalWriteBorrow<'a>,
    data: &'a mut [u8],
}

#[cfg(not(feature = "multithreading"))]
impl Deref for FieldDataWriteGuard<'_> {
    type Target = [u8];

    fn deref(&self) -> &Self::Target {
        self.data
    }
}

#[cfg(not(feature = "multithreading"))]
impl DerefMut for FieldDataWriteGuard<'_> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.data
    }
}

#[cfg(feature = "multithreading")]
fn new_field_storage_data(data: Vec<u8>) -> FieldStorageData {
    RwLock::new(data)
}

#[cfg(not(feature = "multithreading"))]
fn new_field_storage_data(data: Vec<u8>) -> FieldStorageData {
    UnsafeFieldStorageData::new(data)
}

/// A reference to a specific field, carrying its layout type
pub struct FieldRef<'a, T: FieldType> {
    storage: &'a FieldStorage,
    offset: usize,
    _type: PhantomData<T>,
}

impl<T: FieldType> FieldRef<'_, T> {
    pub fn read(&self) -> T {
        self.storage
            .with_data(|data| T::read_from(&data[self.offset..]))
    }
    pub fn write(&self, value: T) {
        self.storage
            .with_data_mut(|data| value.write_to(&mut data[self.offset..]))
    }
}

pub struct BoundedPtr {
    ptr: *mut u8,
    len: usize,
}

impl BoundedPtr {
    pub fn new(ptr: *mut u8, len: usize) -> Self {
        Self { ptr, len }
    }

    /// # Safety
    /// The caller must ensure that `offset` is within the bounds of `self.ptr` and `self.len`.
    pub unsafe fn read<T: FieldType>(&self, offset: usize) -> T {
        assert!(offset + size_of::<T>() <= self.len);
        unsafe {
            T::read_from(std::slice::from_raw_parts(
                self.ptr.add(offset),
                size_of::<T>(),
            ))
        }
    }
}

pub struct FieldStorage {
    #[cfg(any(feature = "memory-validation", debug_assertions))]
    magic: ValidationTag,
    layout: Arc<FieldLayoutManager>,
    data: FieldStorageData,
}

impl Clone for FieldStorage {
    fn clone(&self) -> Self {
        self.validate_magic();
        let cloned_data = self.with_data(|data| data.to_vec());
        Self {
            #[cfg(any(feature = "memory-validation", debug_assertions))]
            magic: ValidationTag::new(FIELD_STORAGE_MAGIC),
            layout: self.layout.clone(),
            data: new_field_storage_data(cloned_data),
        }
    }
}

impl PartialEq for FieldStorage {
    fn eq(&self, other: &Self) -> bool {
        self.validate_magic();
        other.validate_magic();
        if !Arc::ptr_eq(&self.layout, &other.layout) {
            return false;
        }

        self.with_data(|lhs| other.with_data(|rhs| lhs == rhs))
    }
}

impl Drop for FieldStorage {
    fn drop(&mut self) {
        #[cfg(feature = "multithreading")]
        let data: &mut Vec<u8> = self.data.get_mut();

        #[cfg(not(feature = "multithreading"))]
        // SAFETY: `drop` has exclusive access to `self`, so no outstanding borrows remain.
        let data: &mut Vec<u8> = unsafe { &mut *self.data.data_ptr() };

        remove_atomic_locations_in_range(data.as_ptr(), data.len());
    }
}

impl FieldStorage {
    pub fn new(layout: Arc<FieldLayoutManager>, data: Vec<u8>) -> Self {
        Self {
            #[cfg(any(feature = "memory-validation", debug_assertions))]
            magic: ValidationTag::new(FIELD_STORAGE_MAGIC),
            layout,
            data: new_field_storage_data(data),
        }
    }

    fn validate_magic(&self) {
        #[cfg(any(feature = "memory-validation", debug_assertions))]
        self.magic.validate(FIELD_STORAGE_MAGIC, "FieldStorage");
    }

    /// Get a typed field reference — validates layout at construction time
    pub fn field<T: FieldType>(
        &self,
        owner: TypeDescription,
        name: &str,
    ) -> Option<FieldRef<'_, T>> {
        let field = self.layout.get_field(owner, name)?;
        assert_eq!(T::SCALAR.size_const(), field.layout.size().as_usize());
        Some(FieldRef {
            storage: self,
            offset: field.position.as_usize(),
            _type: PhantomData,
        })
    }

    pub fn with_data<T>(&self, f: impl FnOnce(&[u8]) -> T) -> T {
        self.validate_magic();

        #[cfg(feature = "multithreading")]
        {
            f(&self.data.read())
        }

        #[cfg(not(feature = "multithreading"))]
        {
            let _borrow = self.data.read_borrow();
            // SAFETY: `_borrow` guarantees no mutable borrow is active.
            // The slice reference does not outlive `_borrow` because both are
            // scoped to this function call.
            let data = unsafe { &*self.data.data_ptr() };
            f(data)
        }
    }

    pub fn with_data_mut<T>(&self, f: impl FnOnce(&mut [u8]) -> T) -> T {
        self.validate_magic();

        #[cfg(feature = "multithreading")]
        {
            f(&mut self.data.write())
        }

        #[cfg(not(feature = "multithreading"))]
        {
            let _borrow = self.data.write_borrow();
            // SAFETY: `_borrow` guarantees exclusive access to `data` for the
            // duration of this scope.
            let data = unsafe { &mut *self.data.data_ptr() };
            f(data)
        }
    }

    pub fn layout(&self) -> &Arc<FieldLayoutManager> {
        &self.layout
    }

    pub fn has_field(&self, owner: TypeDescription, name: &str) -> bool {
        self.layout.get_field(owner, name).is_some()
    }

    pub fn get_field_local(&self, owner: TypeDescription, name: &str) -> FieldDataReadGuard<'_> {
        self.validate_magic();
        let field = self.layout.get_field(owner, name).expect("Field not found");
        let alignment = field.layout.alignment();
        let range = field.as_range();

        #[cfg(feature = "multithreading")]
        {
            let guard = RwLockReadGuard::map(self.data.read(), |v| &v[range]);
            validate_alignment(guard.as_ptr(), alignment);
            guard
        }

        #[cfg(not(feature = "multithreading"))]
        {
            let borrow = self.data.read_borrow();
            // SAFETY: `borrow` ensures there is no active mutable borrow.
            let data = unsafe { &*self.data.data_ptr() };
            let slice = &data[range];
            validate_alignment(slice.as_ptr(), alignment);
            FieldDataReadGuard {
                _borrow: borrow,
                data: slice,
            }
        }
    }

    pub fn get_field_mut_local(
        &self,
        owner: TypeDescription,
        name: &str,
    ) -> FieldDataWriteGuard<'_> {
        self.validate_magic();
        let field = self.layout.get_field(owner, name).expect("Field not found");
        let alignment = field.layout.alignment();
        let range = field.as_range();

        #[cfg(feature = "multithreading")]
        {
            let guard = RwLockWriteGuard::map(self.data.write(), |v| &mut v[range]);
            validate_alignment(guard.as_ptr(), alignment);
            guard
        }

        #[cfg(not(feature = "multithreading"))]
        {
            let borrow = self.data.write_borrow();
            // SAFETY: `borrow` guarantees exclusive access for this scope.
            let data = unsafe { &mut *self.data.data_ptr() };
            let slice = &mut data[range];
            validate_alignment(slice.as_ptr(), alignment);
            FieldDataWriteGuard {
                _borrow: borrow,
                data: slice,
            }
        }
    }

    /// Returns a copy of the field's data using atomic operations for supported sizes.
    /// This method respects the provided `Ordering` and is suitable for volatile access.
    /// It acquires a read access guard; for sizes > 8 or misaligned fields the guard
    /// protects a non-atomic memcpy fallback instead of a hardware atomic load.
    ///
    /// # Memory Ordering
    /// For .NET volatile loads, `Ordering::Acquire` or `Ordering::SeqCst` should be used.
    /// Using `Ordering::Relaxed` will trigger a validation warning.
    pub fn get_field_atomic(&self, owner: TypeDescription, name: &str, ord: Ordering) -> Vec<u8> {
        let field = self
            .layout
            .get_field(owner.clone(), name)
            .expect("Field not found");
        let alignment = field.layout.alignment();
        let size = field.layout.size();
        let guard = self.get_field_local(owner, name);
        let field_ptr = guard.as_ptr();
        validate_alignment(field_ptr, alignment);
        unsafe { Atomic::load_field(field_ptr, size.as_usize(), ord) }
    }

    /// Sets the field's data using atomic operations for supported sizes.
    /// This method respects the provided `Ordering` and is suitable for volatile access.
    /// It acquires a write access guard; for sizes > 8 or misaligned fields the guard
    /// protects a non-atomic memcpy fallback instead of a hardware atomic store.
    ///
    /// # Memory Ordering
    /// For .NET volatile stores, `Ordering::Release` or `Ordering::SeqCst` should be used.
    /// Using `Ordering::Relaxed` will trigger a validation warning.
    pub fn set_field_atomic(
        &self,
        owner: TypeDescription,
        name: &str,
        value: &[u8],
        ord: Ordering,
    ) {
        let field = self
            .layout
            .get_field(owner.clone(), name)
            .expect("Field not found");
        let alignment = field.layout.alignment();
        // Let's just use get_mut() to be safe and consistent with synchronized.
        let mut guard = self.get_field_mut_local(owner, name);
        let field_ptr = guard.as_mut_ptr();
        validate_alignment(field_ptr, alignment);
        unsafe { Atomic::store_field(field_ptr, value, ord) }
    }

    pub(crate) unsafe fn raw_data_unsynchronized(&self) -> &[u8] {
        self.validate_magic();
        // SAFETY: Caller guarantees data stability per this method's contract.
        unsafe { &*self.data.data_ptr() }
    }

    /// Returns a pointer to the raw data without acquiring normal field access guards.
    ///
    /// # Safety
    /// The caller must ensure that synchronization is provided elsewhere (e.g. during STW GC)
    /// or that the data is otherwise stable and no writers are active.
    pub unsafe fn raw_data_ptr(&self) -> *mut u8 {
        // SAFETY: Caller upholds synchronization guarantees.
        unsafe { (*self.data.data_ptr()).as_mut_ptr() }
    }

    pub fn resurrect<'gc>(
        &self,
        fc: &'gc gc_arena::Finalization<'gc>,
        visited: &mut HashSet<usize>,
        depth: usize,
    ) {
        // SAFETY: Resurrection happens during a stop-the-world pause, so no other
        // threads are running. We can safely access the inner value without
        // acquiring the normal field access guard. This avoids deadlock (or panic)
        // if a thread was already holding exclusive field access when it reached
        // a safe point.
        let data = unsafe { self.raw_data_unsynchronized() };

        self.layout.resurrect(data, fc, visited, depth);
    }
}

unsafe impl<'gc> Collect<'gc> for FieldStorage {
    fn trace<Tr: Trace<'gc>>(&self, cc: &mut Tr) {
        // SAFETY: Tracing also happens during a stop-the-world pause, same reasoning as above
        let data = unsafe { self.raw_data_unsynchronized() };

        if *TRACE_GC_PTR_READ {
            eprintln!(
                "[GC] FieldStorage trace: data_len={} gc_desc_bitmap={:?} gc_desc_unaligned={:?}",
                data.len(),
                self.layout.gc_desc.bitmap,
                self.layout.gc_desc.unaligned_offsets
            );
        }

        self.layout.trace(data, cc);
    }
}

impl Debug for FieldStorage {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        let len = self.with_data(|data| data.len());
        write!(f, "FieldStorage({} bytes)", len)
    }
}

#[cfg(all(test, not(feature = "multithreading")))]
mod tests {
    use super::UnsafeFieldStorageData;

    #[test]
    fn single_thread_storage_allows_multiple_readers() {
        let data = UnsafeFieldStorageData::new(vec![1, 2, 3]);
        let _r1 = data.read_borrow();
        let _r2 = data.read_borrow();
    }

    #[test]
    #[should_panic(expected = "FieldStorage mutable borrow while another borrow is active")]
    fn single_thread_storage_rejects_writer_while_reader_active() {
        let data = UnsafeFieldStorageData::new(vec![1, 2, 3]);
        let _r1 = data.read_borrow();
        let _w = data.write_borrow();
    }

    #[test]
    #[should_panic(expected = "FieldStorage read while mutable borrow is active")]
    fn single_thread_storage_rejects_reader_while_writer_active() {
        let data = UnsafeFieldStorageData::new(vec![1, 2, 3]);
        let _w = data.write_borrow();
        let _r = data.read_borrow();
    }
}

use crate::layout::{FieldLayoutManager, FieldType, HasLayout};
use dotnet_types::TypeDescription;
use dotnet_utils::{
    atomic::{self, Atomic},
    sync::{
        Arc, MappedRwLockReadGuard, MappedRwLockWriteGuard, Ordering, RwLock, RwLockReadGuard,
        RwLockWriteGuard,
    },
    validate_alignment,
};
use gc_arena::{Collect, Collection};
use std::{
    collections::HashSet,
    fmt::{self, Debug, Formatter},
    marker::PhantomData,
};

#[cfg(any(feature = "memory-validation", debug_assertions))]
const FIELD_STORAGE_MAGIC: u64 = 0x5AFE_F1E1_D500_0000;

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
        assert!(offset + std::mem::size_of::<T>() <= self.len);
        unsafe {
            T::read_from(std::slice::from_raw_parts(
                self.ptr.add(offset),
                std::mem::size_of::<T>(),
            ))
        }
    }
}

pub struct FieldStorage {
    #[cfg(any(feature = "memory-validation", debug_assertions))]
    magic: u64,
    layout: Arc<FieldLayoutManager>,
    data: RwLock<Vec<u8>>,
}

impl Clone for FieldStorage {
    fn clone(&self) -> Self {
        self.validate_magic();
        let data = self.data.read();
        Self {
            #[cfg(any(feature = "memory-validation", debug_assertions))]
            magic: FIELD_STORAGE_MAGIC,
            layout: self.layout.clone(),
            data: RwLock::new(data.clone()),
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
        *self.data.read() == *other.data.read()
    }
}

impl FieldStorage {
    pub fn new(layout: Arc<FieldLayoutManager>, data: Vec<u8>) -> Self {
        Self {
            #[cfg(any(feature = "memory-validation", debug_assertions))]
            magic: FIELD_STORAGE_MAGIC,
            layout,
            data: RwLock::new(data),
        }
    }

    fn validate_magic(&self) {
        #[cfg(any(feature = "memory-validation", debug_assertions))]
        {
            if self.magic != FIELD_STORAGE_MAGIC {
                panic!("FieldStorage magic number corrupted: {:#x}", self.magic);
            }
        }
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
        f(&self.data.read())
    }

    pub fn with_data_mut<T>(&self, f: impl FnOnce(&mut [u8]) -> T) -> T {
        self.validate_magic();
        f(&mut self.data.write())
    }

    pub fn layout(&self) -> &Arc<FieldLayoutManager> {
        &self.layout
    }

    pub fn has_field(&self, owner: TypeDescription, name: &str) -> bool {
        self.layout.get_field(owner, name).is_some()
    }

    pub fn get_field_local(
        &self,
        owner: TypeDescription,
        name: &str,
    ) -> MappedRwLockReadGuard<'_, [u8]> {
        self.validate_magic();
        let field = self.layout.get_field(owner, name).expect("Field not found");
        let alignment = field.layout.alignment();
        let range = field.as_range();

        let guard = RwLockReadGuard::map(self.data.read(), |v| &v[range]);
        validate_alignment(guard.as_ptr(), alignment);
        guard
    }

    pub fn get_field_mut_local(
        &self,
        owner: TypeDescription,
        name: &str,
    ) -> MappedRwLockWriteGuard<'_, [u8]> {
        self.validate_magic();
        let field = self.layout.get_field(owner, name).expect("Field not found");
        let alignment = field.layout.alignment();
        let range = field.as_range();

        let guard = RwLockWriteGuard::map(self.data.write(), |v| &mut v[range]);
        validate_alignment(guard.as_ptr(), alignment);
        guard
    }

    /// Returns a copy of the field's data, synchronized via RwLock to prevent tearing.
    /// NOTE: This method ignores the `Ordering` parameter and should only be used
    /// for non-volatile access where tearing prevention is the primary concern.
    pub fn get_field_synchronized(
        &self,
        owner: TypeDescription,
        name: &str,
        _ord: Ordering,
    ) -> Vec<u8> {
        let guard = self.get_field_local(owner, name);
        atomic::validate_atomic_access(guard.as_ptr(), false);
        guard.to_vec()
    }

    /// Sets the field's data, synchronized via RwLock to prevent tearing.
    pub fn set_field_synchronized(
        &self,
        owner: TypeDescription,
        name: &str,
        value: &[u8],
        _ord: Ordering,
    ) {
        let mut dest = self.get_field_mut_local(owner, name);
        atomic::validate_atomic_access(dest.as_ptr(), false);
        dest.copy_from_slice(value);
    }

    /// Returns a copy of the field's data using atomic operations for supported sizes.
    /// This method respects the provided `Ordering` and is suitable for volatile access.
    /// It still acquires a read lock to ensure memory safety and prevent tearing
    /// against synchronized writers for large field sizes.
    ///
    /// # Memory Ordering
    /// For .NET volatile loads, `Ordering::Acquire` or `Ordering::SeqCst` should be used.
    /// Using `Ordering::Relaxed` will trigger a validation warning.
    pub fn get_field_atomic(&self, owner: TypeDescription, name: &str, ord: Ordering) -> Vec<u8> {
        let field = self.layout.get_field(owner, name).expect("Field not found");
        let alignment = field.layout.alignment();
        let size = field.layout.size();
        let guard = self.get_field_local(owner, name);
        let field_ptr = guard.as_ptr();
        validate_alignment(field_ptr, alignment);
        unsafe { Atomic::load_field(field_ptr, size.as_usize(), ord) }
    }

    /// Sets the field's data using atomic operations for supported sizes.
    /// This method respects the provided `Ordering` and is suitable for volatile access.
    /// It acquires a write lock to ensure memory safety and prevent tearing
    /// against other synchronized readers/writers for large field sizes.
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
        let field = self.layout.get_field(owner, name).expect("Field not found");
        let alignment = field.layout.alignment();
        // Let's just use get_mut() to be safe and consistent with synchronized.
        let mut guard = self.get_field_mut_local(owner, name);
        let field_ptr = guard.as_mut_ptr();
        validate_alignment(field_ptr, alignment);
        unsafe { Atomic::store_field(field_ptr, value, ord) }
    }

    pub(crate) unsafe fn raw_data_unsynchronized(&self) -> &[u8] {
        self.validate_magic();
        unsafe { &*self.data.data_ptr() }
    }

    /// Returns a pointer to the raw data without acquiring a lock.
    ///
    /// # Safety
    /// The caller must ensure that the lock is held elsewhere (e.g. during STW GC)
    /// or that the data is otherwise stable and no writers are active.
    pub unsafe fn raw_data_ptr(&self) -> *mut u8 {
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
        // acquiring the lock. This avoids deadlock (or panic) if a thread was
        // already holding the write lock when it reached a safe point.
        let data = unsafe { self.raw_data_unsynchronized() };

        self.layout.resurrect(data, fc, visited, depth);
    }
}

unsafe impl Collect for FieldStorage {
    fn trace(&self, cc: &Collection) {
        // SAFETY: Tracing also happens during a stop-the-world pause, same reasoning as above
        let data = unsafe { self.raw_data_unsynchronized() };

        self.layout.trace(data, cc);
    }
}

impl Debug for FieldStorage {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        let len = self.data.read().len();
        write!(f, "FieldStorage({} bytes)", len)
    }
}

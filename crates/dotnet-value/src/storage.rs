use crate::layout::{FieldLayoutManager, FieldType, HasLayout};

#[cfg(any(feature = "memory-validation", debug_assertions))]
use crate::ValidationTag;
use dotnet_types::TypeDescription;
use dotnet_utils::sync::{
    MappedRwLockReadGuard, MappedRwLockWriteGuard, RwLock, RwLockReadGuard, RwLockWriteGuard,
};
use dotnet_utils::{
    atomic::{Atomic, remove_atomic_locations_in_range},
    is_ptr_aligned_to_field,
    sync::{Arc, Ordering},
};
use gc_arena::{Collect, collect::Trace};
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

type FieldStorageData = RwLock<Vec<u8>>;

pub type FieldDataReadGuard<'a> = MappedRwLockReadGuard<'a, [u8]>;
pub type FieldDataWriteGuard<'a> = MappedRwLockWriteGuard<'a, [u8]>;

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
    #[expect(
        clippy::multiple_unsafe_ops_per_block,
        reason = "the checked offset is converted to a slice and decoded as one field value"
    )]
    pub unsafe fn read<T: FieldType>(&self, offset: usize) -> T {
        assert!(offset + size_of::<T>() <= self.len);
        // SAFETY: F10.RawMemoryAccessValid — The FieldStorage layout and access guard guarantee valid storage for this raw operation.
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
            data: RwLock::new(cloned_data),
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
        let data: &mut Vec<u8> = self.data.get_mut();

        remove_atomic_locations_in_range(data.as_ptr(), data.len());
    }
}

impl FieldStorage {
    pub fn new(layout: Arc<FieldLayoutManager>, data: Vec<u8>) -> Self {
        Self {
            #[cfg(any(feature = "memory-validation", debug_assertions))]
            magic: ValidationTag::new(FIELD_STORAGE_MAGIC),
            layout,
            data: RwLock::new(data),
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

    /// Gets a typed field reference at a layout-resolved byte offset.
    ///
    /// Callers must obtain `offset` from this storage's [`FieldLayoutManager`]
    /// and verify the expected field size before calling this method. This is
    /// used by validated semantic field registries that retain field identity
    /// separately from lazily constructed object layouts.
    pub fn field_at_offset<T: FieldType>(&self, offset: usize) -> Option<FieldRef<'_, T>> {
        let end = offset.checked_add(T::SCALAR.size_const())?;
        if end > self.layout.total_size {
            return None;
        }
        Some(FieldRef {
            storage: self,
            offset,
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

    pub fn get_field_local(&self, owner: TypeDescription, name: &str) -> FieldDataReadGuard<'_> {
        self.validate_magic();
        let field = self.layout.get_field(owner, name).expect("Field not found");
        let alignment = field.layout.alignment();
        let range = field.as_range();

        let guard = RwLockReadGuard::map(self.data.read(), |v| &v[range]);
        debug_assert!(
            is_ptr_aligned_to_field(guard.as_ptr(), alignment),
            "Alignment violation: FieldStorage::get_field_local: ptr {:p} is not aligned to {} bytes",
            guard.as_ptr(),
            alignment
        );
        guard
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

        let guard = RwLockWriteGuard::map(self.data.write(), |v| &mut v[range]);
        debug_assert!(
            is_ptr_aligned_to_field(guard.as_ptr(), alignment),
            "Alignment violation: FieldStorage::get_field_mut_local: ptr {:p} is not aligned to {} bytes",
            guard.as_ptr(),
            alignment
        );
        guard
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
        // Resolve the layout before taking the data lock to avoid borrow re-entry.
        let field = self.layout.get_field(owner, name).expect("Field not found");
        let alignment = field.layout.alignment();
        let size = field.layout.size();

        self.validate_magic();
        let range = field.as_range();
        let guard = RwLockReadGuard::map(self.data.read(), |v| &v[range]);
        let field_ptr = guard.as_ptr();
        debug_assert!(
            is_ptr_aligned_to_field(field_ptr, alignment),
            "Alignment violation: FieldStorage::get_field_atomic: ptr {:p} is not aligned to {} bytes",
            field_ptr,
            alignment
        );
        // SAFETY: F10.RawMemoryAccessValid — The FieldStorage layout and access guard guarantee valid storage for this raw
        // operation. Atomic::is_atomic_field_access_supported provides defense-in-depth for
        // misaligned fields by selecting a lock-guarded memcpy fallback.
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
        // Resolve the layout before taking the data lock to avoid borrow re-entry.
        let field = self.layout.get_field(owner, name).expect("Field not found");
        let alignment = field.layout.alignment();

        self.validate_magic();
        let range = field.as_range();
        let mut guard = RwLockWriteGuard::map(self.data.write(), |v| &mut v[range]);
        let field_ptr = guard.as_mut_ptr();
        debug_assert!(
            is_ptr_aligned_to_field(field_ptr, alignment),
            "Alignment violation: FieldStorage::set_field_atomic: ptr {:p} is not aligned to {} bytes",
            field_ptr,
            alignment
        );
        // SAFETY: F10.RawMemoryAccessValid — The FieldStorage layout and access guard guarantee valid storage for this raw
        // operation. Atomic::is_atomic_field_access_supported provides defense-in-depth for
        // misaligned fields by selecting a lock-guarded memcpy fallback.
        unsafe { Atomic::store_field(field_ptr, value, ord) }
    }

    #[allow(
        unused_unsafe,
        reason = "compat::RwLock::data_ptr is unsafe while parking_lot::RwLock::data_ptr is safe"
    )]
    pub(crate) unsafe fn raw_data_unsynchronized(&self) -> &[u8] {
        self.validate_magic();
        // SAFETY: F10.BorrowedStorageStable — Caller guarantees data stability per this method's contract and the pointer
        // does not outlive the lock.
        let data_ptr = unsafe { self.data.data_ptr() };
        // SAFETY: F10.RawMemoryAccessValid — `data_ptr` points to the storage protected by `self.data`.
        unsafe { &*data_ptr }
    }

    /// Returns a pointer to the raw data without acquiring normal field access guards.
    ///
    /// # Safety
    /// The caller must ensure that synchronization is provided elsewhere (e.g. during STW GC)
    /// or that the data is otherwise stable and no writers are active.
    #[allow(
        unused_unsafe,
        reason = "compat::RwLock::data_ptr is unsafe while parking_lot::RwLock::data_ptr is safe"
    )]
    pub unsafe fn raw_data_ptr(&self) -> *mut u8 {
        // SAFETY: F10.BorrowedStorageStable — Caller upholds synchronization guarantees and the pointer does not outlive
        // the lock.
        let data_ptr = unsafe { self.data.data_ptr() };
        // SAFETY: F10.RawMemoryAccessValid — `data_ptr` points to the storage protected by `self.data`.
        unsafe { (*data_ptr).as_mut_ptr() }
    }

    pub fn resurrect<'gc>(
        &self,
        fc: &'gc gc_arena::Finalization<'gc>,
        visited: &mut HashSet<usize>,
        depth: usize,
    ) {
        // SAFETY: F10.BorrowedStorageStable — Resurrection happens during a stop-the-world pause, so no other
        // threads are running. We can safely access the inner value without
        // acquiring the normal field access guard. This avoids deadlock (or panic)
        // if a thread was already holding exclusive field access when it reached
        // a safe point.
        let data = unsafe { self.raw_data_unsynchronized() };

        self.layout.resurrect(data, fc, visited, depth);
    }
}

// SAFETY: F5.TracesEveryGcRef — `layout.trace` visits every ObjectRef and ManagedPtr slot described by this storage's
// immutable layout. Tracing runs during STW, so `raw_data_unsynchronized` observes stable bytes.
unsafe impl<'gc> Collect<'gc> for FieldStorage {
    fn trace<Tr: Trace<'gc>>(&self, cc: &mut Tr) {
        // SAFETY: F5.TracesEveryGcRef — Tracing also happens during a stop-the-world pause, same reasoning as above
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

#[cfg(all(test, debug_assertions))]
mod alignment_tests {
    use super::*;
    use crate::layout::{FieldKey, FieldLayout, GcDesc, LayoutManager, Scalar};
    use dotnet_types::TypeDescription;

    const FIELD_NAME: &str = "misaligned";

    fn misaligned_int32_storage() -> FieldStorage {
        let data = vec![0; 8];
        let offset = (0..4)
            .find(|offset| {
                !is_ptr_aligned_to_field(data.as_ptr().wrapping_add(*offset), align_of::<i32>())
            })
            .expect("one of four consecutive byte addresses must be misaligned for i32");
        let layout = Arc::new(FieldLayoutManager {
            fields: [(
                FieldKey {
                    owner: TypeDescription::NULL,
                    name: FIELD_NAME.to_string(),
                },
                FieldLayout {
                    position: crate::ByteOffset::new(offset),
                    layout: Arc::new(LayoutManager::Scalar(Scalar::Int32)),
                },
            )]
            .into_iter()
            .collect(),
            total_size: data.len(),
            alignment: align_of::<i32>(),
            gc_desc: GcDesc::default(),
            has_ref_fields: false,
        });

        FieldStorage::new(layout, data)
    }

    #[test]
    #[should_panic(expected = "Alignment violation: FieldStorage::get_field_local")]
    fn get_field_local_rejects_misaligned_storage() {
        let storage = misaligned_int32_storage();
        let _guard = storage.get_field_local(TypeDescription::NULL, FIELD_NAME);
    }

    #[test]
    #[should_panic(expected = "Alignment violation: FieldStorage::get_field_mut_local")]
    fn get_field_mut_local_rejects_misaligned_storage() {
        let storage = misaligned_int32_storage();
        let _guard = storage.get_field_mut_local(TypeDescription::NULL, FIELD_NAME);
    }

    #[test]
    #[should_panic(expected = "Alignment violation: FieldStorage::get_field_atomic")]
    fn get_field_atomic_rejects_misaligned_storage() {
        let storage = misaligned_int32_storage();
        let _ = storage.get_field_atomic(TypeDescription::NULL, FIELD_NAME, Ordering::Acquire);
    }

    #[test]
    #[should_panic(expected = "Alignment violation: FieldStorage::set_field_atomic")]
    fn set_field_atomic_rejects_misaligned_storage() {
        let storage = misaligned_int32_storage();
        storage.set_field_atomic(
            TypeDescription::NULL,
            FIELD_NAME,
            &[0; size_of::<i32>()],
            Ordering::Release,
        );
    }
}

use crate::layout::{FieldLayoutManager, FieldType, HasLayout, Scalar};

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
        self.storage.with_data(|data| {
            // SAFETY: F2.DescriptorMatchesEcmaLayout — construction checks the reference slot kind and the storage guard protects its bytes.
            unsafe { T::read_from(&data[self.offset..]) }
        })
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
    /// The pointer must be live and readable for the selected range, with no concurrent
    /// writes. Bytes must encode a valid `T`, whose owners outlive the result.
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
        if matches!(T::SCALAR, Scalar::ObjectRef | Scalar::ManagedPtr)
            && T::SCALAR.size_const() != field.layout.size().as_usize()
        {
            return None;
        }
        assert_eq!(T::SCALAR.size_const(), field.layout.size().as_usize());
        self.field_at_offset(field.position.as_usize())
    }

    /// Gets a typed field reference at a layout-resolved byte offset.
    ///
    /// Reference types require an exact scalar-kind match at the slot start,
    /// including nested fields and array elements. Plain scalars require bounds.
    pub fn field_at_offset<T: FieldType>(&self, offset: usize) -> Option<FieldRef<'_, T>> {
        let end = offset.checked_add(T::SCALAR.size_const())?;
        if end > self.layout.total_size {
            return None;
        }
        if matches!(T::SCALAR, Scalar::ObjectRef | Scalar::ManagedPtr)
            && !self.layout.fields.values().any(|field| {
                offset
                    .checked_sub(field.position.as_usize())
                    .is_some_and(|relative| {
                        relative < field.layout.size().as_usize()
                            && field.layout.scalar_at(relative, T::SCALAR)
                    })
            })
        {
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

    /// Returns the length of the backing byte vector while excluding concurrent field writers.
    ///
    /// This reads Vec metadata only; it does not construct a slice over the payload bytes.
    pub(crate) fn data_len(&self) -> usize {
        self.validate_magic();
        let _guard = self.data.read();
        #[allow(
            unused_unsafe,
            reason = "compat::RwLock::data_ptr is unsafe while parking_lot::RwLock::data_ptr is safe"
        )]
        // SAFETY: F10.BorrowedStorageStable — The read guard prevents writers from changing Vec
        // metadata while its length is read.
        let data_ptr = unsafe { self.data.data_ptr() };
        // SAFETY: F10.RawMemoryAccessValid — `data_ptr` points to the Vec protected by `self.data`;
        // reading its length does not construct a reference to the payload bytes.
        unsafe { (*data_ptr).len() }
    }

    /// Snapshots the allocation base while excluding concurrent field writers.
    /// The returned pointer retains neither the guard nor permission to access bytes.
    pub(crate) fn observe_data_base(&self) -> *const u8 {
        self.validate_magic();
        let _guard = self.data.read();
        #[allow(
            unused_unsafe,
            reason = "compat::RwLock::data_ptr is unsafe while parking_lot::RwLock::data_ptr is safe"
        )]
        // SAFETY: F10.BorrowedStorageStable — The read guard excludes changes to the Vec.
        let data_ptr = unsafe { self.data.data_ptr() };
        // SAFETY: F10.RawMemoryAccessValid — The guard protects this Vec descriptor; as_ptr
        // obtains its allocation base without constructing a payload slice.
        unsafe { (*data_ptr).as_ptr() }
    }

    /// Invokes `f` with exactly the requested in-bounds byte range, without
    /// first constructing a slice over the whole payload.
    pub(crate) fn try_with_data_range<T>(
        &self,
        offset: usize,
        size: usize,
        f: impl FnOnce(&[u8]) -> T,
    ) -> Option<T> {
        self.with_data_window(offset, size, false, f)
    }

    /// Invokes `f` with up to `max_size` bytes from `offset`, passing a shorter
    /// slice when the payload ends first, as P/Invoke buffers require.
    pub(crate) fn with_data_up_to<T>(
        &self,
        offset: usize,
        max_size: usize,
        f: impl FnOnce(&[u8]) -> T,
    ) -> Option<T> {
        self.with_data_window(offset, max_size, true, f)
    }

    fn with_data_window<T>(
        &self,
        offset: usize,
        requested_size: usize,
        allow_short: bool,
        f: impl FnOnce(&[u8]) -> T,
    ) -> Option<T> {
        self.validate_magic();
        let _guard = self.data.read();
        #[allow(
            unused_unsafe,
            reason = "compat::RwLock::data_ptr is unsafe while parking_lot::RwLock::data_ptr is safe"
        )]
        // SAFETY: F10.BorrowedStorageStable — The read guard prevents field writers from changing
        // the backing Vec or its allocation during range validation and the callback.
        let data_ptr = unsafe { self.data.data_ptr() };
        // SAFETY: F10.RawMemoryAccessValid — `data_ptr` addresses the Vec protected by `_guard`;
        // reading its length does not construct a payload reference.
        let len = unsafe { (*data_ptr).len() };
        // SAFETY: F10.RawMemoryAccessValid — `data_ptr` addresses the Vec protected by `_guard`;
        // `as_ptr` returns the allocation base without constructing a payload slice.
        let base = unsafe { (*data_ptr).as_ptr() };
        let available = len.checked_sub(offset)?;
        let size = if allow_short {
            requested_size.min(available)
        } else {
            requested_size
        };
        let end = offset.checked_add(size)?;
        if end > len {
            return None;
        }

        // SAFETY: F10.RawMemoryAccessValid — `offset <= len` was proven by `checked_sub` and
        // `_guard` keeps the Vec allocation stable.
        let range_ptr = unsafe { base.add(offset) };
        // SAFETY: F10.RawMemoryAccessValid — `end <= len` was checked while `_guard` keeps the
        // Vec allocation stable; this constructs only the requested window.
        let bytes = unsafe { std::slice::from_raw_parts(range_ptr, size) };
        Some(f(bytes))
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
    /// Returns the field bytes without taking the field access guard.
    ///
    /// # Safety
    ///
    /// No thread may write or reallocate this storage's data while the returned slice is
    /// live, for example because the world is stopped.
    pub(crate) unsafe fn raw_data_unsynchronized(&self) -> &[u8] {
        self.validate_magic();
        // SAFETY: F10.BorrowedStorageStable — this function's `# Safety` contract excludes writers and reallocation while the returned slice is live, which the unguarded `data_ptr` read requires.
        let data_ptr = unsafe { self.data.data_ptr() };
        // SAFETY: F10.RawMemoryAccessValid — `data_ptr` points to the storage protected by `self.data`.
        unsafe { &*data_ptr }
    }

    /// Returns the raw data allocation base without acquiring a field access guard.
    /// The address retains neither the owner/root nor a borrow/lock and does not
    /// itself authorize payload access.
    ///
    /// # Safety
    /// The caller must retain the owner and ensure synchronization is provided
    /// elsewhere (e.g. during STW GC), with no writer or reallocation while the
    /// pointer is used. Any access must independently satisfy its range and aliasing contract.
    #[allow(
        unused_unsafe,
        reason = "compat::RwLock::data_ptr is unsafe while parking_lot::RwLock::data_ptr is safe"
    )]
    pub unsafe fn raw_data_ptr(&self) -> *mut u8 {
        // SAFETY: F10.BorrowedStorageStable — this function's `# Safety` contract excludes writers and reallocation while the pointer is used; here it is dereferenced only for the base lookup below.
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
        // SAFETY: F1.StwParked — resurrection runs while every mutator is parked, so nothing writes or reallocates these bytes while `data` is live. No parked thread can be inside a field write guard: a mutator polls a safepoint only between instructions or copy chunks, never inside `with_data(_mut)`, and a `ThreadSafeLock` writer refuses to park (`has_active_thread_safe_write_guard`). Skipping the field guard therefore avoids no real contention; it only avoids a lock that cannot be held.
        let data = unsafe { self.raw_data_unsynchronized() };

        self.layout.resurrect(data, fc, visited, depth);
    }
}

// SAFETY: F5.TracesEveryGcRef — `layout.trace` visits every ObjectRef and ManagedPtr slot described by this storage's
// immutable layout. Tracing runs during STW, so `raw_data_unsynchronized` observes stable bytes.
unsafe impl<'gc> Collect<'gc> for FieldStorage {
    fn trace<Tr: Trace<'gc>>(&self, cc: &mut Tr) {
        // SAFETY: F1.StwParked — `trace` runs only while every mutator is parked, so nothing writes or reallocates these bytes while `data` is live.
        let data = unsafe { self.raw_data_unsynchronized() };

        if *TRACE_GC_PTR_READ {
            eprintln!(
                "[GC] FieldStorage trace: data_len={} gc_desc_bitmap={:?} gc_desc_unaligned={:?}",
                data.len(),
                self.layout.gc_desc.bitmap,
                self.layout.gc_desc.unaligned_offsets
            );
        }

        // SAFETY: F2.DescriptorMatchesEcmaLayout — this storage owns the layout and its initialized bytes; tracing runs during STW.
        unsafe { self.layout.trace(data, cc) };
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

#[cfg(test)]
mod safe_api_tests {
    use super::*;
    use crate::{
        ObjectRef,
        layout::{FieldKey, FieldLayout, GcDesc, LayoutManager},
        pointer::ManagedPtr,
    };
    use hashbrown::HashMap;

    fn storage(kind: Scalar) -> FieldStorage {
        let size = kind.size_const();
        let mut fields = HashMap::new();
        fields.insert(
            FieldKey {
                owner: TypeDescription::NULL,
                name: "slot".into(),
            },
            FieldLayout {
                position: crate::ByteOffset::new(0),
                layout: Arc::new(LayoutManager::Scalar(kind)),
            },
        );
        FieldStorage::new(
            Arc::new(FieldLayoutManager {
                fields,
                total_size: size,
                alignment: 8,
                gc_desc: GcDesc::default(),
                has_ref_fields: false,
            }),
            vec![0; size],
        )
    }

    #[cfg(feature = "multithreading")]
    #[test]
    fn object_size_while_storage_read_guard_is_held() {
        let object = crate::object::Object::new(
            TypeDescription::NULL,
            dotnet_types::generics::GenericLookup::default(),
            storage(Scalar::Int64),
        );
        std::thread::scope(|scope| {
            let (ready_tx, ready_rx) = std::sync::mpsc::channel();
            let (done_tx, done_rx) = std::sync::mpsc::channel();
            let object_ref = &object;
            scope.spawn(move || {
                object_ref.instance_storage.with_data(|_| {
                    ready_tx.send(()).unwrap();
                    done_rx.recv().unwrap();
                })
            });
            ready_rx.recv().unwrap();
            assert_eq!(object.size_bytes(), size_of::<crate::object::Object>() + 8);
            done_tx.send(()).unwrap();
        });
    }

    #[test]
    fn nested_and_array_reference_slots_require_exact_start() {
        let inner = storage(Scalar::ObjectRef);
        let mut outer = storage(Scalar::ObjectRef);
        let width = Scalar::ObjectRef.size_const();
        Arc::make_mut(&mut outer.layout)
            .fields
            .values_mut()
            .next()
            .unwrap()
            .layout = Arc::new(LayoutManager::Array(crate::layout::ArrayLayoutManager {
            element_layout: Arc::new(LayoutManager::Field((*inner.layout).clone())),
            length: 2,
        }));
        Arc::make_mut(&mut outer.layout).total_size = width * 2;
        *outer.data.write() = vec![0; width * 2];
        assert!(outer.field_at_offset::<ObjectRef>(0).is_some());
        assert!(outer.field_at_offset::<ObjectRef>(width).is_some());
        assert!(outer.field_at_offset::<ObjectRef>(1).is_none());
        assert!(outer.field_at_offset::<ObjectRef>(width * 2).is_none());
    }

    #[test]
    fn typed_reference_access_requires_matching_slot() {
        let integer = storage(Scalar::Int64);
        assert!(
            integer
                .field::<ObjectRef>(TypeDescription::NULL, "slot")
                .is_none()
        );
        assert!(integer.field_at_offset::<ObjectRef>(0).is_none());
        assert!(integer.field_at_offset::<ObjectRef>(usize::MAX).is_none());
        let object = storage(Scalar::ObjectRef);
        assert!(object.field_at_offset::<ObjectRef>(0).is_some());
        assert!(object.field_at_offset::<ObjectRef>(1).is_none());
        assert!(object.field_at_offset::<ManagedPtr>(0).is_none());
        let field = object.field_at_offset::<ObjectRef>(0).unwrap();
        field.write(ObjectRef(None));
        assert_eq!(field.read(), ObjectRef(None));
        let managed = storage(Scalar::ManagedPtr);
        assert!(managed.field_at_offset::<ObjectRef>(0).is_none());
        assert!(
            managed
                .field::<ObjectRef>(TypeDescription::NULL, "slot")
                .is_none()
        );
        assert!(managed.field_at_offset::<ManagedPtr>(0).is_some());
    }
}

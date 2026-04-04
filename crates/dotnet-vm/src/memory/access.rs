use crate::{
    error::{CompareExchangeError, MemoryAccessError},
    memory::{heap::HeapManager, validation::*},
};
use dotnet_types::{TypeDescription, generics::GenericLookup, resolution::ResolutionS};
use dotnet_utils::{ArenaId, ByteOffset, atomic::validate_atomic_access, gc::GCHandle};
use dotnet_value::{
    StackValue,
    layout::{HasLayout, LayoutManager, Scalar},
    object::{HeapStorage, Object as ObjectInstance, ObjectRef},
    pointer::ManagedPtr,
    storage::FieldStorage,
};
use std::{cell::RefCell, marker::PhantomData, ptr, sync::Arc};

#[cfg(feature = "multithreading")]
use dotnet_utils::gc::GcLifetime;
#[cfg(feature = "multithreading")]
use dotnet_value::{object::ObjectPtr, pointer::PointerOrigin};

#[derive(Copy, Clone)]
pub enum MemoryOwner<'gc> {
    Local(ObjectRef<'gc>),
    #[cfg(feature = "multithreading")]
    CrossArena(ObjectPtr, ArenaId, GcLifetime<'gc>),
}

#[derive(Copy, Clone)]
pub struct HeapWriteTarget<'gc>(pub MemoryOwner<'gc>);

thread_local! {
    static WB_LOCAL_BUF: RefCell<Vec<(ArenaId, usize)>> = RefCell::new(Vec::with_capacity(128));
}

/// RAII guard that drains the thread-local write-barrier buffer on drop.
///
/// Place one instance (as `let _flush_guard = WriteBarrierFlushGuard;`) immediately
/// before each `WB_LOCAL_BUF.with(...)` write-barrier block.  Because `Drop` runs
/// even during stack unwinding the buffer is always flushed to the GC coordinator,
/// preventing stale cross-arena references from accumulating when a closure panics
/// mid-write.
///
/// # Drop ordering
/// The guard must be declared *before* the `WB_LOCAL_BUF.with(...)` call so that
/// it drops *after* that call returns (Rust drops locals in reverse declaration
/// order).  By the time `Drop` runs, the `RefMut` borrowed inside the `with`
/// closure has already been released, so the re-borrow in `Drop` cannot panic.
pub(crate) struct WriteBarrierFlushGuard;

impl Drop for WriteBarrierFlushGuard {
    fn drop(&mut self) {
        WB_LOCAL_BUF.with(|buf| {
            // SAFETY: The `RefMut` from the write operation that created this
            // guard lives inside the `WB_LOCAL_BUF.with(...)` closure body,
            // which has already returned — either normally or via unwind —
            // before this `Drop` executes.  Therefore `borrow_mut` here cannot
            // encounter an already-active mutable borrow and will not panic.
            #[cfg(feature = "multithreading")]
            for (tid, ptr) in buf.borrow_mut().drain(..) {
                dotnet_utils::gc::record_cross_arena_ref(tid, ptr);
            }
            // Under non-multithreading the recorder push methods are no-ops so
            // the buffer is always empty in production.  We clear explicitly
            // anyway to keep the invariant sound for tests that seed it directly.
            #[cfg(not(feature = "multithreading"))]
            buf.borrow_mut().clear();
        });
    }
}

pub struct WriteBarrierRecorder<'a, 'gc> {
    #[cfg_attr(not(feature = "multithreading"), allow(dead_code))]
    arena_id: ArenaId,
    #[cfg_attr(not(feature = "multithreading"), allow(dead_code))]
    buffer: &'a mut Vec<(ArenaId, usize)>,
    _gc: PhantomData<&'gc ()>,
}

impl<'a, 'gc> WriteBarrierRecorder<'a, 'gc> {
    pub fn new(arena_id: ArenaId, buffer: &'a mut Vec<(ArenaId, usize)>) -> Self {
        Self {
            arena_id,
            buffer,
            _gc: PhantomData,
        }
    }

    #[cfg(feature = "multithreading")]
    pub fn record_ref(&mut self, target: ObjectRef<'gc>) {
        if let Some(h) = target.0 {
            // SAFETY: `h` is a live `Gc` handle; reading immutable `owner_id`
            // does not move or mutate the object.
            let ref_tid = unsafe { (*h.as_ptr()).owner_id };
            if ref_tid != self.arena_id {
                self.buffer
                    .push((ref_tid, gc_arena::Gc::as_ptr(h) as usize));
            }
        }
    }

    #[cfg(feature = "multithreading")]
    pub fn record_managed_ptr(&mut self, target: &ManagedPtr<'gc>) {
        match target.origin() {
            PointerOrigin::CrossArenaObjectRef(p, ref_tid) => {
                if *ref_tid != self.arena_id {
                    self.buffer.push((*ref_tid, p.as_ptr() as usize));
                }
            }
            PointerOrigin::Heap(r) => {
                self.record_ref(*r);
            }
            _ => {}
        }
    }

    #[cfg(not(feature = "multithreading"))]
    pub fn record_ref(&mut self, _target: ObjectRef<'gc>) {}

    #[cfg(not(feature = "multithreading"))]
    pub fn record_managed_ptr(&mut self, _target: &ManagedPtr<'gc>) {}
}

impl<'gc> MemoryOwner<'gc> {
    #[cfg(feature = "multithreading")]
    pub fn cross_arena(gc: GCHandle<'gc>, ptr: ObjectPtr, tid: ArenaId) -> Self {
        Self::CrossArena(ptr, tid, gc.lifetime())
    }

    pub fn owner_id(&self) -> ArenaId {
        match self {
            Self::Local(r) => {
                // SAFETY: `h` is a live `Gc` handle; reading immutable `owner_id`
                // does not move or mutate the object.
                r.0.map(|h| unsafe { (*h.as_ptr()).owner_id })
                    .unwrap_or(ArenaId(0))
            }
            #[cfg(feature = "multithreading")]
            Self::CrossArena(_, tid, _) => *tid,
        }
    }

    pub fn with_data<T>(&self, f: impl FnOnce(&[u8]) -> T) -> T {
        match self {
            Self::Local(r) => r.with_data(f),
            #[cfg(feature = "multithreading")]
            Self::CrossArena(p, _, _) => p.with_data(f),
        }
    }

    pub fn with_data_mut<T>(&self, gc: GCHandle<'gc>, f: impl FnOnce(&mut [u8]) -> T) -> T {
        match self {
            Self::Local(r) => r.with_data_mut(gc, f),
            #[cfg(feature = "multithreading")]
            Self::CrossArena(p, _, _) => p.with_data_mut(gc, f),
        }
    }

    pub fn as_heap_storage<T>(&self, f: impl for<'a> FnOnce(&HeapStorage<'a>) -> T) -> T {
        match self {
            Self::Local(r) => r.as_heap_storage(f),
            #[cfg(feature = "multithreading")]
            Self::CrossArena(p, _, _) => p.as_heap_storage(f),
        }
    }
}

/// Checks whether a pointer `ptr` into a buffer `[base, base+len)` can safely
/// access `size` bytes.  Returns `Err(BoundsCheck)` when the access would
/// exceed the buffer, wrapping around, or go before `base`.
///
/// `base` being null disables the check (unmanaged pointer path).
fn check_bounds(
    ptr: *const u8,
    base: *const u8,
    len: usize,
    size: usize,
) -> Result<(), MemoryAccessError> {
    if !base.is_null() {
        let base_addr = base.addr();
        let ptr_addr = ptr.addr();

        if ptr_addr < base_addr
            || (ptr_addr - base_addr)
                .checked_add(size)
                .is_none_or(|end| end > len)
        {
            return Err(MemoryAccessError::BoundsCheck {
                offset: ptr_addr.wrapping_sub(base_addr),
                size,
                len,
            });
        }
    }
    Ok(())
}

/// Manages unsafe memory access, enforcing bounds checks, GC write barriers, and type integrity.
pub struct RawMemoryAccess<'a, 'gc> {
    _heap: &'a HeapManager<'gc>,
}

impl<'a, 'gc> RawMemoryAccess<'a, 'gc> {
    pub fn new(heap: &'a HeapManager<'gc>) -> Self {
        Self { _heap: heap }
    }

    /// Writes a value to a memory location (owner + offset), performing necessary checks.
    ///
    /// # Safety
    /// The caller must ensure that `offset` represents a valid memory location if `owner` is None.
    pub unsafe fn write_unaligned(
        &mut self,
        gc: GCHandle<'gc>,
        owner: Option<MemoryOwner<'gc>>,
        offset: ByteOffset,
        value: StackValue<'gc>,
        layout: &LayoutManager,
    ) -> Result<(), MemoryAccessError> {
        if let Some(owner) = owner {
            owner.as_heap_storage(|_storage| {}); // Ensure object is valid and magic matches

            // Get layout before locking to avoid deadlock
            let dest_layout = self.get_layout_from_owner(owner);

            // SAFETY: with_data_mut ensures the lock is held for the duration of the closure.
            // write_value_internal will copy the data.  The flush guard drains
            // WB_LOCAL_BUF on drop, ensuring cross-arena refs are recorded even
            // if the inner closure panics.
            let _flush_guard = WriteBarrierFlushGuard;
            WB_LOCAL_BUF.with(|buf| {
                let mut b = buf.borrow_mut();
                let mut recorder = WriteBarrierRecorder::new(owner.owner_id(), &mut b);
                owner.with_data_mut(gc, |data| {
                    let base = data.as_mut_ptr();
                    let len = data.len();
                    let ptr = base.wrapping_add(offset.0);
                    validate_atomic_access(ptr as *const u8, false);

                    // 1. Bounds Check
                    self.check_bounds_internal(ptr, base, len, layout.size().as_usize())?;

                    // 2. Integrity Check
                    self.check_integrity_internal_with_layout(ptr, dest_layout, base, layout)?;

                    // 3. Perform Write
                    unsafe {
                        self.write_value_internal_with_recorder(
                            gc,
                            ptr,
                            Some(owner),
                            value,
                            layout,
                            &mut recorder,
                        )
                    }
                })
            })
        } else {
            let ptr = sptr::from_exposed_addr_mut::<u8>(offset.0);
            if ptr.is_null() {
                return Err(MemoryAccessError::NullPointer(
                    "NullReferenceException: writing to unmanaged null pointer".into(),
                ));
            }
            validate_atomic_access(ptr as *const u8, false);
            // SAFETY: Caller ensures ptr is valid.
            unsafe {
                self.write_value_internal(gc, ptr, None, value, layout)?;
            }
            Ok(())
        }
    }

    /// Reads a value from a memory location.
    ///
    /// # Safety
    /// The caller must ensure that `offset` represents a valid memory location if `owner` is None.
    pub unsafe fn read_unaligned(
        &self,
        gc: GCHandle<'gc>,
        owner: Option<MemoryOwner<'gc>>,
        offset: ByteOffset,
        layout: &LayoutManager,
        type_desc: Option<TypeDescription>,
    ) -> Result<StackValue<'gc>, MemoryAccessError> {
        if let Some(owner) = owner {
            owner.as_heap_storage(|_storage| {}); // Ensure object is valid and magic matches

            // Get layout before locking to avoid deadlock
            let src_layout = self.get_layout_from_owner(owner);

            // SAFETY: with_data ensures the lock is held for the duration of the closure.
            // read_value_internal will read the data.
            owner.with_data(|data| {
                let base = data.as_ptr();
                let len = data.len();
                let ptr = base.wrapping_add(offset.0);
                validate_atomic_access(ptr, false);

                // 1. Bounds Check
                self.check_bounds_internal(ptr as *mut u8, base, len, layout.size().as_usize())?;

                // 2. Read Safety Check
                if offset.0 != 0 || src_layout.is_some() {
                    check_read_safety(layout, src_layout.as_ref(), offset.0)?;
                }

                // 3. Perform Read
                unsafe { self.read_value_internal(gc, ptr, Some(owner), layout, type_desc) }
            })
        } else {
            let ptr = sptr::from_exposed_addr::<u8>(offset.0);
            if ptr.is_null() {
                return Err(MemoryAccessError::NullPointer(
                    "NullReferenceException: reading from unmanaged null pointer".into(),
                ));
            }
            validate_atomic_access(ptr, false);
            // SAFETY: Caller ensures ptr is valid.
            unsafe { self.read_value_internal(gc, ptr, None, layout, type_desc) }
        }
    }

    /// Safely writes raw bytes to a memory location.
    ///
    /// # Safety
    ///
    /// The caller must ensure that `offset` represents a valid memory location if `owner` is None.
    /// If `owner` is provided, it must be the object that contains the memory to ensure GC safety.
    pub unsafe fn write_bytes(
        &mut self,
        gc: GCHandle<'gc>,
        owner: Option<MemoryOwner<'gc>>,
        offset: ByteOffset,
        data: &[u8],
    ) -> Result<(), MemoryAccessError> {
        if let Some(owner) = owner {
            owner.as_heap_storage(|_storage| {});

            // Get layout before locking to avoid deadlock
            #[cfg(feature = "multithreading")]
            let layout = self.get_layout_from_owner(owner);

            // The flush guard drains WB_LOCAL_BUF on drop, ensuring cross-arena
            // refs are recorded even if the inner closure panics.
            let _flush_guard = WriteBarrierFlushGuard;
            WB_LOCAL_BUF.with(|buf| {
                let mut b = buf.borrow_mut();
                let mut _recorder = WriteBarrierRecorder::new(owner.owner_id(), &mut b);

                owner.with_data_mut(gc, |obj_data| {
                    let base = obj_data.as_mut_ptr();
                    let len = obj_data.len();
                    let ptr = base.wrapping_add(offset.0);
                    validate_atomic_access(ptr as *const u8, false);

                    // Bounds Check
                    self.check_bounds_internal(ptr, base, len, data.len())?;

                    // Perform Write
                    // SAFETY: `ptr` points into `obj_data` at an offset that was
                    // just bounds-checked.  `data` (the caller's source slice) is
                    // a separate allocation, so there is no aliasing.  Both
                    // pointers are valid for `data.len()` bytes.
                    unsafe {
                        ptr::copy_nonoverlapping(data.as_ptr(), ptr, data.len());
                    }

                    #[cfg(feature = "multithreading")]
                    {
                        if let Some(layout) = layout {
                            // SAFETY: `base` is the start of the object's backing
                            // storage (held mutably for the duration of this closure).
                            // The range `[offset, offset+data.len())` was just
                            // bounds-checked above; `ptr.add` within that range is valid.
                            unsafe {
                                self.record_refs_in_range_with_recorder(
                                    gc,
                                    base,
                                    &layout,
                                    offset.as_usize(),
                                    offset.as_usize() + data.len(),
                                    &mut _recorder,
                                );
                            }
                        }
                    }
                    Ok(())
                })
            })
        } else {
            let ptr = sptr::from_exposed_addr_mut::<u8>(offset.0);
            if ptr.is_null() {
                return Err(MemoryAccessError::NullPointer(
                    "NullReferenceException: writing bytes to unmanaged null pointer".into(),
                ));
            }
            validate_atomic_access(ptr as *const u8, false);
            // SAFETY: `ptr` is a non-null unmanaged pointer whose validity is
            // guaranteed by the caller (unsafe fn contract).  `data` is a
            // distinct slice, so there is no aliasing.  Both pointers are
            // valid for `data.len()` bytes.
            unsafe {
                ptr::copy_nonoverlapping(data.as_ptr(), ptr, data.len());
            }
            Ok(())
        }
    }

    /// Safely reads raw bytes from a memory location.
    ///
    /// # Safety
    ///
    /// The caller must ensure that `offset` represents a valid memory location if `owner` is None.
    /// If `owner` is provided, it must be the object that contains the memory to ensure GC safety.
    pub unsafe fn read_bytes(
        &self,
        owner: Option<MemoryOwner<'gc>>,
        offset: ByteOffset,
        dest: &mut [u8],
    ) -> Result<(), MemoryAccessError> {
        if let Some(owner) = owner {
            owner.with_data(|data| {
                let start = offset.0;
                let end = start + dest.len();
                if end > data.len() {
                    return Err(MemoryAccessError::BoundsCheck {
                        offset: start,
                        size: dest.len(),
                        len: data.len(),
                    });
                }
                dest.copy_from_slice(&data[start..end]);
                Ok(())
            })
        } else {
            let ptr = sptr::from_exposed_addr::<u8>(offset.0);
            if ptr.is_null() {
                return Err(MemoryAccessError::NullPointer(
                    "NullReferenceException: reading bytes from unmanaged null pointer".into(),
                ));
            }
            validate_atomic_access(ptr, false);
            // SAFETY: Caller ensures ptr is valid.
            unsafe {
                ptr::copy_nonoverlapping(ptr, dest.as_mut_ptr(), dest.len());
            }
            Ok(())
        }
    }

    /// Atomically compares and exchanges a value in memory.
    ///
    /// # Safety
    /// Caller must ensure the offset and size are valid for the owner object.
    #[allow(clippy::too_many_arguments)]
    pub unsafe fn compare_exchange_atomic(
        &mut self,
        gc: GCHandle<'gc>,
        owner: Option<MemoryOwner<'gc>>,
        offset: ByteOffset,
        expected: u64,
        new: u64,
        size: usize,
        success: dotnet_utils::sync::Ordering,
        failure: dotnet_utils::sync::Ordering,
    ) -> Result<u64, CompareExchangeError> {
        use dotnet_utils::atomic::{AtomicAccess, StandardAtomicAccess};

        if let Some(owner) = owner {
            owner.with_data_mut(gc, |data| {
                let base = data.as_mut_ptr();
                let len = data.len();
                // SAFETY: `base` is the start of the object's backing storage.
                // The result may be out-of-bounds, but `check_bounds_internal`
                // (called immediately after) validates the range before any
                // dereference.  The slice length is bounded by `isize::MAX`, so
                // the arithmetic does not overflow.
                let ptr = unsafe { base.add(offset.as_usize()) };

                self.check_bounds_internal(ptr, base, len, size)
                    .map_err(CompareExchangeError::Bounds)?;

                // SAFETY: Bounds are verified above. `ptr` is a valid, aligned
                // pointer into `data` for `size` bytes, as guaranteed by
                // `check_bounds_internal`.
                unsafe {
                    StandardAtomicAccess::compare_exchange_atomic(
                        ptr, size, expected, new, success, failure,
                    )
                }
                .map_err(CompareExchangeError::Mismatch)
            })
        } else {
            let ptr = sptr::from_exposed_addr_mut::<u8>(offset.as_usize());
            // SAFETY: Caller guarantees `offset` encodes a valid unmanaged address
            // when `owner` is None.
            unsafe {
                StandardAtomicAccess::compare_exchange_atomic(
                    ptr, size, expected, new, success, failure,
                )
            }
            .map_err(CompareExchangeError::Mismatch)
        }
    }

    /// Atomically exchanges a value in memory.
    ///
    /// # Safety
    /// Caller must ensure the offset and size are valid for the owner object.
    pub unsafe fn exchange_atomic(
        &mut self,
        gc: GCHandle<'gc>,
        owner: Option<MemoryOwner<'gc>>,
        offset: ByteOffset,
        value: u64,
        size: usize,
        ordering: dotnet_utils::sync::Ordering,
    ) -> Result<u64, MemoryAccessError> {
        use dotnet_utils::atomic::{AtomicAccess, StandardAtomicAccess};

        if let Some(owner) = owner {
            owner.with_data_mut(gc, |data| {
                let base = data.as_mut_ptr();
                let len = data.len();
                // SAFETY: `base` is the start of the object's backing storage.
                // Bounds are checked immediately after by `check_bounds_internal`.
                let ptr = unsafe { base.add(offset.as_usize()) };

                self.check_bounds_internal(ptr, base, len, size)?;

                // SAFETY: `ptr` is within the object's backing storage (just
                // bounds-checked).  `size` bytes at `ptr` support atomic exchange.
                Ok(unsafe { StandardAtomicAccess::exchange_atomic(ptr, size, value, ordering) })
            })
        } else {
            let ptr = sptr::from_exposed_addr_mut::<u8>(offset.as_usize());
            if ptr.is_null() {
                return Err(MemoryAccessError::NullPointer(
                    "NullReferenceException: exchange_atomic to unmanaged null pointer".into(),
                ));
            }
            // SAFETY: `ptr` is a non-null unmanaged address whose validity is
            // guaranteed by the caller (unsafe fn contract).
            Ok(unsafe { StandardAtomicAccess::exchange_atomic(ptr, size, value, ordering) })
        }
    }

    /// Atomically adds a value to a memory location.
    ///
    /// # Safety
    /// Caller must ensure the offset and size are valid for the owner object.
    pub unsafe fn exchange_add_atomic(
        &mut self,
        gc: GCHandle<'gc>,
        owner: Option<MemoryOwner<'gc>>,
        offset: ByteOffset,
        value: u64,
        size: usize,
        ordering: dotnet_utils::sync::Ordering,
    ) -> Result<u64, MemoryAccessError> {
        use dotnet_utils::atomic::{AtomicAccess, StandardAtomicAccess};

        if let Some(owner) = owner {
            owner.with_data_mut(gc, |data| {
                let base = data.as_mut_ptr();
                let len = data.len();
                // SAFETY: `base` is the start of the object's backing storage.
                // Bounds are checked immediately after by `check_bounds_internal`.
                let ptr = unsafe { base.add(offset.as_usize()) };

                self.check_bounds_internal(ptr, base, len, size)?;

                Ok(
                    // SAFETY: `ptr` is within the object's backing storage (just
                    // bounds-checked).  `size` bytes at `ptr` support atomic add.
                    unsafe {
                        StandardAtomicAccess::exchange_add_atomic(ptr, size, value, ordering)
                    },
                )
            })
        } else {
            let ptr = sptr::from_exposed_addr_mut::<u8>(offset.as_usize());
            if ptr.is_null() {
                return Err(MemoryAccessError::NullPointer(
                    "NullReferenceException: exchange_add_atomic to unmanaged null pointer".into(),
                ));
            }
            // SAFETY: `ptr` is a non-null unmanaged address whose validity is
            // guaranteed by the caller (unsafe fn contract).
            Ok(unsafe { StandardAtomicAccess::exchange_add_atomic(ptr, size, value, ordering) })
        }
    }

    /// Atomically loads a value from memory.
    ///
    /// # Safety
    /// Caller must ensure the offset and size are valid for the owner object.
    pub unsafe fn load_atomic(
        &self,
        owner: Option<MemoryOwner<'gc>>,
        offset: ByteOffset,
        size: usize,
        ordering: dotnet_utils::sync::Ordering,
    ) -> Result<u64, MemoryAccessError> {
        use dotnet_utils::atomic::{AtomicAccess, StandardAtomicAccess};
        if let Some(owner) = owner {
            owner.with_data(|data| {
                let base = data.as_ptr();
                let len = data.len();
                // SAFETY: `base` is the start of the object's backing storage
                // (immutable borrow).  Bounds are checked immediately after.
                let ptr = unsafe { base.add(offset.as_usize()) };
                self.check_bounds_internal(ptr, base, len, size)?;
                // SAFETY: `ptr` is within the object's backing storage (just
                // bounds-checked).  The shared borrow prevents concurrent
                // mutation through safe Rust.
                Ok(unsafe { StandardAtomicAccess::load_atomic(ptr, size, ordering) })
            })
        } else {
            let ptr = sptr::from_exposed_addr::<u8>(offset.as_usize());
            if ptr.is_null() {
                return Err(MemoryAccessError::NullPointer(
                    "NullReferenceException: load_atomic from unmanaged null pointer".into(),
                ));
            }
            // SAFETY: `ptr` is a non-null unmanaged address whose validity is
            // guaranteed by the caller (unsafe fn contract).
            Ok(unsafe { StandardAtomicAccess::load_atomic(ptr, size, ordering) })
        }
    }

    /// Atomically stores a value to memory.
    ///
    /// # Safety
    /// Caller must ensure the offset and size are valid for the owner object.
    pub unsafe fn store_atomic(
        &mut self,
        gc: GCHandle<'gc>,
        owner: Option<MemoryOwner<'gc>>,
        offset: ByteOffset,
        value: u64,
        size: usize,
        ordering: dotnet_utils::sync::Ordering,
    ) -> Result<(), MemoryAccessError> {
        use dotnet_utils::atomic::{AtomicAccess, StandardAtomicAccess};
        if let Some(owner) = owner {
            owner.with_data_mut(gc, |data| {
                let base = data.as_mut_ptr();
                let len = data.len();
                // SAFETY: `base` is the start of the object's backing storage.
                // Bounds are checked immediately after by `check_bounds_internal`.
                let ptr = unsafe { base.add(offset.as_usize()) };
                self.check_bounds_internal(ptr, base, len, size)?;
                // SAFETY: `ptr` is within the object's backing storage (just
                // bounds-checked).  `size` bytes at `ptr` support atomic store.
                unsafe {
                    StandardAtomicAccess::store_atomic(ptr, size, value, ordering);
                }
                Ok(())
            })
        } else {
            let ptr = sptr::from_exposed_addr_mut::<u8>(offset.as_usize());
            if ptr.is_null() {
                return Err(MemoryAccessError::NullPointer(
                    "NullReferenceException: store_atomic to unmanaged null pointer".into(),
                ));
            }
            // SAFETY: `ptr` is a non-null unmanaged address whose validity is
            // guaranteed by the caller (unsafe fn contract).
            unsafe {
                StandardAtomicAccess::store_atomic(ptr, size, value, ordering);
            }
            Ok(())
        }
    }

    pub fn get_storage_base(&self, owner: ObjectRef<'gc>) -> (*const u8, usize) {
        if let Some(h) = owner.0 {
            let obj = h.borrow();
            match &obj.storage {
                HeapStorage::Obj(_) | HeapStorage::Boxed(_) | HeapStorage::Vec(_) => {
                    // SAFETY: `obj` is a live, borrow-locked `ObjectInner`.
                    // `raw_data_ptr()` returns a pointer to the inner allocation
                    // that is valid for at least the lifetime of `obj` (the guard).
                    let ptr = unsafe { obj.storage.raw_data_ptr() } as *const u8;
                    let size = obj.storage.size_bytes();
                    (ptr, size)
                }
                _ => (ptr::null(), 0),
            }
        } else {
            (ptr::null(), 0)
        }
    }

    fn check_bounds_internal(
        &self,
        ptr: *const u8,
        base: *const u8,
        len: usize,
        size: usize,
    ) -> Result<(), MemoryAccessError> {
        check_bounds(ptr, base, len, size)
    }

    fn check_integrity_internal_with_layout(
        &self,
        ptr: *const u8,
        dest_layout: Option<LayoutManager>,
        base: *const u8,
        src_layout: &LayoutManager,
    ) -> Result<(), MemoryAccessError> {
        if !base.is_null() {
            let base_addr = base.addr();
            let ptr_addr = ptr.addr();
            let offset = ptr_addr.wrapping_sub(base_addr);

            if let Some(dl) = dest_layout {
                validate_ref_integrity(
                    &dl,
                    0,
                    offset,
                    offset + src_layout.size().as_usize(),
                    src_layout,
                )?;
            }
        }
        Ok(())
    }

    pub fn get_layout_from_owner(&self, owner: MemoryOwner<'gc>) -> Option<LayoutManager> {
        owner.as_heap_storage(|storage| match storage {
            HeapStorage::Obj(o) => Some(LayoutManager::Field(
                o.instance_storage.layout().as_ref().clone(),
            )),
            HeapStorage::Vec(v) => Some(LayoutManager::Array(v.layout.clone())),
            HeapStorage::Boxed(o) => Some(LayoutManager::Field(
                o.instance_storage.layout().as_ref().clone(),
            )),
            _ => None,
        })
    }

    /// Writes a value to a heap-allocated object, ensuring memory bounds,
    /// layout integrity, and GC write barriers.
    ///
    /// # Safety
    /// The caller must ensure that `offset` lies within the `owner`'s storage.
    pub unsafe fn write_to_heap(
        &mut self,
        gc: GCHandle<'gc>,
        target: HeapWriteTarget<'gc>,
        offset: ByteOffset,
        value: StackValue<'gc>,
        layout: &LayoutManager,
    ) -> Result<(), MemoryAccessError> {
        let owner = target.0;
        owner.as_heap_storage(|_storage| {});

        let dest_layout = self.get_layout_from_owner(owner);

        if std::env::var("DOTNET_TRACE_CULTUREDATA_WRITES").is_ok() {
            let write_start = offset.as_usize();
            let write_end = write_start + layout.size().as_usize();
            let target_start = 288usize;
            let target_end = target_start + ObjectRef::SIZE;

            if write_start < target_end && write_end > target_start {
                owner.as_heap_storage(|storage| {
                    if let HeapStorage::Obj(o) = storage
                        && o.description.type_name() == "System.Globalization.CultureData"
                    {
                        let value_kind = match &value {
                            StackValue::Int32(_) => "Int32",
                            StackValue::Int64(_) => "Int64",
                            StackValue::NativeInt(_) => "NativeInt",
                            StackValue::NativeFloat(_) => "NativeFloat",
                            StackValue::ObjectRef(_) => "ObjectRef",
                            StackValue::UnmanagedPtr(_) => "UnmanagedPtr",
                            StackValue::ManagedPtr(_) => "ManagedPtr",
                            StackValue::ValueType(_) => "ValueType",
                            StackValue::TypedRef(_, _) => "TypedRef",
                            #[cfg(feature = "multithreading")]
                            StackValue::CrossArenaObjectRef(_, _) => "CrossArenaObjectRef",
                        };

                        eprintln!(
                            "[GCDBG] CultureData write overlap: offset={} size={} layout_tag={} value_kind={}",
                            write_start,
                            layout.size().as_usize(),
                            layout.type_tag(),
                            value_kind
                        );

                        match &value {
                            StackValue::ObjectRef(ObjectRef(Some(h))) => {
                                eprintln!(
                                    "[GCDBG] CultureData write ObjectRef raw_ptr=0x{:016X}",
                                    gc_arena::Gc::as_ptr(*h) as usize
                                );
                            }
                            StackValue::ObjectRef(ObjectRef(None)) => {
                                eprintln!("[GCDBG] CultureData write ObjectRef raw_ptr=NULL");
                            }
                            StackValue::UnmanagedPtr(p) => {
                                eprintln!(
                                    "[GCDBG] CultureData write UnmanagedPtr raw_ptr=0x{:016X}",
                                    p.0.as_ptr() as usize
                                );
                            }
                            StackValue::ValueType(v) => {
                                eprintln!(
                                    "[GCDBG] CultureData write ValueType type={} size={}",
                                    v.description.type_name(),
                                    v.instance_storage.layout().size().as_usize()
                                );
                            }
                            _ => {}
                        }

                        eprintln!(
                            "[GCDBG] CultureData write backtrace:\n{}",
                            std::backtrace::Backtrace::force_capture()
                        );
                    }
                });
            }
        }

        // The flush guard drains WB_LOCAL_BUF on drop, ensuring cross-arena
        // refs are recorded even if the inner closure panics.
        let _flush_guard = WriteBarrierFlushGuard;
        WB_LOCAL_BUF.with(|buf| {
            let mut b = buf.borrow_mut();
            let mut recorder = WriteBarrierRecorder::new(owner.owner_id(), &mut b);
            owner.with_data_mut(gc, |data| {
                let base = data.as_mut_ptr();
                let len = data.len();
                let ptr = base.wrapping_add(offset.0);
                validate_atomic_access(ptr as *const u8, false);

                self.check_bounds_internal(ptr, base, len, layout.size().as_usize())?;
                self.check_integrity_internal_with_layout(ptr, dest_layout, base, layout)?;

                unsafe {
                    self.write_value_internal_with_recorder(
                        gc,
                        ptr,
                        Some(owner),
                        value,
                        layout,
                        &mut recorder,
                    )
                }
            })
        })
    }

    /// Writes a value to unmanaged or static memory (e.g. stack, static fields, unmanaged pointers).
    ///
    /// # Safety
    /// The caller must ensure that `ptr` is a valid, writable address and has enough space
    /// for the value layout.
    pub unsafe fn write_to_unmanaged(
        &mut self,
        gc: GCHandle<'gc>,
        ptr: *mut u8,
        value: StackValue<'gc>,
        layout: &LayoutManager,
    ) -> Result<(), MemoryAccessError> {
        if ptr.is_null() {
            return Err(MemoryAccessError::NullPointer(
                "NullReferenceException: writing to unmanaged null pointer".into(),
            ));
        }
        validate_atomic_access(ptr as *const u8, false);
        unsafe { self.write_value_internal(gc, ptr, None, value, layout) }
    }

    #[cfg(feature = "multithreading")]
    pub(crate) fn record_objref_cross_arena_with_recorder(
        &self,
        r: ObjectRef<'gc>,
        _owner_tid: ArenaId,
        recorder: &mut WriteBarrierRecorder<'_, 'gc>,
    ) {
        recorder.record_ref(r);
    }

    #[cfg(feature = "multithreading")]
    pub(crate) unsafe fn record_objref_at_ptr_with_recorder(
        &self,
        gc: GCHandle<'gc>,
        ptr: *const u8,
        owner_tid: ArenaId,
        recorder: &mut WriteBarrierRecorder<'_, 'gc>,
    ) {
        let mut buf = [0u8; ObjectRef::SIZE];
        // SAFETY: The caller guarantees `ptr` points to `ObjectRef::SIZE` bytes
        // of readable memory within a live object's storage.  `buf` is a
        // distinct stack allocation so there is no aliasing.
        unsafe {
            ptr::copy_nonoverlapping(ptr, buf.as_mut_ptr(), ObjectRef::SIZE);
            let r = ObjectRef::read_branded(&buf, &gc);
            self.record_objref_cross_arena_with_recorder(r, owner_tid, recorder);
        }
    }

    #[cfg(feature = "multithreading")]
    pub(crate) fn record_managedptr_cross_arena_with_recorder(
        &self,
        m: &ManagedPtr<'gc>,
        _owner_tid: ArenaId,
        recorder: &mut WriteBarrierRecorder<'_, 'gc>,
    ) {
        recorder.record_managed_ptr(m);
    }

    #[cfg(feature = "multithreading")]
    pub(crate) unsafe fn record_managedptr_at_ptr_with_recorder(
        &self,
        gc: GCHandle<'gc>,
        ptr: *const u8,
        owner_tid: ArenaId,
        recorder: &mut WriteBarrierRecorder<'_, 'gc>,
    ) {
        // SAFETY: The caller guarantees `ptr` points to `ManagedPtr::SIZE` bytes
        // of readable memory within a live object's storage.  `from_raw_parts`
        // is sound because `ManagedPtr::SIZE` matches the slice length and `ptr`
        // is valid for that many bytes.
        let info = unsafe {
            ManagedPtr::read_branded(std::slice::from_raw_parts(ptr, ManagedPtr::SIZE), &gc)
                .expect("record_managedptr_at_ptr: failed to read ManagedPtr")
        };
        match &info.origin {
            PointerOrigin::Heap(r) => {
                self.record_objref_cross_arena_with_recorder(*r, owner_tid, recorder)
            }
            PointerOrigin::CrossArenaObjectRef(p, target_tid) => {
                if *target_tid != owner_tid {
                    recorder.buffer.push((*target_tid, p.as_ptr() as usize));
                }
            }
            _ => {}
        }
    }

    pub(crate) unsafe fn write_value_internal(
        &mut self,
        gc: GCHandle<'gc>,
        ptr: *mut u8,
        owner: Option<MemoryOwner<'gc>>,
        value: StackValue<'gc>,
        layout: &LayoutManager,
    ) -> Result<(), MemoryAccessError> {
        // The flush guard drains WB_LOCAL_BUF on drop, ensuring cross-arena
        // refs are recorded even if the inner closure panics.
        let _flush_guard = WriteBarrierFlushGuard;
        WB_LOCAL_BUF.with(|buf| {
            let mut b = buf.borrow_mut();
            let mut _recorder = WriteBarrierRecorder::new(
                owner.map(|o| o.owner_id()).unwrap_or(ArenaId(0)),
                &mut b,
            );
            // SAFETY: `ptr` is a valid, non-null pointer that was verified by
            // the outer unsafe fn's contract.  The `WB_LOCAL_BUF` borrow lives
            // only inside this closure so there is no re-entrant borrow conflict.
            unsafe {
                self.write_value_internal_with_recorder(
                    gc,
                    ptr,
                    owner,
                    value,
                    layout,
                    &mut _recorder,
                )
            }
        })
    }

    pub(crate) unsafe fn write_value_internal_with_recorder(
        &mut self,
        gc: GCHandle<'gc>,
        ptr: *mut u8,
        owner: Option<MemoryOwner<'gc>>,
        value: StackValue<'gc>,
        layout: &LayoutManager,
        _recorder: &mut WriteBarrierRecorder<'_, 'gc>,
    ) -> Result<(), MemoryAccessError> {
        // SAFETY: `ptr` is non-null (checked immediately below) and has been
        // validated by the caller (`write_unaligned` / `write_to_heap` paths
        // perform bounds and integrity checks before reaching here).
        // All `ptr::write_unaligned` calls are sound because `ptr` is valid for
        // the write size and unaligned writes are explicitly allowed.
        unsafe {
            if ptr.is_null() {
                return Err(MemoryAccessError::NullPointer(
                    "RawMemoryAccess::write_value_internal called with null pointer!".to_string(),
                ));
            }

            Self::backward_barrier_for_heap_ref_write(gc, owner, &value, layout);

            match layout {
                LayoutManager::Scalar(s) => match s {
                    Scalar::Int8 => {
                        let v = extract_int(value)? as i8;
                        ptr::write_unaligned(ptr as *mut i8, v);
                    }
                    Scalar::UInt8 => {
                        let v = extract_int(value)? as u8;
                        ptr::write_unaligned(ptr, v);
                    }
                    Scalar::Int16 => {
                        let v = extract_int(value)? as i16;
                        ptr::write_unaligned(ptr as *mut i16, v);
                    }
                    Scalar::UInt16 => {
                        let v = extract_int(value)? as u16;
                        ptr::write_unaligned(ptr as *mut u16, v);
                    }
                    Scalar::Int32 => {
                        let v = extract_int(value)?;
                        ptr::write_unaligned(ptr as *mut i32, v);
                    }
                    Scalar::Int64 => {
                        let v = extract_long(value)?;
                        ptr::write_unaligned(ptr as *mut i64, v);
                    }
                    Scalar::NativeInt => {
                        let v = extract_native_int(value)?;
                        ptr::write_unaligned(ptr as *mut isize, v);
                    }
                    Scalar::Float32 => {
                        let v = extract_float(value)? as f32;
                        ptr::write_unaligned(ptr as *mut f32, v);
                    }
                    Scalar::Float64 => {
                        let v = extract_float(value)?;
                        ptr::write_unaligned(ptr as *mut f64, v);
                    }
                    Scalar::ManagedPtr => {
                        if let StackValue::ManagedPtr(m) = value {
                            m.write(std::slice::from_raw_parts_mut(ptr, ManagedPtr::SIZE));

                            #[cfg(feature = "multithreading")]
                            if let Some(owner) = owner {
                                self.record_managedptr_cross_arena_with_recorder(
                                    &m,
                                    owner.owner_id(),
                                    _recorder,
                                );
                            }
                        } else {
                            return Err(MemoryAccessError::TypeMismatch(
                                "Expected ManagedPtr".into(),
                            ));
                        }
                    }
                    Scalar::ObjectRef => {
                        if let StackValue::ObjectRef(r) = value {
                            // Use ObjectRef::write() to properly serialize the pointer.
                            // This ensures cross-arena references are tagged correctly.
                            r.write(std::slice::from_raw_parts_mut(ptr, ObjectRef::SIZE));

                            #[cfg(feature = "multithreading")]
                            if let Some(owner) = owner {
                                self.record_objref_cross_arena_with_recorder(
                                    r,
                                    owner.owner_id(),
                                    _recorder,
                                );
                            }
                        } else {
                            return Err(MemoryAccessError::TypeMismatch(
                                "Expected ObjectRef".into(),
                            ));
                        }
                    }
                },
                LayoutManager::Field(flm) => {
                    if let StackValue::ValueType(src_obj) = value {
                        let src_ptr = src_obj.instance_storage.raw_data_ptr();
                        ptr::copy_nonoverlapping(src_ptr, ptr, flm.size().as_usize());

                        #[cfg(feature = "multithreading")]
                        if owner.is_some() {
                            self.record_refs_recursive_with_recorder(gc, ptr, layout, _recorder);
                        }
                    } else {
                        return Err(MemoryAccessError::TypeMismatch(
                            "Expected ValueType for Struct write".into(),
                        ));
                    }
                }
                LayoutManager::Array(_) => {
                    return Err(MemoryAccessError::TypeMismatch(
                        "Cannot write entire array unaligned".into(),
                    ));
                }
            }
            Ok(())
        }
    }

    fn backward_barrier_for_heap_ref_write(
        gc: GCHandle<'gc>,
        owner: Option<MemoryOwner<'gc>>,
        value: &StackValue<'gc>,
        layout: &LayoutManager,
    ) {
        let should_barrier = match layout {
            LayoutManager::Scalar(Scalar::ObjectRef) => {
                matches!(value, StackValue::ObjectRef(ObjectRef(Some(_))))
            }
            LayoutManager::Scalar(Scalar::ManagedPtr) => matches!(
                value,
                StackValue::ManagedPtr(m) if m.owner().is_some_and(|r| r.0.is_some())
            ),
            LayoutManager::Field(flm) => {
                flm.has_ref_fields && matches!(value, StackValue::ValueType(_))
            }
            _ => false,
        };

        if !should_barrier {
            return;
        }

        let Some(MemoryOwner::Local(parent)) = owner else {
            return;
        };
        let Some(parent_gc) = parent.0 else {
            return;
        };

        // FieldStorage writes use interior mutability over raw bytes, so we must
        // explicitly trigger a backward barrier before adopting new GC children.
        let _ = gc_arena::Gc::write(gc.mutation(), parent_gc);
    }

    #[cfg(feature = "multithreading")]
    unsafe fn record_refs_recursive_with_recorder(
        &self,
        gc: GCHandle<'gc>,
        ptr: *const u8,
        layout: &LayoutManager,
        recorder: &mut WriteBarrierRecorder<'_, 'gc>,
    ) {
        if !layout.is_or_contains_refs() {
            return;
        }
        let owner_tid = recorder.arena_id;
        match layout {
            // SAFETY: For each scalar variant, the caller guarantees `ptr`
            // points to a valid, readable slot of the appropriate size within
            // a live object's backing storage.
            LayoutManager::Scalar(Scalar::ObjectRef) => unsafe {
                self.record_objref_at_ptr_with_recorder(gc, ptr, owner_tid, recorder);
            },
            LayoutManager::Scalar(Scalar::ManagedPtr) => unsafe {
                self.record_managedptr_at_ptr_with_recorder(gc, ptr, owner_tid, recorder);
            },
            LayoutManager::Field(flm) => {
                // Use GcDesc for fast ObjectRef recording
                let ptr_size = ObjectRef::SIZE;
                for word_index in flm.gc_desc.bitmap.iter_ones() {
                    let offset = word_index * ptr_size;
                    // SAFETY: `offset` is a word-aligned byte offset derived from
                    // the layout's GC bitmap; `ptr.add(offset)` stays within the
                    // field struct whose total size is bounded by the allocation.
                    unsafe {
                        self.record_objref_at_ptr_with_recorder(
                            gc,
                            ptr.add(offset),
                            owner_tid,
                            recorder,
                        );
                    }
                }
                for offset in &flm.gc_desc.unaligned_offsets {
                    // SAFETY: Unaligned offsets are validated by layout construction
                    // to lie within the struct's storage.
                    unsafe {
                        self.record_objref_at_ptr_with_recorder(
                            gc,
                            ptr.add(*offset),
                            owner_tid,
                            recorder,
                        );
                    }
                }
                // Use visit_managed_ptrs for recursive ManagedPtr recording
                if flm.has_ref_fields {
                    // SAFETY: `visit_managed_ptrs` yields offsets that are within
                    // the field struct's backing storage; `ptr.add(offset)` is valid.
                    flm.visit_managed_ptrs(ByteOffset(0), &mut |offset| unsafe {
                        self.record_managedptr_at_ptr_with_recorder(
                            gc,
                            ptr.add(offset.as_usize()),
                            owner_tid,
                            recorder,
                        );
                    });
                }
            }
            LayoutManager::Array(arr) => {
                if arr.element_layout.is_or_contains_refs() {
                    let elem_size = arr.element_layout.size().as_usize();
                    for i in 0..arr.length {
                        // SAFETY: `i * elem_size` is within the array allocation
                        // because `i < arr.length` and `arr.length * elem_size`
                        // equals the total allocation size.
                        unsafe {
                            self.record_refs_recursive_with_recorder(
                                gc,
                                ptr.add(i * elem_size),
                                &arr.element_layout,
                                recorder,
                            );
                        }
                    }
                }
            }
            _ => {}
        }
    }

    #[cfg(feature = "multithreading")]
    unsafe fn record_refs_in_range_with_recorder(
        &self,
        gc: GCHandle<'gc>,
        ptr: *const u8, // Base of the layout
        layout: &LayoutManager,
        range_start: usize,
        range_end: usize,
        recorder: &mut WriteBarrierRecorder<'_, 'gc>,
    ) {
        if !layout.is_or_contains_refs() {
            return;
        }
        match layout {
            LayoutManager::Scalar(Scalar::ObjectRef)
            | LayoutManager::Scalar(Scalar::ManagedPtr) => {
                // If the scalar overlaps at all with the written range, we should re-record it
                // because it might have been partially or fully overwritten.
                // SAFETY: `ptr` is the base of this scalar slot (passed in from
                // the parent call), which is valid for the scalar's size within
                // the enclosing object's backing storage.
                unsafe { self.record_refs_recursive_with_recorder(gc, ptr, layout, recorder) };
            }
            LayoutManager::Field(flm) => {
                for field in flm.fields.values() {
                    let f_start = field.position.as_usize();
                    let f_end = f_start + field.layout.size().as_usize();
                    if f_start < range_end && f_end > range_start {
                        // SAFETY: `f_start` is a field offset within the struct
                        // layout; `ptr.add(f_start)` remains within the struct's
                        // backing storage which the caller guarantees is live.
                        unsafe {
                            self.record_refs_in_range_with_recorder(
                                gc,
                                ptr.add(f_start),
                                &field.layout,
                                range_start.saturating_sub(f_start),
                                range_end.saturating_sub(f_start),
                                recorder,
                            );
                        }
                    }
                }
            }
            LayoutManager::Array(alm) => {
                let elem_size = alm.element_layout.size().as_usize();
                if elem_size > 0 {
                    let start_idx = range_start / elem_size;
                    let end_idx = range_end.div_ceil(elem_size);
                    let start_idx = start_idx.min(alm.length);
                    let end_idx = end_idx.min(alm.length);

                    for i in start_idx..end_idx {
                        let f_start = i * elem_size;
                        // SAFETY: `f_start = i * elem_size` with `i < alm.length`
                        // is within the array's backing storage.
                        unsafe {
                            self.record_refs_in_range_with_recorder(
                                gc,
                                ptr.add(f_start),
                                &alm.element_layout,
                                range_start.saturating_sub(f_start),
                                range_end.saturating_sub(f_start),
                                recorder,
                            );
                        }
                    }
                }
            }
            _ => {}
        }
    }

    pub(crate) unsafe fn read_value_internal(
        &self,
        gc: GCHandle<'gc>,
        ptr: *const u8,
        _owner: Option<MemoryOwner<'gc>>,
        layout: &LayoutManager,
        type_desc: Option<TypeDescription>,
    ) -> Result<StackValue<'gc>, MemoryAccessError> {
        // SAFETY: The caller must ensure `ptr` is valid for reads and within bounds.
        // This is verified by `read_unaligned` before calling this method.
        unsafe {
            if ptr.is_null() {
                return Err(MemoryAccessError::NullPointer(
                    "RawMemoryAccess::read_value_internal called with null pointer!".to_string(),
                ));
            }

            Ok(match layout {
                LayoutManager::Scalar(s) => match s {
                    Scalar::Int8 => StackValue::Int32(ptr::read_unaligned(ptr as *const i8) as i32),
                    Scalar::UInt8 => StackValue::Int32(ptr::read_unaligned(ptr) as i32),
                    Scalar::Int16 => {
                        StackValue::Int32(ptr::read_unaligned(ptr as *const i16) as i32)
                    }
                    Scalar::UInt16 => {
                        StackValue::Int32(ptr::read_unaligned(ptr as *const u16) as i32)
                    }
                    Scalar::Int32 => StackValue::Int32(ptr::read_unaligned(ptr as *const i32)),
                    Scalar::Int64 => StackValue::Int64(ptr::read_unaligned(ptr as *const i64)),
                    Scalar::NativeInt => {
                        StackValue::NativeInt(ptr::read_unaligned(ptr as *const isize))
                    }
                    Scalar::Float32 => {
                        StackValue::NativeFloat(ptr::read_unaligned(ptr as *const f32) as f64)
                    }
                    Scalar::Float64 => {
                        StackValue::NativeFloat(ptr::read_unaligned(ptr as *const f64))
                    }
                    Scalar::ObjectRef => {
                        let mut buf = [0u8; ObjectRef::SIZE];
                        ptr::copy_nonoverlapping(ptr, buf.as_mut_ptr(), ObjectRef::SIZE);
                        StackValue::ObjectRef(ObjectRef::read_branded(&buf, &gc))
                    }
                    Scalar::ManagedPtr => {
                        let info = ManagedPtr::read_branded(
                            std::slice::from_raw_parts(ptr, ManagedPtr::SIZE),
                            &gc,
                        )
                        .map_err(|e| {
                            MemoryAccessError::TypeMismatch(format!(
                                "ManagedPtr read failed: {:?}",
                                e
                            ))
                        })?;

                        let actual_desc = type_desc
                            .unwrap_or(TypeDescription::new(ResolutionS::NULL, std::mem::zeroed()));

                        let m = ManagedPtr::from_info_full(info, actual_desc, false);
                        StackValue::ManagedPtr(m)
                    }
                },
                LayoutManager::Field(flm) => {
                    if let Some(desc) = type_desc {
                        let size = flm.size();
                        let mut data = vec![0u8; size.as_usize()];
                        ptr::copy_nonoverlapping(ptr, data.as_mut_ptr(), size.as_usize());

                        let storage = FieldStorage::new(Arc::new(flm.clone()), data);
                        let obj = ObjectInstance::new(desc, GenericLookup::default(), storage);

                        StackValue::ValueType(obj)
                    } else {
                        return Err(MemoryAccessError::TypeMismatch(
                            "Struct read requires TypeDescription, which is not passed to read_unaligned".into(),
                        ));
                    }
                }
                _ => {
                    return Err(MemoryAccessError::TypeMismatch(
                        "Array read not supported".to_string(),
                    ));
                }
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{WB_LOCAL_BUF, WriteBarrierFlushGuard, check_bounds};
    use crate::error::{CompareExchangeError, MemoryAccessError};
    use dotnet_utils::ArenaId;

    /// `check_bounds` is the gate used by `compare_exchange_atomic` to detect
    /// out-of-bounds accesses.  These tests confirm it produces the correct
    /// `MemoryAccessError::BoundsCheck` payload that gets wrapped into
    /// `CompareExchangeError::Bounds` at the call site.
    #[test]
    fn check_bounds_in_range_ok() {
        let buf = [0u8; 8];
        let base = buf.as_ptr();
        // offset=0, size=4 inside an 8-byte buffer — must succeed
        assert!(check_bounds(base, base, 8, 4).is_ok());
    }

    #[test]
    fn check_bounds_null_base_skips_check() {
        // Null base means unmanaged pointer — check is skipped entirely.
        assert!(check_bounds(std::ptr::null(), std::ptr::null(), 0, 8).is_ok());
    }

    #[test]
    fn check_bounds_out_of_range_err() {
        let buf = [0u8; 4];
        let base = buf.as_ptr();
        // offset=0, size=8 overflows a 4-byte buffer — must fail
        assert_eq!(
            check_bounds(base, base, 4, 8),
            Err(MemoryAccessError::BoundsCheck {
                offset: 0,
                size: 8,
                len: 4,
            })
        );
    }

    #[test]
    fn check_bounds_offset_overflow_err() {
        let buf = [0u8; 8];
        let base = buf.as_ptr();
        // ptr points 6 bytes in; reading 4 bytes would end at offset 10 > 8 — must fail
        // SAFETY: `base.add(6)` stays within the 8-byte allocation.
        let ptr = unsafe { base.add(6) };
        assert_eq!(
            check_bounds(ptr, base, 8, 4),
            Err(MemoryAccessError::BoundsCheck {
                offset: 6,
                size: 4,
                len: 8,
            })
        );
    }

    /// Verify that `CompareExchangeError` variants carry the expected payloads,
    /// covering the two match arms at every call site.
    #[test]
    fn compare_exchange_error_variants() {
        // Mismatch arm — carries the actual current value.
        let mismatch = CompareExchangeError::Mismatch(42);
        match mismatch {
            CompareExchangeError::Mismatch(v) => assert_eq!(v, 42),
            _ => panic!("expected Mismatch"),
        }

        // Bounds arm — carries the underlying MemoryAccessError.
        let bounds_inner = MemoryAccessError::BoundsCheck {
            offset: 0,
            size: 8,
            len: 4,
        };
        let bounds_err = CompareExchangeError::Bounds(bounds_inner.clone());
        match bounds_err {
            CompareExchangeError::Bounds(e) => assert_eq!(e, bounds_inner),
            _ => panic!("expected Bounds"),
        }
    }

    /// Confirm that `check_bounds` errors can be promoted to `CompareExchangeError::Bounds`
    /// exactly as `compare_exchange_atomic` does it via `.map_err(CompareExchangeError::Bounds)`.
    #[test]
    fn check_bounds_error_promoted_to_compare_exchange_error() {
        let buf = [0u8; 4];
        let base = buf.as_ptr();
        let cas_result: Result<(), CompareExchangeError> =
            check_bounds(base, base, 4, 8).map_err(CompareExchangeError::Bounds);
        assert_eq!(
            cas_result,
            Err(CompareExchangeError::Bounds(
                MemoryAccessError::BoundsCheck {
                    offset: 0,
                    size: 8,
                    len: 4,
                }
            ))
        );
    }

    /// Verify that `WriteBarrierFlushGuard::drop` drains `WB_LOCAL_BUF` even
    /// when the surrounding code panics mid-write.
    ///
    /// We manually seed the TLS buffer, then force a panic inside
    /// `catch_unwind` while the guard is live.  After unwinding the buffer
    /// must be empty — confirming that `Drop` ran and drained it.
    #[test]
    fn write_barrier_flush_guard_drains_on_panic() {
        use std::panic::{self, AssertUnwindSafe};

        // Seed the TLS buffer with a dummy entry so there is something to drain.
        WB_LOCAL_BUF.with(|buf| {
            buf.borrow_mut().push((ArenaId(0), 0xDEAD_BEEF));
        });

        // Introduce a guard, then panic.  The guard's Drop must drain the buffer
        // even though control never reaches the end of the closure normally.
        let result = panic::catch_unwind(AssertUnwindSafe(|| {
            let _flush_guard = WriteBarrierFlushGuard;
            panic!("intentional test panic to verify flush-guard drop");
        }));

        assert!(result.is_err(), "expected the panic to be caught");

        // Buffer must be empty: Drop fired and drained it.
        WB_LOCAL_BUF.with(|buf| {
            assert!(
                buf.borrow().is_empty(),
                "WB_LOCAL_BUF should be empty after WriteBarrierFlushGuard dropped on unwind"
            );
        });
    }

    #[cfg(feature = "multithreading")]
    use super::MemoryOwner;
    #[cfg(feature = "multithreading")]
    use dotnet_utils::gc::{ArenaHandle, GCHandle, ThreadSafeLock};
    #[cfg(feature = "multithreading")]
    use dotnet_value::{
        CLRString, ValidationTag,
        object::{HeapStorage, OBJECT_MAGIC, ObjectInner, ObjectPtr},
    };
    #[cfg(feature = "multithreading")]
    use gc_arena::{Arena, Rootable};

    #[cfg(feature = "multithreading")]
    fn storage_is_string<'a>(storage: &HeapStorage<'a>) -> bool {
        matches!(storage, HeapStorage::Str(_))
    }

    #[cfg(feature = "multithreading")]
    fn cross_arena_with_short_lifetime<'short>(
        gc: GCHandle<'short>,
        ptr: ObjectPtr,
        tid: ArenaId,
    ) -> bool {
        let owner = MemoryOwner::cross_arena(gc, ptr, tid);
        owner.as_heap_storage(storage_is_string)
    }

    #[cfg(feature = "multithreading")]
    use dotnet_utils::gc::{register_arena, unregister_arena};
    #[cfg(feature = "multithreading")]
    use std::sync::{Arc, atomic::AtomicBool};

    #[cfg(feature = "multithreading")]
    #[test]
    fn cross_arena_heap_storage_access_supports_non_static_gc_lifetime() {
        let arena_id = ArenaId::new(4043);
        let lock = Box::new(ThreadSafeLock::new(ObjectInner {
            magic: ValidationTag::new(OBJECT_MAGIC),
            owner_id: arena_id,
            storage: HeapStorage::Str(CLRString::from("cross-arena-lifetime")),
        }));
        let raw: *const ThreadSafeLock<ObjectInner<'static>> = Box::leak(lock);
        // SAFETY: `raw` comes from `Box::leak`, is non-null, and remains valid
        // for the duration of this test until reconstructed with `Box::from_raw`.
        let ptr = unsafe { ObjectPtr::from_raw(raw) }.expect("non-null leaked lock pointer");

        // Register the arena so that `validate_arena_id` considers the
        // cross-arena reference live.  Under `memory-validation` the accessor
        // now calls `validate_arena_id`, which checks the global registry;
        // without registration it would panic with "Dangling cross-arena
        // reference".
        register_arena(arena_id, Arc::new(AtomicBool::new(false)));

        type TestRoot = Rootable![()];
        let arena = Arena::<TestRoot>::new(|_mc| ());
        let arena_handle = Box::into_raw(Box::new(ArenaHandle::new(arena_id)));
        assert!(arena.mutate(|mc, _| {
            // SAFETY: `arena_handle` was created by `Box::into_raw` above and
            // is not freed until after `mutate` returns; this shared borrow is valid.
            let arena_inner = unsafe { (&*arena_handle).as_inner() };
            let gc = GCHandle::new(
                mc,
                arena_inner,
                #[cfg(feature = "memory-validation")]
                arena_id,
            );
            cross_arena_with_short_lifetime(gc, ptr, arena_id)
        }));
        // SAFETY: `arena_handle` came from `Box::into_raw` above and remains
        // uniquely owned here; reconstructing it drops the allocation exactly once.
        unsafe {
            drop(Box::from_raw(arena_handle));
        }

        unregister_arena(arena_id);

        // Fix leak for Miri
        // SAFETY: `raw` was obtained from `Box::leak` earlier in this test;
        // we reconstruct the `Box` to release the memory.  No other owner
        // exists at this point, so this is the unique drop.
        unsafe {
            let _ = Box::from_raw(raw as *mut ThreadSafeLock<ObjectInner<'static>>);
        }
    }
}

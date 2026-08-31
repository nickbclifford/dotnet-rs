use crate::{heap::HeapManager, validation::*};
use dotnet_types::{
    TypeDescription,
    error::{CompareExchangeError, MemoryAccessError},
    generics::GenericLookup,
    resolution::ResolutionS,
};
use dotnet_utils::{
    ArenaId, ByteOffset,
    atomic::{Atomic, StandardAtomicAccess, validate_atomic_access},
    gc::GCHandle,
};
use dotnet_value::{
    StackValue,
    layout::{FieldLayoutManager, HasLayout, LayoutManager, Scalar},
    object::{HeapStorage, Object as ObjectInstance, ObjectRef},
    pointer::{ManagedPtr, NoManagedPtrResolver, unmanaged_ptr_from_addr},
    storage::FieldStorage,
};
use std::{ptr, ptr::NonNull, sync::Arc};

#[cfg(all(feature = "bench-instrumentation", feature = "multithreading"))]
use std::time::Instant;

#[cfg(feature = "multithreading")]
use dotnet_value::pointer::PointerOrigin;

use crate::write_barrier::*;

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

unsafe fn load_atomic_with_unaligned_fallback(
    ptr: *const u8,
    size: usize,
    ordering: dotnet_utils::sync::Ordering,
) -> u64 {
    if !matches!(size, 1 | 2 | 4 | 8) {
        panic!("Unsupported atomic size: {size}");
    }

    if dotnet_utils::is_ptr_aligned_to_field(ptr, size) {
        // SAFETY: F4.WidthAligned — The caller guarantees validity, and the branch proves alignment for `size`.
        return unsafe { StandardAtomicAccess::load_atomic_sized(ptr, size, ordering) };
    }

    validate_atomic_access(ptr, false);
    let mut buf = [0u8; 8];
    // SAFETY: F10.RawMemoryAccessValid — The caller guarantees valid storage and synchronization for the memcpy fallback.
    unsafe { ptr::copy_nonoverlapping(ptr, buf.as_mut_ptr(), size) };
    match size {
        1 => buf[0] as u64,
        2 => u16::from_ne_bytes(buf[..2].try_into().expect("buffer prefix is two bytes")) as u64,
        4 => u32::from_ne_bytes(buf[..4].try_into().expect("buffer prefix is four bytes")) as u64,
        8 => u64::from_ne_bytes(buf),
        _ => unreachable!(),
    }
}

unsafe fn store_atomic_with_unaligned_fallback(
    ptr: *mut u8,
    size: usize,
    value: u64,
    ordering: dotnet_utils::sync::Ordering,
) {
    if !matches!(size, 1 | 2 | 4 | 8) {
        panic!("Unsupported atomic size: {size}");
    }

    if dotnet_utils::is_ptr_aligned_to_field(ptr, size) {
        // SAFETY: F4.WidthAligned — The caller guarantees validity, and the branch proves alignment for `size`.
        unsafe { StandardAtomicAccess::store_atomic_sized(ptr, size, value, ordering) };
        return;
    }

    match size {
        1 => {
            // SAFETY: F10.RawMemoryAccessValid — The caller guarantees valid storage and synchronization for the memcpy fallback.
            unsafe { Atomic::store_field(ptr, &(value as u8).to_ne_bytes(), ordering) };
        }
        2 => {
            // SAFETY: F10.RawMemoryAccessValid — The caller guarantees valid storage and synchronization for the memcpy fallback.
            unsafe { Atomic::store_field(ptr, &(value as u16).to_ne_bytes(), ordering) };
        }
        4 => {
            // SAFETY: F10.RawMemoryAccessValid — The caller guarantees valid storage and synchronization for the memcpy fallback.
            unsafe { Atomic::store_field(ptr, &(value as u32).to_ne_bytes(), ordering) };
        }
        8 => {
            // SAFETY: F10.RawMemoryAccessValid — The caller guarantees valid storage and synchronization for the memcpy fallback.
            unsafe { Atomic::store_field(ptr, &value.to_ne_bytes(), ordering) };
        }
        _ => unreachable!(),
    }
}

fn field_layout_contains_heap_refs(layout: &FieldLayoutManager) -> bool {
    layout.has_ref_fields
        || !layout.gc_desc.bitmap.is_empty()
        || !layout.gc_desc.unaligned_offsets.is_empty()
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

            // Get layout before acquiring field access to avoid deadlock/borrow re-entry.
            let dest_layout = self.get_layout_from_owner(owner);

            // SAFETY: F10.RawMemoryAccessValid — with_data_mut provides exclusive access for the duration of the closure.
            // write_value_internal will copy the data.
            self.write_heap_value_with_barrier(gc, owner, offset, value, layout, dest_layout)
        } else {
            // SAFETY: F10.RawMemoryAccessValid — The ownerless API contract requires `offset` to be a
            // valid unmanaged address for this write.
            let ptr = unsafe { unmanaged_ptr_from_addr(offset.as_usize()) };
            if ptr.is_null() {
                return Err(MemoryAccessError::NullPointer(
                    "NullReferenceException: writing to unmanaged null pointer",
                ));
            }
            validate_atomic_access(ptr as *const u8, false);
            // SAFETY: F10.RawMemoryAccessValid — Caller ensures ptr is valid.
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

            // Get layout before acquiring field access to avoid deadlock/borrow re-entry.
            let src_layout = self.get_layout_from_owner(owner);

            // SAFETY: F10.RawMemoryAccessValid — with_data provides stable shared access for the duration of the closure.
            // read_value_internal will read the data.
            owner.with_data(|data| {
                let base = data.as_ptr();
                let len = data.len();
                let ptr = base.wrapping_add(offset.as_usize());
                validate_atomic_access(ptr, false);

                // 1. Bounds Check
                self.check_bounds_internal(ptr as *mut u8, base, len, layout.size().as_usize())?;

                // 2. Read Safety Check
                if offset.as_usize() != 0 || src_layout.is_some() {
                    check_read_safety(layout, src_layout.as_ref(), offset.as_usize())?;
                }

                // 3. Perform Read
                // SAFETY: F10.RawMemoryAccessValid — The closure keeps `owner`'s storage stable; the preceding bounds and
                // layout checks prove this derived pointer is valid for the requested read.
                unsafe { self.read_value_internal(gc, ptr, Some(owner), layout, type_desc) }
            })
        } else {
            // SAFETY: F10.RawMemoryAccessValid — The ownerless API contract requires `offset` to be a
            // valid unmanaged address for this read.
            let ptr = unsafe { unmanaged_ptr_from_addr(offset.as_usize()) };
            if ptr.is_null() {
                return Err(MemoryAccessError::NullPointer(
                    "NullReferenceException: reading from unmanaged null pointer",
                ));
            }
            validate_atomic_access(ptr, false);
            // SAFETY: F10.RawMemoryAccessValid — Caller ensures ptr is valid.
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

            // Get layout before acquiring field access to avoid deadlock/borrow re-entry.
            #[cfg(feature = "multithreading")]
            let layout = self.get_layout_from_owner(owner);

            // The panic-flush guard preserves unwind safety while allowing
            // normal-path write-barrier batching.
            let _flush_guard = WriteBarrierPanicFlushGuard;
            WB_LOCAL_BUF.with(|buf| {
                let mut b = buf.borrow_mut();
                let mut _recorder = WriteBarrierRecorder::new(owner.owner_id(), &mut b);

                owner.with_data_mut(gc, |obj_data| {
                    let base = obj_data.as_mut_ptr();
                    let len = obj_data.len();
                    let ptr = base.wrapping_add(offset.as_usize());
                    validate_atomic_access(ptr as *const u8, false);

                    // Bounds Check
                    self.check_bounds_internal(ptr, base, len, data.len())?;

                    // Perform Write
                    // SAFETY: F10.RawMemoryAccessValid — `ptr` points into `obj_data` at an offset that was
                    // just bounds-checked.  `data` (the caller's source slice) is
                    // a separate allocation, so there is no aliasing.  Both
                    // pointers are valid for `data.len()` bytes.
                    unsafe {
                        ptr::copy_nonoverlapping(data.as_ptr(), ptr, data.len());
                    }

                    #[cfg(feature = "multithreading")]
                    {
                        if let Some(layout) = layout {
                            // SAFETY: F10.RawMemoryAccessValid — `base` is the start of the object's backing
                            // storage (held mutably for the duration of this closure).
                            // The range `[offset, offset+data.len())` was just
                            // bounds-checked above; `ptr.add` within that range is valid.
                            #[cfg(feature = "bench-instrumentation")]
                            let layout_scan_start = Instant::now();
                            // SAFETY: F10.RawMemoryAccessValid — The layout scan uses the same live, bounds-checked object range.
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
                            #[cfg(feature = "bench-instrumentation")]
                            dotnet_metrics::record_active_layout_scan_timing(
                                "record_refs_in_range_with_recorder",
                                layout_scan_start.elapsed(),
                            );
                        }
                    }
                    Ok(())
                })
            })
        } else {
            // SAFETY: F10.RawMemoryAccessValid — The ownerless API contract requires `offset` to be a
            // valid unmanaged address for this byte write.
            let ptr = unsafe { unmanaged_ptr_from_addr(offset.as_usize()) };
            if ptr.is_null() {
                return Err(MemoryAccessError::NullPointer(
                    "NullReferenceException: writing bytes to unmanaged null pointer",
                ));
            }
            validate_atomic_access(ptr as *const u8, false);
            // SAFETY: F10.RawMemoryAccessValid — `ptr` is a non-null unmanaged pointer whose validity is
            // guaranteed by the caller (unsafe fn contract).  `data` is a
            // distinct slice, so there is no aliasing.  Both pointers are
            // valid for `data.len()` bytes.
            unsafe {
                ptr::copy_nonoverlapping(data.as_ptr(), ptr, data.len());
            }
            Ok(())
        }
    }

    /// Writes raw bytes through a provenance-carrying pointer.
    ///
    /// # Safety
    ///
    /// The caller must ensure that `ptr` is valid and writable for `data.len()` bytes,
    /// and that the write is appropriately synchronized. The pointed-to storage must not
    /// require a heap-owner write barrier.
    pub unsafe fn write_bytes_ptr(
        &mut self,
        ptr: NonNull<u8>,
        data: &[u8],
    ) -> Result<(), MemoryAccessError> {
        let ptr = ptr.as_ptr();
        validate_atomic_access(ptr as *const u8, false);
        // SAFETY: F10.RawMemoryAccessValid — The caller guarantees that `ptr` is valid and writable for `data.len()`
        // bytes. `data` is the caller's separate source slice, so the regions do not alias.
        unsafe {
            ptr::copy_nonoverlapping(data.as_ptr(), ptr, data.len());
        }
        Ok(())
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
                let start = offset.as_usize();
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
            // SAFETY: F10.RawMemoryAccessValid — The ownerless API contract requires `offset` to be a
            // valid unmanaged address for this byte read.
            let ptr = unsafe { unmanaged_ptr_from_addr(offset.as_usize()) };
            if ptr.is_null() {
                return Err(MemoryAccessError::NullPointer(
                    "NullReferenceException: reading bytes from unmanaged null pointer",
                ));
            }
            validate_atomic_access(ptr, false);
            // SAFETY: F10.RawMemoryAccessValid — Caller ensures ptr is valid.
            unsafe {
                ptr::copy_nonoverlapping(ptr, dest.as_mut_ptr(), dest.len());
            }
            Ok(())
        }
    }

    /// Reads raw bytes through a provenance-carrying pointer.
    ///
    /// # Safety
    ///
    /// The caller must ensure that `ptr` is valid and readable for `dest.len()` bytes,
    /// and that the read is appropriately synchronized.
    pub unsafe fn read_bytes_ptr(
        &self,
        ptr: NonNull<u8>,
        dest: &mut [u8],
    ) -> Result<(), MemoryAccessError> {
        let ptr = ptr.as_ptr();
        validate_atomic_access(ptr, false);
        // SAFETY: F10.RawMemoryAccessValid — The caller guarantees that `ptr` is valid and readable for `dest.len()`
        // bytes. `dest` is the caller's separate destination slice, so the regions do not alias.
        unsafe {
            ptr::copy_nonoverlapping(ptr, dest.as_mut_ptr(), dest.len());
        }
        Ok(())
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
        if let Some(owner) = owner {
            let result = owner.with_data_mut(gc, |data| {
                let base = data.as_mut_ptr();
                let len = data.len();
                // SAFETY: F10.RawMemoryAccessValid — `base` is the start of the object's backing storage.
                // The result may be out-of-bounds, but `check_bounds_internal`
                // (called immediately after) validates the range before any
                // dereference.  The slice length is bounded by `isize::MAX`, so
                // the arithmetic does not overflow.
                let ptr = unsafe { base.add(offset.as_usize()) };

                self.check_bounds_internal(ptr, base, len, size)
                    .map_err(CompareExchangeError::Bounds)?;

                if !dotnet_utils::is_ptr_aligned_to_field(ptr as *const u8, size) {
                    return Err(CompareExchangeError::Bounds(
                        MemoryAccessError::UnalignedAccess(ptr as usize),
                    ));
                }

                // SAFETY: F4.WidthAligned — The preceding checks prove `ptr` is in bounds and aligned for `size`.
                unsafe {
                    StandardAtomicAccess::compare_exchange_atomic_sized(
                        ptr, size, expected, new, success, failure,
                    )
                }
                .map_err(CompareExchangeError::Mismatch)
            });

            // Successful CAS mutates heap storage in-place. Treat it like any
            // other heap write so incremental GC revisits the parent object.
            if result.is_ok() {
                Self::backward_barrier_for_heap_atomic_write(gc, Some(owner));
            }

            result
        } else {
            // SAFETY: F10.RawMemoryAccessValid — The ownerless atomic API contract requires `offset` to
            // be a valid synchronized unmanaged address for this operation.
            let ptr = unsafe { unmanaged_ptr_from_addr(offset.as_usize()) };
            if !dotnet_utils::is_ptr_aligned_to_field(ptr as *const u8, size) {
                return Err(CompareExchangeError::Bounds(
                    MemoryAccessError::UnalignedAccess(ptr as usize),
                ));
            }
            // SAFETY: F4.WidthAligned — The caller guarantees validity, and the preceding check proves alignment.
            unsafe {
                StandardAtomicAccess::compare_exchange_atomic_sized(
                    ptr, size, expected, new, success, failure,
                )
            }
            .map_err(CompareExchangeError::Mismatch)
        }
    }

    /// Atomically compares and exchanges through a provenance-carrying pointer.
    ///
    /// # Safety
    ///
    /// The caller must ensure that `ptr` is valid for a read and write of `size` bytes, remains
    /// live for the operation, and is appropriately synchronized. The pointer must be aligned
    /// for `size`; unaligned pointers return `MemoryAccessError::UnalignedAccess`. The pointed-to
    /// storage must not require a heap-owner write barrier.
    #[allow(clippy::too_many_arguments)]
    pub unsafe fn compare_exchange_atomic_ptr(
        &mut self,
        ptr: NonNull<u8>,
        expected: u64,
        new: u64,
        size: usize,
        success: dotnet_utils::sync::Ordering,
        failure: dotnet_utils::sync::Ordering,
    ) -> Result<u64, CompareExchangeError> {
        let ptr = ptr.as_ptr();
        if !dotnet_utils::is_ptr_aligned_to_field(ptr as *const u8, size) {
            return Err(CompareExchangeError::Bounds(
                MemoryAccessError::UnalignedAccess(ptr as usize),
            ));
        }
        // SAFETY: F10.RawMemoryAccessValid — The caller guarantees valid synchronized storage, and the preceding check
        // proves that `ptr` is aligned for `size`.
        unsafe {
            StandardAtomicAccess::compare_exchange_atomic_sized(
                ptr, size, expected, new, success, failure,
            )
        }
        .map_err(CompareExchangeError::Mismatch)
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
        if let Some(owner) = owner {
            let result = owner.with_data_mut(gc, |data| {
                let base = data.as_mut_ptr();
                let len = data.len();
                // SAFETY: F10.RawMemoryAccessValid — `base` is the start of the object's backing storage.
                // Bounds are checked immediately after by `check_bounds_internal`.
                let ptr = unsafe { base.add(offset.as_usize()) };

                self.check_bounds_internal(ptr, base, len, size)?;

                if !dotnet_utils::is_ptr_aligned_to_field(ptr as *const u8, size) {
                    return Err(MemoryAccessError::UnalignedAccess(ptr as usize));
                }

                // SAFETY: F4.WidthAligned — The preceding checks prove `ptr` is in bounds and aligned for `size`.
                Ok(unsafe {
                    StandardAtomicAccess::exchange_atomic_sized(ptr, size, value, ordering)
                })
            });

            if result.is_ok() {
                Self::backward_barrier_for_heap_atomic_write(gc, Some(owner));
            }

            result
        } else {
            // SAFETY: F10.RawMemoryAccessValid — The ownerless atomic API contract requires `offset` to
            // be a valid synchronized unmanaged address for this operation.
            let ptr = unsafe { unmanaged_ptr_from_addr(offset.as_usize()) };
            if ptr.is_null() {
                return Err(MemoryAccessError::NullPointer(
                    "NullReferenceException: exchange_atomic to unmanaged null pointer",
                ));
            }
            if !dotnet_utils::is_ptr_aligned_to_field(ptr as *const u8, size) {
                return Err(MemoryAccessError::UnalignedAccess(ptr as usize));
            }
            // SAFETY: F4.WidthAligned — The caller guarantees validity; preceding checks prove `ptr` is non-null and aligned.
            Ok(unsafe { StandardAtomicAccess::exchange_atomic_sized(ptr, size, value, ordering) })
        }
    }

    /// Atomically exchanges a value through a provenance-carrying pointer.
    ///
    /// # Safety
    ///
    /// The caller must ensure that `ptr` is valid for a read and write of `size` bytes, remains
    /// live for the operation, and is appropriately synchronized. The pointer must be aligned
    /// for `size`; unaligned pointers return `MemoryAccessError::UnalignedAccess`. The pointed-to
    /// storage must not require a heap-owner write barrier.
    pub unsafe fn exchange_atomic_ptr(
        &mut self,
        ptr: NonNull<u8>,
        value: u64,
        size: usize,
        ordering: dotnet_utils::sync::Ordering,
    ) -> Result<u64, MemoryAccessError> {
        let ptr = ptr.as_ptr();
        if !dotnet_utils::is_ptr_aligned_to_field(ptr as *const u8, size) {
            return Err(MemoryAccessError::UnalignedAccess(ptr as usize));
        }
        // SAFETY: F10.RawMemoryAccessValid — The caller guarantees valid synchronized storage, and the preceding check
        // proves that `ptr` is aligned for `size`.
        Ok(unsafe { StandardAtomicAccess::exchange_atomic_sized(ptr, size, value, ordering) })
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
        if let Some(owner) = owner {
            owner.with_data_mut(gc, |data| {
                let base = data.as_mut_ptr();
                let len = data.len();
                // SAFETY: F10.RawMemoryAccessValid — `base` is the start of the object's backing storage.
                // Bounds are checked immediately after by `check_bounds_internal`.
                let ptr = unsafe { base.add(offset.as_usize()) };

                self.check_bounds_internal(ptr, base, len, size)?;

                if !dotnet_utils::is_ptr_aligned_to_field(ptr as *const u8, size) {
                    return Err(MemoryAccessError::UnalignedAccess(ptr as usize));
                }

                Ok(
                    // SAFETY: F4.WidthAligned — The preceding checks prove `ptr` is in bounds and aligned for `size`.
                    unsafe {
                        StandardAtomicAccess::exchange_add_atomic_sized(ptr, size, value, ordering)
                    },
                )
            })
        } else {
            // SAFETY: F10.RawMemoryAccessValid — The ownerless atomic API contract requires `offset` to
            // be a valid synchronized unmanaged address for this operation.
            let ptr = unsafe { unmanaged_ptr_from_addr(offset.as_usize()) };
            if ptr.is_null() {
                return Err(MemoryAccessError::NullPointer(
                    "NullReferenceException: exchange_add_atomic to unmanaged null pointer",
                ));
            }
            if !dotnet_utils::is_ptr_aligned_to_field(ptr as *const u8, size) {
                return Err(MemoryAccessError::UnalignedAccess(ptr as usize));
            }
            // SAFETY: F4.WidthAligned — The caller guarantees validity; preceding checks prove `ptr` is non-null and aligned.
            Ok(unsafe {
                StandardAtomicAccess::exchange_add_atomic_sized(ptr, size, value, ordering)
            })
        }
    }

    /// Atomically adds a value through a provenance-carrying pointer.
    ///
    /// # Safety
    ///
    /// The caller must ensure that `ptr` is valid for a read and write of `size` bytes, remains
    /// live for the operation, and is appropriately synchronized. The pointer must be aligned
    /// for `size`; unaligned pointers return `MemoryAccessError::UnalignedAccess`. The pointed-to
    /// storage must not require a heap-owner write barrier.
    pub unsafe fn exchange_add_atomic_ptr(
        &mut self,
        ptr: NonNull<u8>,
        value: u64,
        size: usize,
        ordering: dotnet_utils::sync::Ordering,
    ) -> Result<u64, MemoryAccessError> {
        let ptr = ptr.as_ptr();
        if !dotnet_utils::is_ptr_aligned_to_field(ptr as *const u8, size) {
            return Err(MemoryAccessError::UnalignedAccess(ptr as usize));
        }
        // SAFETY: F10.RawMemoryAccessValid — The caller guarantees valid synchronized storage, and the preceding check
        // proves that `ptr` is aligned for `size`.
        Ok(unsafe { StandardAtomicAccess::exchange_add_atomic_sized(ptr, size, value, ordering) })
    }

    /// Loads a value atomically when aligned, with synchronized memcpy for misaligned storage.
    ///
    /// # Safety
    /// Caller must ensure the offset and size are valid for the owner object.
    /// A misaligned unmanaged access must not race with another access to the same bytes.
    pub unsafe fn load_atomic(
        &self,
        owner: Option<MemoryOwner<'gc>>,
        offset: ByteOffset,
        size: usize,
        ordering: dotnet_utils::sync::Ordering,
    ) -> Result<u64, MemoryAccessError> {
        if let Some(owner) = owner {
            owner.with_data(|data| {
                let base = data.as_ptr();
                let len = data.len();
                // SAFETY: F10.RawMemoryAccessValid — `base` is the start of the object's backing storage
                // (immutable borrow).  Bounds are checked immediately after.
                let ptr = unsafe { base.add(offset.as_usize()) };
                self.check_bounds_internal(ptr, base, len, size)?;
                // SAFETY: F10.RawMemoryAccessValid — Bounds and the storage guard provide valid synchronized access.
                Ok(unsafe { load_atomic_with_unaligned_fallback(ptr, size, ordering) })
            })
        } else {
            // SAFETY: F10.RawMemoryAccessValid — The ownerless atomic API contract requires `offset` to
            // be a valid synchronized unmanaged address for this load.
            let ptr = unsafe { unmanaged_ptr_from_addr(offset.as_usize()) };
            if ptr.is_null() {
                return Err(MemoryAccessError::NullPointer(
                    "NullReferenceException: load_atomic from unmanaged null pointer",
                ));
            }
            // SAFETY: F10.RawMemoryAccessValid — The caller guarantees valid synchronized unmanaged access.
            Ok(unsafe { load_atomic_with_unaligned_fallback(ptr, size, ordering) })
        }
    }

    /// Loads through a provenance-carrying pointer, using the unaligned fallback when needed.
    ///
    /// # Safety
    ///
    /// The caller must ensure that `ptr` is valid for a read of `size` bytes, remains live for
    /// the operation, and is appropriately synchronized. As with `load_atomic`, an unaligned
    /// access must not race with another access to the same bytes.
    pub unsafe fn load_atomic_ptr(
        &self,
        ptr: NonNull<u8>,
        size: usize,
        ordering: dotnet_utils::sync::Ordering,
    ) -> Result<u64, MemoryAccessError> {
        let ptr = ptr.as_ptr();
        // SAFETY: F10.RawMemoryAccessValid — The caller guarantees valid synchronized storage; the helper retains the
        // aligned atomic operation and unaligned fallback used by `load_atomic`.
        Ok(unsafe { load_atomic_with_unaligned_fallback(ptr, size, ordering) })
    }

    /// Stores a value atomically when aligned, with synchronized memcpy for misaligned storage.
    ///
    /// # Safety
    /// Caller must ensure the offset and size are valid for the owner object.
    /// A misaligned unmanaged access must not race with another access to the same bytes.
    pub unsafe fn store_atomic(
        &mut self,
        gc: GCHandle<'gc>,
        owner: Option<MemoryOwner<'gc>>,
        offset: ByteOffset,
        value: u64,
        size: usize,
        ordering: dotnet_utils::sync::Ordering,
    ) -> Result<(), MemoryAccessError> {
        if let Some(owner) = owner {
            let result = owner.with_data_mut(gc, |data| {
                let base = data.as_mut_ptr();
                let len = data.len();
                // SAFETY: F10.RawMemoryAccessValid — `base` is the start of the object's backing storage.
                // Bounds are checked immediately after by `check_bounds_internal`.
                let ptr = unsafe { base.add(offset.as_usize()) };
                self.check_bounds_internal(ptr, base, len, size)?;
                // SAFETY: F10.RawMemoryAccessValid — Bounds and the storage guard provide valid synchronized access.
                unsafe { store_atomic_with_unaligned_fallback(ptr, size, value, ordering) };
                Ok(())
            });

            if result.is_ok() {
                Self::backward_barrier_for_heap_atomic_write(gc, Some(owner));
            }

            result
        } else {
            // SAFETY: F10.RawMemoryAccessValid — The ownerless atomic API contract requires `offset` to
            // be a valid synchronized unmanaged address for this store.
            let ptr = unsafe { unmanaged_ptr_from_addr(offset.as_usize()) };
            if ptr.is_null() {
                return Err(MemoryAccessError::NullPointer(
                    "NullReferenceException: store_atomic to unmanaged null pointer",
                ));
            }
            // SAFETY: F10.RawMemoryAccessValid — The caller guarantees valid synchronized unmanaged access.
            unsafe { store_atomic_with_unaligned_fallback(ptr, size, value, ordering) };
            Ok(())
        }
    }

    /// Stores through a provenance-carrying pointer, using the unaligned fallback when needed.
    ///
    /// # Safety
    ///
    /// The caller must ensure that `ptr` is valid for a write of `size` bytes, remains live for
    /// the operation, and is appropriately synchronized. As with `store_atomic`, an unaligned
    /// access must not race with another access to the same bytes. The pointed-to storage must
    /// not require a heap-owner write barrier.
    pub unsafe fn store_atomic_ptr(
        &mut self,
        ptr: NonNull<u8>,
        value: u64,
        size: usize,
        ordering: dotnet_utils::sync::Ordering,
    ) -> Result<(), MemoryAccessError> {
        let ptr = ptr.as_ptr();
        // SAFETY: F10.RawMemoryAccessValid — The caller guarantees valid synchronized storage; the helper retains the
        // aligned atomic operation and unaligned fallback used by `store_atomic`.
        unsafe { store_atomic_with_unaligned_fallback(ptr, size, value, ordering) };
        Ok(())
    }

    pub fn get_storage_base(&self, owner: ObjectRef<'gc>) -> (*const u8, usize) {
        if let Some(h) = owner.0 {
            let obj = h.borrow();
            match &obj.storage {
                HeapStorage::Obj(_) | HeapStorage::Boxed(_) | HeapStorage::Vec(_) => {
                    // SAFETY: F10.RawMemoryAccessValid — `obj` is a live, borrow-locked `ObjectInner`.
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

    fn write_heap_value_with_barrier(
        &mut self,
        gc: GCHandle<'gc>,
        owner: MemoryOwner<'gc>,
        offset: ByteOffset,
        value: StackValue<'gc>,
        layout: &LayoutManager,
        dest_layout: Option<LayoutManager>,
    ) -> Result<(), MemoryAccessError> {
        // The panic-flush guard drains WB_LOCAL_BUF only during unwinding;
        // normal-path writes batch until threshold or safepoint flush.
        let _flush_guard = WriteBarrierPanicFlushGuard;
        WB_LOCAL_BUF.with(|buf| {
            let mut b = buf.borrow_mut();
            let mut recorder = WriteBarrierRecorder::new(owner.owner_id(), &mut b);
            owner.with_data_mut(gc, |data| {
                let base = data.as_mut_ptr();
                let len = data.len();
                let ptr = base.wrapping_add(offset.as_usize());
                validate_atomic_access(ptr as *const u8, false);

                self.check_bounds_internal(ptr, base, len, layout.size().as_usize())?;
                self.check_integrity_internal_with_layout(ptr, dest_layout, base, layout)?;

                // SAFETY: F10.RawMemoryAccessValid — bounds/integrity are validated above; owner/data originate from
                // a live heap object and with_data_mut guarantees exclusive access.
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

        self.write_heap_value_with_barrier(gc, owner, offset, value, layout, dest_layout)
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
                "NullReferenceException: writing to unmanaged null pointer",
            ));
        }
        validate_atomic_access(ptr as *const u8, false);
        // SAFETY: F10.RawMemoryAccessValid — This unsafe function's documented precondition supplies a non-null writable
        // range for `ptr`; the callee performs the layout-specific write within that range.
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
        // SAFETY: F10.RawMemoryAccessValid — The caller guarantees `ptr` points to `ObjectRef::SIZE` readable bytes in a
        // live object. `buf` is a distinct stack allocation of exactly that size.
        unsafe { ptr::copy_nonoverlapping(ptr, buf.as_mut_ptr(), ObjectRef::SIZE) };
        // SAFETY: F6.NoEscapeAcrossArena — `buf` now holds one complete branded ObjectRef serialization copied above.
        let r = unsafe { ObjectRef::read_branded(&buf, &gc) };
        self.record_objref_cross_arena_with_recorder(r, owner_tid, recorder);
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
        if recorder.arena_id == ArenaId::INVALID {
            return;
        }

        // SAFETY: F10.RawMemoryAccessValid — The caller guarantees `ptr` points to `ManagedPtr::SIZE` readable bytes in a
        // live object; the resulting slice uses exactly that serialization width.
        let bytes = unsafe { std::slice::from_raw_parts(ptr, ManagedPtr::SIZE) };
        // SAFETY: F10.RawMemoryAccessValid — `bytes` is the complete live ManagedPtr representation validated above.
        let info = unsafe { ManagedPtr::read_resolved_branded(bytes, &gc, &NoManagedPtrResolver) }
            .expect("record_managedptr_at_ptr: failed to read ManagedPtr");
        match &info.origin {
            PointerOrigin::Heap(r) => {
                self.record_objref_cross_arena_with_recorder(*r, owner_tid, recorder)
            }
            PointerOrigin::CrossArenaObjectRef(p, target_tid) if *target_tid != owner_tid => {
                recorder.buffer.push((*target_tid, p.as_ptr().addr()));
                maybe_flush_write_barrier_entries(recorder.buffer);
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
        // The panic-flush guard drains WB_LOCAL_BUF only during unwinding;
        // normal-path writes batch until threshold or safepoint flush.
        let _flush_guard = WriteBarrierPanicFlushGuard;
        WB_LOCAL_BUF.with(|buf| {
            let mut b = buf.borrow_mut();
            let mut _recorder = WriteBarrierRecorder::new(
                // INVALID sentinel: unowned write; the recorder skips cross-arena tracking.
                owner.map(|o| o.owner_id()).unwrap_or(ArenaId::INVALID),
                &mut b,
            );
            // SAFETY: F10.RawMemoryAccessValid — `ptr` is a valid, non-null pointer that was verified by
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
        #[expect(
            clippy::multiple_unsafe_ops_per_block,
            reason = "the layout-dispatched writes share the caller-validated raw storage range"
        )]
        // SAFETY: F10.RawMemoryAccessValid — `ptr` is non-null (checked immediately below) and has been validated by the
        // caller (`write_unaligned` / `write_to_heap` paths perform bounds and integrity checks).
        // All layout-dispatched unaligned writes stay within that valid range.
        unsafe {
            if ptr.is_null() {
                return Err(MemoryAccessError::NullPointer(
                    "RawMemoryAccess::write_value_internal called with null pointer!",
                ));
            }

            Self::backward_barrier_for_heap_ref_write(gc, owner, &value, layout);

            match layout {
                LayoutManager::Scalar(s) => match s {
                    Scalar::Int8 => {
                        let v = extract_int(value)? as i8;
                        (ptr as *mut i8).write_unaligned(v);
                    }
                    Scalar::UInt8 => {
                        let v = extract_int(value)? as u8;
                        ptr.write_unaligned(v);
                    }
                    Scalar::Int16 => {
                        let v = extract_int(value)? as i16;
                        (ptr as *mut i16).write_unaligned(v);
                    }
                    Scalar::UInt16 => {
                        let v = extract_int(value)? as u16;
                        (ptr as *mut u16).write_unaligned(v);
                    }
                    Scalar::Int32 => {
                        let v = extract_int(value)?;
                        (ptr as *mut i32).write_unaligned(v);
                    }
                    Scalar::Int64 => {
                        let v = extract_long(value)?;
                        (ptr as *mut i64).write_unaligned(v);
                    }
                    Scalar::NativeInt => {
                        let v = extract_native_int(value)?;
                        (ptr as *mut isize).write_unaligned(v);
                    }
                    Scalar::Float32 => {
                        let v = extract_float(value)? as f32;
                        (ptr as *mut f32).write_unaligned(v);
                    }
                    Scalar::Float64 => {
                        let v = extract_float(value)?;
                        (ptr as *mut f64).write_unaligned(v);
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
                            #[cfg(feature = "bench-instrumentation")]
                            let layout_scan_start = Instant::now();
                            self.record_refs_recursive_with_recorder(gc, ptr, layout, _recorder);
                            #[cfg(feature = "bench-instrumentation")]
                            dotnet_metrics::record_active_layout_scan_timing(
                                "record_refs_recursive_with_recorder",
                                layout_scan_start.elapsed(),
                            );
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

    fn backward_barrier_for_heap_atomic_write(gc: GCHandle<'gc>, owner: Option<MemoryOwner<'gc>>) {
        let Some(MemoryOwner::Local(parent)) = owner else {
            return;
        };
        let Some(parent_gc) = parent.0 else {
            return;
        };

        let _ = gc_arena::Gc::write(gc.mutation(), parent_gc);
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
                field_layout_contains_heap_refs(flm) && matches!(value, StackValue::ValueType(_))
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
    /// Recursively records GC references reachable from `layout` rooted at `ptr`.
    ///
    /// # Safety
    ///
    /// The caller must ensure that `ptr` points into live object memory and is valid to read as
    /// initialized `FieldStorage` bytes for `layout` (including all recursively visited subfields).
    /// No concurrent mutation may occur while this scan is in progress; it is only valid during
    /// stop-the-world (STW) execution where writers are excluded.
    unsafe fn record_refs_recursive_with_recorder(
        &self,
        gc: GCHandle<'gc>,
        ptr: *const u8,
        layout: &LayoutManager,
        recorder: &mut WriteBarrierRecorder<'_, 'gc>,
    ) {
        if recorder.arena_id == ArenaId::INVALID {
            return;
        }

        if !layout.is_or_contains_refs() {
            return;
        }
        let owner_tid = recorder.arena_id;
        match layout {
            // SAFETY: F10.RawMemoryAccessValid — For each scalar variant, the caller guarantees `ptr`
            // points to a valid, readable slot of the appropriate size within
            // a live object's backing storage.
            LayoutManager::Scalar(Scalar::ObjectRef) => unsafe {
                self.record_objref_at_ptr_with_recorder(gc, ptr, owner_tid, recorder);
            },
            LayoutManager::Scalar(Scalar::ManagedPtr) => {
                // SAFETY: F10.RawMemoryAccessValid — `ptr` names the caller-validated live scalar slot and this branch uses
                // the matching ManagedPtr layout.
                unsafe { self.record_managedptr_at_ptr_with_recorder(gc, ptr, owner_tid, recorder) }
            }
            LayoutManager::Field(flm) => {
                // Use GcDesc for fast ObjectRef recording
                let ptr_size = ObjectRef::SIZE;
                flm.gc_desc.for_each_word_index(|word_index| {
                    let offset = word_index * ptr_size;
                    // SAFETY: F10.RawMemoryAccessValid — `offset` is a word-aligned byte offset from the layout's GC bitmap,
                    // so it lies within this live field-storage allocation.
                    let child = unsafe { ptr.add(offset) };
                    // SAFETY: F10.RawMemoryAccessValid — `child` is the validated ObjectRef slot derived above.
                    unsafe {
                        self.record_objref_at_ptr_with_recorder(gc, child, owner_tid, recorder)
                    };
                });
                for offset in &flm.gc_desc.unaligned_offsets {
                    // SAFETY: F10.RawMemoryAccessValid — Layout construction validates every unaligned offset as lying
                    // within this live field-storage allocation.
                    let child = unsafe { ptr.add(*offset) };
                    // SAFETY: F10.RawMemoryAccessValid — `child` is the validated ObjectRef slot derived above.
                    unsafe {
                        self.record_objref_at_ptr_with_recorder(gc, child, owner_tid, recorder)
                    };
                }
                // Use visit_managed_ptrs for recursive ManagedPtr recording
                if flm.has_ref_fields {
                    // SAFETY: F10.RawMemoryAccessValid — `visit_managed_ptrs` yields offsets that are within
                    // the field struct's backing storage; `ptr.add(offset)` is valid.
                    flm.visit_managed_ptrs(ByteOffset::new(0), &mut |offset| {
                        // SAFETY: F10.RawMemoryAccessValid — The layout visitor yields offsets within this live field range.
                        let child = unsafe { ptr.add(offset.as_usize()) };
                        // SAFETY: F10.RawMemoryAccessValid — `child` is the validated ManagedPtr slot derived above.
                        unsafe {
                            self.record_managedptr_at_ptr_with_recorder(
                                gc, child, owner_tid, recorder,
                            )
                        };
                    });
                }
            }
            LayoutManager::Array(arr) if arr.element_layout.is_or_contains_refs() => {
                let elem_size = arr.element_layout.size().as_usize();
                for i in 0..arr.length {
                    // SAFETY: F10.RawMemoryAccessValid — `i < arr.length`, and `arr.length * elem_size` is the live array's
                    // total allocation size, so this element offset is in bounds.
                    let child = unsafe { ptr.add(i * elem_size) };
                    // SAFETY: F10.RawMemoryAccessValid — `child` is the validated element base derived above.
                    unsafe {
                        self.record_refs_recursive_with_recorder(
                            gc,
                            child,
                            &arr.element_layout,
                            recorder,
                        )
                    };
                }
            }
            _ => {}
        }
    }

    #[cfg(feature = "multithreading")]
    /// Records GC references that may overlap the byte range [`range_start`, `range_end`) within
    /// `layout` rooted at `ptr`.
    ///
    /// # Safety
    ///
    /// The caller must ensure `ptr` is the base of a live allocation for `layout`, and that the
    /// scanned byte range refers only to initialized field-storage bytes that may be interpreted as
    /// GC-reference-containing fields. No concurrent mutation may race this scan; callers must
    /// provide stop-the-world (STW) exclusion while recording.
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
                // SAFETY: F10.RawMemoryAccessValid — `ptr` is the base of this scalar slot (passed in from
                // the parent call), which is valid for the scalar's size within
                // the enclosing object's backing storage.
                unsafe { self.record_refs_recursive_with_recorder(gc, ptr, layout, recorder) };
            }
            LayoutManager::Field(flm) => {
                for field in flm.fields.values() {
                    let f_start = field.position.as_usize();
                    let f_end = f_start + field.layout.size().as_usize();
                    if f_start < range_end && f_end > range_start {
                        // SAFETY: F10.RawMemoryAccessValid — `f_start` is a layout-provided field offset within the
                        // caller-validated live struct storage.
                        let child = unsafe { ptr.add(f_start) };
                        // SAFETY: F10.RawMemoryAccessValid — `child` is the validated field base derived above.
                        unsafe {
                            self.record_refs_in_range_with_recorder(
                                gc,
                                child,
                                &field.layout,
                                range_start.saturating_sub(f_start),
                                range_end.saturating_sub(f_start),
                                recorder,
                            )
                        };
                    }
                }
            }
            LayoutManager::Array(alm) => {
                let elem_size = alm.element_layout.size().as_usize();
                let Some(start_idx) = range_start.checked_div(elem_size) else {
                    return;
                };
                let Some(end_quot) = range_end.checked_div(elem_size) else {
                    return;
                };
                let end_idx = end_quot + usize::from(!range_end.is_multiple_of(elem_size));

                let start_idx = start_idx.min(alm.length);
                let end_idx = end_idx.min(alm.length);

                for i in start_idx..end_idx {
                    let f_start = i * elem_size;
                    // SAFETY: F10.RawMemoryAccessValid — `f_start = i * elem_size` with `i < alm.length`, so it lies in the
                    // caller-validated live array storage.
                    let child = unsafe { ptr.add(f_start) };
                    // SAFETY: F10.RawMemoryAccessValid — `child` is the validated element base derived above.
                    unsafe {
                        self.record_refs_in_range_with_recorder(
                            gc,
                            child,
                            &alm.element_layout,
                            range_start.saturating_sub(f_start),
                            range_end.saturating_sub(f_start),
                            recorder,
                        )
                    };
                }
            }
            _ => {}
        }
    }

    /// Reads a value of the provided layout from `ptr` without additional bounds checks.
    ///
    /// # Safety
    ///
    /// The caller must guarantee `ptr` is non-null, properly aligned/valid for reads of
    /// `layout.size()` bytes, and points to initialized storage for the given layout.
    pub unsafe fn read_value_internal(
        &self,
        gc: GCHandle<'gc>,
        ptr: *const u8,
        _owner: Option<MemoryOwner<'gc>>,
        layout: &LayoutManager,
        type_desc: Option<TypeDescription>,
    ) -> Result<StackValue<'gc>, MemoryAccessError> {
        #[expect(
            clippy::multiple_unsafe_ops_per_block,
            reason = "the layout-dispatched reads share the caller-validated raw storage range"
        )]
        // SAFETY: F10.RawMemoryAccessValid — The caller ensures `ptr` is valid for reads and within bounds, as verified by
        // `read_unaligned` before reaching this layout-dispatched operation.
        unsafe {
            if ptr.is_null() {
                return Err(MemoryAccessError::NullPointer(
                    "RawMemoryAccess::read_value_internal called with null pointer!",
                ));
            }

            Ok(match layout {
                LayoutManager::Scalar(s) => match s {
                    Scalar::Int8 => StackValue::Int32((ptr as *const i8).read_unaligned() as i32),
                    Scalar::UInt8 => StackValue::Int32(ptr.read_unaligned() as i32),
                    Scalar::Int16 => StackValue::Int32((ptr as *const i16).read_unaligned() as i32),
                    Scalar::UInt16 => {
                        StackValue::Int32((ptr as *const u16).read_unaligned() as i32)
                    }
                    Scalar::Int32 => StackValue::Int32((ptr as *const i32).read_unaligned()),
                    Scalar::Int64 => StackValue::Int64((ptr as *const i64).read_unaligned()),
                    Scalar::NativeInt => {
                        StackValue::NativeInt((ptr as *const isize).read_unaligned())
                    }
                    Scalar::Float32 => {
                        StackValue::NativeFloat((ptr as *const f32).read_unaligned() as f64)
                    }
                    Scalar::Float64 => {
                        StackValue::NativeFloat((ptr as *const f64).read_unaligned())
                    }
                    Scalar::ObjectRef => {
                        let mut buf = [0u8; ObjectRef::SIZE];
                        ptr::copy_nonoverlapping(ptr, buf.as_mut_ptr(), ObjectRef::SIZE);
                        StackValue::ObjectRef(ObjectRef::read_branded(&buf, &gc))
                    }
                    Scalar::ManagedPtr => {
                        let info = ManagedPtr::read_resolved_branded(
                            std::slice::from_raw_parts(ptr, ManagedPtr::SIZE),
                            &gc,
                            &NoManagedPtrResolver,
                        )
                        .map_err(|e| {
                            MemoryAccessError::TypeMismatch(
                                format!("ManagedPtr read failed: {:?}", e).into(),
                            )
                        })?;

                        let actual_desc = type_desc
                            .unwrap_or(TypeDescription::new(ResolutionS::NULL, std::mem::zeroed()));

                        let m = ManagedPtr::from_info_full(info, actual_desc, false);
                        StackValue::ManagedPtr(m.into())
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
                        "Array read not supported".into(),
                    ));
                }
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        WB_LOCAL_BUF, WriteBarrierPanicFlushGuard, check_bounds, field_layout_contains_heap_refs,
        load_atomic_with_unaligned_fallback, store_atomic_with_unaligned_fallback,
    };
    use dotnet_types::error::{CompareExchangeError, MemoryAccessError};
    use dotnet_utils::ArenaId;
    use dotnet_value::layout::{FieldLayoutManager, GcDesc};

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
    fn bytes_ptr_access_preserves_live_pointer_provenance() {
        let heap = crate::heap::HeapManager::new();
        let mut memory = super::RawMemoryAccess::new(&heap);
        let mut backing = [0u8; 4];
        let ptr = std::ptr::NonNull::new(backing.as_mut_ptr().wrapping_add(1))
            .expect("array-derived pointer is non-null");

        // SAFETY: F10.RawMemoryAccessValid — `ptr` points to the final three writable bytes of `backing`,
        // which are exclusively held by this test.
        unsafe {
            memory
                .write_bytes_ptr(ptr, &[0xA1, 0xB2, 0xC3])
                .expect("live pointer write succeeds");
        }
        assert_eq!(backing, [0, 0xA1, 0xB2, 0xC3]);

        let mut dest = [0u8; 3];
        // SAFETY: F10.RawMemoryAccessValid — `ptr` points to three initialized readable bytes of `backing`,
        // and `dest` is a separate writable array.
        unsafe {
            memory
                .read_bytes_ptr(ptr, &mut dest)
                .expect("live pointer read succeeds");
        }
        assert_eq!(dest, [0xA1, 0xB2, 0xC3]);
    }

    #[test]
    fn atomic_ptr_access_preserves_live_pointer_provenance_and_contracts() {
        let heap = crate::heap::HeapManager::new();
        let mut memory = super::RawMemoryAccess::new(&heap);
        let mut backing = [0u64; 2];
        let ptr = std::ptr::NonNull::new(backing.as_mut_ptr().cast::<u8>())
            .expect("array-derived pointer is non-null");

        // SAFETY: F4.WidthAligned — `ptr` is derived from the live, eight-byte-aligned first element of `backing`.
        // This test exclusively owns the backing storage and synchronizes every access with SeqCst.
        unsafe {
            memory
                .store_atomic_ptr(ptr, 5, 8, dotnet_utils::sync::Ordering::SeqCst)
                .expect("aligned pointer store succeeds");
        }
        // SAFETY: F10.RawMemoryAccessValid — `ptr` points to initialized, live backing storage exclusively accessed by this test.
        let loaded = unsafe {
            memory
                .load_atomic_ptr(ptr, 8, dotnet_utils::sync::Ordering::SeqCst)
                .expect("aligned pointer load succeeds")
        };
        assert_eq!(loaded, 5);
        // SAFETY: F4.WidthAligned — `ptr` is a live, aligned, exclusively accessed eight-byte storage location.
        let previous = unsafe {
            memory
                .compare_exchange_atomic_ptr(
                    ptr,
                    5,
                    9,
                    8,
                    dotnet_utils::sync::Ordering::SeqCst,
                    dotnet_utils::sync::Ordering::SeqCst,
                )
                .expect("aligned pointer compare-exchange succeeds")
        };
        assert_eq!(previous, 5);
        // SAFETY: F4.WidthAligned — `ptr` is a live, aligned, exclusively accessed eight-byte storage location.
        let previous = unsafe {
            memory
                .exchange_atomic_ptr(ptr, 12, 8, dotnet_utils::sync::Ordering::SeqCst)
                .expect("aligned pointer exchange succeeds")
        };
        assert_eq!(previous, 9);
        // SAFETY: F4.WidthAligned — `ptr` is a live, aligned, exclusively accessed eight-byte storage location.
        let previous = unsafe {
            memory
                .exchange_add_atomic_ptr(ptr, 7, 8, dotnet_utils::sync::Ordering::SeqCst)
                .expect("aligned pointer exchange-add succeeds")
        };
        assert_eq!(previous, 12);
        // SAFETY: F10.RawMemoryAccessValid — `ptr` points to initialized, live backing storage exclusively accessed by this test.
        let loaded = unsafe {
            memory
                .load_atomic_ptr(ptr, 8, dotnet_utils::sync::Ordering::SeqCst)
                .expect("aligned pointer load succeeds")
        };
        assert_eq!(loaded, 19);

        // SAFETY: F10.RawMemoryAccessValid — Adding one stays within the live backing allocation and deliberately produces
        // a pointer unaligned for four-byte atomic operations.
        let unaligned = std::ptr::NonNull::new(unsafe { ptr.as_ptr().add(1) })
            .expect("derived pointer is non-null");
        // SAFETY: F4.WidthAligned — `unaligned` has four writable bytes in live, exclusively owned backing storage.
        unsafe {
            memory
                .store_atomic_ptr(
                    unaligned,
                    0xDEAD_BEEF,
                    4,
                    dotnet_utils::sync::Ordering::SeqCst,
                )
                .expect("unaligned pointer store uses fallback");
        }
        // SAFETY: F4.WidthAligned — `unaligned` has four initialized readable bytes in exclusively owned backing storage.
        let loaded = unsafe {
            memory
                .load_atomic_ptr(unaligned, 4, dotnet_utils::sync::Ordering::SeqCst)
                .expect("unaligned pointer load uses fallback")
        };
        assert_eq!(loaded, 0xDEAD_BEEF);
        // SAFETY: F4.WidthAligned — The method checks alignment before dereferencing `unaligned`.
        let compare_exchange = unsafe {
            memory.compare_exchange_atomic_ptr(
                unaligned,
                0,
                1,
                4,
                dotnet_utils::sync::Ordering::SeqCst,
                dotnet_utils::sync::Ordering::SeqCst,
            )
        };
        assert!(matches!(
            compare_exchange,
            Err(CompareExchangeError::Bounds(
                MemoryAccessError::UnalignedAccess(_)
            ))
        ));
        // SAFETY: F4.WidthAligned — The method checks alignment before dereferencing `unaligned`.
        let exchange = unsafe {
            memory.exchange_atomic_ptr(unaligned, 1, 4, dotnet_utils::sync::Ordering::SeqCst)
        };
        assert!(matches!(
            exchange,
            Err(MemoryAccessError::UnalignedAccess(_))
        ));
        // SAFETY: F4.WidthAligned — The method checks alignment before dereferencing `unaligned`.
        let exchange_add = unsafe {
            memory.exchange_add_atomic_ptr(unaligned, 1, 4, dotnet_utils::sync::Ordering::SeqCst)
        };
        assert!(matches!(
            exchange_add,
            Err(MemoryAccessError::UnalignedAccess(_))
        ));
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
        // SAFETY: F10.RawMemoryAccessValid — `base.add(6)` stays within the 8-byte allocation.
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

    #[test]
    fn atomic_value_access_uses_unaligned_fallback() {
        let mut backing = [0u64; 2];
        // SAFETY: F10.RawMemoryAccessValid — Offset one remains within the live, eight-byte-aligned backing allocation.
        let ptr = unsafe { backing.as_mut_ptr().cast::<u8>().add(1) };

        // SAFETY: F10.RawMemoryAccessValid — `ptr` has four live bytes and this test provides exclusive access.
        unsafe {
            store_atomic_with_unaligned_fallback(
                ptr,
                4,
                0xDEAD_BEEF,
                dotnet_utils::sync::Ordering::SeqCst,
            );
        }
        // SAFETY: F10.RawMemoryAccessValid — `ptr` has four initialized live bytes and this test provides exclusive access.
        let loaded = unsafe {
            load_atomic_with_unaligned_fallback(ptr, 4, dotnet_utils::sync::Ordering::SeqCst)
        };
        assert_eq!(loaded, 0xDEAD_BEEF);
    }

    #[test]
    fn field_layout_with_object_ref_gc_desc_requires_heap_ref_barrier() {
        let mut gc_desc = GcDesc::default();
        gc_desc.set_offset(0);
        let layout = FieldLayoutManager {
            fields: Default::default(),
            total_size: 8,
            alignment: 8,
            gc_desc,
            has_ref_fields: false,
        };

        assert!(field_layout_contains_heap_refs(&layout));
    }

    #[test]
    fn field_layout_without_gc_desc_or_managed_ptrs_needs_no_heap_ref_barrier() {
        let layout = FieldLayoutManager {
            fields: Default::default(),
            total_size: 4,
            alignment: 4,
            gc_desc: GcDesc::default(),
            has_ref_fields: false,
        };

        assert!(!field_layout_contains_heap_refs(&layout));
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

    /// Verify that `WriteBarrierPanicFlushGuard::drop` drains `WB_LOCAL_BUF` even
    /// when the surrounding code panics mid-write.
    ///
    /// We manually seed the TLS buffer, then force a panic inside
    /// `catch_unwind` while the guard is live.  After unwinding the buffer
    /// must be empty — confirming that `Drop` ran and drained it.
    #[test]
    fn write_barrier_panic_flush_guard_drains_on_panic() {
        use std::panic::{self, AssertUnwindSafe};

        // Seed the TLS buffer with a dummy entry so there is something to drain.
        WB_LOCAL_BUF.with(|buf| {
            buf.borrow_mut().push((ArenaId::new(0), 0xDEAD_BEEF));
        });

        // Introduce a guard, then panic.  The guard's Drop must drain the buffer
        // even though control never reaches the end of the closure normally.
        let result = panic::catch_unwind(AssertUnwindSafe(|| {
            let _flush_guard = WriteBarrierPanicFlushGuard;
            panic!("intentional test panic to verify flush-guard drop");
        }));

        assert!(result.is_err(), "expected the panic to be caught");

        // Buffer must be empty: Drop fired and drained it.
        WB_LOCAL_BUF.with(|buf| {
            assert!(
                buf.borrow().is_empty(),
                "WB_LOCAL_BUF should be empty after WriteBarrierPanicFlushGuard dropped on unwind"
            );
        });
    }

    #[test]
    fn write_barrier_panic_flush_guard_does_not_flush_on_normal_drop() {
        WB_LOCAL_BUF.with(|buf| {
            let mut b = buf.borrow_mut();
            b.clear();
            b.push((ArenaId::new(0), 0xDEAD_BEEF));
        });

        {
            let _flush_guard = WriteBarrierPanicFlushGuard;
        }

        WB_LOCAL_BUF.with(|buf| {
            assert_eq!(buf.borrow().len(), 1);
            buf.borrow_mut().clear();
        });
    }

    #[cfg(feature = "multithreading")]
    use super::MemoryOwner;
    #[cfg(feature = "multithreading")]
    use dotnet_utils::gc::{ArenaHandle, GCHandle, ThreadSafeLock};
    #[cfg(feature = "multithreading")]
    use dotnet_value::{
        CLRString,
        object::{HeapStorage, ObjectInner, ObjectPtr},
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
        let lock = Box::new(ThreadSafeLock::new(ObjectInner::new(
            HeapStorage::Str(CLRString::from("cross-arena-lifetime")),
            arena_id,
        )));
        let raw: *const ThreadSafeLock<ObjectInner<'static>> = Box::leak(lock);
        // SAFETY: F10.RawMemoryAccessValid — `raw` comes from `Box::leak`, is non-null, and remains valid
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
            // SAFETY: F10.RawMemoryAccessValid — `arena_handle` was created by `Box::into_raw` above and
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
        // SAFETY: F10.RawMemoryAccessValid — `arena_handle` came from `Box::into_raw` above and remains
        // uniquely owned here; reconstructing it drops the allocation exactly once.
        unsafe {
            drop(Box::from_raw(arena_handle));
        }

        unregister_arena(arena_id);

        // Fix leak for Miri
        // SAFETY: F10.RawMemoryAccessValid — `raw` was obtained from `Box::leak` earlier in this test;
        // we reconstruct the `Box` to release the memory.  No other owner
        // exists at this point, so this is the unique drop.
        unsafe {
            let _ = Box::from_raw(raw as *mut ThreadSafeLock<ObjectInner<'static>>);
        }
    }
}

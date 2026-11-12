use crate::{
    ByteOffset,
    error::{CompareExchangeError, MemoryAccessError},
    stack::{
        context::VesContext,
        ops::{RawMemoryOps, VesInternals, VmRawMemoryOps, VmStaticsOps},
    },
    threading::ThreadManagerOps,
};
use dotnet_runtime_memory::{HeapWriteTarget, MemoryOwner, RawMemoryAccess};
use dotnet_types::TypeDescription;
use dotnet_utils::{BorrowScopeOps, GcScopeGuard, gc::GCHandle};
use dotnet_value::{
    StackValue,
    layout::{HasLayout, LayoutManager, Scalar},
    pointer::{
        FrameHomeSerial, PointerOrigin, RetainedPtr, frame_home_window, unmanaged_ptr_from_addr,
    },
};
use dotnet_vm_data::frame_alloc::FrameAllocationError;
use std::{mem::size_of, ops::Range, ptr::NonNull};

/// Validate only the extent of an inline storage access, preserving the requested
/// offset and size on failure. Access still requires its own storage and codec proof.
fn checked_window(
    offset: ByteOffset,
    size: usize,
    len: usize,
) -> Result<Range<usize>, MemoryAccessError> {
    let start = offset.as_usize();
    match start.checked_add(size) {
        Some(end) if end <= len => Ok(start..end),
        _ => Err(MemoryAccessError::BoundsCheck {
            offset: start,
            size,
            len,
        }),
    }
}

impl VesContext<'_, '_> {
    /// Native-int indirect access to a byref variable transfers the full encoding,
    /// including its origin, not merely the machine word holding its cached address.
    fn frame_home_access_layout<'l>(
        &self,
        serial: FrameHomeSerial,
        offset: ByteOffset,
        layout: &'l LayoutManager,
    ) -> &'l LayoutManager {
        if matches!(layout, LayoutManager::Scalar(Scalar::NativeInt))
            && self.is_byref_home(serial, offset.as_usize())
        {
            &LayoutManager::Scalar(Scalar::ManagedPtr)
        } else {
            layout
        }
    }
}

/// # Safety
///
/// `layout` must describe `value`, and the caller must keep the inline storage's
/// GC borrow scope active for the duration of the write.
unsafe fn write_to_inline_storage<'gc>(
    memory: &mut RawMemoryAccess<'_, 'gc>,
    gc: GCHandle<'gc>,
    data: &mut [u8],
    offset: ByteOffset,
    value: StackValue<'gc>,
    layout: &LayoutManager,
) -> Result<(), MemoryAccessError> {
    let range = checked_window(offset, layout.size().as_usize(), data.len())?;
    // SAFETY: F10.RawMemoryAccessValid — `offset + layout.size()` was checked against `data.len()` above, so `.add` stays in `data`.
    let ptr = unsafe { data.as_mut_ptr().add(range.start) };
    // SAFETY: F10.RawMemoryAccessValid — `ptr` has `layout.size()` writable bytes in `data` (checked above); this function's
    // `# Safety` covers the layout match and the GC borrow scope.
    unsafe { memory.write_to_unmanaged(gc, ptr, value, layout) }
}

/// # Safety
///
/// The bytes selected by `layout` must be initialized for that layout, and the
/// caller must keep the inline storage's GC borrow scope active for the read.
unsafe fn read_from_inline_storage<'gc>(
    memory: &RawMemoryAccess<'_, 'gc>,
    gc: GCHandle<'gc>,
    data: &[u8],
    offset: ByteOffset,
    layout: &LayoutManager,
    type_desc: Option<TypeDescription>,
) -> Result<StackValue<'gc>, MemoryAccessError> {
    let range = checked_window(offset, layout.size().as_usize(), data.len())?;
    // SAFETY: F10.RawMemoryAccessValid — `offset + layout.size()` was checked against `data.len()` above, so `.add` stays in `data`.
    let ptr = unsafe { data.as_ptr().add(range.start) };
    // SAFETY: F10.RawMemoryAccessValid — `ptr` has `layout.size()` readable bytes in `data` (checked above); this function's
    // `# Safety` covers initialization and the GC borrow scope.
    unsafe { memory.read_value_internal(gc, ptr, None, layout, type_desc) }
}

fn bounds_checked_slice_copy(
    dest: &mut [u8],
    src: &[u8],
    offset: ByteOffset,
) -> Result<(), MemoryAccessError> {
    let range = checked_window(offset, src.len(), dest.len())?;
    dest[range].copy_from_slice(src);
    Ok(())
}

fn bounds_checked_slice_read(
    src: &[u8],
    dest: &mut [u8],
    offset: ByteOffset,
) -> Result<(), MemoryAccessError> {
    let range = checked_window(offset, dest.len(), src.len())?;
    dest.copy_from_slice(&src[range]);
    Ok(())
}

impl<'a, 'gc> BorrowScopeOps for VesContext<'a, 'gc> {
    fn enter_gc_scope(&self) {
        self.local
            .active_borrows
            .set(self.local.active_borrows.get() + 1);
    }

    fn exit_gc_scope(&self) {
        let current = self.local.active_borrows.get();
        if current == 0 {
            panic!("Exiting borrow scope when none are active");
        }
        self.local.active_borrows.set(current - 1);
    }

    fn active_gc_scope_depth(&self) -> usize {
        self.local.active_borrows.get()
    }
}

impl<'a, 'gc> RawMemoryOps<'gc> for VesContext<'a, 'gc> {
    fn retained_range(
        &self,
        pointer: &RetainedPtr<'gc>,
        size: usize,
    ) -> Result<*mut u8, MemoryAccessError> {
        pointer.checked_range(self, size)
    }

    fn as_borrow_scope(&self) -> &dyn BorrowScopeOps {
        self
    }

    /// # Safety
    ///
    /// Owned origins validate the complete range internally. For Unmanaged, the caller
    /// must supply a live address valid for the access, with synchronization.
    /// The `layout` must match the expected type of `value`.
    #[inline]
    unsafe fn write_unaligned(
        &mut self,
        origin: PointerOrigin<'gc>,
        offset: ByteOffset,
        value: StackValue<'gc>,
        layout: &LayoutManager,
    ) -> Result<(), MemoryAccessError> {
        let _gc_scope = GcScopeGuard::enter(self, self.gc_ready_token());
        match origin {
            PointerOrigin::FrameHome(serial) => {
                let layout = self.frame_home_access_layout(serial, offset, layout);
                let ptr =
                    frame_home_window(self, serial, offset.as_usize(), layout.size().as_usize())?;
                let mut memory = RawMemoryAccess::new(&self.local.heap);
                // SAFETY: F10.FrameHomeChecked — `frame_home_window` found the live frame and checked
                // that the accessed bytes lie inside its home block.
                unsafe { memory.write_to_unmanaged(self.gc, ptr.as_ptr(), value, layout) }
            }
            PointerOrigin::Static(metadata) => {
                let storage = self.statics().get_for_metadata(&metadata);
                storage.storage.with_data_mut(|data| {
                    let mut memory = RawMemoryAccess::new(&self.local.heap);
                    // SAFETY: F10.RawMemoryAccessValid — the static storage stays borrowed for the closure, the helper bounds-checks
                    // `offset` against it, and this function's caller guarantees `layout` matches.
                    unsafe {
                        write_to_inline_storage(&mut memory, self.gc, data, offset, value, layout)
                    }
                })
            }
            PointerOrigin::Heap(owner) => {
                let heap = &self.local.heap;
                let mut memory = RawMemoryAccess::new(heap);
                // SAFETY: F10.RawMemoryAccessValid — The heap owner is passed as the object containing the location, as `RawMemoryAccess`
                // requires; the memory layer checks the complete access range.
                unsafe {
                    memory.write_to_heap(
                        self.gc,
                        HeapWriteTarget(MemoryOwner::Local(owner)),
                        offset,
                        value,
                        layout,
                    )
                }
            }
            PointerOrigin::Unmanaged => {
                let heap = &self.local.heap;
                let mut memory = RawMemoryAccess::new(heap);
                // SAFETY: F10.RawMemoryAccessValid — This is the explicit Unmanaged-origin branch; the raw
                // memory operation's caller owns the address validity contract.
                let ptr = unsafe { unmanaged_ptr_from_addr(offset.as_usize()) };
                // SAFETY: F10.RawMemoryAccessValid — Unmanaged origin: `offset` is an absolute address and this function's caller
                // guarantees it is valid for the access.
                unsafe { memory.write_to_unmanaged(self.gc, ptr, value, layout) }
            }
            #[cfg(feature = "multithreading")]
            PointerOrigin::CrossArenaObjectRef(origin) => {
                let heap = &self.local.heap;
                let mut memory = RawMemoryAccess::new(heap);
                // SAFETY: F10.RawMemoryAccessValid — The cross-arena owner is passed as the object containing the location, as `RawMemoryAccess`
                // requires; the memory layer checks the complete access range.
                unsafe {
                    memory.write_to_heap(
                        self.gc,
                        HeapWriteTarget(MemoryOwner::cross_arena(self.gc, origin)),
                        offset,
                        value,
                        layout,
                    )
                }
            }
            PointerOrigin::Transient(obj) => {
                obj.instance_storage.with_data_mut(|data: &mut [u8]| {
                    let mut memory = RawMemoryAccess::new(&self.local.heap);
                    // SAFETY: F10.RawMemoryAccessValid — the Transient owner's storage stays borrowed for the closure, the helper bounds-checks
                    // `offset` against it, and this function's caller guarantees `layout` matches.
                    unsafe {
                        write_to_inline_storage(&mut memory, self.gc, data, offset, value, layout)
                    }
                })
            }
        }
    }

    /// # Safety
    ///
    /// Owned origins validate the complete range internally. For Unmanaged, the caller
    /// must supply a live address valid for the access, with synchronization.
    /// The `layout` must match the expected type stored at the location.
    #[inline]
    unsafe fn read_unaligned(
        &self,
        origin: PointerOrigin<'gc>,
        offset: ByteOffset,
        layout: &LayoutManager,
        type_desc: Option<TypeDescription>,
    ) -> Result<StackValue<'gc>, MemoryAccessError> {
        let _gc_scope = GcScopeGuard::enter(self, self.gc_ready_token());
        match origin {
            PointerOrigin::FrameHome(serial) => {
                let layout = self.frame_home_access_layout(serial, offset, layout);
                let ptr =
                    frame_home_window(self, serial, offset.as_usize(), layout.size().as_usize())?;
                let memory = RawMemoryAccess::new(&self.local.heap);
                // SAFETY: F10.FrameHomeChecked — `frame_home_window` found the live frame and checked
                // that the accessed bytes lie inside its home block.
                unsafe {
                    memory.read_value_internal(self.gc, ptr.as_ptr(), None, layout, type_desc)
                }
            }
            PointerOrigin::Static(metadata) => {
                let storage = self.statics().get_for_metadata(&metadata);
                storage.storage.with_data(|data| {
                    let memory = RawMemoryAccess::new(&self.local.heap);
                    // SAFETY: F10.RawMemoryAccessValid — the static storage stays borrowed for the closure, the helper bounds-checks
                    // `offset` against it, and this function's caller guarantees `layout` matches.
                    unsafe {
                        read_from_inline_storage(&memory, self.gc, data, offset, layout, type_desc)
                    }
                })
            }
            PointerOrigin::Heap(owner) => {
                let heap = &self.local.heap;
                let memory = RawMemoryAccess::new(heap);
                // SAFETY: F10.RawMemoryAccessValid — The heap owner is passed as the object containing the location, as `RawMemoryAccess`
                // requires; the memory layer checks the complete access range.
                unsafe {
                    memory.read_unaligned(
                        self.gc,
                        Some(MemoryOwner::Local(owner)),
                        offset,
                        layout,
                        type_desc,
                    )
                }
            }
            PointerOrigin::Unmanaged => {
                let heap = &self.local.heap;
                let memory = RawMemoryAccess::new(heap);
                // SAFETY: F10.RawMemoryAccessValid — Unmanaged origin: `offset` is an absolute address and this function's caller
                // guarantees it is valid for the access.
                unsafe { memory.read_unaligned(self.gc, None, offset, layout, type_desc) }
            }
            #[cfg(feature = "multithreading")]
            PointerOrigin::CrossArenaObjectRef(origin) => {
                let heap = &self.local.heap;
                let memory = RawMemoryAccess::new(heap);
                // SAFETY: F10.RawMemoryAccessValid — The cross-arena owner is passed as the object containing the location, as `RawMemoryAccess`
                // requires; the memory layer checks the complete access range.
                unsafe {
                    memory.read_unaligned(
                        self.gc,
                        Some(MemoryOwner::cross_arena(self.gc, origin)),
                        offset,
                        layout,
                        type_desc,
                    )
                }
            }
            PointerOrigin::Transient(obj) => obj.instance_storage.with_data(|data: &[u8]| {
                let memory = RawMemoryAccess::new(&self.local.heap);
                // SAFETY: F10.RawMemoryAccessValid — the Transient owner's storage stays borrowed for the closure, the helper bounds-checks
                // `offset` against it, and this function's caller guarantees `layout` matches.
                unsafe {
                    read_from_inline_storage(&memory, self.gc, data, offset, layout, type_desc)
                }
            }),
        }
    }

    #[inline]
    unsafe fn write_bytes(
        &mut self,
        origin: PointerOrigin<'gc>,
        offset: ByteOffset,
        data: &[u8],
    ) -> Result<(), MemoryAccessError> {
        let _gc_scope = GcScopeGuard::enter(self, self.gc_ready_token());
        match origin {
            PointerOrigin::FrameHome(serial) => {
                let ptr = frame_home_window(self, serial, offset.as_usize(), data.len())?;
                let mut memory = RawMemoryAccess::new(&self.local.heap);
                // SAFETY: F10.FrameHomeChecked — `frame_home_window` found the live frame and checked
                // that the accessed bytes lie inside its home block.
                unsafe { memory.write_bytes_ptr(ptr, data) }
            }
            PointerOrigin::Static(metadata) => {
                let storage = self.statics().get_for_metadata(&metadata);
                storage
                    .storage
                    .with_data_mut(|obj_data| bounds_checked_slice_copy(obj_data, data, offset))
            }
            PointerOrigin::Heap(owner) => {
                let heap = &self.local.heap;
                let mut memory = RawMemoryAccess::new(heap);
                // SAFETY: F10.RawMemoryAccessValid — The heap owner is passed as the object containing the location, as `RawMemoryAccess`
                // requires; the memory layer checks the complete access range.
                unsafe {
                    memory.write_bytes(self.gc, Some(MemoryOwner::Local(owner)), offset, data)
                }
            }
            PointerOrigin::Unmanaged => {
                let heap = &self.local.heap;
                let mut memory = RawMemoryAccess::new(heap);
                // SAFETY: F10.RawMemoryAccessValid — Unmanaged origin: `offset` is an absolute address and this function's caller
                // guarantees it is valid for the access.
                unsafe { memory.write_bytes(self.gc, None, offset, data) }
            }
            #[cfg(feature = "multithreading")]
            PointerOrigin::CrossArenaObjectRef(origin) => {
                let heap = &self.local.heap;
                let mut memory = RawMemoryAccess::new(heap);
                // SAFETY: F10.RawMemoryAccessValid — The cross-arena owner is passed as the object containing the location, as `RawMemoryAccess`
                // requires; the memory layer checks the complete access range.
                unsafe {
                    memory.write_bytes(
                        self.gc,
                        Some(MemoryOwner::cross_arena(self.gc, origin)),
                        offset,
                        data,
                    )
                }
            }
            PointerOrigin::Transient(obj) => obj
                .instance_storage
                .with_data_mut(|obj_data| bounds_checked_slice_copy(obj_data, data, offset)),
        }
    }

    #[inline]
    unsafe fn read_bytes(
        &self,
        origin: PointerOrigin<'gc>,
        offset: ByteOffset,
        dest: &mut [u8],
    ) -> Result<(), MemoryAccessError> {
        let _gc_scope = GcScopeGuard::enter(self, self.gc_ready_token());
        match origin {
            PointerOrigin::FrameHome(serial) => {
                let ptr = frame_home_window(self, serial, offset.as_usize(), dest.len())?;
                let memory = RawMemoryAccess::new(&self.local.heap);
                // SAFETY: F10.FrameHomeChecked — `frame_home_window` found the live frame and checked
                // that the accessed bytes lie inside its home block.
                unsafe { memory.read_bytes_ptr(ptr, dest) }
            }
            PointerOrigin::Static(metadata) => {
                let storage = self.statics().get_for_metadata(&metadata);
                storage
                    .storage
                    .with_data(|obj_data| bounds_checked_slice_read(obj_data, dest, offset))
            }
            PointerOrigin::Heap(owner) => {
                let heap = &self.local.heap;
                let memory = RawMemoryAccess::new(heap);
                // SAFETY: F10.RawMemoryAccessValid — The heap owner is passed as the object containing the location, as `RawMemoryAccess`
                // requires; the memory layer checks the complete access range.
                unsafe { memory.read_bytes(Some(MemoryOwner::Local(owner)), offset, dest) }
            }
            PointerOrigin::Unmanaged => {
                let heap = &self.local.heap;
                let memory = RawMemoryAccess::new(heap);
                // SAFETY: F10.RawMemoryAccessValid — Unmanaged origin: `offset` is an absolute address and this function's caller
                // guarantees it is valid for the access.
                unsafe { memory.read_bytes(None, offset, dest) }
            }
            #[cfg(feature = "multithreading")]
            PointerOrigin::CrossArenaObjectRef(origin) => {
                let heap = &self.local.heap;
                let memory = RawMemoryAccess::new(heap);
                // SAFETY: F10.RawMemoryAccessValid — The cross-arena owner is passed as the object containing the location, as `RawMemoryAccess`
                // requires; the memory layer checks the complete access range.
                unsafe {
                    memory.read_bytes(
                        Some(MemoryOwner::cross_arena(self.gc, origin)),
                        offset,
                        dest,
                    )
                }
            }
            PointerOrigin::Transient(obj) => obj
                .instance_storage
                .with_data(|obj_data| bounds_checked_slice_read(obj_data, dest, offset)),
        }
    }

    #[inline]
    unsafe fn object_atomic(
        &mut self,
        origin: PointerOrigin<'gc>,
        offset: ByteOffset,
        op: dotnet_utils::atomic::PointerAtomicOp,
    ) -> Result<*mut u8, MemoryAccessError> {
        let _gc_scope = GcScopeGuard::enter(self, self.gc_ready_token());
        match origin {
            PointerOrigin::Heap(owner) => {
                let mut memory = RawMemoryAccess::new(&self.local.heap);
                // SAFETY: F10.RawMemoryAccessValid — The caller identifies a live object slot; memory checks its bounds.
                unsafe {
                    memory.object_atomic(self.gc, Some(MemoryOwner::Local(owner)), offset, op)
                }
            }
            #[cfg(feature = "multithreading")]
            PointerOrigin::CrossArenaObjectRef(origin) => {
                let mut memory = RawMemoryAccess::new(&self.local.heap);
                // SAFETY: F10.RawMemoryAccessValid — The cross-arena owner supplies the guarded storage.
                unsafe {
                    memory.object_atomic(
                        self.gc,
                        Some(MemoryOwner::cross_arena(self.gc, origin)),
                        offset,
                        op,
                    )
                }
            }
            PointerOrigin::Static(metadata) => {
                let storage = self.statics().get_for_metadata(&metadata);
                storage.storage.with_data_mut(|data| {
                    let range = checked_window(offset, size_of::<*mut u8>(), data.len())?;
                    let slot = NonNull::new(data.as_mut_ptr().wrapping_add(range.start))
                        .expect("static slot is non-null");
                    let mut memory = RawMemoryAccess::new(&self.local.heap);
                    // SAFETY: F10.BorrowedStorageStable — the static storage guard remains held;
                    // `checked_window` bounds the slot and the caller supplies an initialized object slot.
                    unsafe { memory.object_atomic_ptr(slot, op) }
                })
            }

            PointerOrigin::FrameHome(serial) => {
                let slot =
                    frame_home_window(self, serial, offset.as_usize(), size_of::<*mut u8>())?;
                let mut memory = RawMemoryAccess::new(&self.local.heap);
                // SAFETY: F10.FrameHomeChecked — `frame_home_window` found the live frame and checked
                // that the accessed bytes lie inside its home block.
                unsafe { memory.object_atomic_ptr(slot, op) }
            }
            PointerOrigin::Transient(obj) => obj.instance_storage.with_data_mut(|data| {
                let range = checked_window(offset, size_of::<*mut u8>(), data.len())?;
                let slot = NonNull::new(data.as_mut_ptr().wrapping_add(range.start))
                    .expect("transient slot is non-null");
                let mut memory = RawMemoryAccess::new(&self.local.heap);
                // SAFETY: F10.BorrowedStorageStable — the transient owner's exclusive guard
                // remains held; `checked_window` bounds the caller's initialized object slot.
                unsafe { memory.object_atomic_ptr(slot, op) }
            }),
            PointerOrigin::Unmanaged => {
                let mut memory = RawMemoryAccess::new(&self.local.heap);
                // SAFETY: F10.RawMemoryAccessValid — The caller guarantees a valid unmanaged slot.
                unsafe { memory.object_atomic(self.gc, None, offset, op) }
            }
        }
    }

    unsafe fn compare_exchange_atomic(
        &mut self,
        origin: PointerOrigin<'gc>,
        offset: ByteOffset,
        expected: u64,
        new: u64,
        size: usize,
        success: dotnet_utils::sync::Ordering,
        failure: dotnet_utils::sync::Ordering,
    ) -> Result<u64, CompareExchangeError> {
        let _gc_scope = GcScopeGuard::enter(self, self.gc_ready_token());
        match origin {
            PointerOrigin::Heap(owner) => {
                let heap = &self.local.heap;
                let mut memory = RawMemoryAccess::new(heap);
                // SAFETY: F10.RawMemoryAccessValid — The heap owner is passed as the object containing the location, as `RawMemoryAccess`
                // requires; the memory layer checks the complete access range.
                unsafe {
                    memory.compare_exchange_atomic(
                        self.gc,
                        Some(MemoryOwner::Local(owner)),
                        offset,
                        expected,
                        new,
                        size,
                        success,
                        failure,
                    )
                }
            }
            PointerOrigin::Static(metadata) => {
                let storage = self.statics().get_for_metadata(&metadata);
                // SAFETY: F10.BorrowedStorageStable — `storage` is retained by the statics registry for this call, and the atomic
                // operation below synchronizes the access.
                let base_ptr = unsafe { storage.storage.raw_data_ptr() };
                // SAFETY: F10.RawMemoryAccessValid — this function's caller guarantees `offset` (and width) lie within the static
                // storage; no bounds check is done here.
                let abs_ptr = unsafe { base_ptr.add(offset.as_usize()) };
                let abs_ptr =
                    NonNull::new(abs_ptr).expect("static storage pointer must be non-null");
                let mut memory = RawMemoryAccess::new(&self.local.heap);
                // SAFETY: F10.RawMemoryAccessValid — `abs_ptr` lies within the retained static storage (caller contract), and the
                // atomic operation synchronizes the access.
                unsafe {
                    memory
                        .compare_exchange_atomic_ptr(abs_ptr, expected, new, size, success, failure)
                }
            }

            PointerOrigin::FrameHome(serial) => {
                let ptr = frame_home_window(self, serial, offset.as_usize(), size)
                    .map_err(CompareExchangeError::Bounds)?;
                let mut memory = RawMemoryAccess::new(&self.local.heap);
                // SAFETY: F10.FrameHomeChecked — `frame_home_window` found the live frame and checked
                // that the accessed bytes lie inside its home block.
                unsafe {
                    memory.compare_exchange_atomic_ptr(ptr, expected, new, size, success, failure)
                }
            }
            PointerOrigin::Transient(obj) => obj.instance_storage.with_data(|data| {
                let base_ptr = data.as_ptr();
                // SAFETY: F10.RawMemoryAccessValid — this function's caller guarantees `offset` (and width) lie within the
                // Transient owner's storage; no bounds check is done here.
                let abs_ptr = unsafe { base_ptr.add(offset.as_usize()) };
                let abs_ptr = NonNull::new(abs_ptr.cast_mut())
                    .expect("transient storage pointer must be non-null");
                let mut memory = RawMemoryAccess::new(&self.local.heap);
                // SAFETY: F10.RawMemoryAccessValid — `abs_ptr` lies within the Transient owner's storage, which stays borrowed for
                // the closure (caller contract for range).
                unsafe {
                    memory
                        .compare_exchange_atomic_ptr(abs_ptr, expected, new, size, success, failure)
                }
            }),
            PointerOrigin::Unmanaged => {
                let heap = &self.local.heap;
                let mut memory = RawMemoryAccess::new(heap);
                // SAFETY: F10.RawMemoryAccessValid — Unmanaged origin: `offset` is an absolute address and this function's caller
                // guarantees it is valid for the access.
                unsafe {
                    memory.compare_exchange_atomic(
                        self.gc, None, offset, expected, new, size, success, failure,
                    )
                }
            }
            #[cfg(feature = "multithreading")]
            PointerOrigin::CrossArenaObjectRef(origin) => {
                let heap = &self.local.heap;
                let mut memory = RawMemoryAccess::new(heap);
                // SAFETY: F10.RawMemoryAccessValid — The cross-arena owner is passed as the object containing the location, as `RawMemoryAccess`
                // requires; the memory layer checks the complete access range.
                unsafe {
                    memory.compare_exchange_atomic(
                        self.gc,
                        Some(MemoryOwner::cross_arena(self.gc, origin)),
                        offset,
                        expected,
                        new,
                        size,
                        success,
                        failure,
                    )
                }
            }
        }
    }

    #[inline]
    unsafe fn exchange_atomic(
        &mut self,
        origin: PointerOrigin<'gc>,
        offset: ByteOffset,
        value: u64,
        size: usize,
        ordering: dotnet_utils::sync::Ordering,
    ) -> Result<u64, MemoryAccessError> {
        let _gc_scope = GcScopeGuard::enter(self, self.gc_ready_token());
        match origin {
            PointerOrigin::Heap(owner) => {
                let heap = &self.local.heap;
                let mut memory = RawMemoryAccess::new(heap);
                // SAFETY: F10.RawMemoryAccessValid — The heap owner is passed as the object containing the location, as `RawMemoryAccess`
                // requires; the memory layer checks the complete access range.
                unsafe {
                    memory.exchange_atomic(
                        self.gc,
                        Some(MemoryOwner::Local(owner)),
                        offset,
                        value,
                        size,
                        ordering,
                    )
                }
            }
            PointerOrigin::Static(metadata) => {
                let storage = self.statics().get_for_metadata(&metadata);
                // SAFETY: F10.BorrowedStorageStable — `storage` is retained by the statics registry for this call, and the atomic
                // operation below synchronizes the access.
                let base_ptr = unsafe { storage.storage.raw_data_ptr() };
                // SAFETY: F10.RawMemoryAccessValid — this function's caller guarantees `offset` (and width) lie within the static
                // storage; no bounds check is done here.
                let abs_ptr = unsafe { base_ptr.add(offset.as_usize()) };
                let abs_ptr =
                    NonNull::new(abs_ptr).expect("static storage pointer must be non-null");
                let mut memory = RawMemoryAccess::new(&self.local.heap);
                // SAFETY: F10.RawMemoryAccessValid — `abs_ptr` lies within the retained static storage (caller contract), and the
                // atomic operation synchronizes the access.
                unsafe { memory.exchange_atomic_ptr(abs_ptr, value, size, ordering) }
            }

            PointerOrigin::FrameHome(serial) => {
                let ptr = frame_home_window(self, serial, offset.as_usize(), size)?;
                let mut memory = RawMemoryAccess::new(&self.local.heap);
                // SAFETY: F10.FrameHomeChecked — `frame_home_window` found the live frame and checked
                // that the accessed bytes lie inside its home block.
                unsafe { memory.exchange_atomic_ptr(ptr, value, size, ordering) }
            }
            PointerOrigin::Transient(obj) => obj.instance_storage.with_data(|data| {
                let base_ptr = data.as_ptr();
                // SAFETY: F10.RawMemoryAccessValid — this function's caller guarantees `offset` (and width) lie within the
                // Transient owner's storage; no bounds check is done here.
                let abs_ptr = unsafe { base_ptr.add(offset.as_usize()) };
                let abs_ptr = NonNull::new(abs_ptr.cast_mut())
                    .expect("transient storage pointer must be non-null");
                let mut memory = RawMemoryAccess::new(&self.local.heap);
                // SAFETY: F10.RawMemoryAccessValid — `abs_ptr` lies within the Transient owner's storage, which stays borrowed for
                // the closure (caller contract for range).
                unsafe { memory.exchange_atomic_ptr(abs_ptr, value, size, ordering) }
            }),
            PointerOrigin::Unmanaged => {
                let heap = &self.local.heap;
                let mut memory = RawMemoryAccess::new(heap);
                // SAFETY: F10.RawMemoryAccessValid — Unmanaged origin: `offset` is an absolute address and this function's caller
                // guarantees it is valid for the access.
                unsafe { memory.exchange_atomic(self.gc, None, offset, value, size, ordering) }
            }
            #[cfg(feature = "multithreading")]
            PointerOrigin::CrossArenaObjectRef(origin) => {
                let heap = &self.local.heap;
                let mut memory = RawMemoryAccess::new(heap);
                // SAFETY: F10.RawMemoryAccessValid — The cross-arena owner is passed as the object containing the location, as `RawMemoryAccess`
                // requires; the memory layer checks the complete access range.
                unsafe {
                    memory.exchange_atomic(
                        self.gc,
                        Some(MemoryOwner::cross_arena(self.gc, origin)),
                        offset,
                        value,
                        size,
                        ordering,
                    )
                }
            }
        }
    }

    #[inline]
    unsafe fn exchange_add_atomic(
        &mut self,
        origin: PointerOrigin<'gc>,
        offset: ByteOffset,
        value: u64,
        size: usize,
        ordering: dotnet_utils::sync::Ordering,
    ) -> Result<u64, MemoryAccessError> {
        let _gc_scope = GcScopeGuard::enter(self, self.gc_ready_token());
        match origin {
            PointerOrigin::Heap(owner) => {
                let heap = &self.local.heap;
                let mut memory = RawMemoryAccess::new(heap);
                // SAFETY: F10.RawMemoryAccessValid — The heap owner is passed as the object containing the location, as `RawMemoryAccess`
                // requires; the memory layer checks the complete access range.
                unsafe {
                    memory.exchange_add_atomic(
                        self.gc,
                        Some(MemoryOwner::Local(owner)),
                        offset,
                        value,
                        size,
                        ordering,
                    )
                }
            }
            PointerOrigin::Static(metadata) => {
                let storage = self.statics().get_for_metadata(&metadata);
                // SAFETY: F10.BorrowedStorageStable — `storage` is retained by the statics registry for this call, and the atomic
                // operation below synchronizes the access.
                let base_ptr = unsafe { storage.storage.raw_data_ptr() };
                // SAFETY: F10.RawMemoryAccessValid — this function's caller guarantees `offset` (and width) lie within the static
                // storage; no bounds check is done here.
                let abs_ptr = unsafe { base_ptr.add(offset.as_usize()) };
                let abs_ptr =
                    NonNull::new(abs_ptr).expect("static storage pointer must be non-null");
                let mut memory = RawMemoryAccess::new(&self.local.heap);
                // SAFETY: F10.RawMemoryAccessValid — `abs_ptr` lies within the retained static storage (caller contract), and the
                // atomic operation synchronizes the access.
                unsafe { memory.exchange_add_atomic_ptr(abs_ptr, value, size, ordering) }
            }

            PointerOrigin::FrameHome(serial) => {
                let ptr = frame_home_window(self, serial, offset.as_usize(), size)?;
                let mut memory = RawMemoryAccess::new(&self.local.heap);
                // SAFETY: F10.FrameHomeChecked — `frame_home_window` found the live frame and checked
                // that the accessed bytes lie inside its home block.
                unsafe { memory.exchange_add_atomic_ptr(ptr, value, size, ordering) }
            }
            PointerOrigin::Transient(obj) => obj.instance_storage.with_data(|data| {
                let base_ptr = data.as_ptr();
                // SAFETY: F10.RawMemoryAccessValid — this function's caller guarantees `offset` (and width) lie within the
                // Transient owner's storage; no bounds check is done here.
                let abs_ptr = unsafe { base_ptr.add(offset.as_usize()) };
                let abs_ptr = NonNull::new(abs_ptr.cast_mut())
                    .expect("transient storage pointer must be non-null");
                let mut memory = RawMemoryAccess::new(&self.local.heap);
                // SAFETY: F10.RawMemoryAccessValid — `abs_ptr` lies within the Transient owner's storage, which stays borrowed for
                // the closure (caller contract for range).
                unsafe { memory.exchange_add_atomic_ptr(abs_ptr, value, size, ordering) }
            }),
            PointerOrigin::Unmanaged => {
                let heap = &self.local.heap;
                let mut memory = RawMemoryAccess::new(heap);
                // SAFETY: F10.RawMemoryAccessValid — Unmanaged origin: `offset` is an absolute address and this function's caller
                // guarantees it is valid for the access.
                unsafe { memory.exchange_add_atomic(self.gc, None, offset, value, size, ordering) }
            }
            #[cfg(feature = "multithreading")]
            PointerOrigin::CrossArenaObjectRef(origin) => {
                let heap = &self.local.heap;
                let mut memory = RawMemoryAccess::new(heap);
                // SAFETY: F10.RawMemoryAccessValid — The cross-arena owner is passed as the object containing the location, as `RawMemoryAccess`
                // requires; the memory layer checks the complete access range.
                unsafe {
                    memory.exchange_add_atomic(
                        self.gc,
                        Some(MemoryOwner::cross_arena(self.gc, origin)),
                        offset,
                        value,
                        size,
                        ordering,
                    )
                }
            }
        }
    }

    #[inline]
    unsafe fn load_atomic(
        &self,
        origin: PointerOrigin<'gc>,
        offset: ByteOffset,
        size: usize,
        ordering: dotnet_utils::sync::Ordering,
    ) -> Result<u64, MemoryAccessError> {
        let _gc_scope = GcScopeGuard::enter(self, self.gc_ready_token());
        match origin {
            PointerOrigin::Heap(owner) => {
                let heap = &self.local.heap;
                let memory = RawMemoryAccess::new(heap);
                // SAFETY: F10.RawMemoryAccessValid — The heap owner is passed as the object containing the location, as `RawMemoryAccess`
                // requires; the memory layer checks the complete access range.
                unsafe {
                    memory.load_atomic(Some(MemoryOwner::Local(owner)), offset, size, ordering)
                }
            }
            PointerOrigin::Static(metadata) => {
                let storage = self.statics().get_for_metadata(&metadata);
                storage.storage.with_data(|data| {
                    // SAFETY: F10.RawMemoryAccessValid — Static-field metadata keeps `offset` within this live storage.
                    let abs_ptr = unsafe { data.as_ptr().add(offset.as_usize()) };
                    let abs_ptr = NonNull::new(abs_ptr.cast_mut())
                        .expect("static storage pointer must be non-null");
                    let memory = RawMemoryAccess::new(&self.local.heap);
                    // SAFETY: F10.BorrowedStorageStable — The read guard stabilizes storage, and the offset and width come from the by-ref operation.
                    unsafe { memory.load_atomic_ptr(abs_ptr, size, ordering) }
                })
            }

            PointerOrigin::FrameHome(serial) => {
                let ptr = frame_home_window(self, serial, offset.as_usize(), size)?;
                let memory = RawMemoryAccess::new(&self.local.heap);
                // SAFETY: F10.FrameHomeChecked — `frame_home_window` found the live frame and checked
                // that the accessed bytes lie inside its home block.
                unsafe { memory.load_atomic_ptr(ptr, size, ordering) }
            }
            PointerOrigin::Transient(obj) => obj.instance_storage.with_data(|data| {
                let base_ptr = data.as_ptr();
                // SAFETY: F10.RawMemoryAccessValid — this function's caller guarantees `offset` (and width) lie within the
                // Transient owner's storage; no bounds check is done here.
                let abs_ptr = unsafe { base_ptr.add(offset.as_usize()) };
                let abs_ptr = NonNull::new(abs_ptr.cast_mut())
                    .expect("transient storage pointer must be non-null");
                let memory = RawMemoryAccess::new(&self.local.heap);
                // SAFETY: F10.RawMemoryAccessValid — `abs_ptr` lies within the Transient owner's storage, which stays borrowed for
                // the closure (caller contract for range).
                unsafe { memory.load_atomic_ptr(abs_ptr, size, ordering) }
            }),
            PointerOrigin::Unmanaged => {
                let heap = &self.local.heap;
                let memory = RawMemoryAccess::new(heap);
                // SAFETY: F10.RawMemoryAccessValid — Unmanaged origin: `offset` is an absolute address and this function's caller
                // guarantees it is valid for the access.
                unsafe { memory.load_atomic(None, offset, size, ordering) }
            }
            #[cfg(feature = "multithreading")]
            PointerOrigin::CrossArenaObjectRef(origin) => {
                let heap = &self.local.heap;
                let memory = RawMemoryAccess::new(heap);
                // SAFETY: F10.RawMemoryAccessValid — The cross-arena owner is passed as the object containing the location, as `RawMemoryAccess`
                // requires; the memory layer checks the complete access range.
                unsafe {
                    memory.load_atomic(
                        Some(MemoryOwner::cross_arena(self.gc, origin)),
                        offset,
                        size,
                        ordering,
                    )
                }
            }
        }
    }

    #[inline]
    unsafe fn store_atomic(
        &mut self,
        origin: PointerOrigin<'gc>,
        offset: ByteOffset,
        value: u64,
        size: usize,
        ordering: dotnet_utils::sync::Ordering,
    ) -> Result<(), MemoryAccessError> {
        let _gc_scope = GcScopeGuard::enter(self, self.gc_ready_token());
        match origin {
            PointerOrigin::Heap(owner) => {
                let heap = &self.local.heap;
                let mut memory = RawMemoryAccess::new(heap);
                // SAFETY: F10.RawMemoryAccessValid — The heap owner is passed as the object containing the location, as `RawMemoryAccess`
                // requires; the memory layer checks the complete access range.
                unsafe {
                    memory.store_atomic(
                        self.gc,
                        Some(MemoryOwner::Local(owner)),
                        offset,
                        value,
                        size,
                        ordering,
                    )
                }
            }
            PointerOrigin::Static(metadata) => {
                let storage = self.statics().get_for_metadata(&metadata);
                storage.storage.with_data_mut(|data| {
                    // SAFETY: F10.RawMemoryAccessValid — Static-field metadata keeps `offset` within this live storage.
                    let abs_ptr = unsafe { data.as_mut_ptr().add(offset.as_usize()) };
                    let abs_ptr =
                        NonNull::new(abs_ptr).expect("static storage pointer must be non-null");
                    let mut memory = RawMemoryAccess::new(&self.local.heap);
                    // SAFETY: F10.BorrowedStorageStable — The write guard stabilizes storage, and the offset and width come from the by-ref operation.
                    unsafe { memory.store_atomic_ptr(abs_ptr, value, size, ordering) }
                })
            }

            PointerOrigin::FrameHome(serial) => {
                let ptr = frame_home_window(self, serial, offset.as_usize(), size)?;
                let mut memory = RawMemoryAccess::new(&self.local.heap);
                // SAFETY: F10.FrameHomeChecked — `frame_home_window` found the live frame and checked
                // that the accessed bytes lie inside its home block.
                unsafe { memory.store_atomic_ptr(ptr, value, size, ordering) }
            }
            PointerOrigin::Transient(obj) => obj.instance_storage.with_data_mut(|data| {
                let base_ptr = data.as_mut_ptr();
                // SAFETY: F10.BorrowedStorageStable — Transient-field metadata keeps `offset` within this write-guarded storage.
                let abs_ptr = unsafe { base_ptr.add(offset.as_usize()) };
                let abs_ptr =
                    NonNull::new(abs_ptr).expect("transient storage pointer must be non-null");
                let mut memory = RawMemoryAccess::new(&self.local.heap);
                // SAFETY: F10.BorrowedStorageStable — The write guard provides synchronized access for the by-ref width.
                unsafe { memory.store_atomic_ptr(abs_ptr, value, size, ordering) }
            }),
            PointerOrigin::Unmanaged => {
                let heap = &self.local.heap;
                let mut memory = RawMemoryAccess::new(heap);
                // SAFETY: F10.RawMemoryAccessValid — Unmanaged origin: `offset` is an absolute address and this function's caller
                // guarantees it is valid for the access.
                unsafe { memory.store_atomic(self.gc, None, offset, value, size, ordering) }
            }
            #[cfg(feature = "multithreading")]
            PointerOrigin::CrossArenaObjectRef(origin) => {
                let heap = &self.local.heap;
                let mut memory = RawMemoryAccess::new(heap);
                // SAFETY: F10.RawMemoryAccessValid — The cross-arena owner is passed as the object containing the location, as `RawMemoryAccess`
                // requires; the memory layer checks the complete access range.
                unsafe {
                    memory.store_atomic(
                        self.gc,
                        Some(MemoryOwner::cross_arena(self.gc, origin)),
                        offset,
                        value,
                        size,
                        ordering,
                    )
                }
            }
        }
    }

    #[inline]
    fn check_gc_safe_point(&self) -> bool {
        if self.local.active_borrows.get() > 0 {
            // Cannot reach safe point while holding borrows!
            return false;
        }
        let thread_manager = &self.shared.thread_manager;
        thread_manager.is_gc_stop_requested()
    }
}

impl<'a, 'gc> VmRawMemoryOps<'gc> for VesContext<'a, 'gc> {
    #[inline]
    fn localloc(&mut self, size: usize) -> Result<NonNull<u8>, FrameAllocationError> {
        self.state_mut().allocations.allocate(size)
    }

    fn resolve_address(
        &self,
        origin: &PointerOrigin<'gc>,
        offset: ByteOffset,
    ) -> Result<NonNull<u8>, MemoryAccessError> {
        Ok(match origin {

            PointerOrigin::Static(metadata) => {
                let storage = self
                    .statics()
                    .get_for_metadata(metadata);
                storage.storage.with_data(|data| {
                    #[expect(clippy::multiple_unsafe_ops_per_block, reason = "forming the static offset address and wrapping its proven non-null result share one storage proof")]
                    // SAFETY: F10.RawMemoryAccessValid — Static origins use the unchecked
                    // path: the caller supplies an in-bounds offset into the static storage,
                    // so `.add` stays in the allocation and the result is non-null. Forming
                    // the address grants no later access right.
                    unsafe { NonNull::new_unchecked(data.as_ptr().add(offset.as_usize()).cast_mut()) }
                })
            }
            PointerOrigin::FrameHome(serial) => frame_home_window(self, *serial, offset.as_usize(), 0)?,
            PointerOrigin::Heap(owner) => {
                let handle = owner.0.expect("resolve_address: null owner handle");
                let inner = handle.borrow();
                inner
                    .storage
                    .observe_address(offset.as_usize())
                    .expect("resolve_address: heap address is null or wrapped to zero")
            }
            PointerOrigin::Unmanaged => {
                // SAFETY: F10.RawMemoryAccessValid — This is the explicit Unmanaged-origin branch; callers
                // guarantee that a nonzero offset is a live raw address.
                NonNull::new(unsafe { unmanaged_ptr_from_addr(offset.as_usize()) })
                    .expect("resolve_address: null unmanaged pointer (offset=0)")
            }
            #[cfg(feature = "multithreading")]
            PointerOrigin::CrossArenaObjectRef(origin) => {
                let _lease = dotnet_utils::gc::try_acquire_lease_for_identity(origin.identity())
                    .ok_or(MemoryAccessError::CrossArenaViolation)?;
                // SAFETY: F1.ArenaGenerationMatch; F1.GcHandleRooted — the exact lease covers this access and the origin's caller keeps the allocation rooted.
                let ptr = unsafe { origin.object_ptr() };
                ptr.as_heap_storage(|storage| {
                    storage
                        .observe_address(offset.as_usize())
                        .expect("resolve_address: cross-arena address is null or wrapped to zero")
                })
            }
            // The borrowed Transient origin retains its shared owner during resolution;
            // the returned address alone retains no owner or access guard.
            PointerOrigin::Transient(obj) => obj.instance_storage.with_data(|data| {
                #[expect(clippy::multiple_unsafe_ops_per_block, reason = "forming the transient offset address and wrapping its proven non-null result share one storage proof")]
                // SAFETY: F10.RawMemoryAccessValid — the caller supplies an in-bounds offset
                // into the Transient owner's storage, so `.add` stays in the allocation and
                // the result is non-null. Forming the address grants no later access right.
                unsafe { NonNull::new_unchecked(data.as_ptr().add(offset.as_usize()).cast_mut()) }
            }),
        })
    }
}

#[cfg(test)]
mod safe_api_tests {
    use super::*;
    use crate::stack::ops::{VmCallOps, VmResolutionOps};
    use dotnet_runtime_memory::HeapManager;
    use gc_arena::{Arena, Rootable};

    #[test]
    #[allow(
        clippy::arc_with_non_send_sync,
        reason = "fixture stays within one arena mutation"
    )]
    fn owned_origin_typed_access_checks_complete_range() {
        use crate::{
            state::SharedGlobalState,
            sync::Arc,
            test_utils::{new_test_arena, test_gc_handle},
        };
        use dotnet_assemblies::AssemblyLoader;
        use dotnet_types::generics::GenericLookup;
        use dotnet_value::{
            layout::FieldLayoutManager,
            object::{HeapStorage, Object, ObjectRef},
            pointer::StaticMetadata,
            storage::FieldStorage,
        };
        use dotnetdll::{prelude::*, resolved::members::Accessibility as MemberAccessibility};

        let loader = Arc::new(AssemblyLoader::new_bare("safe_api_bounds".into()).unwrap());
        let mut res = Resolution::new(Module::new("SafeApiBounds.dll"));
        res.assembly = Some(Assembly::new("SafeApiBounds"));
        let ty = res.push_type_definition(TypeDefinition::new(None, "Storage"));
        res.push_field(
            ty,
            Field {
                attributes: vec![],
                name: "Value".into(),
                type_modifiers: vec![],
                by_ref: false,
                return_type: MemberType::Base(Box::new(BaseType::Int32)),
                accessibility: MemberAccessibility::Access(Accessibility::Public),
                static_member: true,
                init_only: false,
                literal: false,
                default: None,
                not_serialized: false,
                special_name: false,
                pinvoke: None,
                runtime_special_name: false,
                offset: None,
                marshal: None,
                initial_value: None,
            },
        );
        let _method = res.push_method(
            ty,
            Method::new(
                Accessibility::Public,
                crate::test_utils::parameterless_i32_signature(),
                "Main",
                None,
            ),
        );
        let resolution = loader.register_owned_assembly(res);
        let entry = dotnet_types::members::MethodDescription::new(
            TypeDescription::new(resolution.clone(), ty),
            Default::default(),
            resolution.clone(),
            MethodMemberIndex::Method(0),
        );
        let td = TypeDescription::new(resolution, ty);
        let shared = Arc::new(SharedGlobalState::new(loader));
        let mut arena = new_test_arena(&shared);
        arena.mutate_root(|mc, engine| {
            // SAFETY: F1.GcHandleRooted — engine remains the live arena root throughout this mutation.
            let gc = unsafe { test_gc_handle(mc, engine) };
            let info = shared
                .caches
                .get_method_info(entry.clone(), &Default::default(), shared.clone())
                .unwrap();
            let mut ctx = engine.ves_context(gc);
            ctx.entrypoint_frame(info, Default::default(), vec![])
                .unwrap();
            let fields = Arc::new(FieldLayoutManager {
                fields: Default::default(),
                total_size: 8,
                alignment: 4,
                gc_desc: Default::default(),
                has_ref_fields: false,
            });
            let make_object = || {
                Object::new(
                    td.clone(),
                    GenericLookup::default(),
                    FieldStorage::new(fields.clone(), vec![0; 8]),
                )
            };
            let heap = ObjectRef::new(gc, HeapStorage::Obj(Box::new(make_object())));
            let transient = Arc::new(make_object());
            let metadata = Arc::new(StaticMetadata {
                type_desc: td,
                generics: GenericLookup::default(),
                thread_id: None,
            });
            let generics = GenericLookup::default();
            ctx.shared
                .statics
                .init(
                    metadata.type_desc.clone(),
                    &ctx.with_generics(&generics),
                    ctx.thread_id.get(),
                )
                .unwrap();
            let static_len = ctx
                .statics()
                .get_for_metadata(&metadata)
                .storage
                .with_data(|data| data.len());
            // A live frame block exercises the same operations through the serial resolver.
            let serial = ctx.frame_stack.issue_home_serial();
            let home_layout = dotnet_vm_data::frame_homes::FrameHomeLayout::new(
                FieldLayoutManager {
                    fields: Default::default(),
                    total_size: 8,
                    alignment: 8,
                    gc_desc: Default::default(),
                    has_ref_fields: false,
                },
                Box::new([]),
                Box::new([]),
            );
            ctx.frame_stack.current_frame_mut().homes =
                dotnet_vm_data::frame_homes::FrameHomes::allocate(Arc::new(home_layout), serial)
                    .unwrap();

            // A pointer-sized slot ending exactly at the transient storage boundary
            // accepts a pointer operation; invalid slots below must never be touched.
            let transient_slot = PointerOrigin::Transient(transient.clone());
            let store_null = dotnet_utils::atomic::PointerAtomicOp::Store(
                std::ptr::null_mut(),
                dotnet_utils::sync::Ordering::Relaxed,
            );
            // SAFETY: F10.RawMemoryAccessValid — the zeroed pointer-width storage is live,
            // exclusively guarded, and contains a valid null pointer representation.
            unsafe {
                ctx.object_atomic(
                    transient_slot.clone(),
                    (8 - size_of::<*mut u8>()).into(),
                    store_null,
                )
            }
            .unwrap();
            let load =
                dotnet_utils::atomic::PointerAtomicOp::Load(dotnet_utils::sync::Ordering::Relaxed);
            // SAFETY: F10.RawMemoryAccessValid — the same guarded slot was initialized as a pointer above.
            let loaded = unsafe {
                ctx.object_atomic(transient_slot, (8 - size_of::<*mut u8>()).into(), load)
            }
            .unwrap();
            assert!(loaded.is_null());
            for (origin, len) in [
                (PointerOrigin::Transient(transient.clone()), 8),
                (PointerOrigin::Static(metadata.clone()), static_len),
            ] {
                for offset in [len, usize::MAX] {
                    let op = dotnet_utils::atomic::PointerAtomicOp::Load(
                        dotnet_utils::sync::Ordering::Relaxed,
                    );
                    // SAFETY: F10.RawMemoryAccessValid — the owned origin rejects this out-of-bounds pointer slot before accessing it.
                    let result = unsafe { ctx.object_atomic(origin.clone(), offset.into(), op) };
                    assert_eq!(
                        result,
                        Err(MemoryAccessError::BoundsCheck {
                            offset,
                            size: size_of::<*mut u8>(),
                            len
                        })
                    );
                }
            }
            let layout = LayoutManager::Scalar(Scalar::Int32);
            for (origin, len) in [
                (PointerOrigin::Heap(heap), 8),
                (PointerOrigin::Transient(transient), 8),
                (PointerOrigin::Static(metadata), static_len),
                (PointerOrigin::FrameHome(serial), 8),
            ] {
                for offset in [len - 3, len, len + 1, usize::MAX - 1, usize::MAX] {
                    // SAFETY: F10.RawMemoryAccessValid — fixture retains initialized integer storage;
                    // owned origins must reject invalid ranges internally before any access.
                    let read =
                        unsafe { ctx.read_unaligned(origin.clone(), offset.into(), &layout, None) };
                    assert!(
                        matches!(read, Err(MemoryAccessError::BoundsCheck { .. })),
                        "offset {offset}: {read:?}"
                    );
                    // SAFETY: F10.RawMemoryAccessValid — value and layout match; range validation belongs to the owned origin.
                    let write = unsafe {
                        ctx.write_unaligned(
                            origin.clone(),
                            offset.into(),
                            StackValue::Int32(42),
                            &layout,
                        )
                    };
                    assert!(
                        matches!(write, Err(MemoryAccessError::BoundsCheck { .. })),
                        "offset {offset}: {write:?}"
                    );
                }
                // Copies at the end may be empty, but an overflowing or out-of-range
                // request must return its original offset, size and storage length.
                for (offset, size) in [(len, 0), (len - 1, 1)] {
                    let input = vec![0x5a; size];
                    let mut output = vec![0; size];
                    // SAFETY: F10.RawMemoryAccessValid — the fixture retains the owner and the requested range is in bounds.
                    unsafe { ctx.write_bytes(origin.clone(), offset.into(), &input) }.unwrap();
                    // SAFETY: F10.RawMemoryAccessValid — the same initialized range remains live.
                    unsafe { ctx.read_bytes(origin.clone(), offset.into(), &mut output) }.unwrap();
                    assert_eq!(output, input);
                }
                if !matches!(origin, PointerOrigin::Heap(_)) {
                    for (offset, size) in
                        [(len + 1, 0), (len, 1), (usize::MAX, 1), (usize::MAX - 1, 2)]
                    {
                        let expected = MemoryAccessError::BoundsCheck { offset, size, len };
                        let input = vec![0; size];
                        let mut output = vec![0; size];
                        // SAFETY: F10.RawMemoryAccessValid — owned origins reject the invalid range before access.
                        let write =
                            unsafe { ctx.write_bytes(origin.clone(), offset.into(), &input) };
                        assert_eq!(write, Err(expected.clone()));
                        // SAFETY: F10.RawMemoryAccessValid — owned origins reject the invalid range before access.
                        let read =
                            unsafe { ctx.read_bytes(origin.clone(), offset.into(), &mut output) };
                        assert_eq!(read, Err(expected));
                    }
                }
                // Restore the final byte before checking the initial payload.
                // SAFETY: F10.RawMemoryAccessValid — the last byte belongs to the live storage.
                unsafe { ctx.write_bytes(origin.clone(), (len - 1).into(), &[0]) }.unwrap();
                let mut bytes = vec![0; len];
                // SAFETY: F10.RawMemoryAccessValid — the retained integer payload is live and len bytes long.
                unsafe { ctx.read_bytes(origin.clone(), 0usize.into(), &mut bytes) }.unwrap();
                assert_eq!(bytes, vec![0; len]);
                // SAFETY: F10.RawMemoryAccessValid — final complete integer slot in retained storage, matching value and layout.
                unsafe {
                    ctx.write_unaligned(
                        origin.clone(),
                        (len - 4).into(),
                        StackValue::Int32(42),
                        &layout,
                    )
                }
                .unwrap();
                // SAFETY: F10.RawMemoryAccessValid — same initialized final integer slot, retained throughout this mutation.
                let read =
                    unsafe { ctx.read_unaligned(origin, (len - 4).into(), &layout, None) }.unwrap();
                assert!(matches!(read, StackValue::Int32(42)));
            }

            ctx.frame_stack.current_frame_mut().homes.release();
            // SAFETY: F10.FrameHomeChecked — the resolver must reject the retired serial before any access.
            let read = unsafe {
                ctx.read_unaligned(
                    PointerOrigin::FrameHome(serial),
                    0usize.into(),
                    &layout,
                    None,
                )
            };
            assert!(matches!(read, Err(MemoryAccessError::InvalidOrigin)));
            // SAFETY: F10.FrameHomeChecked — the resolver must reject the retired serial before any access.
            let write = unsafe {
                ctx.write_unaligned(
                    PointerOrigin::FrameHome(serial),
                    0usize.into(),
                    StackValue::Int32(1),
                    &layout,
                )
            };
            assert_eq!(write, Err(MemoryAccessError::InvalidOrigin));

            // A serialized byref must keep the original thread's storage, even when
            // the executor resolving it is a different thread.
            let first = Arc::new(StaticMetadata {
                type_desc: entry.parent.clone(),
                generics: generics.clone(),
                thread_id: Some(100),
            });
            let second = Arc::new(StaticMetadata {
                thread_id: Some(200),
                ..(*first).clone()
            });
            let first_origin = PointerOrigin::Static(first.clone());
            let second_origin = PointerOrigin::Static(second);
            let address = ctx
                .resolve_address(&first_origin, ByteOffset::ZERO)
                .unwrap();
            let pointer = dotnet_value::pointer::ManagedPtr::new_static_for_thread(
                Some(address),
                TypeDescription::NULL,
                first.type_desc.clone(),
                generics,
                first.thread_id,
                false,
                ByteOffset::ZERO,
            );
            let mut encoded = dotnet_value::pointer::ManagedPtr::serialization_buffer();
            pointer.write(&mut encoded);
            // SAFETY: F10.RawMemoryAccessValid — the bytes were written above and static storage remains rooted by shared state.
            let restored = unsafe {
                dotnet_value::pointer::ManagedPtr::read_resolved_unchecked(&encoded, &ctx)
            }
            .unwrap();
            // SAFETY: F10.RawMemoryAccessValid — both addresses refer to integer slots in retained static allocations.
            unsafe {
                ctx.write_unaligned(
                    restored.origin.clone(),
                    ByteOffset::ZERO,
                    StackValue::Int32(73),
                    &layout,
                )
            }
            .unwrap();
            let read = |origin| {
                // SAFETY: F10.RawMemoryAccessValid — both origins own the same valid integer layout.
                unsafe { ctx.read_unaligned(origin, ByteOffset::ZERO, &layout, None) }.unwrap()
            };
            assert!(matches!(read(first_origin), StackValue::Int32(73)));
            assert!(matches!(read(second_origin), StackValue::Int32(0)));
        });
    }

    #[test]
    fn inline_typed_access_rejects_out_of_bounds_and_overflow() {
        let arena = Arena::<Rootable![()]>::new(|_| ());
        #[cfg(feature = "multithreading")]
        let arena_handle = Box::into_raw(Box::new(dotnet_utils::gc::ArenaHandle::new(
            dotnet_utils::ArenaId::new(5050),
        )));
        arena.mutate(|mc, _| {
            let gc = GCHandle::new(
                mc,
                #[cfg(feature = "multithreading")]
                // SAFETY: F1.GcHandleRooted — the boxed handle remains live until mutate returns.
                unsafe {
                    (&*arena_handle).as_inner()
                },
                #[cfg(feature = "memory-validation")]
                dotnet_utils::ArenaId::new(5050),
            );
            let heap = HeapManager::new();
            let mut memory = RawMemoryAccess::new(&heap);
            let layout = LayoutManager::Scalar(Scalar::Int32);
            let mut data = [0u8; 8];
            for offset in [5, 8, usize::MAX - 1, usize::MAX] {
                // SAFETY: F10.RawMemoryAccessValid — data is live initialized integer storage; the helper must reject the range before forming a pointer, and this mutation scope retains the GC context.
                let read = unsafe {
                    read_from_inline_storage(&memory, gc, &data, offset.into(), &layout, None)
                };
                assert!(matches!(read, Err(MemoryAccessError::BoundsCheck { offset: actual, size: 4, len: 8 }) if actual == offset));
                // SAFETY: F10.RawMemoryAccessValid — the exclusive slice and integer layout match; invalid ranges are checked by the helper before writing.
                let write = unsafe {
                    write_to_inline_storage(
                        &mut memory,
                        gc,
                        &mut data,
                        offset.into(),
                        StackValue::Int32(42),
                        &layout,
                    )
                };
                assert_eq!(write, Err(MemoryAccessError::BoundsCheck { offset, size: 4, len: 8 }));
            }
            assert_eq!(data, [0; 8]);
            // SAFETY: F10.RawMemoryAccessValid — offset four selects the final four bytes of this exclusive eight-byte integer buffer.
            unsafe {
                write_to_inline_storage(
                    &mut memory,
                    gc,
                    &mut data,
                    4usize.into(),
                    StackValue::Int32(42),
                    &layout,
                )
            }
            .unwrap();
            // SAFETY: F10.RawMemoryAccessValid — the same in-bounds integer slot was initialized above and stays borrowed in this GC scope.
            let read = unsafe {
                read_from_inline_storage(&memory, gc, &data, 4usize.into(), &layout, None)
            }
            .unwrap();
            assert!(matches!(read, StackValue::Int32(42)));
        });
        #[cfg(feature = "multithreading")]
        // SAFETY: F10.RawMemoryAccessValid — mutate finished using the uniquely allocated handle.
        unsafe {
            drop(Box::from_raw(arena_handle))
        };
    }
}

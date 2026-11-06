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
    pointer::{PointerOrigin, frame_home_window, unmanaged_ptr_from_addr},
};
use dotnet_vm_data::frame_alloc::FrameAllocationError;
use std::ptr::NonNull;

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
    if offset
        .as_usize()
        .checked_add(layout.size().as_usize())
        .is_none_or(|end| end > data.len())
    {
        return Err(MemoryAccessError::BoundsCheck {
            offset: offset.as_usize(),
            size: layout.size().as_usize(),
            len: data.len(),
        });
    }
    // SAFETY: F10.RawMemoryAccessValid — `offset + layout.size()` was checked against `data.len()` above, so `.add` stays in `data`.
    let ptr = unsafe { data.as_mut_ptr().add(offset.as_usize()) };
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
    if offset
        .as_usize()
        .checked_add(layout.size().as_usize())
        .is_none_or(|end| end > data.len())
    {
        return Err(MemoryAccessError::BoundsCheck {
            offset: offset.as_usize(),
            size: layout.size().as_usize(),
            len: data.len(),
        });
    }
    // SAFETY: F10.RawMemoryAccessValid — `offset + layout.size()` was checked against `data.len()` above, so `.add` stays in `data`.
    let ptr = unsafe { data.as_ptr().add(offset.as_usize()) };
    // SAFETY: F10.RawMemoryAccessValid — `ptr` has `layout.size()` readable bytes in `data` (checked above); this function's
    // `# Safety` covers initialization and the GC borrow scope.
    unsafe { memory.read_value_internal(gc, ptr, None, layout, type_desc) }
}

fn bounds_checked_slice_copy(
    dest: &mut [u8],
    src: &[u8],
    offset: ByteOffset,
) -> Result<(), MemoryAccessError> {
    if offset.as_usize() + src.len() > dest.len() {
        return Err(MemoryAccessError::BoundsCheck {
            offset: offset.as_usize(),
            size: src.len(),
            len: dest.len(),
        });
    }
    dest[offset.as_usize()..offset.as_usize() + src.len()].copy_from_slice(src);
    Ok(())
}

fn bounds_checked_slice_read(
    src: &[u8],
    dest: &mut [u8],
    offset: ByteOffset,
) -> Result<(), MemoryAccessError> {
    if offset.as_usize() + dest.len() > src.len() {
        return Err(MemoryAccessError::BoundsCheck {
            offset: offset.as_usize(),
            size: dest.len(),
            len: src.len(),
        });
    }
    dest.copy_from_slice(&src[offset.as_usize()..offset.as_usize() + dest.len()]);
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
        pointer: &dotnet_value::pointer::RetainedPtr<'gc>,
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
        layout: &dotnet_value::layout::LayoutManager,
    ) -> Result<(), MemoryAccessError> {
        let _gc_scope = GcScopeGuard::enter(self, self.gc_ready_token());
        match origin {
            PointerOrigin::FrameHome(serial) => {
                // ldind.i/stind.i on a byref variable transfer the full managed-pointer
                // encoding, preserving its origin rather than copying only a native word.
                let layout = if matches!(layout, LayoutManager::Scalar(Scalar::NativeInt))
                    && self.frame_stack.is_byref_home(serial, offset.as_usize())
                {
                    &LayoutManager::Scalar(Scalar::ManagedPtr)
                } else {
                    layout
                };
                let ptr =
                    frame_home_window(self, serial, offset.as_usize(), layout.size().as_usize())?;
                let mut memory = dotnet_runtime_memory::RawMemoryAccess::new(&self.local.heap);
                // SAFETY: F10.FrameHomeChecked — `frame_home_window` found the live frame and checked
                // that the accessed bytes lie inside its home block.
                unsafe { memory.write_to_unmanaged(self.gc, ptr.as_ptr(), value, layout) }
            }
            PointerOrigin::Static(metadata) => {
                let storage = self
                    .statics()
                    .get(metadata.type_desc.clone(), &metadata.generics);
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
                let mut memory = dotnet_runtime_memory::RawMemoryAccess::new(heap);
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
                let mut memory = dotnet_runtime_memory::RawMemoryAccess::new(heap);
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
                let mut memory = dotnet_runtime_memory::RawMemoryAccess::new(heap);
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
        layout: &dotnet_value::layout::LayoutManager,
        type_desc: Option<TypeDescription>,
    ) -> Result<StackValue<'gc>, MemoryAccessError> {
        let _gc_scope = GcScopeGuard::enter(self, self.gc_ready_token());
        match origin {
            PointerOrigin::FrameHome(serial) => {
                // ldind.i/stind.i on a byref variable transfer the full managed-pointer
                // encoding, preserving its origin rather than copying only a native word.
                let layout = if matches!(layout, LayoutManager::Scalar(Scalar::NativeInt))
                    && self.frame_stack.is_byref_home(serial, offset.as_usize())
                {
                    &LayoutManager::Scalar(Scalar::ManagedPtr)
                } else {
                    layout
                };
                let ptr =
                    frame_home_window(self, serial, offset.as_usize(), layout.size().as_usize())?;
                let memory = dotnet_runtime_memory::RawMemoryAccess::new(&self.local.heap);
                // SAFETY: F10.FrameHomeChecked — `frame_home_window` found the live frame and checked
                // that the accessed bytes lie inside its home block.
                unsafe {
                    memory.read_value_internal(self.gc, ptr.as_ptr(), None, layout, type_desc)
                }
            }
            PointerOrigin::Static(metadata) => {
                let storage = self
                    .statics()
                    .get(metadata.type_desc.clone(), &metadata.generics);
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
                let memory = dotnet_runtime_memory::RawMemoryAccess::new(heap);
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
                let memory = dotnet_runtime_memory::RawMemoryAccess::new(heap);
                // SAFETY: F10.RawMemoryAccessValid — Unmanaged origin: `offset` is an absolute address and this function's caller
                // guarantees it is valid for the access.
                unsafe { memory.read_unaligned(self.gc, None, offset, layout, type_desc) }
            }
            #[cfg(feature = "multithreading")]
            PointerOrigin::CrossArenaObjectRef(origin) => {
                let heap = &self.local.heap;
                let memory = dotnet_runtime_memory::RawMemoryAccess::new(heap);
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
                let mut memory = dotnet_runtime_memory::RawMemoryAccess::new(&self.local.heap);
                // SAFETY: F10.FrameHomeChecked — `frame_home_window` found the live frame and checked
                // that the accessed bytes lie inside its home block.
                unsafe { memory.write_bytes_ptr(ptr, data) }
            }
            PointerOrigin::Static(metadata) => {
                let storage = self
                    .statics()
                    .get(metadata.type_desc.clone(), &metadata.generics);
                storage
                    .storage
                    .with_data_mut(|obj_data| bounds_checked_slice_copy(obj_data, data, offset))
            }
            PointerOrigin::Heap(owner) => {
                let heap = &self.local.heap;
                let mut memory = dotnet_runtime_memory::RawMemoryAccess::new(heap);
                // SAFETY: F10.RawMemoryAccessValid — The heap owner is passed as the object containing the location, as `RawMemoryAccess`
                // requires; the memory layer checks the complete access range.
                unsafe {
                    memory.write_bytes(self.gc, Some(MemoryOwner::Local(owner)), offset, data)
                }
            }
            PointerOrigin::Unmanaged => {
                let heap = &self.local.heap;
                let mut memory = dotnet_runtime_memory::RawMemoryAccess::new(heap);
                // SAFETY: F10.RawMemoryAccessValid — Unmanaged origin: `offset` is an absolute address and this function's caller
                // guarantees it is valid for the access.
                unsafe { memory.write_bytes(self.gc, None, offset, data) }
            }
            #[cfg(feature = "multithreading")]
            PointerOrigin::CrossArenaObjectRef(origin) => {
                let heap = &self.local.heap;
                let mut memory = dotnet_runtime_memory::RawMemoryAccess::new(heap);
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
                let memory = dotnet_runtime_memory::RawMemoryAccess::new(&self.local.heap);
                // SAFETY: F10.FrameHomeChecked — `frame_home_window` found the live frame and checked
                // that the accessed bytes lie inside its home block.
                unsafe { memory.read_bytes_ptr(ptr, dest) }
            }
            PointerOrigin::Static(metadata) => {
                let storage = self
                    .statics()
                    .get(metadata.type_desc.clone(), &metadata.generics);
                storage
                    .storage
                    .with_data(|obj_data| bounds_checked_slice_read(obj_data, dest, offset))
            }
            PointerOrigin::Heap(owner) => {
                let heap = &self.local.heap;
                let memory = dotnet_runtime_memory::RawMemoryAccess::new(heap);
                // SAFETY: F10.RawMemoryAccessValid — The heap owner is passed as the object containing the location, as `RawMemoryAccess`
                // requires; the memory layer checks the complete access range.
                unsafe { memory.read_bytes(Some(MemoryOwner::Local(owner)), offset, dest) }
            }
            PointerOrigin::Unmanaged => {
                let heap = &self.local.heap;
                let memory = dotnet_runtime_memory::RawMemoryAccess::new(heap);
                // SAFETY: F10.RawMemoryAccessValid — Unmanaged origin: `offset` is an absolute address and this function's caller
                // guarantees it is valid for the access.
                unsafe { memory.read_bytes(None, offset, dest) }
            }
            #[cfg(feature = "multithreading")]
            PointerOrigin::CrossArenaObjectRef(origin) => {
                let heap = &self.local.heap;
                let memory = dotnet_runtime_memory::RawMemoryAccess::new(heap);
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
                let storage = self
                    .statics()
                    .get(metadata.type_desc.clone(), &metadata.generics);
                storage.storage.with_data_mut(|data| {
                    let end = offset
                        .as_usize()
                        .checked_add(std::mem::size_of::<*mut u8>());
                    if end.is_none_or(|end| end > data.len()) {
                        return Err(MemoryAccessError::BoundsCheck {
                            offset: offset.as_usize(),
                            size: std::mem::size_of::<*mut u8>(),
                            len: data.len(),
                        });
                    }
                    let slot = NonNull::new(data.as_mut_ptr().wrapping_add(offset.as_usize()))
                        .expect("static slot is non-null");
                    let mut memory = RawMemoryAccess::new(&self.local.heap);
                    // SAFETY: F10.BorrowedStorageStable — The guard and bounds check protect this slot.
                    unsafe { memory.object_atomic_ptr(slot, op) }
                })
            }

            PointerOrigin::FrameHome(serial) => {
                let slot = frame_home_window(
                    self,
                    serial,
                    offset.as_usize(),
                    std::mem::size_of::<*mut u8>(),
                )?;
                let mut memory = RawMemoryAccess::new(&self.local.heap);
                // SAFETY: F10.FrameHomeChecked — `frame_home_window` found the live frame and checked
                // that the accessed bytes lie inside its home block.
                unsafe { memory.object_atomic_ptr(slot, op) }
            }
            PointerOrigin::Transient(obj) => obj.instance_storage.with_data_mut(|data| {
                let end = offset
                    .as_usize()
                    .checked_add(std::mem::size_of::<*mut u8>());
                if end.is_none_or(|end| end > data.len()) {
                    return Err(MemoryAccessError::BoundsCheck {
                        offset: offset.as_usize(),
                        size: std::mem::size_of::<*mut u8>(),
                        len: data.len(),
                    });
                }
                let slot = NonNull::new(data.as_mut_ptr().wrapping_add(offset.as_usize()))
                    .expect("transient slot is non-null");
                let mut memory = RawMemoryAccess::new(&self.local.heap);
                // SAFETY: F10.BorrowedStorageStable — The guard and bounds check protect this slot.
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
                let mut memory = dotnet_runtime_memory::RawMemoryAccess::new(heap);
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
                let storage = self
                    .statics()
                    .get(metadata.type_desc.clone(), &metadata.generics);
                // SAFETY: F10.BorrowedStorageStable — `storage` is retained by the statics registry for this call, and the atomic
                // operation below synchronizes the access.
                let base_ptr = unsafe { storage.storage.raw_data_ptr() };
                // SAFETY: F10.RawMemoryAccessValid — this function's caller guarantees `offset` (and width) lie within the static
                // storage; no bounds check is done here.
                let abs_ptr = unsafe { base_ptr.add(offset.as_usize()) };
                let abs_ptr =
                    NonNull::new(abs_ptr).expect("static storage pointer must be non-null");
                let mut memory = dotnet_runtime_memory::RawMemoryAccess::new(&self.local.heap);
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
                let mut memory = dotnet_runtime_memory::RawMemoryAccess::new(&self.local.heap);
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
                let mut memory = dotnet_runtime_memory::RawMemoryAccess::new(&self.local.heap);
                // SAFETY: F10.RawMemoryAccessValid — `abs_ptr` lies within the Transient owner's storage, which stays borrowed for
                // the closure (caller contract for range).
                unsafe {
                    memory
                        .compare_exchange_atomic_ptr(abs_ptr, expected, new, size, success, failure)
                }
            }),
            PointerOrigin::Unmanaged => {
                let heap = &self.local.heap;
                let mut memory = dotnet_runtime_memory::RawMemoryAccess::new(heap);
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
                let mut memory = dotnet_runtime_memory::RawMemoryAccess::new(heap);
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
                let mut memory = dotnet_runtime_memory::RawMemoryAccess::new(heap);
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
                let storage = self
                    .statics()
                    .get(metadata.type_desc.clone(), &metadata.generics);
                // SAFETY: F10.BorrowedStorageStable — `storage` is retained by the statics registry for this call, and the atomic
                // operation below synchronizes the access.
                let base_ptr = unsafe { storage.storage.raw_data_ptr() };
                // SAFETY: F10.RawMemoryAccessValid — this function's caller guarantees `offset` (and width) lie within the static
                // storage; no bounds check is done here.
                let abs_ptr = unsafe { base_ptr.add(offset.as_usize()) };
                let abs_ptr =
                    NonNull::new(abs_ptr).expect("static storage pointer must be non-null");
                let mut memory = dotnet_runtime_memory::RawMemoryAccess::new(&self.local.heap);
                // SAFETY: F10.RawMemoryAccessValid — `abs_ptr` lies within the retained static storage (caller contract), and the
                // atomic operation synchronizes the access.
                unsafe { memory.exchange_atomic_ptr(abs_ptr, value, size, ordering) }
            }

            PointerOrigin::FrameHome(serial) => {
                let ptr = frame_home_window(self, serial, offset.as_usize(), size)?;
                let mut memory = dotnet_runtime_memory::RawMemoryAccess::new(&self.local.heap);
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
                let mut memory = dotnet_runtime_memory::RawMemoryAccess::new(&self.local.heap);
                // SAFETY: F10.RawMemoryAccessValid — `abs_ptr` lies within the Transient owner's storage, which stays borrowed for
                // the closure (caller contract for range).
                unsafe { memory.exchange_atomic_ptr(abs_ptr, value, size, ordering) }
            }),
            PointerOrigin::Unmanaged => {
                let heap = &self.local.heap;
                let mut memory = dotnet_runtime_memory::RawMemoryAccess::new(heap);
                // SAFETY: F10.RawMemoryAccessValid — Unmanaged origin: `offset` is an absolute address and this function's caller
                // guarantees it is valid for the access.
                unsafe { memory.exchange_atomic(self.gc, None, offset, value, size, ordering) }
            }
            #[cfg(feature = "multithreading")]
            PointerOrigin::CrossArenaObjectRef(origin) => {
                let heap = &self.local.heap;
                let mut memory = dotnet_runtime_memory::RawMemoryAccess::new(heap);
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
                let mut memory = dotnet_runtime_memory::RawMemoryAccess::new(heap);
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
                let storage = self
                    .statics()
                    .get(metadata.type_desc.clone(), &metadata.generics);
                // SAFETY: F10.BorrowedStorageStable — `storage` is retained by the statics registry for this call, and the atomic
                // operation below synchronizes the access.
                let base_ptr = unsafe { storage.storage.raw_data_ptr() };
                // SAFETY: F10.RawMemoryAccessValid — this function's caller guarantees `offset` (and width) lie within the static
                // storage; no bounds check is done here.
                let abs_ptr = unsafe { base_ptr.add(offset.as_usize()) };
                let abs_ptr =
                    NonNull::new(abs_ptr).expect("static storage pointer must be non-null");
                let mut memory = dotnet_runtime_memory::RawMemoryAccess::new(&self.local.heap);
                // SAFETY: F10.RawMemoryAccessValid — `abs_ptr` lies within the retained static storage (caller contract), and the
                // atomic operation synchronizes the access.
                unsafe { memory.exchange_add_atomic_ptr(abs_ptr, value, size, ordering) }
            }

            PointerOrigin::FrameHome(serial) => {
                let ptr = frame_home_window(self, serial, offset.as_usize(), size)?;
                let mut memory = dotnet_runtime_memory::RawMemoryAccess::new(&self.local.heap);
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
                let mut memory = dotnet_runtime_memory::RawMemoryAccess::new(&self.local.heap);
                // SAFETY: F10.RawMemoryAccessValid — `abs_ptr` lies within the Transient owner's storage, which stays borrowed for
                // the closure (caller contract for range).
                unsafe { memory.exchange_add_atomic_ptr(abs_ptr, value, size, ordering) }
            }),
            PointerOrigin::Unmanaged => {
                let heap = &self.local.heap;
                let mut memory = dotnet_runtime_memory::RawMemoryAccess::new(heap);
                // SAFETY: F10.RawMemoryAccessValid — Unmanaged origin: `offset` is an absolute address and this function's caller
                // guarantees it is valid for the access.
                unsafe { memory.exchange_add_atomic(self.gc, None, offset, value, size, ordering) }
            }
            #[cfg(feature = "multithreading")]
            PointerOrigin::CrossArenaObjectRef(origin) => {
                let heap = &self.local.heap;
                let mut memory = dotnet_runtime_memory::RawMemoryAccess::new(heap);
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
                let memory = dotnet_runtime_memory::RawMemoryAccess::new(heap);
                // SAFETY: F10.RawMemoryAccessValid — The heap owner is passed as the object containing the location, as `RawMemoryAccess`
                // requires; the memory layer checks the complete access range.
                unsafe {
                    memory.load_atomic(Some(MemoryOwner::Local(owner)), offset, size, ordering)
                }
            }
            PointerOrigin::Static(metadata) => {
                let storage = self
                    .statics()
                    .get(metadata.type_desc.clone(), &metadata.generics);
                storage.storage.with_data(|data| {
                    // SAFETY: F10.RawMemoryAccessValid — Static-field metadata keeps `offset` within this live storage.
                    let abs_ptr = unsafe { data.as_ptr().add(offset.as_usize()) };
                    let abs_ptr = NonNull::new(abs_ptr.cast_mut())
                        .expect("static storage pointer must be non-null");
                    let memory = dotnet_runtime_memory::RawMemoryAccess::new(&self.local.heap);
                    // SAFETY: F10.BorrowedStorageStable — The read guard stabilizes storage, and the offset and width come from the by-ref operation.
                    unsafe { memory.load_atomic_ptr(abs_ptr, size, ordering) }
                })
            }

            PointerOrigin::FrameHome(serial) => {
                let ptr = frame_home_window(self, serial, offset.as_usize(), size)?;
                let memory = dotnet_runtime_memory::RawMemoryAccess::new(&self.local.heap);
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
                let memory = dotnet_runtime_memory::RawMemoryAccess::new(&self.local.heap);
                // SAFETY: F10.RawMemoryAccessValid — `abs_ptr` lies within the Transient owner's storage, which stays borrowed for
                // the closure (caller contract for range).
                unsafe { memory.load_atomic_ptr(abs_ptr, size, ordering) }
            }),
            PointerOrigin::Unmanaged => {
                let heap = &self.local.heap;
                let memory = dotnet_runtime_memory::RawMemoryAccess::new(heap);
                // SAFETY: F10.RawMemoryAccessValid — Unmanaged origin: `offset` is an absolute address and this function's caller
                // guarantees it is valid for the access.
                unsafe { memory.load_atomic(None, offset, size, ordering) }
            }
            #[cfg(feature = "multithreading")]
            PointerOrigin::CrossArenaObjectRef(origin) => {
                let heap = &self.local.heap;
                let memory = dotnet_runtime_memory::RawMemoryAccess::new(heap);
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
                let mut memory = dotnet_runtime_memory::RawMemoryAccess::new(heap);
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
                let storage = self
                    .statics()
                    .get(metadata.type_desc.clone(), &metadata.generics);
                storage.storage.with_data_mut(|data| {
                    // SAFETY: F10.RawMemoryAccessValid — Static-field metadata keeps `offset` within this live storage.
                    let abs_ptr = unsafe { data.as_mut_ptr().add(offset.as_usize()) };
                    let abs_ptr =
                        NonNull::new(abs_ptr).expect("static storage pointer must be non-null");
                    let mut memory = dotnet_runtime_memory::RawMemoryAccess::new(&self.local.heap);
                    // SAFETY: F10.BorrowedStorageStable — The write guard stabilizes storage, and the offset and width come from the by-ref operation.
                    unsafe { memory.store_atomic_ptr(abs_ptr, value, size, ordering) }
                })
            }

            PointerOrigin::FrameHome(serial) => {
                let ptr = frame_home_window(self, serial, offset.as_usize(), size)?;
                let mut memory = dotnet_runtime_memory::RawMemoryAccess::new(&self.local.heap);
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
                let mut memory = dotnet_runtime_memory::RawMemoryAccess::new(&self.local.heap);
                // SAFETY: F10.BorrowedStorageStable — The write guard provides synchronized access for the by-ref width.
                unsafe { memory.store_atomic_ptr(abs_ptr, value, size, ordering) }
            }),
            PointerOrigin::Unmanaged => {
                let heap = &self.local.heap;
                let mut memory = dotnet_runtime_memory::RawMemoryAccess::new(heap);
                // SAFETY: F10.RawMemoryAccessValid — Unmanaged origin: `offset` is an absolute address and this function's caller
                // guarantees it is valid for the access.
                unsafe { memory.store_atomic(self.gc, None, offset, value, size, ordering) }
            }
            #[cfg(feature = "multithreading")]
            PointerOrigin::CrossArenaObjectRef(origin) => {
                let heap = &self.local.heap;
                let mut memory = dotnet_runtime_memory::RawMemoryAccess::new(heap);
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
                    .get(metadata.type_desc.clone(), &metadata.generics);
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
    use dotnet_runtime_memory::HeapManager;
    use gc_arena::{Arena, Rootable};

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
                assert!(matches!(read, Err(MemoryAccessError::BoundsCheck { .. })));
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
                assert!(matches!(write, Err(MemoryAccessError::BoundsCheck { .. })));
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

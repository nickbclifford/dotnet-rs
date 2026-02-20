use crate::{
    memory::access::MemoryOwner,
    stack::{
        context::VesContext,
        ops::{PoolOps, RawMemoryOps, StaticsOps, VesOps},
    },
    threading::ThreadManagerOps,
};
use dotnet_types::TypeDescription;
use dotnet_utils::{BorrowGuard, BorrowScopeOps};
use dotnet_value::{StackValue, layout::HasLayout, pointer::PointerOrigin};
use sptr::Strict;

impl<'a, 'gc, 'm: 'gc> PoolOps for VesContext<'a, 'gc, 'm> {
    /// # Safety
    ///
    /// The returned pointer is valid for the duration of the current method frame.
    /// It must not be stored in a way that outlives the frame.
    #[inline]
    fn localloc(&mut self, size: usize) -> *mut u8 {
        let s = self.state_mut();
        // Ensure 8-byte alignment for the allocated block
        let current_len = s.memory_pool.len();
        let aligned_len = (current_len + 7) & !7;
        let padding = aligned_len - current_len;

        if padding > 0 {
            s.memory_pool.resize(aligned_len, 0);
        }

        let loc = s.memory_pool.len();
        s.memory_pool.resize(loc + size, 0);
        s.memory_pool[loc..].as_mut_ptr()
    }
}

impl<'a, 'gc, 'm: 'gc> BorrowScopeOps for VesContext<'a, 'gc, 'm> {
    fn enter_borrow_scope(&self) {
        self.local
            .active_borrows
            .set(self.local.active_borrows.get() + 1);
    }

    fn exit_borrow_scope(&self) {
        let current = self.local.active_borrows.get();
        if current == 0 {
            panic!("Borrow scope underflow");
        }
        self.local.active_borrows.set(current - 1);
    }
}

impl<'a, 'gc, 'm: 'gc> RawMemoryOps<'gc> for VesContext<'a, 'gc, 'm> {
    fn resolve_address(
        &self,
        origin: PointerOrigin<'gc>,
        offset: dotnet_utils::ByteOffset,
    ) -> std::ptr::NonNull<u8> {
        let _guard = BorrowGuard::new(self);
        match origin {
            PointerOrigin::Stack(idx) => {
                let slot = self.evaluation_stack.get_slot_ref(idx);
                let base_ptr = slot.data_location().as_ptr();
                unsafe { std::ptr::NonNull::new_unchecked(base_ptr.add(offset.as_usize())) }
            }
            PointerOrigin::Static(static_type, lookup) => {
                let storage = self.statics().get(static_type, &lookup);
                storage.storage.with_data(|data| unsafe {
                    std::ptr::NonNull::new_unchecked(
                        data.as_ptr().add(offset.as_usize()).cast_mut(),
                    )
                })
            }
            PointerOrigin::Heap(owner) => {
                let handle = owner.0.expect("resolve_address: null owner handle");
                let inner = handle.borrow();
                let ptr = unsafe { inner.storage.raw_data_ptr() };
                unsafe { std::ptr::NonNull::new_unchecked(ptr.add(offset.as_usize())) }
            }
            PointerOrigin::Unmanaged => {
                std::ptr::NonNull::new(sptr::from_exposed_addr_mut::<u8>(offset.as_usize()))
                    .expect("resolve_address: null unmanaged pointer (offset=0)")
            }
            #[cfg(feature = "multithreaded-gc")]
            PointerOrigin::CrossArenaObjectRef(ptr, _tid) => {
                let lock = unsafe { ptr.0.as_ref() };
                let guard = lock.borrow();
                let storage = &guard.storage;
                unsafe {
                    std::ptr::NonNull::new_unchecked(storage.raw_data_ptr().add(offset.as_usize()))
                }
            }
            PointerOrigin::Transient(obj) => obj.instance_storage.with_data(|data| unsafe {
                std::ptr::NonNull::new_unchecked(data.as_ptr().add(offset.as_usize()).cast_mut())
            }),
        }
    }

    /// # Safety
    ///
    /// The caller must ensure that `offset` represents a valid memory location relative to `origin`.
    /// The `layout` must match the expected type of `value`.
    #[inline]
    unsafe fn write_unaligned(
        &mut self,
        origin: PointerOrigin<'gc>,
        offset: dotnet_utils::ByteOffset,
        value: StackValue<'gc>,
        layout: &dotnet_value::layout::LayoutManager,
    ) -> Result<(), String> {
        let _guard = BorrowGuard::new(self);
        match origin {
            PointerOrigin::Stack(_idx) => {
                let ptr = self.resolve_address(origin, offset);
                let mut memory = crate::memory::RawMemoryAccess::new(&self.local.heap);
                unsafe { memory.perform_write(self.gc, ptr.as_ptr(), None, value, layout) }
            }
            PointerOrigin::Static(static_type, lookup) => {
                let storage = self.statics().get(static_type, &lookup);
                storage.storage.with_data_mut(|data| {
                    if offset.as_usize() + layout.size().as_usize() > data.len() {
                        return Err("Static storage access out of bounds".to_string());
                    }
                    let ptr = unsafe { data.as_mut_ptr().add(offset.as_usize()) };
                    let mut memory = crate::memory::RawMemoryAccess::new(&self.local.heap);
                    unsafe { memory.perform_write(self.gc, ptr, None, value, layout) }
                })
            }
            PointerOrigin::Heap(owner) => {
                let heap = &self.local.heap;
                let mut memory = crate::memory::RawMemoryAccess::new(heap);
                unsafe {
                    memory.write_unaligned(
                        self.gc,
                        Some(MemoryOwner::Local(owner)),
                        offset,
                        value,
                        layout,
                    )
                }
            }
            PointerOrigin::Unmanaged => {
                let heap = &self.local.heap;
                let mut memory = crate::memory::RawMemoryAccess::new(heap);
                unsafe { memory.write_unaligned(self.gc, None, offset, value, layout) }
            }
            #[cfg(feature = "multithreaded-gc")]
            PointerOrigin::CrossArenaObjectRef(ptr, tid) => {
                let heap = &self.local.heap;
                let mut memory = crate::memory::RawMemoryAccess::new(heap);
                unsafe {
                    memory.write_unaligned(
                        self.gc,
                        Some(MemoryOwner::CrossArena(ptr, tid)),
                        offset,
                        value,
                        layout,
                    )
                }
            }
            PointerOrigin::Transient(obj) => {
                obj.instance_storage.with_data_mut(|data: &mut [u8]| {
                    if offset.as_usize() + layout.size().as_usize() > data.len() {
                        return Err("Transient object access out of bounds".to_string());
                    }
                    let ptr = unsafe { data.as_mut_ptr().add(offset.as_usize()) };
                    let mut memory = crate::memory::RawMemoryAccess::new(&self.local.heap);
                    unsafe { memory.perform_write(self.gc, ptr, None, value, layout) }
                })
            }
        }
    }

    /// # Safety
    ///
    /// The caller must ensure that `offset` represents a valid memory location relative to `origin`.
    /// The `layout` must match the expected type stored at the location.
    #[inline]
    unsafe fn read_unaligned(
        &self,
        origin: PointerOrigin<'gc>,
        offset: dotnet_utils::ByteOffset,
        layout: &dotnet_value::layout::LayoutManager,
        type_desc: Option<TypeDescription>,
    ) -> Result<StackValue<'gc>, String> {
        let _guard = BorrowGuard::new(self);
        match origin {
            PointerOrigin::Stack(_idx) => {
                let ptr = self.resolve_address(origin, offset);
                let memory = crate::memory::RawMemoryAccess::new(&self.local.heap);
                unsafe { memory.perform_read(self.gc, ptr.as_ptr(), None, layout, type_desc) }
            }
            PointerOrigin::Static(static_type, lookup) => {
                let storage = self.statics().get(static_type, &lookup);
                storage.storage.with_data(|data| {
                    if offset.as_usize() + layout.size().as_usize() > data.len() {
                        return Err("Static storage access out of bounds".to_string());
                    }
                    let ptr = unsafe { data.as_ptr().add(offset.as_usize()) };
                    let memory = crate::memory::RawMemoryAccess::new(&self.local.heap);
                    unsafe { memory.perform_read(self.gc, ptr, None, layout, type_desc) }
                })
            }
            PointerOrigin::Heap(owner) => {
                let heap = &self.local.heap;
                let memory = crate::memory::RawMemoryAccess::new(heap);
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
                let memory = crate::memory::RawMemoryAccess::new(heap);
                unsafe { memory.read_unaligned(self.gc, None, offset, layout, type_desc) }
            }
            #[cfg(feature = "multithreaded-gc")]
            PointerOrigin::CrossArenaObjectRef(ptr, tid) => {
                let heap = &self.local.heap;
                let memory = crate::memory::RawMemoryAccess::new(heap);
                unsafe {
                    memory.read_unaligned(
                        self.gc,
                        Some(MemoryOwner::CrossArena(ptr, tid)),
                        offset,
                        layout,
                        type_desc,
                    )
                }
            }
            PointerOrigin::Transient(obj) => obj.instance_storage.with_data(|data: &[u8]| {
                if offset.as_usize() + layout.size().as_usize() > data.len() {
                    return Err("Transient object access out of bounds".to_string());
                }
                let ptr = unsafe { data.as_ptr().add(offset.as_usize()) };
                let memory = crate::memory::RawMemoryAccess::new(&self.local.heap);
                unsafe { memory.perform_read(self.gc, ptr, None, layout, type_desc) }
            }),
        }
    }

    #[inline]
    unsafe fn write_bytes(
        &mut self,
        origin: PointerOrigin<'gc>,
        offset: dotnet_utils::ByteOffset,
        data: &[u8],
    ) -> Result<(), String> {
        let _guard = BorrowGuard::new(self);
        match origin {
            PointerOrigin::Stack(_idx) => {
                let ptr = self.resolve_address(origin, offset);
                let mut memory = crate::memory::RawMemoryAccess::new(&self.local.heap);
                unsafe {
                    memory.write_bytes(
                        self.gc,
                        None,
                        dotnet_utils::ByteOffset(ptr.as_ptr().expose_addr()),
                        data,
                    )
                }
            }
            PointerOrigin::Static(type_desc, lookup) => {
                let storage = self.statics().get(type_desc, &lookup);
                let mut obj_data = storage.storage.get_mut();
                if offset.as_usize() + data.len() > obj_data.len() {
                    return Err("Static storage access out of bounds".to_string());
                }
                obj_data[offset.as_usize()..offset.as_usize() + data.len()].copy_from_slice(data);
                Ok(())
            }
            PointerOrigin::Heap(owner) => {
                let heap = &self.local.heap;
                let mut memory = crate::memory::RawMemoryAccess::new(heap);
                unsafe {
                    memory.write_bytes(self.gc, Some(MemoryOwner::Local(owner)), offset, data)
                }
            }
            PointerOrigin::Unmanaged => {
                let heap = &self.local.heap;
                let mut memory = crate::memory::RawMemoryAccess::new(heap);
                unsafe { memory.write_bytes(self.gc, None, offset, data) }
            }
            #[cfg(feature = "multithreaded-gc")]
            PointerOrigin::CrossArenaObjectRef(ptr, tid) => {
                let heap = &self.local.heap;
                let mut memory = crate::memory::RawMemoryAccess::new(heap);
                unsafe {
                    memory.write_bytes(
                        self.gc,
                        Some(MemoryOwner::CrossArena(ptr, tid)),
                        offset,
                        data,
                    )
                }
            }
            PointerOrigin::Transient(obj) => {
                let mut obj_data = obj.instance_storage.get_mut();
                if offset.as_usize() + data.len() > obj_data.len() {
                    return Err("Transient object access out of bounds".to_string());
                }
                obj_data[offset.as_usize()..offset.as_usize() + data.len()].copy_from_slice(data);
                Ok(())
            }
        }
    }

    #[inline]
    unsafe fn read_bytes(
        &self,
        origin: PointerOrigin<'gc>,
        offset: dotnet_utils::ByteOffset,
        dest: &mut [u8],
    ) -> Result<(), String> {
        let _guard = BorrowGuard::new(self);
        match origin {
            PointerOrigin::Stack(_idx) => {
                let ptr = self.resolve_address(origin, offset);
                let memory = crate::memory::RawMemoryAccess::new(&self.local.heap);
                unsafe {
                    memory.read_bytes(
                        None,
                        dotnet_utils::ByteOffset(ptr.as_ptr().expose_addr()),
                        dest,
                    )
                }
            }
            PointerOrigin::Static(type_desc, lookup) => {
                let storage = self.statics().get(type_desc, &lookup);
                let obj_data = storage.storage.get();
                if offset.as_usize() + dest.len() > obj_data.len() {
                    return Err("Static storage access out of bounds".to_string());
                }
                dest.copy_from_slice(&obj_data[offset.as_usize()..offset.as_usize() + dest.len()]);
                Ok(())
            }
            PointerOrigin::Heap(owner) => {
                let heap = &self.local.heap;
                let memory = crate::memory::RawMemoryAccess::new(heap);
                unsafe { memory.read_bytes(Some(MemoryOwner::Local(owner)), offset, dest) }
            }
            PointerOrigin::Unmanaged => {
                let heap = &self.local.heap;
                let memory = crate::memory::RawMemoryAccess::new(heap);
                unsafe { memory.read_bytes(None, offset, dest) }
            }
            #[cfg(feature = "multithreaded-gc")]
            PointerOrigin::CrossArenaObjectRef(ptr, tid) => {
                let heap = &self.local.heap;
                let memory = crate::memory::RawMemoryAccess::new(heap);
                unsafe { memory.read_bytes(Some(MemoryOwner::CrossArena(ptr, tid)), offset, dest) }
            }
            PointerOrigin::Transient(obj) => {
                let obj_data = obj.instance_storage.get();
                if offset.as_usize() + dest.len() > obj_data.len() {
                    return Err("Transient object access out of bounds".to_string());
                }
                dest.copy_from_slice(&obj_data[offset.as_usize()..offset.as_usize() + dest.len()]);
                Ok(())
            }
        }
    }

    #[inline]
    unsafe fn compare_exchange_atomic(
        &mut self,
        origin: PointerOrigin<'gc>,
        offset: dotnet_utils::ByteOffset,
        expected: u64,
        new: u64,
        size: usize,
        success: dotnet_utils::sync::Ordering,
        failure: dotnet_utils::sync::Ordering,
    ) -> Result<u64, u64> {
        let _guard = BorrowGuard::new(self);
        match origin {
            PointerOrigin::Heap(owner) => {
                let heap = &self.local.heap;
                let mut memory = crate::memory::RawMemoryAccess::new(heap);
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
            PointerOrigin::Static(static_type, lookup) => {
                let storage = self.statics().get(static_type, &lookup);
                let base_ptr = unsafe { storage.storage.raw_data_ptr() };
                let abs_ptr = unsafe { base_ptr.add(offset.as_usize()) };
                let mut memory = crate::memory::RawMemoryAccess::new(&self.local.heap);
                unsafe {
                    memory.compare_exchange_atomic(
                        self.gc,
                        None,
                        dotnet_utils::ByteOffset(abs_ptr.expose_addr()),
                        expected,
                        new,
                        size,
                        success,
                        failure,
                    )
                }
            }
            PointerOrigin::Stack(_idx) => {
                let ptr = self.resolve_address(origin, offset);
                let mut memory = crate::memory::RawMemoryAccess::new(&self.local.heap);
                unsafe {
                    memory.compare_exchange_atomic(
                        self.gc,
                        None,
                        dotnet_utils::ByteOffset(ptr.as_ptr().expose_addr()),
                        expected,
                        new,
                        size,
                        success,
                        failure,
                    )
                }
            }
            PointerOrigin::Transient(obj) => obj.instance_storage.with_data(|data| {
                let base_ptr = data.as_ptr();
                let abs_ptr = unsafe { base_ptr.add(offset.as_usize()) };
                let mut memory = crate::memory::RawMemoryAccess::new(&self.local.heap);
                unsafe {
                    memory.compare_exchange_atomic(
                        self.gc,
                        None,
                        dotnet_utils::ByteOffset(abs_ptr.expose_addr()),
                        expected,
                        new,
                        size,
                        success,
                        failure,
                    )
                }
            }),
            PointerOrigin::Unmanaged => {
                let heap = &self.local.heap;
                let mut memory = crate::memory::RawMemoryAccess::new(heap);
                unsafe {
                    memory.compare_exchange_atomic(
                        self.gc, None, offset, expected, new, size, success, failure,
                    )
                }
            }
            #[cfg(feature = "multithreaded-gc")]
            PointerOrigin::CrossArenaObjectRef(ptr, tid) => {
                let heap = &self.local.heap;
                let mut memory = crate::memory::RawMemoryAccess::new(heap);
                unsafe {
                    memory.compare_exchange_atomic(
                        self.gc,
                        Some(MemoryOwner::CrossArena(ptr, tid)),
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
        offset: dotnet_utils::ByteOffset,
        value: u64,
        size: usize,
        ordering: dotnet_utils::sync::Ordering,
    ) -> Result<u64, String> {
        let _guard = BorrowGuard::new(self);
        match origin {
            PointerOrigin::Heap(owner) => {
                let heap = &self.local.heap;
                let mut memory = crate::memory::RawMemoryAccess::new(heap);
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
            PointerOrigin::Static(static_type, lookup) => {
                let storage = self.statics().get(static_type, &lookup);
                let base_ptr = unsafe { storage.storage.raw_data_ptr() };
                let abs_ptr = unsafe { base_ptr.add(offset.as_usize()) };
                let mut memory = crate::memory::RawMemoryAccess::new(&self.local.heap);
                unsafe {
                    memory.exchange_atomic(
                        self.gc,
                        None,
                        dotnet_utils::ByteOffset(abs_ptr.expose_addr()),
                        value,
                        size,
                        ordering,
                    )
                }
            }
            PointerOrigin::Stack(_idx) => {
                let ptr = self.resolve_address(origin, offset);
                let mut memory = crate::memory::RawMemoryAccess::new(&self.local.heap);
                unsafe {
                    memory.exchange_atomic(
                        self.gc,
                        None,
                        dotnet_utils::ByteOffset(ptr.as_ptr().expose_addr()),
                        value,
                        size,
                        ordering,
                    )
                }
            }
            PointerOrigin::Transient(obj) => obj.instance_storage.with_data(|data| {
                let base_ptr = data.as_ptr();
                let abs_ptr = unsafe { base_ptr.add(offset.as_usize()) };
                let mut memory = crate::memory::RawMemoryAccess::new(&self.local.heap);
                unsafe {
                    memory.exchange_atomic(
                        self.gc,
                        None,
                        dotnet_utils::ByteOffset(abs_ptr.expose_addr()),
                        value,
                        size,
                        ordering,
                    )
                }
            }),
            PointerOrigin::Unmanaged => {
                let heap = &self.local.heap;
                let mut memory = crate::memory::RawMemoryAccess::new(heap);
                unsafe { memory.exchange_atomic(self.gc, None, offset, value, size, ordering) }
            }
            #[cfg(feature = "multithreaded-gc")]
            PointerOrigin::CrossArenaObjectRef(ptr, tid) => {
                let heap = &self.local.heap;
                let mut memory = crate::memory::RawMemoryAccess::new(heap);
                unsafe {
                    memory.exchange_atomic(
                        self.gc,
                        Some(MemoryOwner::CrossArena(ptr, tid)),
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
        offset: dotnet_utils::ByteOffset,
        size: usize,
        ordering: dotnet_utils::sync::Ordering,
    ) -> Result<u64, String> {
        let _guard = BorrowGuard::new(self);
        match origin {
            PointerOrigin::Heap(owner) => {
                let heap = &self.local.heap;
                let memory = crate::memory::RawMemoryAccess::new(heap);
                unsafe {
                    memory.load_atomic(Some(MemoryOwner::Local(owner)), offset, size, ordering)
                }
            }
            PointerOrigin::Static(static_type, lookup) => {
                let storage = self.statics().get(static_type, &lookup);
                let base_ptr = unsafe { storage.storage.raw_data_ptr() };
                let abs_ptr = unsafe { base_ptr.add(offset.as_usize()) };
                let memory = crate::memory::RawMemoryAccess::new(&self.local.heap);
                unsafe {
                    memory.load_atomic(
                        None,
                        dotnet_utils::ByteOffset(abs_ptr.expose_addr()),
                        size,
                        ordering,
                    )
                }
            }
            PointerOrigin::Stack(_idx) => {
                let ptr = self.resolve_address(origin, offset);
                let memory = crate::memory::RawMemoryAccess::new(&self.local.heap);
                unsafe {
                    memory.load_atomic(
                        None,
                        dotnet_utils::ByteOffset(ptr.as_ptr().expose_addr()),
                        size,
                        ordering,
                    )
                }
            }
            PointerOrigin::Transient(obj) => obj.instance_storage.with_data(|data| {
                let base_ptr = data.as_ptr();
                let abs_ptr = unsafe { base_ptr.add(offset.as_usize()) };
                let memory = crate::memory::RawMemoryAccess::new(&self.local.heap);
                unsafe {
                    memory.load_atomic(
                        None,
                        dotnet_utils::ByteOffset(abs_ptr.expose_addr()),
                        size,
                        ordering,
                    )
                }
            }),
            PointerOrigin::Unmanaged => {
                let heap = &self.local.heap;
                let memory = crate::memory::RawMemoryAccess::new(heap);
                unsafe { memory.load_atomic(None, offset, size, ordering) }
            }
            #[cfg(feature = "multithreaded-gc")]
            PointerOrigin::CrossArenaObjectRef(ptr, tid) => {
                let heap = &self.local.heap;
                let memory = crate::memory::RawMemoryAccess::new(heap);
                unsafe {
                    memory.load_atomic(
                        Some(MemoryOwner::CrossArena(ptr, tid)),
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
        offset: dotnet_utils::ByteOffset,
        value: u64,
        size: usize,
        ordering: dotnet_utils::sync::Ordering,
    ) -> Result<(), String> {
        let _guard = BorrowGuard::new(self);
        match origin {
            PointerOrigin::Heap(owner) => {
                let heap = &self.local.heap;
                let mut memory = crate::memory::RawMemoryAccess::new(heap);
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
            PointerOrigin::Static(static_type, lookup) => {
                let storage = self.statics().get(static_type, &lookup);
                let base_ptr = unsafe { storage.storage.raw_data_ptr() };
                let abs_ptr = unsafe { base_ptr.add(offset.as_usize()) };
                let mut memory = crate::memory::RawMemoryAccess::new(&self.local.heap);
                unsafe {
                    memory.store_atomic(
                        self.gc,
                        None,
                        dotnet_utils::ByteOffset(abs_ptr.expose_addr()),
                        value,
                        size,
                        ordering,
                    )
                }
            }
            PointerOrigin::Stack(_idx) => {
                let ptr = self.resolve_address(origin, offset);
                let mut memory = crate::memory::RawMemoryAccess::new(&self.local.heap);
                unsafe {
                    memory.store_atomic(
                        self.gc,
                        None,
                        dotnet_utils::ByteOffset(ptr.as_ptr().expose_addr()),
                        value,
                        size,
                        ordering,
                    )
                }
            }
            PointerOrigin::Transient(obj) => obj.instance_storage.with_data(|data| {
                let base_ptr = data.as_ptr();
                let abs_ptr = unsafe { base_ptr.add(offset.as_usize()) };
                let mut memory = crate::memory::RawMemoryAccess::new(&self.local.heap);
                unsafe {
                    memory.store_atomic(
                        self.gc,
                        None,
                        dotnet_utils::ByteOffset(abs_ptr.expose_addr()),
                        value,
                        size,
                        ordering,
                    )
                }
            }),
            PointerOrigin::Unmanaged => {
                let heap = &self.local.heap;
                let mut memory = crate::memory::RawMemoryAccess::new(heap);
                unsafe { memory.store_atomic(self.gc, None, offset, value, size, ordering) }
            }
            #[cfg(feature = "multithreaded-gc")]
            PointerOrigin::CrossArenaObjectRef(ptr, tid) => {
                let heap = &self.local.heap;
                let mut memory = crate::memory::RawMemoryAccess::new(heap);
                unsafe {
                    memory.store_atomic(
                        self.gc,
                        Some(MemoryOwner::CrossArena(ptr, tid)),
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
    fn check_gc_safe_point(&self) {
        if self.local.active_borrows.get() > 0 {
            // Cannot reach safe point while holding borrows!
            return;
        }
        let thread_manager = &self.shared.thread_manager;
        if thread_manager.is_gc_stop_requested() {
            let managed_id = self.thread_id.get();
            if managed_id != dotnet_utils::ArenaId::INVALID {
                #[cfg(feature = "multithreaded-gc")]
                thread_manager.safe_point(managed_id, &self.shared.gc_coordinator);
                #[cfg(not(feature = "multithreaded-gc"))]
                thread_manager.safe_point(managed_id, &Default::default());
            }
        }
    }
}

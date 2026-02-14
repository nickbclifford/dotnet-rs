use crate::{
    stack::{
        context::VesContext,
        ops::{PoolOps, RawMemoryOps, StaticsOps, VesOps},
    },
    threading::ThreadManagerOps,
};
use dotnet_types::TypeDescription;
use dotnet_value::{StackValue, layout::HasLayout, pointer::PointerOrigin};

impl<'a, 'gc, 'm: 'gc> PoolOps for VesContext<'a, 'gc, 'm> {
    /// # Safety
    ///
    /// The returned pointer is valid for the duration of the current method frame.
    /// It must not be stored in a way that outlives the frame.
    #[inline]
    fn localloc(&mut self, size: usize) -> *mut u8 {
        let s = self.state_mut();
        let loc = s.memory_pool.len();
        s.memory_pool.extend(vec![0; size]);
        s.memory_pool[loc..].as_mut_ptr()
    }
}

impl<'a, 'gc, 'm: 'gc> RawMemoryOps<'gc> for VesContext<'a, 'gc, 'm> {
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
        match origin {
            PointerOrigin::Stack(idx, base_offset) => {
                let slot = self.evaluation_stack.get_slot_ref(idx);
                let base_ptr = slot.data_location().as_ptr();
                let total_offset = base_offset + offset;
                let mut memory = crate::memory::RawMemoryAccess::new(&self.local.heap);
                unsafe {
                    memory.perform_write(base_ptr.add(total_offset.as_usize()), None, value, layout)
                }
            }
            PointerOrigin::Static(static_type, lookup) => {
                let storage = self.statics().get(static_type, &lookup);
                storage.storage.with_data_mut(|data| {
                    if offset.as_usize() + layout.size().as_usize() > data.len() {
                        return Err("Static storage access out of bounds".to_string());
                    }
                    let ptr = unsafe { data.as_mut_ptr().add(offset.as_usize()) };
                    let mut memory = crate::memory::RawMemoryAccess::new(&self.local.heap);
                    unsafe { memory.perform_write(ptr, None, value, layout) }
                })
            }
            PointerOrigin::Heap(owner) => {
                let heap = &self.local.heap;
                let mut memory = crate::memory::RawMemoryAccess::new(heap);
                unsafe { memory.write_unaligned(self.gc, Some(owner), offset, value, layout) }
            }
            PointerOrigin::Unmanaged => {
                let heap = &self.local.heap;
                let mut memory = crate::memory::RawMemoryAccess::new(heap);
                unsafe { memory.write_unaligned(self.gc, None, offset, value, layout) }
            }
            #[cfg(feature = "multithreaded-gc")]
            PointerOrigin::CrossArenaObjectRef(ptr, _tid) => {
                let lock = unsafe { ptr.0.as_ref() };
                let mut guard = lock.borrow_mut(&self.gc);
                let storage = &mut guard.storage;
                let storage_slice: &mut [u8] = unsafe {
                    std::slice::from_raw_parts_mut(storage.raw_data_ptr(), storage.size_bytes())
                };
                if offset.as_usize() + layout.size().as_usize() > storage_slice.len() {
                    return Err("Cross-arena access out of bounds".to_string());
                }
                let ptr = unsafe { storage_slice.as_mut_ptr().add(offset.as_usize()) };
                let mut memory = crate::memory::RawMemoryAccess::new(&self.local.heap);
                unsafe { memory.perform_write(ptr, None, value, layout) }
            }
            PointerOrigin::Transient(obj) => {
                obj.instance_storage.with_data_mut(|data: &mut [u8]| {
                    if offset.as_usize() + layout.size().as_usize() > data.len() {
                        return Err("Transient object access out of bounds".to_string());
                    }
                    let ptr = unsafe { data.as_mut_ptr().add(offset.as_usize()) };
                    let mut memory = crate::memory::RawMemoryAccess::new(&self.local.heap);
                    unsafe { memory.perform_write(ptr, None, value, layout) }
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
        match origin {
            PointerOrigin::Stack(idx, base_offset) => {
                let slot = self.evaluation_stack.get_slot_ref(idx);
                let base_ptr = slot.data_location().as_ptr();
                let total_offset = base_offset + offset;
                let memory = crate::memory::RawMemoryAccess::new(&self.local.heap);
                unsafe {
                    memory.perform_read(
                        self.gc,
                        base_ptr.add(total_offset.as_usize()),
                        None,
                        layout,
                        type_desc,
                    )
                }
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
                unsafe { memory.read_unaligned(self.gc, Some(owner), offset, layout, type_desc) }
            }
            PointerOrigin::Unmanaged => {
                let heap = &self.local.heap;
                let memory = crate::memory::RawMemoryAccess::new(heap);
                unsafe { memory.read_unaligned(self.gc, None, offset, layout, type_desc) }
            }
            #[cfg(feature = "multithreaded-gc")]
            PointerOrigin::CrossArenaObjectRef(ptr, _tid) => {
                let lock = unsafe { ptr.0.as_ref() };
                let guard = lock.borrow();
                let storage = &guard.storage;
                let storage_slice: &[u8] = unsafe {
                    std::slice::from_raw_parts(storage.raw_data_ptr(), storage.size_bytes())
                };
                if offset.as_usize() + layout.size().as_usize() > storage_slice.len() {
                    return Err("Cross-arena access out of bounds".to_string());
                }
                let ptr = unsafe { storage_slice.as_ptr().add(offset.as_usize()) };
                let memory = crate::memory::RawMemoryAccess::new(&self.local.heap);
                unsafe { memory.perform_read(self.gc, ptr, None, layout, type_desc) }
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
        match origin {
            PointerOrigin::Stack(idx, base_offset) => {
                let slot = self.evaluation_stack.get_slot_ref(idx);
                let base_ptr = slot.data_location().as_ptr();
                let total_offset = base_offset + offset;
                let mut memory = crate::memory::RawMemoryAccess::new(&self.local.heap);
                unsafe {
                    memory.write_bytes(
                        self.gc,
                        None,
                        dotnet_utils::ByteOffset(base_ptr.add(total_offset.as_usize()) as usize),
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
                unsafe { memory.write_bytes(self.gc, Some(owner), offset, data) }
            }
            PointerOrigin::Unmanaged => {
                let heap = &self.local.heap;
                let mut memory = crate::memory::RawMemoryAccess::new(heap);
                unsafe { memory.write_bytes(self.gc, None, offset, data) }
            }
            #[cfg(feature = "multithreaded-gc")]
            PointerOrigin::CrossArenaObjectRef(ptr, _tid) => {
                let lock = unsafe { ptr.0.as_ref() };
                let mut guard = lock.borrow_mut(&self.gc);
                let storage = &mut guard.storage;
                let storage_slice: &mut [u8] = unsafe {
                    std::slice::from_raw_parts_mut(storage.raw_data_ptr(), storage.size_bytes())
                };
                if offset.as_usize() + data.len() > storage_slice.len() {
                    return Err("Cross-arena access out of bounds".to_string());
                }
                storage_slice[offset.as_usize()..offset.as_usize() + data.len()]
                    .copy_from_slice(data);
                Ok(())
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
        match origin {
            PointerOrigin::Stack(idx, base_offset) => {
                let slot = self.evaluation_stack.get_slot_ref(idx);
                let base_ptr = slot.data_location().as_ptr();
                let total_offset = base_offset + offset;
                let memory = crate::memory::RawMemoryAccess::new(&self.local.heap);
                unsafe {
                    memory.read_bytes(
                        None,
                        dotnet_utils::ByteOffset(base_ptr.add(total_offset.as_usize()) as usize),
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
                unsafe { memory.read_bytes(Some(owner), offset, dest) }
            }
            PointerOrigin::Unmanaged => {
                let heap = &self.local.heap;
                let memory = crate::memory::RawMemoryAccess::new(heap);
                unsafe { memory.read_bytes(None, offset, dest) }
            }
            #[cfg(feature = "multithreaded-gc")]
            PointerOrigin::CrossArenaObjectRef(ptr, _tid) => {
                let lock = unsafe { ptr.0.as_ref() };
                let guard = lock.borrow();
                let storage = &guard.storage;
                let storage_slice: &[u8] = unsafe {
                    std::slice::from_raw_parts(storage.raw_data_ptr(), storage.size_bytes())
                };
                if offset.as_usize() + dest.len() > storage_slice.len() {
                    return Err("Cross-arena access out of bounds".to_string());
                }
                dest.copy_from_slice(
                    &storage_slice[offset.as_usize()..offset.as_usize() + dest.len()],
                );
                Ok(())
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
        match origin {
            PointerOrigin::Heap(owner) => {
                let heap = &self.local.heap;
                let mut memory = crate::memory::RawMemoryAccess::new(heap);
                unsafe {
                    memory.compare_exchange_atomic(
                        self.gc,
                        Some(owner),
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
                        dotnet_utils::ByteOffset(abs_ptr as usize),
                        expected,
                        new,
                        size,
                        success,
                        failure,
                    )
                }
            }
            PointerOrigin::Stack(idx, base_offset) => {
                let slot = self.evaluation_stack.get_slot_ref(idx);
                let base_ptr = slot.data_location().as_ptr();
                let abs_ptr = unsafe { base_ptr.add((base_offset + offset).as_usize()) };
                let mut memory = crate::memory::RawMemoryAccess::new(&self.local.heap);
                unsafe {
                    memory.compare_exchange_atomic(
                        self.gc,
                        None,
                        dotnet_utils::ByteOffset(abs_ptr as usize),
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
                        dotnet_utils::ByteOffset(abs_ptr as usize),
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
            PointerOrigin::CrossArenaObjectRef(ptr, _tid) => {
                let lock = unsafe { ptr.0.as_ref() };
                let guard = lock.borrow();
                let base_ptr = unsafe { guard.storage.raw_data_ptr() };
                let abs_ptr = unsafe { base_ptr.add(offset.as_usize()) };
                let mut memory = crate::memory::RawMemoryAccess::new(&self.local.heap);
                unsafe {
                    memory.compare_exchange_atomic(
                        self.gc,
                        None,
                        dotnet_utils::ByteOffset(abs_ptr as usize),
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
        match origin {
            PointerOrigin::Heap(owner) => {
                let heap = &self.local.heap;
                let mut memory = crate::memory::RawMemoryAccess::new(heap);
                unsafe {
                    memory.exchange_atomic(self.gc, Some(owner), offset, value, size, ordering)
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
                        dotnet_utils::ByteOffset(abs_ptr as usize),
                        value,
                        size,
                        ordering,
                    )
                }
            }
            PointerOrigin::Stack(idx, base_offset) => {
                let slot = self.evaluation_stack.get_slot_ref(idx);
                let base_ptr = slot.data_location().as_ptr();
                let abs_ptr = unsafe { base_ptr.add((base_offset + offset).as_usize()) };
                let mut memory = crate::memory::RawMemoryAccess::new(&self.local.heap);
                unsafe {
                    memory.exchange_atomic(
                        self.gc,
                        None,
                        dotnet_utils::ByteOffset(abs_ptr as usize),
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
                        dotnet_utils::ByteOffset(abs_ptr as usize),
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
            PointerOrigin::CrossArenaObjectRef(ptr, _tid) => {
                let lock = unsafe { ptr.0.as_ref() };
                let guard = lock.borrow();
                let base_ptr = unsafe { guard.storage.raw_data_ptr() };
                let abs_ptr = unsafe { base_ptr.add(offset.as_usize()) };
                let mut memory = crate::memory::RawMemoryAccess::new(&self.local.heap);
                unsafe {
                    memory.exchange_atomic(
                        self.gc,
                        None,
                        dotnet_utils::ByteOffset(abs_ptr as usize),
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
        match origin {
            PointerOrigin::Heap(owner) => {
                let heap = &self.local.heap;
                let memory = crate::memory::RawMemoryAccess::new(heap);
                unsafe { memory.load_atomic(Some(owner), offset, size, ordering) }
            }
            PointerOrigin::Static(static_type, lookup) => {
                let storage = self.statics().get(static_type, &lookup);
                let base_ptr = unsafe { storage.storage.raw_data_ptr() };
                let abs_ptr = unsafe { base_ptr.add(offset.as_usize()) };
                let memory = crate::memory::RawMemoryAccess::new(&self.local.heap);
                unsafe {
                    memory.load_atomic(
                        None,
                        dotnet_utils::ByteOffset(abs_ptr as usize),
                        size,
                        ordering,
                    )
                }
            }
            PointerOrigin::Stack(idx, base_offset) => {
                let slot = self.evaluation_stack.get_slot_ref(idx);
                let base_ptr = slot.data_location().as_ptr();
                let abs_ptr = unsafe { base_ptr.add((base_offset + offset).as_usize()) };
                let memory = crate::memory::RawMemoryAccess::new(&self.local.heap);
                unsafe {
                    memory.load_atomic(
                        None,
                        dotnet_utils::ByteOffset(abs_ptr as usize),
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
                        dotnet_utils::ByteOffset(abs_ptr as usize),
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
            PointerOrigin::CrossArenaObjectRef(ptr, _tid) => {
                let lock = unsafe { ptr.0.as_ref() };
                let guard = lock.borrow();
                let base_ptr = unsafe { guard.storage.raw_data_ptr() };
                let abs_ptr = unsafe { base_ptr.add(offset.as_usize()) };
                let memory = crate::memory::RawMemoryAccess::new(&self.local.heap);
                unsafe {
                    memory.load_atomic(
                        None,
                        dotnet_utils::ByteOffset(abs_ptr as usize),
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
        match origin {
            PointerOrigin::Heap(owner) => {
                let heap = &self.local.heap;
                let mut memory = crate::memory::RawMemoryAccess::new(heap);
                unsafe { memory.store_atomic(self.gc, Some(owner), offset, value, size, ordering) }
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
                        dotnet_utils::ByteOffset(abs_ptr as usize),
                        value,
                        size,
                        ordering,
                    )
                }
            }
            PointerOrigin::Stack(idx, base_offset) => {
                let slot = self.evaluation_stack.get_slot_ref(idx);
                let base_ptr = slot.data_location().as_ptr();
                let abs_ptr = unsafe { base_ptr.add((base_offset + offset).as_usize()) };
                let mut memory = crate::memory::RawMemoryAccess::new(&self.local.heap);
                unsafe {
                    memory.store_atomic(
                        self.gc,
                        None,
                        dotnet_utils::ByteOffset(abs_ptr as usize),
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
                        dotnet_utils::ByteOffset(abs_ptr as usize),
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
            PointerOrigin::CrossArenaObjectRef(ptr, _tid) => {
                let lock = unsafe { ptr.0.as_ref() };
                let guard = lock.borrow();
                let base_ptr = unsafe { guard.storage.raw_data_ptr() };
                let abs_ptr = unsafe { base_ptr.add(offset.as_usize()) };
                let mut memory = crate::memory::RawMemoryAccess::new(&self.local.heap);
                unsafe {
                    memory.store_atomic(
                        self.gc,
                        None,
                        dotnet_utils::ByteOffset(abs_ptr as usize),
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

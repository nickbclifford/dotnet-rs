use crate::{
    stack::{
        context::VesContext,
        ops::{PoolOps, RawMemoryOps, VesOps},
    },
    threading::ThreadManagerOps,
};
use dotnet_types::TypeDescription;
use dotnet_value::{StackValue, object::ObjectRef};

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
    /// The caller must ensure that `ptr` is valid for writes and properly represents the memory location.
    /// If `owner` is provided, it must be the object that contains `ptr` to ensure GC safety.
    /// The `layout` must match the expected type of `value`.
    #[inline]
    unsafe fn write_unaligned(
        &mut self,
        ptr: *mut u8,
        owner: Option<ObjectRef<'gc>>,
        value: StackValue<'gc>,
        layout: &dotnet_value::layout::LayoutManager,
    ) -> Result<(), String> {
        let heap = &self.local.heap;
        let mut memory = crate::memory::RawMemoryAccess::new(heap);
        // SAFETY: The caller of RawMemoryOps::write_unaligned must ensure ptr is valid.
        // We delegate to RawMemoryAccess which performs owner-based validation.
        unsafe { memory.write_unaligned(ptr, owner, value, layout) }
    }

    /// # Safety
    ///
    /// The caller must ensure that `ptr` is valid for reads and points to a memory location.
    /// If `owner` is provided, it must be the object that contains `ptr` to ensure GC safety.
    /// The `layout` must match the expected type stored at `ptr`.
    #[inline]
    unsafe fn read_unaligned(
        &self,
        ptr: *const u8,
        owner: Option<ObjectRef<'gc>>,
        layout: &dotnet_value::layout::LayoutManager,
        type_desc: Option<TypeDescription>,
    ) -> Result<StackValue<'gc>, String> {
        let heap = &self.local.heap;
        let memory = crate::memory::RawMemoryAccess::new(heap);
        // SAFETY: The caller of RawMemoryOps::read_unaligned must ensure ptr is valid.
        // We delegate to RawMemoryAccess which performs owner-based validation.
        unsafe { memory.read_unaligned(ptr, owner, layout, type_desc) }
    }

    #[inline]
    fn check_gc_safe_point(&self) {
        let thread_manager = &self.shared.thread_manager;
        if thread_manager.is_gc_stop_requested() {
            let managed_id = self.thread_id.get();
            if managed_id != 0 {
                #[cfg(feature = "multithreaded-gc")]
                thread_manager.safe_point(managed_id, &self.shared.gc_coordinator);
                #[cfg(not(feature = "multithreaded-gc"))]
                thread_manager.safe_point(managed_id, &Default::default());
            }
        }
    }
}

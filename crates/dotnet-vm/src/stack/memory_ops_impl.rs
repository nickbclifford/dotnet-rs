use crate::stack::context::VesContext;
use dotnet_runtime_memory::ops::MemoryOps;

impl<'a, 'gc> MemoryOps<'gc> for VesContext<'a, 'gc> {
    #[inline]
    fn heap(&self) -> &dotnet_runtime_memory::heap::HeapManager<'gc> {
        &self.local.heap
    }

    fn gc_handles(&self) -> &dotnet_runtime_memory::handles::SharedGcHandles {
        &self.shared.gc_handles
    }

    fn has_global_pending_finalizers(&self) -> bool {
        self.shared.has_pending_finalizers()
    }

    fn set_finalizer_registered(
        &self,
        object: dotnet_value::object::ObjectRef<'gc>,
        registered: bool,
    ) {
        #[cfg(feature = "multithreading")]
        if self
            .shared
            .finalizer_started
            .load(dotnet_utils::sync::Ordering::Acquire)
        {
            self.shared.finalizers.set_registered(object, registered);
            return;
        }
        if let Some(handle) = object.0 {
            use dotnet_runtime_memory::ops::BaseMemoryOps;
            if let Some(inner) = dotnet_value::ThreadSafeLock::borrow_mut(
                handle,
                &self.gc_with_token(&self.no_active_borrows_token()),
            )
            .storage
            .as_obj_mut()
            {
                inner.finalizer_suppressed = !registered;
                if registered {
                    let mut queue = self.local.heap.finalization_queue.borrow_mut();
                    if !queue.contains(&object) {
                        queue.push(object);
                    }
                }
            }
        }
    }
}

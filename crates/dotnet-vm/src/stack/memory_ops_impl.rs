use crate::stack::context::VesContext;
use dotnet_runtime_memory::ops::MemoryOps;

impl<'a, 'gc> MemoryOps<'gc> for VesContext<'a, 'gc> {
    #[inline]
    fn heap(&self) -> &dotnet_runtime_memory::heap::HeapManager<'gc> {
        &self.local.heap
    }
}

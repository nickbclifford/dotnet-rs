use crate::{memory::ops::MemoryOps, stack::context::VesContext};

impl<'a, 'gc> MemoryOps<'gc> for VesContext<'a, 'gc> {
    #[inline]
    fn heap(&self) -> &crate::memory::heap::HeapManager<'gc> {
        &self.local.heap
    }
}

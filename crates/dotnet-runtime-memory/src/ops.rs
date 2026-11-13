//! Memory and heap operations.
//!
//! This module defines the [`MemoryOps`] trait which provides an abstraction over
//! object allocation and heap management.
use crate::heap::HeapManager;

pub use dotnet_vm_ops::ops::MemoryOps as BaseMemoryOps;

pub trait MemoryOps<'gc>: BaseMemoryOps<'gc> {
    fn heap(&self) -> &HeapManager<'gc>;
    fn gc_handles(&self) -> &crate::handles::SharedGcHandles;
    fn has_global_pending_finalizers(&self) -> bool;
    fn set_finalizer_registered(
        &self,
        object: dotnet_value::object::ObjectRef<'gc>,
        registered: bool,
    );
}

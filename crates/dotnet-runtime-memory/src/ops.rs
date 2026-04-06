//! Memory and heap operations.
//!
//! This module defines the [`MemoryOps`] trait which provides an abstraction over
//! object allocation and heap management.
use crate::heap::HeapManager;

pub use dotnet_vm_ops::ops::MemoryOps as BaseMemoryOps;

pub trait MemoryOps<'gc>: BaseMemoryOps<'gc> {
    fn heap(&self) -> &HeapManager<'gc>;
}

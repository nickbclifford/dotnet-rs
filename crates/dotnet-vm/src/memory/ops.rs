//! Memory and heap operations.
//!
//! This module defines the [`MemoryOps`] trait which provides an abstraction over
//! object allocation and heap management.

use dotnet_value::StackValue;
use dotnet_value::object::{Object as ObjectInstance, ObjectRef, CTSValue, ValueType, Vector};
use dotnet_types::TypeDescription;
use dotnet_types::generics::ConcreteType;
use dotnet_utils::gc::GCHandle;

pub trait MemoryOps<'gc> {
    fn new_vector(&self, element: ConcreteType, size: usize) -> Vector<'gc>;
    fn new_object(&self, td: TypeDescription) -> ObjectInstance<'gc>;
    fn new_value_type(&self, t: &ConcreteType, data: StackValue<'gc>) -> ValueType<'gc>;
    fn new_cts_value(&self, t: &ConcreteType, data: StackValue<'gc>) -> CTSValue<'gc>;
    fn read_cts_value(&self, t: &ConcreteType, data: &[u8], gc: GCHandle<'gc>) -> CTSValue<'gc>;
    fn register_new_object(&self, instance: &ObjectRef<'gc>);
    fn heap(&self) -> &crate::memory::heap::HeapManager<'gc>;
}

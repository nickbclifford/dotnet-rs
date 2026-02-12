//! Memory and heap operations.
//!
//! This module defines the [`MemoryOps`] trait which provides an abstraction over
//! object allocation and heap management.
use dotnet_types::{TypeDescription, generics::ConcreteType};
use dotnet_utils::gc::GCHandle;
use dotnet_value::{
    StackValue,
    object::{CTSValue, Object as ObjectInstance, ObjectRef, ValueType, Vector},
};

pub trait MemoryOps<'gc> {
    fn gc(&self) -> GCHandle<'gc>;
    #[must_use]
    fn new_vector(&self, element: ConcreteType, size: usize) -> Vector<'gc>;
    #[must_use]
    fn new_object(&self, td: TypeDescription) -> ObjectInstance<'gc>;
    #[must_use]
    fn new_value_type(&self, t: &ConcreteType, data: StackValue<'gc>) -> ValueType<'gc>;
    #[must_use]
    fn new_cts_value(&self, t: &ConcreteType, data: StackValue<'gc>) -> CTSValue<'gc>;
    #[must_use]
    fn read_cts_value(&self, t: &ConcreteType, data: &[u8]) -> CTSValue<'gc>;
    #[must_use]
    fn clone_object(&self, obj: ObjectRef<'gc>) -> ObjectRef<'gc>;
    fn register_new_object(&self, instance: &ObjectRef<'gc>);
    fn heap(&self) -> &crate::memory::heap::HeapManager<'gc>;
}

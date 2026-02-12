//! Memory and heap operations.
//!
//! This module defines the [`MemoryOps`] trait which provides an abstraction over
//! object allocation and heap management.
use dotnet_types::{TypeDescription, error::TypeResolutionError, generics::ConcreteType};
use dotnet_utils::gc::GCHandle;
use dotnet_value::{
    StackValue,
    object::{CTSValue, Object as ObjectInstance, ObjectRef, ValueType, Vector},
};

pub trait MemoryOps<'gc> {
    fn gc(&self) -> GCHandle<'gc>;
    fn new_vector(
        &self,
        element: ConcreteType,
        size: usize,
    ) -> Result<Vector<'gc>, TypeResolutionError>;
    fn new_object(&self, td: TypeDescription) -> Result<ObjectInstance<'gc>, TypeResolutionError>;
    fn new_value_type(
        &self,
        t: &ConcreteType,
        data: StackValue<'gc>,
    ) -> Result<ValueType<'gc>, TypeResolutionError>;
    fn new_cts_value(
        &self,
        t: &ConcreteType,
        data: StackValue<'gc>,
    ) -> Result<CTSValue<'gc>, TypeResolutionError>;
    fn read_cts_value(
        &self,
        t: &ConcreteType,
        data: &[u8],
    ) -> Result<CTSValue<'gc>, TypeResolutionError>;
    #[must_use]
    fn clone_object(&self, obj: ObjectRef<'gc>) -> ObjectRef<'gc>;
    fn register_new_object(&self, instance: &ObjectRef<'gc>);
    fn heap(&self) -> &crate::memory::heap::HeapManager<'gc>;
}

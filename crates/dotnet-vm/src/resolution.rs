use crate::context::ResolutionContext;
use dotnet_types::{TypeDescription, generics::ConcreteType};
use dotnet_utils::gc::GCHandle;
use dotnet_value::{
    StackValue,
    object::{CTSValue, Object, ValueType, Vector},
    storage::FieldStorage,
};

pub trait TypeResolutionExt {
    fn is_value_type(&self, ctx: &ResolutionContext) -> bool;
    fn has_finalizer(&self, ctx: &ResolutionContext) -> bool;
}

pub trait ValueResolution {
    fn stack_value_type(&self, val: &StackValue) -> TypeDescription;
    fn new_object<'gc>(&self, td: TypeDescription) -> Object<'gc>;
    fn new_instance_fields(&self, td: TypeDescription) -> FieldStorage;
    fn new_static_fields(&self, td: TypeDescription) -> FieldStorage;
    fn new_value_type<'gc>(&self, t: &ConcreteType, data: StackValue<'gc>) -> ValueType<'gc>;
    fn value_type_description<'gc>(&self, vt: &ValueType<'gc>) -> TypeDescription;
    fn new_cts_value<'gc>(&self, t: &ConcreteType, data: StackValue<'gc>) -> CTSValue<'gc>;
    fn read_cts_value<'gc>(
        &self,
        t: &ConcreteType,
        data: &[u8],
        gc: GCHandle<'gc>,
    ) -> CTSValue<'gc>;
    fn new_vector<'gc>(&self, element: ConcreteType, size: usize) -> Vector<'gc>;
}

impl<'a, 'm> ValueResolution for ResolutionContext<'a, 'm> {
    fn stack_value_type(&self, val: &StackValue) -> TypeDescription {
        self.resolver().stack_value_type(val)
    }

    fn new_object<'gc>(&self, td: TypeDescription) -> Object<'gc> {
        self.resolver().new_object(td, self)
    }

    fn new_instance_fields(&self, td: TypeDescription) -> FieldStorage {
        self.resolver().new_instance_fields(td, self)
    }

    fn new_static_fields(&self, td: TypeDescription) -> FieldStorage {
        self.resolver().new_static_fields(td, self)
    }

    fn new_value_type<'gc>(&self, t: &ConcreteType, data: StackValue<'gc>) -> ValueType<'gc> {
        self.resolver().new_value_type(t, data, self)
    }

    fn value_type_description<'gc>(&self, vt: &ValueType<'gc>) -> TypeDescription {
        self.resolver().value_type_description(vt)
    }

    fn new_cts_value<'gc>(&self, t: &ConcreteType, data: StackValue<'gc>) -> CTSValue<'gc> {
        self.resolver().new_cts_value(t, data, self)
    }

    fn read_cts_value<'gc>(
        &self,
        t: &ConcreteType,
        data: &[u8],
        gc: GCHandle<'gc>,
    ) -> CTSValue<'gc> {
        self.resolver().read_cts_value(t, data, gc, self)
    }

    fn new_vector<'gc>(&self, element: ConcreteType, size: usize) -> Vector<'gc> {
        self.resolver().new_vector(element, size, self)
    }
}

impl TypeResolutionExt for TypeDescription {
    fn is_value_type(&self, ctx: &ResolutionContext) -> bool {
        ctx.resolver().is_value_type(*self)
    }

    fn has_finalizer(&self, ctx: &ResolutionContext) -> bool {
        ctx.resolver().has_finalizer(*self)
    }
}

use crate::context::ResolutionContext;
use dotnet_types::{TypeDescription, generics::ConcreteType, error::TypeResolutionError};
use dotnet_utils::gc::GCHandle;
use dotnet_value::{
    StackValue,
    object::{CTSValue, Object, ValueType, Vector},
    storage::FieldStorage,
};

pub trait TypeResolutionExt {
    fn is_value_type(&self, ctx: &ResolutionContext) -> Result<bool, TypeResolutionError>;
    fn has_finalizer(&self, ctx: &ResolutionContext) -> Result<bool, TypeResolutionError>;
}

pub trait ValueResolution {
    fn stack_value_type(&self, val: &StackValue) -> Result<TypeDescription, TypeResolutionError>;
    fn new_object<'gc>(&self, td: TypeDescription) -> Result<Object<'gc>, TypeResolutionError>;
    fn new_instance_fields(&self, td: TypeDescription) -> Result<FieldStorage, TypeResolutionError>;
    fn new_static_fields(&self, td: TypeDescription) -> Result<FieldStorage, TypeResolutionError>;
    fn new_value_type<'gc>(&self, t: &ConcreteType, data: StackValue<'gc>) -> Result<ValueType<'gc>, TypeResolutionError>;
    fn value_type_description<'gc>(&self, vt: &ValueType<'gc>) -> Result<TypeDescription, TypeResolutionError>;
    fn new_cts_value<'gc>(&self, t: &ConcreteType, data: StackValue<'gc>) -> Result<CTSValue<'gc>, TypeResolutionError>;
    fn read_cts_value<'gc>(
        &self,
        t: &ConcreteType,
        data: &[u8],
        gc: GCHandle<'gc>,
    ) -> Result<CTSValue<'gc>, TypeResolutionError>;
    fn new_vector<'gc>(&self, element: ConcreteType, size: usize) -> Result<Vector<'gc>, TypeResolutionError>;
}

impl<'a, 'm> ValueResolution for ResolutionContext<'a, 'm> {
    fn stack_value_type(&self, val: &StackValue) -> Result<TypeDescription, TypeResolutionError> {
        self.resolver().stack_value_type(val)
    }

    fn new_object<'gc>(&self, td: TypeDescription) -> Result<Object<'gc>, TypeResolutionError> {
        self.resolver().new_object(td, self)
    }

    fn new_instance_fields(&self, td: TypeDescription) -> Result<FieldStorage, TypeResolutionError> {
        self.resolver().new_instance_fields(td, self)
    }

    fn new_static_fields(&self, td: TypeDescription) -> Result<FieldStorage, TypeResolutionError> {
        self.resolver().new_static_fields(td, self)
    }

    fn new_value_type<'gc>(&self, t: &ConcreteType, data: StackValue<'gc>) -> Result<ValueType<'gc>, TypeResolutionError> {
        self.resolver().new_value_type(t, data, self)
    }

    fn value_type_description<'gc>(&self, vt: &ValueType<'gc>) -> Result<TypeDescription, TypeResolutionError> {
        self.resolver().value_type_description(vt)
    }

    fn new_cts_value<'gc>(&self, t: &ConcreteType, data: StackValue<'gc>) -> Result<CTSValue<'gc>, TypeResolutionError> {
        self.resolver().new_cts_value(t, data, self)
    }

    fn read_cts_value<'gc>(
        &self,
        t: &ConcreteType,
        data: &[u8],
        gc: GCHandle<'gc>,
    ) -> Result<CTSValue<'gc>, TypeResolutionError> {
        self.resolver().read_cts_value(t, data, gc, self)
    }

    fn new_vector<'gc>(&self, element: ConcreteType, size: usize) -> Result<Vector<'gc>, TypeResolutionError> {
        self.resolver().new_vector(element, size, self)
    }
}

impl TypeResolutionExt for TypeDescription {
    fn is_value_type(&self, ctx: &ResolutionContext) -> Result<bool, TypeResolutionError> {
        ctx.resolver().is_value_type(*self)
    }

    fn has_finalizer(&self, ctx: &ResolutionContext) -> Result<bool, TypeResolutionError> {
        ctx.resolver().has_finalizer(*self)
    }
}

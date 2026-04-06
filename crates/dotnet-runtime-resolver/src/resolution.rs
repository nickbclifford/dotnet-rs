use crate::ResolverProvider;
use dotnet_types::{TypeDescription, error::TypeResolutionError, generics::ConcreteType};
use dotnet_utils::gc::GCHandle;
use dotnet_value::{
    StackValue,
    object::{CTSValue, Object, ValueType, Vector},
    storage::FieldStorage,
};

pub trait TypeResolutionExt {
    fn is_value_type<P: ResolverProvider>(&self, ctx: &P) -> Result<bool, TypeResolutionError>;
    fn has_finalizer<P: ResolverProvider>(&self, ctx: &P) -> Result<bool, TypeResolutionError>;
}

pub trait ValueResolution {
    fn stack_value_type(&self, val: &StackValue) -> Result<TypeDescription, TypeResolutionError>;
    fn new_object<'gc>(&self, td: TypeDescription) -> Result<Object<'gc>, TypeResolutionError>;
    fn new_instance_fields(&self, td: TypeDescription)
    -> Result<FieldStorage, TypeResolutionError>;
    fn new_static_fields(&self, td: TypeDescription) -> Result<FieldStorage, TypeResolutionError>;
    fn new_value_type<'gc>(
        &self,
        t: &ConcreteType,
        data: StackValue<'gc>,
    ) -> Result<ValueType<'gc>, TypeResolutionError>;
    fn value_type_description<'gc>(
        &self,
        vt: &ValueType<'gc>,
    ) -> Result<TypeDescription, TypeResolutionError>;
    fn new_cts_value<'gc>(
        &self,
        t: &ConcreteType,
        data: StackValue<'gc>,
    ) -> Result<CTSValue<'gc>, TypeResolutionError>;
    fn read_cts_value<'gc>(
        &self,
        t: &ConcreteType,
        data: &[u8],
        gc: GCHandle<'gc>,
    ) -> Result<CTSValue<'gc>, TypeResolutionError>;
    fn new_vector<'gc>(
        &self,
        element: ConcreteType,
        size: usize,
    ) -> Result<Vector<'gc>, TypeResolutionError>;
}

impl<P: ResolverProvider> ValueResolution for P {
    fn stack_value_type(&self, val: &StackValue) -> Result<TypeDescription, TypeResolutionError> {
        self.resolver_service().stack_value_type(val)
    }

    fn new_object<'gc>(&self, td: TypeDescription) -> Result<Object<'gc>, TypeResolutionError> {
        self.resolver_service().new_object(td, self)
    }

    fn new_instance_fields(
        &self,
        td: TypeDescription,
    ) -> Result<FieldStorage, TypeResolutionError> {
        self.resolver_service().new_instance_fields(td, self)
    }

    fn new_static_fields(&self, td: TypeDescription) -> Result<FieldStorage, TypeResolutionError> {
        self.resolver_service().new_static_fields(td, self)
    }

    fn new_value_type<'gc>(
        &self,
        t: &ConcreteType,
        data: StackValue<'gc>,
    ) -> Result<ValueType<'gc>, TypeResolutionError> {
        self.resolver_service().new_value_type(t, data, self)
    }

    fn value_type_description<'gc>(
        &self,
        vt: &ValueType<'gc>,
    ) -> Result<TypeDescription, TypeResolutionError> {
        self.resolver_service().value_type_description(vt)
    }

    fn new_cts_value<'gc>(
        &self,
        t: &ConcreteType,
        data: StackValue<'gc>,
    ) -> Result<CTSValue<'gc>, TypeResolutionError> {
        self.resolver_service().new_cts_value(t, data, self)
    }

    fn read_cts_value<'gc>(
        &self,
        t: &ConcreteType,
        data: &[u8],
        gc: GCHandle<'gc>,
    ) -> Result<CTSValue<'gc>, TypeResolutionError> {
        self.resolver_service().read_cts_value(t, data, gc, self)
    }

    fn new_vector<'gc>(
        &self,
        element: ConcreteType,
        size: usize,
    ) -> Result<Vector<'gc>, TypeResolutionError> {
        self.resolver_service().new_vector(element, size, self)
    }
}

impl TypeResolutionExt for TypeDescription {
    fn is_value_type<P: ResolverProvider>(&self, ctx: &P) -> Result<bool, TypeResolutionError> {
        ctx.resolver_service().is_value_type(self.clone())
    }

    fn has_finalizer<P: ResolverProvider>(&self, ctx: &P) -> Result<bool, TypeResolutionError> {
        ctx.resolver_service().has_finalizer(self.clone())
    }
}

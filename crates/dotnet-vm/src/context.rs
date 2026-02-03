use crate::{resolver::ResolverService, state::GlobalCaches, sync::Arc, MethodType};
use dotnet_assemblies::{Ancestor, AssemblyLoader};
use dotnet_types::{
    generics::{ConcreteType, GenericLookup},
    members::{FieldDescription, MethodDescription},
    resolution::ResolutionS,
    TypeDescription,
};
use dotnet_value::object::ObjectHandle;
use dotnetdll::prelude::{FieldSource, UserMethod, UserType};

#[derive(Clone)]
pub struct ResolutionContext<'a, 'm> {
    pub generics: &'a GenericLookup,
    pub loader: &'m AssemblyLoader,
    pub resolution: ResolutionS,
    pub type_owner: Option<TypeDescription>,
    pub method_owner: Option<MethodDescription>,
    pub caches: Arc<GlobalCaches>,
}

impl<'a, 'm> ResolutionContext<'a, 'm> {
    pub fn new(
        generics: &'a GenericLookup,
        loader: &'m AssemblyLoader,
        resolution: ResolutionS,
        caches: Arc<GlobalCaches>,
    ) -> Self {
        Self {
            generics,
            loader,
            resolution,
            type_owner: None,
            method_owner: None,
            caches,
        }
    }

    pub fn for_method(
        method: MethodDescription,
        loader: &'m AssemblyLoader,
        generics: &'a GenericLookup,
        caches: Arc<GlobalCaches>,
    ) -> Self {
        Self {
            generics,
            loader,
            resolution: method.resolution(),
            type_owner: Some(method.parent),
            method_owner: Some(method),
            caches,
        }
    }

    pub fn resolver(&self) -> ResolverService<'m> {
        ResolverService::from_parts(self.loader, self.caches.clone())
    }

    pub fn with_generics(&self, generics: &'a GenericLookup) -> ResolutionContext<'a, 'm> {
        ResolutionContext {
            generics,
            loader: self.loader,
            resolution: self.resolution,
            type_owner: self.type_owner,
            method_owner: self.method_owner,
            caches: self.caches.clone(),
        }
    }

    pub fn for_type(&self, td: TypeDescription) -> ResolutionContext<'a, 'm> {
        self.for_type_with_generics(td, self.generics)
    }

    pub fn for_type_with_generics(
        &self,
        td: TypeDescription,
        generics: &'a GenericLookup,
    ) -> ResolutionContext<'a, 'm> {
        ResolutionContext {
            resolution: td.resolution,
            generics,
            loader: self.loader,
            type_owner: Some(td),
            method_owner: None,
            caches: self.caches.clone(),
        }
    }

    pub fn locate_type(&self, handle: UserType) -> TypeDescription {
        self.resolver().locate_type(self.resolution, handle)
    }

    pub fn locate_method(
        &self,
        handle: UserMethod,
        generic_inst: &GenericLookup,
    ) -> MethodDescription {
        self.resolver()
            .locate_method(self.resolution, handle, generic_inst)
    }

    pub fn locate_field(&self, field: FieldSource) -> (FieldDescription, GenericLookup) {
        self.resolver()
            .locate_field(self.resolution, field, self.generics)
    }

    pub fn get_ancestors(
        &self,
        child_type: TypeDescription,
    ) -> impl Iterator<Item = Ancestor<'m>> + 'm {
        self.loader.ancestors(child_type)
    }

    pub fn is_a(&self, value: ConcreteType, ancestor: ConcreteType) -> bool {
        self.resolver().is_a(value, ancestor)
    }

    pub fn get_heap_description(&self, object: ObjectHandle) -> TypeDescription {
        self.resolver().get_heap_description(object)
    }

    pub fn make_concrete<T: Clone + Into<MethodType>>(&self, t: &T) -> ConcreteType {
        self.resolver()
            .make_concrete(self.resolution, self.generics, t)
    }

    pub fn get_field_type(&self, field: FieldDescription) -> ConcreteType {
        self.resolver()
            .get_field_type(self.resolution, self.generics, field)
    }

    pub fn get_field_desc(&self, field: FieldDescription) -> TypeDescription {
        self.resolver()
            .get_field_desc(self.resolution, self.generics, field)
    }

    pub fn normalize_type(&self, t: ConcreteType) -> ConcreteType {
        self.resolver().normalize_type(t)
    }
}

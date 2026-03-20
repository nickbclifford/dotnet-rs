use crate::{
    MethodType,
    resolver::ResolverService,
    state::{GlobalCaches, SharedGlobalState},
    sync::{Arc, Weak},
};
use dotnet_assemblies::{Ancestor, AssemblyLoader};
use dotnet_types::{
    TypeDescription,
    error::TypeResolutionError,
    generics::{ConcreteType, GenericLookup},
    members::{FieldDescription, MethodDescription},
    resolution::ResolutionS,
};
use dotnet_value::{StackValue, object::ObjectHandle};
use dotnet_vm_ops::ops::ResolutionOps;
use dotnetdll::prelude::{FieldSource, UserMethod, UserType};
use std::sync::OnceLock;

pub struct ResolutionShared {
    pub loader: Arc<AssemblyLoader>,
    pub caches: Arc<GlobalCaches>,
    pub shared: Option<Weak<SharedGlobalState>>,
    pub(crate) resolver_cache: OnceLock<ResolverService>,
}

impl std::fmt::Debug for ResolutionShared {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ResolutionShared")
            .field("shared", &self.shared.is_some())
            .finish_non_exhaustive()
    }
}

impl ResolutionShared {
    pub fn new(
        loader: Arc<AssemblyLoader>,
        caches: Arc<GlobalCaches>,
        shared: Option<Weak<SharedGlobalState>>,
    ) -> Self {
        Self {
            loader,
            caches,
            shared,
            resolver_cache: OnceLock::new(),
        }
    }

    pub fn resolver(&self) -> &ResolverService {
        self.resolver_cache.get_or_init(|| {
            if let Some(shared_weak) = &self.shared
                && let Some(shared) = shared_weak.upgrade()
            {
                return ResolverService::new(shared);
            }
            ResolverService::from_parts(self.loader.clone(), self.caches.clone())
        })
    }
}

#[derive(Clone)]
pub struct ResolutionContext<'a> {
    pub generics: &'a GenericLookup,
    pub state: Arc<ResolutionShared>,
    pub resolution: ResolutionS,
    pub type_owner: Option<TypeDescription>,
    pub method_owner: Option<MethodDescription>,
}

impl<'a> ResolutionContext<'a> {
    pub fn new(
        generics: &'a GenericLookup,
        loader: Arc<AssemblyLoader>,
        resolution: ResolutionS,
        caches: Arc<GlobalCaches>,
        shared: Option<Weak<SharedGlobalState>>,
    ) -> Self {
        Self {
            generics,
            state: Arc::new(ResolutionShared::new(loader, caches, shared)),
            resolution,
            type_owner: None,
            method_owner: None,
        }
    }

    pub fn for_method(
        method: MethodDescription,
        loader: Arc<AssemblyLoader>,
        generics: &'a GenericLookup,
        caches: Arc<GlobalCaches>,
        shared: Option<Weak<SharedGlobalState>>,
    ) -> Self {
        Self {
            generics,
            state: Arc::new(ResolutionShared::new(loader, caches, shared)),
            resolution: method.resolution(),
            type_owner: Some(method.parent.clone()),
            method_owner: Some(method),
        }
    }

    pub fn loader(&self) -> &Arc<AssemblyLoader> {
        &self.state.loader
    }

    pub fn caches(&self) -> &Arc<GlobalCaches> {
        &self.state.caches
    }

    pub fn shared_state(&self) -> Option<Arc<SharedGlobalState>> {
        self.state.shared.as_ref().and_then(|w| w.upgrade())
    }

    pub fn resolver(&self) -> &ResolverService {
        self.state.resolver()
    }

    pub fn with_generics(&self, generics: &'a GenericLookup) -> ResolutionContext<'a> {
        ResolutionContext {
            generics,
            state: self.state.clone(),
            resolution: self.resolution.clone(),
            type_owner: self.type_owner.clone(),
            method_owner: self.method_owner.clone(),
        }
    }

    pub fn for_type(&self, td: TypeDescription) -> ResolutionContext<'a> {
        self.for_type_with_generics(td, self.generics)
    }

    pub fn for_type_with_generics(
        &self,
        td: TypeDescription,
        generics: &'a GenericLookup,
    ) -> ResolutionContext<'a> {
        ResolutionContext {
            resolution: td.resolution.clone(),
            generics,
            state: self.state.clone(),
            type_owner: Some(td),
            method_owner: None,
        }
    }

    pub fn locate_type(&self, handle: UserType) -> Result<TypeDescription, TypeResolutionError> {
        self.resolver().locate_type(self.resolution.clone(), handle)
    }

    pub fn locate_method(
        &self,
        handle: UserMethod,
        generic_inst: &GenericLookup,
        pre_resolved_parent: Option<ConcreteType>,
    ) -> Result<MethodDescription, TypeResolutionError> {
        self.resolver().locate_method(
            self.resolution.clone(),
            handle,
            generic_inst,
            pre_resolved_parent,
        )
    }

    pub fn locate_field(
        &self,
        field: FieldSource,
    ) -> Result<(FieldDescription, GenericLookup), TypeResolutionError> {
        self.resolver()
            .locate_field(self.resolution.clone(), field, self.generics)
    }

    pub fn get_ancestors(
        &self,
        child_type: TypeDescription,
    ) -> impl Iterator<Item = Ancestor<'static>> + '_ {
        self.state.loader.ancestors(child_type)
    }

    pub fn is_a(
        &self,
        value: ConcreteType,
        ancestor: ConcreteType,
    ) -> Result<bool, TypeResolutionError> {
        self.resolver().is_a(value, ancestor)
    }

    pub fn get_heap_description(
        &self,
        object: ObjectHandle,
    ) -> Result<TypeDescription, TypeResolutionError> {
        self.resolver().get_heap_description(object)
    }

    pub fn make_concrete<T: Clone + Into<MethodType>>(
        &self,
        t: &T,
    ) -> Result<ConcreteType, TypeResolutionError> {
        self.resolver()
            .make_concrete(self.resolution.clone(), self.generics, t)
    }

    pub fn get_field_type(
        &self,
        field: FieldDescription,
    ) -> Result<ConcreteType, TypeResolutionError> {
        self.resolver()
            .get_field_type(self.resolution.clone(), self.generics, field)
    }

    pub fn get_field_desc(
        &self,
        field: FieldDescription,
    ) -> Result<TypeDescription, TypeResolutionError> {
        self.resolver()
            .get_field_desc(self.resolution.clone(), self.generics, field)
    }

    pub fn normalize_type(&self, t: ConcreteType) -> Result<ConcreteType, TypeResolutionError> {
        self.resolver().normalize_type(t)
    }
}

impl<'a> ResolutionOps<'_> for ResolutionContext<'a> {
    fn stack_value_type(
        &self,
        val: &StackValue<'_>,
    ) -> Result<TypeDescription, TypeResolutionError> {
        self.resolver().stack_value_type(val)
    }

    fn make_concrete(
        &self,
        t: &dotnetdll::prelude::MethodType,
    ) -> Result<ConcreteType, TypeResolutionError> {
        self.make_concrete(t)
    }

    fn is_a(
        &self,
        value: ConcreteType,
        ancestor: ConcreteType,
    ) -> Result<bool, TypeResolutionError> {
        self.is_a(value, ancestor)
    }

    fn instance_field_layout(
        &self,
        td: TypeDescription,
    ) -> Result<dotnet_value::layout::FieldLayoutManager, TypeResolutionError> {
        self.resolver().instance_fields(td, self)
    }
}

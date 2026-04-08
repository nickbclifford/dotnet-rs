#![allow(clippy::mutable_key_type)]
//! Type, method, and field resolution with cache and layout adapters.
use dotnet_assemblies::AssemblyLoader;
use dotnet_types::{
    TypeDescription,
    generics::{ConcreteType, GenericLookup},
    members::{FieldDescription, MethodDescription},
    resolution::ResolutionS,
};
use dotnet_value::layout::{FieldLayoutManager, LayoutManager};
use std::{collections::HashMap, sync::Arc};

mod factory;
mod layout;
mod methods;
pub mod resolution;
mod types;

pub use layout::LayoutFactory;

pub trait ResolverCacheAdapter: Send + Sync + 'static {
    fn get_intrinsic_cached(&self, method: &MethodDescription) -> Option<bool>;
    fn set_intrinsic_cached(&self, method: MethodDescription, is_intrinsic: bool);
    fn compute_is_intrinsic(&self, method: MethodDescription, loader: &AssemblyLoader) -> bool;

    fn get_intrinsic_field_cached(&self, field: &FieldDescription) -> Option<bool>;
    fn set_intrinsic_field_cached(&self, field: FieldDescription, is_intrinsic: bool);
    fn compute_is_intrinsic_field(&self, field: FieldDescription, loader: &AssemblyLoader) -> bool;

    fn get_vmt_cached(
        &self,
        base_method: &MethodDescription,
        this_type: &TypeDescription,
        generics: &GenericLookup,
    ) -> Option<MethodDescription>;
    fn set_vmt_cached(
        &self,
        base_method: MethodDescription,
        this_type: TypeDescription,
        generics: GenericLookup,
        method: MethodDescription,
    );
    fn record_vmt_key_clones(&self, _count: u64) {}

    fn get_overrides_cached(
        &self,
        key: &(TypeDescription, GenericLookup),
    ) -> Option<Arc<HashMap<MethodDescription, MethodDescription>>>;
    fn set_overrides_cached(
        &self,
        key: (TypeDescription, GenericLookup),
        overrides: Arc<HashMap<MethodDescription, MethodDescription>>,
    );

    fn get_hierarchy_cached(&self, child: &ConcreteType, parent: &ConcreteType) -> Option<bool>;
    fn set_hierarchy_cached(&self, child: ConcreteType, parent: ConcreteType, is_match: bool);
    fn record_hierarchy_key_clones(&self, _count: u64) {}

    fn get_value_type_cached(&self, td: &TypeDescription) -> Option<bool>;
    fn set_value_type_cached(&self, td: TypeDescription, is_value_type: bool);

    fn get_has_finalizer_cached(&self, td: &TypeDescription) -> Option<bool>;
    fn set_has_finalizer_cached(&self, td: TypeDescription, has_finalizer: bool);
}

pub trait ResolverLayoutAdapter: Send + Sync + Clone + 'static {
    fn get_layout_cached(&self, key: &ConcreteType) -> Option<Arc<LayoutManager>>;
    fn set_layout_cached(&self, key: ConcreteType, layout: Arc<LayoutManager>);

    fn get_instance_field_layout_cached(
        &self,
        key: &(TypeDescription, GenericLookup),
    ) -> Option<Arc<FieldLayoutManager>>;
    fn set_instance_field_layout_cached(
        &self,
        key: (TypeDescription, GenericLookup),
        layout: Arc<FieldLayoutManager>,
    );
}

pub trait ResolverExecutionContext {
    fn generics(&self) -> &GenericLookup;
    fn resolution(&self) -> &ResolutionS;
}

pub trait ResolverProvider: ResolverExecutionContext {
    type Caches: ResolverCacheAdapter;
    type Layout: ResolverLayoutAdapter;

    fn resolver_service(&self) -> &ResolverService<Self::Caches, Self::Layout>;
}

#[derive(Clone)]
pub struct ResolverService<C: ResolverCacheAdapter, L: ResolverLayoutAdapter> {
    pub loader: Arc<AssemblyLoader>,
    pub caches: Arc<C>,
    pub layout: L,
}

impl<C: ResolverCacheAdapter, L: ResolverLayoutAdapter> std::fmt::Debug for ResolverService<C, L> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ResolverService").finish_non_exhaustive()
    }
}

impl<C: ResolverCacheAdapter, L: ResolverLayoutAdapter> ResolverService<C, L> {
    pub fn from_parts(loader: Arc<AssemblyLoader>, caches: Arc<C>, layout: L) -> Self {
        Self {
            loader,
            caches,
            layout,
        }
    }

    pub fn loader(&self) -> &AssemblyLoader {
        &self.loader
    }
}

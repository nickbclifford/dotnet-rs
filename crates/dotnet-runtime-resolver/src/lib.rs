//! Runtime resolver services for type, method, and field lookups.
//!
//! This crate centralizes metadata-driven resolution and caching that the VM
//! depends on while executing IL. Internally, functionality is split across
//! `types.rs` (type hierarchy and trait/value-type queries), `methods.rs`
//! (method dispatch and override resolution), `layout.rs` (instance/type layout
//! caching), and `factory.rs` (construction helpers). The public `resolution`
//! module exposes resolution data structures used by call sites.
//!
//! `LayoutFactory` is re-exported as the crate's layout-construction entry point,
//! and `ResolverThreadSafety` provides a feature-gated thread-safety boundary:
//! with `multithreading`, adapters must be `Send + Sync`; otherwise the bound is
//! relaxed for single-threaded configurations.
//!
//! For design context and cache behavior details, see
//! `docs/TYPE_RESOLUTION_AND_CACHING.md`.

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

#[cfg(feature = "multithreading")]
pub trait ResolverThreadSafety: Send + Sync {}
#[cfg(feature = "multithreading")]
impl<T: Send + Sync> ResolverThreadSafety for T {}

#[cfg(not(feature = "multithreading"))]
pub trait ResolverThreadSafety {}
#[cfg(not(feature = "multithreading"))]
impl<T> ResolverThreadSafety for T {}

pub trait IntrinsicCacheAdapter: ResolverThreadSafety + 'static {
    fn get_intrinsic_cached(&self, method: &MethodDescription) -> Option<bool>;
    fn set_intrinsic_cached(&self, method: MethodDescription, is_intrinsic: bool);
    fn compute_is_intrinsic(&self, method: MethodDescription, loader: &AssemblyLoader) -> bool;

    fn get_intrinsic_field_cached(&self, field: &FieldDescription) -> Option<bool>;
    fn set_intrinsic_field_cached(&self, field: FieldDescription, is_intrinsic: bool);
    fn compute_is_intrinsic_field(&self, field: FieldDescription, loader: &AssemblyLoader) -> bool;
}

pub trait VmtCacheAdapter: ResolverThreadSafety + 'static {
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
}

/// The only currently supported static constrained call form. Keeping the call kind in the key
/// makes a later static constrained form opt in explicitly instead of accidentally sharing a
/// metadata result with different dispatch semantics.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum StaticConstrainedCallKind {
    Call,
}

/// Exact, loader-owned key for metadata that is invariant for a static constrained call.
///
/// This deliberately contains no receiver, managed pointer, delegate target, or tail-call state.
/// `GenericLookup` equality is structural, so equivalent generic slices from separate frames
/// share only when they describe the same closed source instantiation.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct StaticConstrainedCacheKey {
    pub kind: StaticConstrainedCallKind,
    pub constraint: ConcreteType,
    pub base_method: MethodDescription,
    pub source_lookup: GenericLookup,
}

pub trait StaticConstrainedCacheAdapter: ResolverThreadSafety + 'static {
    fn get_static_constrained_cached(
        &self,
        key: &StaticConstrainedCacheKey,
    ) -> Option<MethodDescription>;
    fn set_static_constrained_cached(
        &self,
        key: StaticConstrainedCacheKey,
        method: MethodDescription,
    );
    fn record_static_constrained_key_clones(&self, _count: u64) {}
}

pub trait TypePropertyCacheAdapter: ResolverThreadSafety + 'static {
    fn get_hierarchy_cached(&self, child: &ConcreteType, parent: &ConcreteType) -> Option<bool>;
    fn set_hierarchy_cached(&self, child: ConcreteType, parent: ConcreteType, is_match: bool);
    fn record_hierarchy_key_clones(&self, _count: u64) {}

    fn get_value_type_cached(&self, td: &TypeDescription) -> Option<bool>;
    fn set_value_type_cached(&self, td: TypeDescription, is_value_type: bool);

    fn get_has_finalizer_cached(&self, td: &TypeDescription) -> Option<bool>;
    fn set_has_finalizer_cached(&self, td: TypeDescription, has_finalizer: bool);
}

pub trait ResolverCacheAdapter:
    IntrinsicCacheAdapter + VmtCacheAdapter + StaticConstrainedCacheAdapter + TypePropertyCacheAdapter
{
}

impl<
    T: IntrinsicCacheAdapter
        + VmtCacheAdapter
        + StaticConstrainedCacheAdapter
        + TypePropertyCacheAdapter,
> ResolverCacheAdapter for T
{
}

pub trait ResolverLayoutAdapter: ResolverThreadSafety + Clone + 'static {
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

//! Type, method, and field resolution with caching.
//!
//! This module provides the [`ResolverService`] for resolving metadata tokens to
//! their corresponding type definitions, method signatures, and field descriptors.
//! Resolution involves looking up entities in loaded assemblies, handling generic
//! instantiation, and caching results for performance.
//!
//! # Architecture
//!
//! The [`ResolverService`] acts as a facade over several resolution subsystems:
//!
//! - **Assembly Loading**: Delegates to [`AssemblyLoader`] for loading and parsing
//!   .NET assemblies from the filesystem.
//!
//! - **Generic Instantiation**: Uses [`GenericLookup`] to substitute type parameters
//!   with concrete types in generic methods and types.
//!
//! - **Caching**: Maintains several caches in [`GlobalCaches`]:
//!   - Intrinsic method cache (is a method implemented natively?)
//!   - Intrinsic field cache (is a field implemented natively?)
//!   - Type layout cache (computed memory layouts for value types)
//!   - Virtual dispatch cache (resolved virtual method targets)
//!
//! - **Type Layouts**: Computes memory layouts for value types, including field
//!   offsets, alignment, and total size.
//!
//! - **Interface Resolution**: Resolves interface methods to their concrete
//!   implementations in classes that implement those interfaces.
//!
//! # Resolution Process
//!
//! ## Type Resolution
//!
//! Types are resolved from metadata tokens (`TypeDefOrRef`, `TypeSpec`) to
//! [`TypeDescription`] instances. For generic types, the resolver substitutes
//! type parameters with concrete arguments from the current generic context.
//!
//! ## Method Resolution
//!
//! Methods are resolved in several phases:
//! 1. **Token to descriptor**: Map metadata token to [`MethodDescription`]
//! 2. **Virtual dispatch**: For `callvirt`, resolve to most-derived implementation
//! 3. **Generic instantiation**: Substitute type/method generic parameters
//! 4. **Intrinsic check**: Determine if method has native implementation
//!
//! ## Field Resolution
//!
//! Fields are resolved to [`FieldDescription`] with computed offsets within their
//! containing type. Value type field access requires layout calculation.
//!
//! # Caching Strategy
//!
//! The resolver maintains several caches to avoid redundant work:
//!
//! - **Intrinsic caches**: Method and field intrinsic checks are cached, as these
//!   involve string comparisons and registry lookups.
//!
//! - **Layout cache**: Type layouts are expensive to compute (requires recursive
//!   field traversal) and are cached by concrete type.
//!
//! - **Virtual dispatch cache**: Resolved virtual methods are cached by (method, type)
//!   pair to avoid repeated inheritance chain traversal.
//!
//! Cache eviction is not currently implemented, as the cache sizes are bounded by
//! the number of unique types/methods in the loaded assemblies.
//!
//! # Example
//!
//! ```ignore
//! let resolver = ResolverService::new(shared_state);
//!
//! // Resolve a type from a metadata token
//! let type_desc = resolver.resolve_type(type_token, generic_context)?;
//!
//! // Check if a method is intrinsic (natively implemented)
//! if resolver.is_intrinsic_cached(method_desc) {
//!     // Call intrinsic implementation
//! } else {
//!     // Interpret CIL bytecode
//! }
//!
//! // Compute type layout for value type
//! let layout = resolver.compute_layout(value_type_desc)?;
//! ```
use crate::{
    metrics::RuntimeMetrics,
    state::{GlobalCaches, SharedGlobalState},
};
use dotnet_assemblies::AssemblyLoader;
use std::sync::Arc;

/// Service for resolving types, methods, and fields with caching.
pub struct ResolverService<'m> {
    pub loader: &'m AssemblyLoader,
    pub caches: Arc<GlobalCaches>,
    pub shared: Option<Arc<SharedGlobalState<'m>>>,
}

mod factory;
mod layout;
mod methods;
mod types;

impl<'m> ResolverService<'m> {
    pub fn new(shared: Arc<SharedGlobalState<'m>>) -> Self {
        Self {
            loader: shared.loader,
            caches: shared.caches.clone(),
            shared: Some(shared),
        }
    }

    pub fn from_parts(loader: &'m AssemblyLoader, caches: Arc<GlobalCaches>) -> Self {
        Self {
            loader,
            caches,
            shared: None,
        }
    }

    fn metrics(&self) -> Option<&RuntimeMetrics> {
        self.shared.as_ref().map(|s| &s.metrics)
    }

    pub fn loader(&self) -> &'m AssemblyLoader {
        self.loader
    }
}

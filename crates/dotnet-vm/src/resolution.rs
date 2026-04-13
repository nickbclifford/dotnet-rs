use crate::{
    context::ResolutionContext,
    resolver::{VmResolverCaches, VmResolverLayout, VmResolverService},
};

pub use dotnet_runtime_resolver::resolution::{TypeResolutionExt, ValueResolution};

impl dotnet_runtime_resolver::ResolverExecutionContext for ResolutionContext<'_> {
    fn generics(&self) -> &dotnet_types::generics::GenericLookup {
        self.generics
    }

    fn resolution(&self) -> &dotnet_types::resolution::ResolutionS {
        &self.resolution
    }
}

impl dotnet_runtime_resolver::ResolverProvider for ResolutionContext<'_> {
    type Caches = VmResolverCaches;
    type Layout = VmResolverLayout;

    fn resolver_service(
        &self,
    ) -> &dotnet_runtime_resolver::ResolverService<Self::Caches, Self::Layout> {
        let resolver: &VmResolverService = self.resolver();
        resolver
    }
}

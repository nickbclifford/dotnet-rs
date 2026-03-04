use crate::{
    context::ResolutionContext, layout::type_layout_with_metrics, resolver::ResolverService,
};
use dotnet_types::{error::TypeResolutionError, generics::ConcreteType};
use dotnet_value::layout::LayoutManager;
use std::sync::Arc;

impl ResolverService {
    pub fn type_layout_cached(
        &self,
        t: ConcreteType,
        ctx: &ResolutionContext<'_>,
    ) -> Result<Arc<LayoutManager>, TypeResolutionError> {
        type_layout_with_metrics(t, ctx, self.metrics())
    }
}

use crate::{
    context::ResolutionContext, layout::type_layout_with_metrics, resolver::ResolverService,
};
use dotnet_types::{error::TypeResolutionError, generics::ConcreteType};
use std::sync::Arc;

impl<'m> ResolverService<'m> {
    pub fn type_layout_cached(
        &self,
        t: ConcreteType,
        ctx: &ResolutionContext<'_, 'm>,
    ) -> Result<Arc<dotnet_value::layout::LayoutManager>, TypeResolutionError> {
        type_layout_with_metrics(t, ctx, self.metrics())
    }
}

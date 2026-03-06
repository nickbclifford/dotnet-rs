use crate::{
    context::ResolutionContext,
    layout::{LayoutFactory, type_layout_with_metrics},
    resolver::ResolverService,
};
use dotnet_types::{TypeDescription, error::TypeResolutionError, generics::ConcreteType};
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

    pub fn instance_fields(
        &self,
        td: TypeDescription,
        ctx: &ResolutionContext<'_>,
    ) -> Result<dotnet_value::layout::FieldLayoutManager, TypeResolutionError> {
        LayoutFactory::instance_fields_with_metrics(td, ctx, self.metrics())
    }
}

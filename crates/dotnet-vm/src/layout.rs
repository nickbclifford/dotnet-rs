use crate::{context::ResolutionContext, sync::Arc};
use dotnet_metrics::RuntimeMetrics;
use dotnet_types::{TypeDescription, error::TypeResolutionError, generics::ConcreteType};
use dotnet_value::layout::{ArrayLayoutManager, FieldLayoutManager, LayoutManager};

#[cfg(test)]
use dotnet_value::layout::GcDesc;

pub struct LayoutFactory;

impl LayoutFactory {
    pub fn instance_fields(
        td: TypeDescription,
        context: &ResolutionContext,
    ) -> Result<FieldLayoutManager, TypeResolutionError> {
        context.resolver().instance_fields(td, context)
    }

    pub fn instance_field_layout_cached(
        td: TypeDescription,
        context: &ResolutionContext,
        _metrics: Option<&RuntimeMetrics>,
    ) -> Result<Arc<FieldLayoutManager>, TypeResolutionError> {
        context
            .resolver()
            .instance_field_layout_cached_with_lookup(td, context.generics)
    }

    pub fn static_fields(
        td: TypeDescription,
        context: &ResolutionContext,
    ) -> Result<FieldLayoutManager, TypeResolutionError> {
        context.resolver().static_fields(td, context)
    }

    pub fn create_array_layout(
        element: ConcreteType,
        length: usize,
        context: &ResolutionContext,
    ) -> Result<ArrayLayoutManager, TypeResolutionError> {
        context
            .resolver()
            .create_array_layout(element, length, context)
    }

    #[cfg(test)]
    pub(crate) fn populate_gc_desc(layout: &LayoutManager, base_offset: usize, desc: &mut GcDesc) {
        dotnet_runtime_resolver::LayoutFactory::populate_gc_desc(layout, base_offset, desc)
    }
}

pub fn type_layout(
    t: ConcreteType,
    context: &ResolutionContext,
) -> Result<Arc<LayoutManager>, TypeResolutionError> {
    context.resolver().type_layout_cached(t, context)
}

#[cfg(test)]
mod tests {
    use super::LayoutFactory;
    use crate::{context::ResolutionContext, state::SharedGlobalState};
    use dotnet_assemblies::{AssemblyLoader, find_dotnet_app_path};
    use dotnet_types::generics::{ConcreteType, GenericLookup};
    use dotnet_value::layout::{LayoutManager, Scalar};
    use dotnetdll::prelude::{BaseType, TypeSource, UserType};
    use std::sync::Arc;

    #[test]
    fn nullable_int_layout_has_empty_gc_desc() {
        let loader = Arc::new(
            AssemblyLoader::new(
                find_dotnet_app_path()
                    .expect("could not find .NET shared path")
                    .display()
                    .to_string(),
            )
            .unwrap(),
        );
        let shared = Arc::new(SharedGlobalState::new(loader.clone()));
        let empty = GenericLookup::default();

        let nullable_td = loader.corlib_type("System.Nullable`1").unwrap();
        let int_ct = ConcreteType::new(nullable_td.resolution.clone(), BaseType::Int32);
        let lookup = GenericLookup::new(vec![int_ct.clone()]);

        let ctx = ResolutionContext::new(
            &empty,
            loader.clone(),
            nullable_td.resolution.clone(),
            shared.caches.clone(),
            Some(Arc::downgrade(&shared)),
        );
        let typed_ctx = ctx.for_type_with_generics(nullable_td.clone(), &lookup);

        let layout =
            LayoutFactory::instance_field_layout_cached(nullable_td.clone(), &typed_ctx, None)
                .unwrap();

        assert_eq!(layout.total_size, 8, "Nullable<int> should be 8 bytes");
        assert!(
            layout.gc_desc.bitmap.not_any(),
            "Nullable<int> should not contain object references"
        );
        assert!(
            layout.gc_desc.unaligned_offsets.is_empty(),
            "Nullable<int> should not contain unaligned object-reference offsets"
        );

        let nullable_int = ConcreteType::new(
            nullable_td.resolution.clone(),
            BaseType::Type {
                source: TypeSource::Generic {
                    base: UserType::Definition(nullable_td.index),
                    parameters: vec![int_ct],
                },
                value_kind: None,
            },
        );
        let resolved_nullable = loader.find_concrete_type(nullable_int).unwrap();
        assert_eq!(resolved_nullable, nullable_td);
    }

    #[test]
    fn gc_desc_tracks_unaligned_reference_offsets_without_word_rounding() {
        let mut desc = dotnet_value::layout::GcDesc::default();
        LayoutFactory::populate_gc_desc(&LayoutManager::Scalar(Scalar::ObjectRef), 1, &mut desc);

        assert!(
            desc.bitmap.not_any(),
            "unaligned refs must not be rounded into bitmap words"
        );
        assert_eq!(desc.unaligned_offsets, vec![1]);

        let mut nested = dotnet_value::layout::GcDesc::default();
        nested.set_offset(3);
        let nested_layout = LayoutManager::Field(dotnet_value::layout::FieldLayoutManager {
            fields: std::collections::HashMap::new(),
            total_size: 16,
            alignment: 1,
            gc_desc: nested,
            has_ref_fields: false,
        });

        let mut outer = dotnet_value::layout::GcDesc::default();
        LayoutFactory::populate_gc_desc(&nested_layout, 5, &mut outer);
        assert!(outer.unaligned_offsets.is_empty());
        assert!(outer.bitmap.get(1).is_some_and(|b| *b));
    }
}

use crate::{context::ResolutionContext, resolver::ResolverService};
use dotnet_types::{
    TypeDescription,
    error::TypeResolutionError,
    generics::{ConcreteType, GenericLookup},
    members::{FieldDescription, MethodDescription},
    resolution::ResolutionS,
};
use dotnetdll::prelude::*;
use std::sync::Arc;

impl<'m> ResolverService<'m> {
    pub fn is_intrinsic_cached(&self, method: MethodDescription) -> bool {
        if let Some(cached) = self.caches.intrinsic_cache.get(&method) {
            if let Some(metrics) = self.metrics() {
                metrics.record_intrinsic_cache_hit();
            }
            return *cached;
        }

        if let Some(metrics) = self.metrics() {
            metrics.record_intrinsic_cache_miss();
        }
        let result =
            crate::intrinsics::is_intrinsic(method, self.loader(), &self.caches.intrinsic_registry);
        self.caches.intrinsic_cache.insert(method, result);
        result
    }

    pub fn is_intrinsic_field_cached(&self, field: FieldDescription) -> bool {
        if let Some(cached) = self.caches.intrinsic_field_cache.get(&field) {
            if let Some(metrics) = self.metrics() {
                metrics.record_intrinsic_field_cache_hit();
            }
            return *cached;
        }

        if let Some(metrics) = self.metrics() {
            metrics.record_intrinsic_field_cache_miss();
        }
        let result = crate::intrinsics::is_intrinsic_field(
            field,
            self.loader(),
            &self.caches.intrinsic_registry,
        );
        self.caches.intrinsic_field_cache.insert(field, result);
        result
    }

    pub fn find_generic_method(
        &self,
        source: &MethodSource,
        ctx: &ResolutionContext<'_, 'm>,
    ) -> Result<(MethodDescription, GenericLookup), TypeResolutionError> {
        let mut new_lookup = ctx.generics.clone();

        let method = match source {
            MethodSource::User(u) => *u,
            MethodSource::Generic(g) => {
                let params: Vec<_> = g
                    .parameters
                    .iter()
                    .map(|t| ctx.make_concrete(t))
                    .collect::<Result<Vec<_>, _>>()?;
                new_lookup.method_generics = params.into();
                g.base
            }
        };

        let mut parent_type = None;
        if let UserMethod::Reference(r) = method
            && let MethodReferenceParent::Type(t) = &ctx.resolution[r].parent
        {
            let parent = ctx.make_concrete(t)?;
            if let BaseType::Type {
                source: TypeSource::Generic { parameters, .. },
                ..
            } = parent.get()
            {
                new_lookup.type_generics = parameters.clone().into();
            }
            parent_type = Some(parent);
        }

        Ok((
            ctx.locate_method(method, &new_lookup, parent_type)?,
            new_lookup,
        ))
    }

    pub fn resolve_virtual_method(
        &self,
        base_method: MethodDescription,
        this_type: TypeDescription,
        generics: &GenericLookup,
        ctx: &ResolutionContext<'_, 'm>,
    ) -> Result<MethodDescription, TypeResolutionError> {
        let mut normalized_generics = generics.clone();
        if this_type.definition().generic_parameters.is_empty() {
            normalized_generics.type_generics = Arc::new([]);
        }
        if base_method.method.generic_parameters.is_empty() {
            normalized_generics.method_generics = Arc::new([]);
        }

        let key = (base_method, this_type, normalized_generics);
        if let Some(cached) = self.caches.vmt_cache.get(&key) {
            if let Some(metrics) = self.metrics() {
                metrics.record_vmt_cache_hit();
            }
            return Ok(*cached);
        }

        if let Some(metrics) = self.metrics() {
            metrics.record_vmt_cache_miss();
        }

        // Standard virtual method resolution: search ancestors
        for (parent, _) in ctx.get_ancestors(this_type) {
            if let Some(this_method) = self.find_and_cache_method(parent, base_method, &key.2)? {
                self.caches.vmt_cache.insert(key, this_method);
                return Ok(this_method);
            }
        }

        Err(TypeResolutionError::MethodNotFound(format!(
            "could not find virtual method implementation of {:?} in {:?} with generics {:?}",
            base_method, this_type, generics
        )))
    }

    fn find_and_cache_method(
        &self,
        this_type: TypeDescription,
        method: MethodDescription,
        generics: &GenericLookup,
    ) -> Result<Option<MethodDescription>, TypeResolutionError> {
        let def = this_type.definition();
        if !def.overrides.is_empty() {
            let cache_key = (this_type, generics.clone());
            let overrides = if let Some(map) = self.caches.overrides_cache.get(&cache_key) {
                map.clone()
            } else {
                let mut map = std::collections::HashMap::new();
                for ovr in def.overrides.iter() {
                    let decl = self.loader.locate_method(
                        this_type.resolution,
                        ovr.declaration,
                        generics,
                        None,
                    )?;
                    let impl_m = self.loader.locate_method(
                        this_type.resolution,
                        ovr.implementation,
                        generics,
                        Some(this_type.into()),
                    )?;
                    map.insert(decl.method as *const _ as usize, impl_m);
                }
                let arc_map = Arc::new(map);
                self.caches
                    .overrides_cache
                    .insert(cache_key, arc_map.clone());
                arc_map
            };

            if let Some(impl_m) = overrides.get(&(method.method as *const _ as usize)) {
                self.caches
                    .vmt_cache
                    .insert((method, this_type, generics.clone()), *impl_m);
                return Ok(Some(*impl_m));
            }
        }

        if let Some(this_method) = self.loader.find_method_in_type_with_substitution(
            this_type,
            &method.method.name,
            &method.method.signature,
            method.resolution(),
            generics,
        ) {
            self.caches
                .vmt_cache
                .insert((method, this_type, generics.clone()), this_method);
            return Ok(Some(this_method));
        }

        Ok(None)
    }

    pub fn locate_method(
        &self,
        resolution: ResolutionS,
        handle: UserMethod,
        generic_inst: &GenericLookup,
        pre_resolved_parent: Option<ConcreteType>,
    ) -> Result<MethodDescription, TypeResolutionError> {
        self.loader
            .locate_method(resolution, handle, generic_inst, pre_resolved_parent)
    }

    pub fn locate_field(
        &self,
        resolution: ResolutionS,
        field: FieldSource,
        generics: &GenericLookup,
    ) -> Result<(FieldDescription, GenericLookup), TypeResolutionError> {
        self.loader.locate_field(resolution, field, generics)
    }
}

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

impl ResolverService {
    fn is_delegate_type_in_hierarchy(
        &self,
        this_type: &TypeDescription,
        ctx: &ResolutionContext<'_>,
    ) -> bool {
        ctx.get_ancestors(this_type.clone()).any(|(parent, _)| {
            let raw_type_name = parent.type_name();
            let type_name = self.loader.canonical_type_name(&raw_type_name);
            type_name == "System.Delegate" || type_name == "System.MulticastDelegate"
        })
    }

    pub fn is_intrinsic_cached(&self, method: MethodDescription) -> bool {
        if let Some(cached) = self.caches.intrinsic_cache.get(&method) {
            if let Some(shared) = self.shared_state() {
                shared.metrics.record_intrinsic_cache_hit();
            }
            return *cached;
        }

        if let Some(shared) = self.shared_state() {
            shared.metrics.record_intrinsic_cache_miss();
        }
        let result = crate::intrinsics::is_intrinsic(
            method.clone(),
            self.loader(),
            &self.caches.intrinsic_registry,
        );
        self.caches.intrinsic_cache.insert(method.clone(), result);
        result
    }

    pub fn is_intrinsic_field_cached(&self, field: FieldDescription) -> bool {
        if let Some(cached) = self.caches.intrinsic_field_cache.get(&field) {
            if let Some(shared) = self.shared_state() {
                shared.metrics.record_intrinsic_field_cache_hit();
            }
            return *cached;
        }

        if let Some(shared) = self.shared_state() {
            shared.metrics.record_intrinsic_field_cache_miss();
        }
        let result = crate::intrinsics::is_intrinsic_field(
            field.clone(),
            self.loader(),
            &self.caches.intrinsic_registry,
        );
        self.caches.intrinsic_field_cache.insert(field, result);
        result
    }

    pub fn find_generic_method(
        &self,
        source: &MethodSource,
        ctx: &ResolutionContext<'_>,
    ) -> Result<(MethodDescription, GenericLookup), TypeResolutionError> {
        tracing::debug!(
            "find_generic_method: source={:?}, ctx.generics={:?}",
            source,
            ctx.generics
        );
        let mut new_lookup = ctx.generics.clone();

        let method = match source {
            MethodSource::User(u) => {
                let m = *u;
                tracing::debug!("find_generic_method: User method={:?}", m);
                m
            }
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
            tracing::debug!("find_generic_method: parent_type={:?}", t);
            let parent = ctx.make_concrete(t)?;
            tracing::debug!("find_generic_method: parent_type concrete={:?}", parent);
            if let BaseType::Type {
                source: TypeSource::Generic { parameters, .. },
                ..
            } = parent.get()
            {
                new_lookup.type_generics = parameters.clone().into();
            }
            parent_type = Some(parent);
        }

        let method_desc = ctx.locate_method(method, &new_lookup, parent_type)?;

        #[cfg(feature = "generic-constraint-validation")]
        {
            if !method_desc.method().generic_parameters.is_empty() {
                new_lookup.validate_constraints(
                    method_desc.resolution(),
                    self.loader(),
                    &method_desc.method().generic_parameters,
                    true,
                )?;
            }
        }

        Ok((method_desc, new_lookup))
    }

    pub fn resolve_virtual_method(
        &self,
        base_method: MethodDescription,
        this_type: TypeDescription,
        generics: &GenericLookup,
        ctx: &ResolutionContext<'_>,
    ) -> Result<MethodDescription, TypeResolutionError> {
        let key = (base_method.clone(), this_type.clone(), generics.clone());
        if let Some(cached) = self.caches.vmt_cache.get(&key) {
            if let Some(shared) = self.shared_state() {
                shared.metrics.record_vmt_cache_hit();
            }
            return Ok(cached.clone());
        }

        if let Some(shared) = self.shared_state() {
            shared.metrics.record_vmt_cache_miss();
        }

        // Delegate Invoke/BeginInvoke/EndInvoke methods are runtime-synthesized and have no
        // concrete virtual override entries in metadata tables.
        let method_name = &*base_method.method().name;
        if base_method.method().body.is_none()
            && matches!(method_name, "Invoke" | "BeginInvoke" | "EndInvoke")
            && self.is_delegate_type_in_hierarchy(&this_type, ctx)
        {
            self.caches.vmt_cache.insert(key, base_method.clone());
            return Ok(base_method);
        }

        // Standard virtual method resolution: search ancestors
        let is_interface = matches!(base_method.parent.definition().flags.kind, Kind::Interface);

        for (parent, _) in ctx.get_ancestors(this_type.clone()) {
            if let Some(this_method) =
                self.find_and_cache_method(parent, base_method.clone(), &key.2, is_interface)?
            {
                self.caches.vmt_cache.insert(key, this_method.clone());
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
        allow_variance: bool,
    ) -> Result<Option<MethodDescription>, TypeResolutionError> {
        let def = this_type.definition();
        if !def.overrides.is_empty() {
            let cache_key = (this_type.clone(), generics.clone());
            let overrides = if let Some(map) = self.caches.overrides_cache.get(&cache_key) {
                if let Some(shared) = self.shared_state() {
                    shared.metrics.record_overrides_cache_hit();
                }
                map.clone()
            } else {
                if let Some(shared) = self.shared_state() {
                    shared.metrics.record_overrides_cache_miss();
                }
                let mut map = std::collections::HashMap::new();
                for ovr in def.overrides.iter() {
                    let decl = self.loader.locate_method(
                        this_type.resolution.clone(),
                        ovr.declaration,
                        generics,
                        None,
                    )?;
                    let impl_m = self.loader.locate_method(
                        this_type.resolution.clone(),
                        ovr.implementation,
                        generics,
                        Some(this_type.clone().into()),
                    )?;
                    map.insert(decl, impl_m);
                }
                let arc_map = Arc::new(map);
                self.caches
                    .overrides_cache
                    .insert(cache_key, arc_map.clone());
                arc_map
            };

            if let Some(impl_m) = overrides.get(&method) {
                self.caches.vmt_cache.insert(
                    (method.clone(), this_type.clone(), generics.clone()),
                    impl_m.clone(),
                );
                return Ok(Some(impl_m.clone()));
            }
        }

        if let Some(this_method) = self.loader.find_method_in_type_with_substitution(
            this_type.clone(),
            &method.method().name,
            &method.method().signature,
            method.resolution(),
            generics,
            allow_variance,
        ) {
            self.caches
                .vmt_cache
                .insert((method, this_type, generics.clone()), this_method.clone());
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

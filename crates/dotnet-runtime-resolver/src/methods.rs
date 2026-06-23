use crate::{ResolverExecutionContext, ResolverService};
use dotnet_types::{
    TypeDescription,
    error::TypeResolutionError,
    generics::{ConcreteType, GenericLookup},
    members::{FieldDescription, MethodDescription},
    resolution::ResolutionS,
};
use dotnetdll::prelude::*;
use std::sync::Arc;

impl<C, L> ResolverService<C, L>
where
    C: crate::ResolverCacheAdapter,
    L: crate::ResolverLayoutAdapter,
{
    #[inline]
    fn is_delegate_type_in_hierarchy(&self, this_type: &TypeDescription) -> bool {
        self.loader.ancestors(this_type.clone()).any(|(parent, _)| {
            let raw_type_name = parent.type_name();
            let type_name = self.loader.canonical_type_name(&raw_type_name);
            type_name == "System.Delegate" || type_name == "System.MulticastDelegate"
        })
    }

    #[inline]
    pub fn is_intrinsic_cached(&self, method: MethodDescription) -> bool {
        if let Some(cached) = self.caches.get_intrinsic_cached(&method) {
            return cached;
        }

        let result = self
            .caches
            .compute_is_intrinsic(method.clone(), self.loader());
        self.caches.set_intrinsic_cached(method, result);
        result
    }

    #[inline]
    pub fn is_intrinsic_field_cached(&self, field: FieldDescription) -> bool {
        if let Some(cached) = self.caches.get_intrinsic_field_cached(&field) {
            return cached;
        }

        let result = self
            .caches
            .compute_is_intrinsic_field(field.clone(), self.loader());
        self.caches.set_intrinsic_field_cached(field, result);
        result
    }

    pub fn find_generic_method<Ctx: ResolverExecutionContext>(
        &self,
        source: &MethodSource,
        ctx: &Ctx,
    ) -> Result<(MethodDescription, GenericLookup), TypeResolutionError> {
        tracing::debug!(
            "find_generic_method: source={:?}, ctx.generics={:?}",
            source,
            ctx.generics()
        );
        let mut new_lookup = ctx.generics().clone();

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
                    .map(|t| self.make_concrete(ctx.resolution().clone(), ctx.generics(), t))
                    .collect::<Result<Vec<_>, _>>()?;
                new_lookup.method_generics = params.into();
                g.base
            }
        };

        let mut parent_type = None;
        if let UserMethod::Reference(r) = method
            && let MethodReferenceParent::Type(t) = &ctx.resolution()[r].parent
        {
            tracing::debug!("find_generic_method: parent_type={:?}", t);
            let parent = self.make_concrete(ctx.resolution().clone(), ctx.generics(), t)?;
            tracing::debug!("find_generic_method: parent_type concrete={:?}", parent);
            // Use make_lookup() which goes through the thread-local LRU cache.
            let parent_lookup = parent.make_lookup();
            if !parent_lookup.type_generics.is_empty() {
                new_lookup.type_generics = parent_lookup.type_generics;
            }
            parent_type = Some(parent);
        }

        let method_desc =
            self.locate_method(ctx.resolution().clone(), method, &new_lookup, parent_type)?;

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

    pub fn resolve_virtual_method<Ctx: ResolverExecutionContext>(
        &self,
        base_method: MethodDescription,
        this_type: TypeDescription,
        generics: &GenericLookup,
        _ctx: &Ctx,
    ) -> Result<MethodDescription, TypeResolutionError> {
        // Strip method generics that exceed the base method's declared arity.
        // Callers in generic contexts (e.g. a callvirt inside a generic method) pass
        // the full calling-context lookup, which may contain spurious method generics
        // unrelated to the dispatched method. These extra generics corrupt override map
        // keys and VMT cache lookups, causing MethodNotFound for valid overrides.
        let method_generic_arity = base_method.method().generic_parameters.len();
        let trimmed_generics;
        let generics = if generics.method_generics.len() > method_generic_arity {
            trimmed_generics = GenericLookup {
                type_generics: generics.type_generics.clone(),
                method_generics: generics.method_generics[..method_generic_arity].into(),
            };
            &trimmed_generics
        } else {
            generics
        };

        // `callvirt` is used for both virtual and non-virtual instance calls.
        // For non-virtual members, dispatch target is the referenced method itself.
        if !base_method.method().virtual_member {
            self.caches.record_vmt_key_clones(3);
            self.caches.set_vmt_cached(
                base_method.clone(),
                this_type.clone(),
                generics.clone(),
                base_method.clone(),
            );
            return Ok(base_method);
        }

        if let Some(cached) = self
            .caches
            .get_vmt_cached(&base_method, &this_type, generics)
        {
            return Ok(cached);
        }

        let is_interface = matches!(base_method.parent.definition().flags.kind, Kind::Interface);

        if is_interface
            && let Some(array_helper_method) =
                self.try_resolve_array_generic_interface_method(&base_method, &this_type, generics)?
        {
            self.caches.record_vmt_key_clones(3);
            self.caches.set_vmt_cached(
                base_method.clone(),
                this_type.clone(),
                generics.clone(),
                array_helper_method.clone(),
            );
            return Ok(array_helper_method);
        }

        // Delegate Invoke/BeginInvoke/EndInvoke methods are runtime-synthesized and have no
        // concrete virtual override entries in metadata tables.
        let method_name = &*base_method.method().name;
        // Order matters under lazy method-body decoding: the cheap name/type predicates are checked
        // first so `body()` (which decodes IL on first access) only fires for delegate-named methods
        // on delegate types, not on every virtual-method resolution.
        if matches!(method_name, "Invoke" | "BeginInvoke" | "EndInvoke")
            && self.is_delegate_type_in_hierarchy(&this_type)
            && base_method.body().is_none()
        {
            self.caches.record_vmt_key_clones(3);
            self.caches.set_vmt_cached(
                base_method.clone(),
                this_type.clone(),
                generics.clone(),
                base_method.clone(),
            );
            return Ok(base_method);
        }

        // Standard virtual method resolution: search ancestors.
        //
        // Each ancestor must be searched with the type-generic lookup that is valid
        // for that ancestor itself. Reusing the receiver lookup unchanged across the
        // whole chain is incorrect for inherited generic bases (e.g.
        // GroupByIterator<TSource,TKey> : Iterator<IGrouping<TKey,TSource>>), and can
        // cause fields on the base to be read with the wrong runtime type.
        let ancestors: Vec<_> = self.loader.ancestors(this_type.clone()).collect();
        let mut ancestor_lookup = generics.clone();

        for (index, (parent, extends_generics)) in ancestors.iter().enumerate() {
            if let Some(this_method) = self.find_and_cache_method(
                parent.clone(),
                base_method.clone(),
                &ancestor_lookup,
                is_interface,
            )? {
                self.caches.record_vmt_key_clones(3);
                self.caches.set_vmt_cached(
                    base_method.clone(),
                    this_type.clone(),
                    generics.clone(),
                    this_method.clone(),
                );
                return Ok(this_method);
            }

            if index + 1 < ancestors.len() {
                let next_type_generics = extends_generics
                    .iter()
                    .map(|t| self.make_concrete(parent.resolution.clone(), &ancestor_lookup, *t))
                    .collect::<Result<Vec<_>, _>>()?;
                ancestor_lookup = GenericLookup {
                    type_generics: next_type_generics.into(),
                    method_generics: generics.method_generics.clone(),
                };
            }
        }

        Err(TypeResolutionError::MethodNotFound(
            format!(
                "could not find virtual method implementation of {:?} in {:?} with generics {:?}",
                base_method, this_type, generics
            )
            .into(),
        ))
    }

    fn try_resolve_array_generic_interface_method(
        &self,
        base_method: &MethodDescription,
        this_type: &TypeDescription,
        generics: &GenericLookup,
    ) -> Result<Option<MethodDescription>, TypeResolutionError> {
        let this_type_name = this_type.type_name();
        if self.loader.canonical_type_name(&this_type_name) != "System.Array" {
            return Ok(None);
        }

        let parent_name = base_method.parent.type_name();
        let canonical_parent = self.loader.canonical_type_name(&parent_name);
        let supported = matches!(
            canonical_parent,
            "System.Collections.Generic.IEnumerable`1"
                | "System.Collections.Generic.ICollection`1"
                | "System.Collections.Generic.IList`1"
                | "System.Collections.Generic.IReadOnlyCollection`1"
                | "System.Collections.Generic.IReadOnlyList`1"
        );
        if !supported {
            return Ok(None);
        }

        let Some(element_type) = base_method.parent_generics.type_generics.first().cloned() else {
            return Ok(None);
        };

        let mut signature_lookup = base_method.parent_generics.clone();
        signature_lookup.method_generics = generics.method_generics.clone();

        let helper_generics = GenericLookup {
            type_generics: vec![element_type].into(),
            method_generics: vec![].into(),
        };

        let helper_type = self.loader.corlib_type("DotnetRs.SZArrayHelper`1")?;

        Ok(self.loader.find_method_in_type_internal(
            helper_type,
            &base_method.method().name,
            base_method.signature(),
            base_method.resolution(),
            Some(&signature_lookup),
            Some(&helper_generics),
            true,
        ))
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
            let overrides = if let Some(map) = self.caches.get_overrides_cached(&cache_key) {
                map
            } else {
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
                self.caches.set_overrides_cached(cache_key, arc_map.clone());
                arc_map
            };

            if let Some(impl_m) = overrides.get(&method) {
                self.caches.record_vmt_key_clones(3);
                self.caches.set_vmt_cached(
                    method.clone(),
                    this_type.clone(),
                    generics.clone(),
                    impl_m.clone(),
                );
                return Ok(Some(impl_m.clone()));
            }

            // Bridge declaration identity across facade/CoreLib duplicates where
            // `MethodDescription` equality can miss due differing parent/resolution
            // identities despite equivalent canonical type + signature.
            if let Some((_, impl_m)) = overrides.iter().find(|(decl, _)| {
                let decl_parent_name = decl.parent.type_name();
                let method_parent_name = method.parent.type_name();
                let canonical_decl = self.loader.canonical_type_name(&decl_parent_name);
                let canonical_method = self.loader.canonical_type_name(&method_parent_name);
                if canonical_decl != canonical_method || decl.method().name != method.method().name
                {
                    return false;
                }

                let comparer = self.loader.comparer();
                if allow_variance {
                    comparer.signatures_compatible_with_variance(
                        &method.method_resolution,
                        method.signature(),
                        Some(&method.parent_generics),
                        &decl.method_resolution,
                        decl.signature(),
                        Some(&decl.parent_generics),
                    )
                } else {
                    comparer.signatures_equal(
                        &method.method_resolution,
                        method.signature(),
                        Some(&method.parent_generics),
                        &decl.method_resolution,
                        decl.signature(),
                        Some(&decl.parent_generics),
                    )
                }
            }) {
                self.caches.record_vmt_key_clones(3);
                self.caches.set_vmt_cached(
                    method.clone(),
                    this_type.clone(),
                    generics.clone(),
                    impl_m.clone(),
                );
                return Ok(Some(impl_m.clone()));
            }
        }

        // Signature-side generics must follow the base declaration's declaring type
        // (e.g., Iterator<T>), while candidate-side generics follow the current
        // runtime type being searched (e.g., IteratorSelectIterator<TSource,TResult>).
        let mut signature_lookup = method.parent_generics.clone();
        signature_lookup.method_generics = generics.method_generics.clone();

        if let Some(this_method) = self.loader.find_method_in_type_internal(
            this_type.clone(),
            &method.method().name,
            method.signature(),
            method.resolution(),
            Some(&signature_lookup),
            Some(generics),
            allow_variance,
        ) {
            self.caches.record_vmt_key_clones(3);
            self.caches
                .set_vmt_cached(method, this_type, generics.clone(), this_method.clone());
            return Ok(Some(this_method));
        }

        Ok(None)
    }

    #[inline]
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

    #[inline]
    pub fn locate_field(
        &self,
        resolution: ResolutionS,
        field: FieldSource,
        generics: &GenericLookup,
    ) -> Result<(FieldDescription, GenericLookup), TypeResolutionError> {
        self.loader.locate_field(resolution, field, generics)
    }
}

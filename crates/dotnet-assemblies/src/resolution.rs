use crate::{AssemblyLoader, error::AssemblyLoadError, loader::SUPPORT_ASSEMBLY};
use dotnet_types::{
    TypeDescription, TypeResolver,
    error::TypeResolutionError,
    generics::{ConcreteType, GenericLookup},
    members::{FieldDescription, MethodDescription},
    resolution::{MetadataArena, ResolutionS},
};
use dotnet_utils::sync::Ordering;
use dotnetdll::prelude::{FieldSource, *};
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
    ptr,
    sync::Arc,
};
use thiserror::Error;

impl TypeResolver for AssemblyLoader {
    fn corlib_type(&self, name: &str) -> Result<TypeDescription, TypeResolutionError> {
        self.corlib_type(name)
    }

    fn locate_type(
        &self,
        resolution: ResolutionS,
        handle: UserType,
    ) -> Result<TypeDescription, TypeResolutionError> {
        self.locate_type(resolution, handle)
    }

    fn find_concrete_type(&self, ty: ConcreteType) -> Result<TypeDescription, TypeResolutionError> {
        self.find_concrete_type(ty)
    }

    fn canonical_type_name<'a>(&'a self, name: &'a str) -> &'a str {
        self.canonical_type_name(name)
    }
}

impl AssemblyLoader {
    fn find_exported_type(
        &self,
        resolution: ResolutionS,
        e: &ExportedType,
    ) -> Result<TypeDescription, TypeResolutionError> {
        match e.implementation {
            TypeImplementation::Nested(_) => Err(TypeResolutionError::TypeNotFound(
                format!("Nested exported type not implemented: {}", e.type_name()).into(),
            )),
            TypeImplementation::ModuleFile { .. } => Err(TypeResolutionError::TypeNotFound(
                format!(
                    "ModuleFile exported type not implemented: {}",
                    e.type_name()
                )
                .into(),
            )),
            TypeImplementation::TypeForwarder(a) => {
                self.find_in_assembly(&resolution[a], &e.type_name())
            }
        }
    }

    fn parse_and_find_type_definition_index<'a>(
        &self,
        resolution: &ResolutionS,
        full_name: &'a str,
        include_nested_fallback: bool,
    ) -> (&'a str, &'a str, Option<usize>) {
        let (namespace, name) = if let Some(idx) = full_name.rfind('.') {
            (&full_name[..idx], &full_name[idx + 1..])
        } else {
            ("", full_name)
        };

        if let Some(index) = resolution
            .definition()
            .type_definitions
            .iter()
            .enumerate()
            .find(|(_, t)| t.name == name && t.namespace.as_deref().unwrap_or("") == namespace)
            .map(|(index, _)| index)
        {
            return (namespace, name, Some(index));
        }

        let nested_index = if include_nested_fallback {
            self.find_nested_type_definition_index(resolution, full_name)
        } else {
            None
        };

        (namespace, name, nested_index)
    }

    fn find_nested_type_definition_index(
        &self,
        resolution: &ResolutionS,
        full_name: &str,
    ) -> Option<usize> {
        if !(full_name.contains('+') || full_name.contains('/')) {
            return None;
        }

        let normalized_target = full_name.replace('/', "+");
        for (index, t) in resolution.definition().type_definitions.iter().enumerate() {
            if t.encloser.is_some() {
                let type_index = resolution.definition().type_definition_index(index)?;
                let td = TypeDescription::new(resolution.clone(), type_index);
                if td.type_name().replace('/', "+") == normalized_target {
                    return Some(index);
                }
            }
        }

        None
    }

    pub fn find_in_assembly(
        &self,
        assembly: &ExternalAssemblyReference,
        full_name: &str,
    ) -> Result<TypeDescription, TypeResolutionError> {
        if let Some(t) = self.stubs.get(full_name) {
            return Ok(t.clone());
        }

        let res = self
            .get_assembly_with_version(assembly.name.as_ref(), Some(assembly.version))
            .map_err(|e| TypeResolutionError::AssemblyLoad(e.to_string().into()))?;

        let (namespace, name, type_def_index) =
            self.parse_and_find_type_definition_index(&res, full_name, false);

        if let Some(index) = type_def_index {
            let type_index = res
                .definition()
                .type_definition_index(index)
                .ok_or_else(|| {
                    TypeResolutionError::TypeNotFound(
                        "Internal error: invalid type definition index".into(),
                    )
                })?;
            return Ok(TypeDescription::new(res, type_index));
        }

        for e in &res.definition().exported_types {
            if e.name == name && e.namespace.as_deref().unwrap_or("") == namespace {
                return self.find_exported_type(res, e);
            }
        }

        Err(TypeResolutionError::TypeNotFound(
            format!(
                "could not find type {} in assembly {}",
                full_name, assembly.name
            )
            .into(),
        ))
    }

    pub fn corlib_type(&self, name: &str) -> Result<TypeDescription, TypeResolutionError> {
        if let Some(t) = self.corlib_cache.get(name) {
            return Ok(t.clone());
        }

        let result = self.corlib_type_internal(name)?;
        self.corlib_cache.insert(name.to_string(), result.clone());
        Ok(result)
    }

    fn corlib_type_internal(&self, name: &str) -> Result<TypeDescription, TypeResolutionError> {
        if let Some(t) = self.stubs.get(name) {
            return Ok(t.clone());
        }

        let mut tried_mscorlib = false;
        if self.external.read().contains_key("mscorlib") {
            let res = self
                .get_assembly("mscorlib")
                .map_err(|e| TypeResolutionError::AssemblyLoad(e.to_string().into()))?;
            if let Some(t) = self.try_find_in_assembly(res, name) {
                return Ok(t);
            }
            tried_mscorlib = true;
        }

        if self.external.read().contains_key("System.Private.CoreLib") {
            let res = self
                .get_assembly("System.Private.CoreLib")
                .map_err(|e| TypeResolutionError::AssemblyLoad(e.to_string().into()))?;
            if let Some(t) = self.try_find_in_assembly(res, name) {
                return Ok(t);
            }
        }

        if self.external.read().contains_key(SUPPORT_ASSEMBLY) {
            let res = self
                .get_assembly(SUPPORT_ASSEMBLY)
                .map_err(|e| TypeResolutionError::AssemblyLoad(e.to_string().into()))?;
            if let Some(t) = self.try_find_in_assembly(res, name) {
                return Ok(t);
            }
        }

        // Fallback to old behavior
        if tried_mscorlib {
            Err(TypeResolutionError::TypeNotFound(
                format!("could not find type {} in corlib", name).into(),
            ))
        } else {
            self.find_in_assembly(&ExternalAssemblyReference::new("mscorlib"), name)
        }
    }

    fn try_find_in_assembly(
        &self,
        resolution: ResolutionS,
        full_name: &str,
    ) -> Option<TypeDescription> {
        let (namespace, name, type_def_index) =
            self.parse_and_find_type_definition_index(&resolution, full_name, false);

        if let Some(index) = type_def_index {
            let type_index = resolution
                .definition()
                .type_definition_index(index)
                .unwrap();
            return Some(TypeDescription::new(resolution, type_index));
        }

        for e in &resolution.definition().exported_types {
            if e.name == name && e.namespace.as_deref().unwrap_or("") == namespace {
                return self.find_exported_type(resolution, e).ok();
            }
        }

        let (_, _, nested_type_index) =
            self.parse_and_find_type_definition_index(&resolution, full_name, true);
        if let Some(index) = nested_type_index {
            let type_index = resolution
                .definition()
                .type_definition_index(index)
                .unwrap();
            return Some(TypeDescription::new(resolution, type_index));
        }

        None
    }

    pub fn locate_type(
        &self,
        resolution: ResolutionS,
        handle: UserType,
    ) -> Result<TypeDescription, TypeResolutionError> {
        let key = (resolution.clone(), handle);
        if let Some(cached) = self.type_cache.get(&key) {
            self.type_cache_hits.fetch_add(1, Ordering::Relaxed);
            return Ok(cached.clone());
        }

        self.type_cache_misses.fetch_add(1, Ordering::Relaxed);
        let result = match handle {
            // Definition handles are already resolved against a concrete metadata table.
            // Redirecting them to support stubs can mismatch member indices and method bodies.
            UserType::Definition(d) => TypeDescription::new(resolution, d),
            UserType::Reference(r) => self.locate_type_ref(resolution, r)?,
        };

        self.type_cache.insert(key, result.clone());
        Ok(result)
    }

    fn locate_type_ref(
        &self,
        resolution: ResolutionS,
        r: TypeRefIndex,
    ) -> Result<TypeDescription, TypeResolutionError> {
        let type_ref = &resolution[r];

        use ResolutionScope::*;
        match &type_ref.scope {
            ExternalModule(_) => Err(TypeResolutionError::TypeNotFound(
                format!(
                    "ExternalModule scope not implemented for type: {}",
                    type_ref.type_name()
                )
                .into(),
            )),
            CurrentModule => Err(TypeResolutionError::TypeNotFound(
                format!(
                    "CurrentModule scope not implemented for type: {}",
                    type_ref.type_name()
                )
                .into(),
            )),
            Assembly(a) => self.find_in_assembly(&resolution[*a], &type_ref.type_name()),
            Exported => Err(TypeResolutionError::TypeNotFound(
                format!(
                    "Exported (type-forwarder) scope not implemented for type: {}",
                    type_ref.type_name()
                )
                .into(),
            )),
            Nested(o) => {
                let td = self.locate_type_ref(resolution.clone(), *o)?;
                let res = td.resolution.clone();
                let owner = td.definition();

                for t in &res.definition().type_definitions {
                    if let Some(enc) = t.encloser
                        && t.type_name() == type_ref.type_name()
                        && ptr::eq(&res[enc], owner)
                    {
                        let index = res
                            .definition()
                            .type_definitions
                            .iter()
                            .position(|td| ptr::eq(td, t))
                            .ok_or_else(|| {
                                TypeResolutionError::TypeNotFound("Internal error".into())
                            })?;
                        let type_index =
                            res.definition()
                                .type_definition_index(index)
                                .ok_or_else(|| {
                                    TypeResolutionError::TypeNotFound("Internal error".into())
                                })?;
                        return Ok(TypeDescription::new(res, type_index));
                    }
                }

                Err(TypeResolutionError::TypeNotFound(
                    format!(
                        "could not find type {} nested in {}",
                        type_ref.type_name(),
                        owner.type_name()
                    )
                    .into(),
                ))
            }
        }
    }

    pub fn find_concrete_type(
        &self,
        ty: ConcreteType,
    ) -> Result<TypeDescription, TypeResolutionError> {
        match ty.get() {
            BaseType::Type { source, .. } => {
                let parent = match source {
                    TypeSource::User(base) | TypeSource::Generic { base, .. } => *base,
                };

                self.locate_type(ty.resolution(), parent)
            }
            BaseType::Boolean => self.corlib_type("System.Boolean"),
            BaseType::Char => self.corlib_type("System.Char"),
            BaseType::Int8 => self.corlib_type("System.SByte"),
            BaseType::UInt8 => self.corlib_type("System.Byte"),
            BaseType::Int16 => self.corlib_type("System.Int16"),
            BaseType::UInt16 => self.corlib_type("System.UInt16"),
            BaseType::Int32 => self.corlib_type("System.Int32"),
            BaseType::UInt32 => self.corlib_type("System.UInt32"),
            BaseType::Int64 => self.corlib_type("System.Int64"),
            BaseType::UInt64 => self.corlib_type("System.UInt64"),
            BaseType::Float32 => self.corlib_type("System.Single"),
            BaseType::Float64 => self.corlib_type("System.Double"),
            BaseType::IntPtr | BaseType::ValuePointer(_, _) | BaseType::FunctionPointer(_) => {
                self.corlib_type("System.IntPtr")
            }
            BaseType::UIntPtr => self.corlib_type("System.UIntPtr"),
            BaseType::Object => self.corlib_type("System.Object"),
            BaseType::String => self.corlib_type("System.String"),
            BaseType::Vector(_, _) | BaseType::Array(_, _) => self.corlib_type("System.Array"),
        }
    }

    pub fn find_method_in_type_with_substitution(
        &self,
        desc: TypeDescription,
        name: &str,
        signature: &ManagedMethod<MethodType>,
        sig_res: ResolutionS,
        generics: &GenericLookup,
        allow_variance: bool,
    ) -> Option<MethodDescription> {
        tracing::debug!(
            "find_method_in_type_with_substitution: desc={}, name={}, generics={:?}",
            desc.type_name(),
            name,
            generics
        );
        self.comparer().find_method_in_type_with_substitution(
            desc,
            name,
            signature,
            sig_res,
            generics,
            allow_variance,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn find_method_in_type_internal(
        &self,
        desc: TypeDescription,
        name: &str,
        signature: &ManagedMethod<MethodType>,
        sig_res: ResolutionS,
        sig_generics: Option<&GenericLookup>,
        type_generics: Option<&GenericLookup>,
        allow_variance: bool,
    ) -> Option<MethodDescription> {
        self.comparer().find_method_in_type_internal(
            desc,
            name,
            signature,
            sig_res,
            sig_generics,
            type_generics,
            allow_variance,
        )
    }

    pub fn find_method_in_type(
        &self,
        desc: TypeDescription,
        name: &str,
        signature: &ManagedMethod<MethodType>,
        sig_res: ResolutionS,
    ) -> Option<MethodDescription> {
        self.comparer()
            .find_method_in_type(desc, name, signature, sig_res)
    }

    /// Recover the `MethodMemberIndex` for a `MethodIndex` by scanning all sub-collections
    /// of the parent `TypeDefinition` via pointer equality.  This is needed because
    /// `MethodIndex::member` is `pub(crate)` in dotnetdll and has no public accessor.
    fn method_member_index_from_index(
        resolution: &Resolution<'static>,
        d: MethodIndex,
    ) -> MethodMemberIndex {
        let method: *const Method = &resolution[d];
        let def = &resolution[d.parent_type()];
        for (i, m) in def.methods.iter().enumerate() {
            if ptr::eq(m as *const Method, method) {
                return MethodMemberIndex::Method(i);
            }
        }
        for (prop_idx, prop) in def.properties.iter().enumerate() {
            if let Some(getter) = &prop.getter
                && ptr::eq(getter as *const Method, method)
            {
                return MethodMemberIndex::PropertyGetter(prop_idx);
            }
            if let Some(setter) = &prop.setter
                && ptr::eq(setter as *const Method, method)
            {
                return MethodMemberIndex::PropertySetter(prop_idx);
            }
            for (other_idx, other) in prop.other.iter().enumerate() {
                if ptr::eq(other as *const Method, method) {
                    return MethodMemberIndex::PropertyOther {
                        property: prop_idx,
                        other: other_idx,
                    };
                }
            }
        }
        for (event_idx, event) in def.events.iter().enumerate() {
            if ptr::eq(&event.add_listener as *const Method, method) {
                return MethodMemberIndex::EventAdd(event_idx);
            }
            if ptr::eq(&event.remove_listener as *const Method, method) {
                return MethodMemberIndex::EventRemove(event_idx);
            }
            if let Some(raise) = &event.raise_event
                && ptr::eq(raise as *const Method, method)
            {
                return MethodMemberIndex::EventRaise(event_idx);
            }
            for (other_idx, other) in event.other.iter().enumerate() {
                if ptr::eq(other as *const Method, method) {
                    return MethodMemberIndex::EventOther {
                        event: event_idx,
                        other: other_idx,
                    };
                }
            }
        }
        // invariant: a MethodIndex must point into exactly one member sub-collection of its parent type.
        panic!(
            "method_member_index_from_index: method pointer not found in any sub-collection of parent type"
        );
    }

    fn try_resolve_method_via_corelib_fallback(
        &self,
        parent_type: &TypeDescription,
        method_ref: &ExternalMethodReference,
        method_ref_idx: MethodRefIndex,
        resolution: ResolutionS,
        lookup: &GenericLookup,
    ) -> Option<MethodDescription> {
        let corelib_res = self.get_assembly("System.Private.CoreLib").ok()?;
        let corelib_parent = self.try_find_in_assembly(corelib_res, &parent_type.type_name())?;
        let sig = resolution
            .definition()
            .method_ref_signature(method_ref_idx)
            .ok()?;
        self.find_method_in_type_with_substitution(
            corelib_parent,
            &method_ref.name,
            sig,
            resolution,
            lookup,
            false,
        )
    }

    pub fn locate_method(
        &self,
        resolution: ResolutionS,
        handle: UserMethod,
        generic_inst: &GenericLookup,
        pre_resolved_parent: Option<ConcreteType>,
    ) -> Result<MethodDescription, TypeResolutionError> {
        tracing::debug!(
            "locate_method: handle={:?}, generics={:?}, pre_resolved_parent={:?}",
            handle,
            generic_inst,
            pre_resolved_parent
        );
        let key = (
            resolution.clone(),
            handle,
            generic_inst.clone(),
            pre_resolved_parent.clone(),
        );
        if let Some(cached) = self.method_cache.get(&key) {
            self.method_cache_hits.fetch_add(1, Ordering::Relaxed);
            return Ok(cached.clone());
        }

        self.method_cache_misses.fetch_add(1, Ordering::Relaxed);
        let result = match handle {
            UserMethod::Definition(d) => {
                let parent_type = self.locate_type(resolution.clone(), d.parent_type().into())?;
                let method = &resolution[d];

                // `MethodIndex::member` is pub(crate) in dotnetdll with no public accessor.
                // Use the ptr-scan helper to recover the correct `MethodMemberIndex` variant.
                let member_index = Self::method_member_index_from_index(resolution.definition(), d);

                // Prefer support-stub `System.Array.Copy` implementations for definition-bound
                // CoreLib calls (e.g., HashSet.Resize), but keep other definition dispatches
                // on their original declaring type to avoid broad behavior changes.
                let parent_type_name = parent_type.type_name();
                let canonical_parent = self.canonical_type_name(&parent_type_name);
                let method_sig = MethodDescription::new(
                    parent_type.clone(),
                    GenericLookup::default(),
                    resolution.clone(),
                    member_index,
                )
                .signature();
                let should_redirect_array_copy = canonical_parent == "System.Array"
                    && method.name == "Copy"
                    && !method_sig.instance;

                if should_redirect_array_copy
                    && let Some(stub_type) = self.stubs.get(canonical_parent)
                    && let Some(stub_method) = self.find_method_in_type_with_substitution(
                        stub_type.clone(),
                        &method.name,
                        method_sig,
                        resolution.clone(),
                        generic_inst,
                        false,
                    )
                {
                    return Ok(stub_method);
                }
                Ok(MethodDescription::new(
                    parent_type,
                    GenericLookup::default(),
                    resolution.clone(),
                    member_index,
                ))
            }
            UserMethod::Reference(r) => {
                let method_ref = &resolution.definition()[r];

                use MethodReferenceParent::*;
                match &method_ref.parent {
                    Type(t) => {
                        let concrete = if let Some(p) = pre_resolved_parent {
                            p
                        } else {
                            generic_inst.make_concrete(resolution.clone(), t.clone(), self)?
                        };

                        if method_ref.name == ".ctor"
                            && let BaseType::Array(_, _) = concrete.get()
                        {
                            let array_type = self.corlib_type("System.Array")?;
                            for (idx, method) in array_type.definition().methods.iter().enumerate()
                            {
                                if method.name == "CtorArraySentinel" {
                                    return Ok(MethodDescription::new(
                                        array_type.clone(),
                                        GenericLookup::default(),
                                        array_type.resolution.clone(),
                                        MethodMemberIndex::Method(idx),
                                    ));
                                }
                            }
                            return Err(TypeResolutionError::MethodNotFound(
                                "CtorArraySentinel not found in System.Array".into(),
                            ));
                        }

                        let parent_type: TypeDescription =
                            self.find_concrete_type(concrete.clone())?;
                        let parent_generics = concrete.make_lookup();
                        let mut lookup_for_substitution = parent_generics.clone();
                        lookup_for_substitution.method_generics =
                            generic_inst.method_generics.clone();
                        let ref_sig =
                            resolution
                                .definition()
                                .method_ref_signature(r)
                                .map_err(|e| {
                                    TypeResolutionError::MethodNotFound(
                                        format!("failed to decode method ref signature: {e}")
                                            .into(),
                                    )
                                })?;
                        match self.find_method_in_type_with_substitution(
                            parent_type.clone(),
                            &method_ref.name,
                            ref_sig,
                            resolution.clone(),
                            &lookup_for_substitution,
                            false,
                        ) {
                            None => {
                                // Some framework references resolve to facade assemblies where
                                // the referenced type exists but method bodies/signatures are
                                // forwarded to CoreLib. If lookup fails on the facade type, try
                                // the CoreLib type with the same full name before reporting a
                                // hard method-missing error.
                                if let Some(mut method) = self
                                    .try_resolve_method_via_corelib_fallback(
                                        &parent_type,
                                        method_ref,
                                        r,
                                        resolution.clone(),
                                        &lookup_for_substitution,
                                    )
                                {
                                    method.parent_generics = parent_generics;
                                    return Ok(method);
                                }

                                Err(TypeResolutionError::MethodNotFound(
                                    format!(
                                        "could not find {} in type {}",
                                        ref_sig.show_with_name(
                                            resolution.definition(),
                                            &method_ref.name
                                        ),
                                        parent_type.type_name()
                                    )
                                    .into(),
                                ))
                            }
                            Some(mut m) => {
                                m.parent_generics = parent_generics;
                                Ok(m)
                            }
                        }
                    }
                    Module(_) => Err(TypeResolutionError::MethodNotFound(
                        format!(
                            "Module method references are not implemented: {}",
                            method_ref.name
                        )
                        .into(),
                    )),
                    VarargMethod(_) => Err(TypeResolutionError::MethodNotFound(
                        format!(
                            "Vararg method references are not implemented: {}",
                            method_ref.name
                        )
                        .into(),
                    )),
                }
            }
        };

        if let Ok(m) = result {
            self.method_cache.insert(key, m.clone());
            Ok(m)
        } else {
            result
        }
    }

    pub fn locate_attribute(
        &self,
        resolution: ResolutionS,
        attribute: &Attribute,
    ) -> Result<MethodDescription, TypeResolutionError> {
        self.locate_method(
            resolution,
            attribute.constructor,
            &GenericLookup::default(),
            None,
        )
    }

    pub fn locate_field(
        &self,
        resolution: ResolutionS,
        field: FieldSource,
        generic_inst: &GenericLookup,
    ) -> Result<(FieldDescription, GenericLookup), TypeResolutionError> {
        match field {
            FieldSource::Definition(d) => {
                let parent = self.locate_type(resolution.clone(), d.parent_type().into())?;
                let field = &resolution.definition()[d];
                let index = parent
                    .definition()
                    .fields
                    .iter()
                    .position(|f| ptr::eq(f, field))
                    .ok_or_else(|| {
                        TypeResolutionError::FieldNotFound(field.name.as_ref().into())
                    })?;
                Ok((
                    FieldDescription::new(parent, resolution, index),
                    generic_inst.clone(),
                ))
            }
            FieldSource::Reference(r) => {
                let field_ref = &resolution.definition()[r];

                use FieldReferenceParent::*;
                match &field_ref.parent {
                    Type(t) => {
                        let t = generic_inst.make_concrete(resolution, t.clone(), self)?;
                        let parent_type: TypeDescription = self.find_concrete_type(t.clone())?;

                        for (i, field) in parent_type.definition().fields.iter().enumerate() {
                            if field.name == field_ref.name {
                                let type_generics = if let BaseType::Type {
                                    source: TypeSource::Generic { parameters, .. },
                                    ..
                                } = t.get()
                                {
                                    parameters.clone()
                                } else {
                                    vec![]
                                };

                                return Ok((
                                    FieldDescription::new(
                                        parent_type.clone(),
                                        parent_type.resolution.clone(),
                                        i,
                                    ),
                                    GenericLookup::new(type_generics),
                                ));
                            }
                        }

                        Err(TypeResolutionError::FieldNotFound(
                            format!(
                                "could not find {}::{}",
                                parent_type.type_name(),
                                field_ref.name
                            )
                            .into(),
                        ))
                    }
                    _ => Err(TypeResolutionError::FieldNotFound(
                        format!(
                            "Field references with non-type parents are not implemented: {}",
                            field_ref.name
                        )
                        .into(),
                    )),
                }
            }
        }
    }
}

#[derive(Debug, Error)]
#[error("could not resolve attribute")]
pub struct AttrResolveError;

impl<'a> Resolver<'a> for &'a AssemblyLoader {
    type Error = AttrResolveError;

    fn find_type(
        &self,
        name: &str,
    ) -> Result<(&'a TypeDefinition<'a>, &'a Resolution<'a>), Self::Error> {
        if name.contains("=") {
            return Err(AttrResolveError);
        }
        let td = self.corlib_type(name).map_err(|_| AttrResolveError)?;
        Ok((td.definition(), td.resolution.definition()))
    }
}

pub(crate) fn load_resolution_core(
    path: impl AsRef<Path>,
    arena: &Arc<MetadataArena>,
    options: ReadOptions,
) -> Result<ResolutionS, AssemblyLoadError> {
    let path_ref = path.as_ref();
    let mut file = fs::File::open(path_ref)?;

    // Read directly into an 8-byte-aligned Vec<u64>, skipping the intermediate Vec<u8>
    // that a read_to_end + copy approach would require (saves ~12 MB for corlib).
    let len = file.metadata()?.len() as usize;
    let cap = len.div_ceil(8);
    let mut aligned: Vec<u64> = vec![0u64; cap];
    // SAFETY: aligned has `cap * 8 >= len` bytes; the u8 slice is valid for that range.
    let byte_buf = unsafe { std::slice::from_raw_parts_mut(aligned.as_mut_ptr() as *mut u8, len) };
    file.read_exact(byte_buf)?;

    let aligned_boxed = aligned.into_boxed_slice();
    let aligned_ptr = Box::into_raw(aligned_boxed);
    // SAFETY: We manually track this leaked box to reclaim it later.
    let aligned_slice: &'static mut [u64] = unsafe { &mut *aligned_ptr };
    unsafe {
        arena.add_u64_slice(aligned_ptr);
    }

    // Create the byte slice view
    let byte_slice =
        // SAFETY: 'aligned_slice' is valid for its entire length; it contains 'len' bytes read
        // from the file.
        unsafe { std::slice::from_raw_parts(aligned_slice.as_ptr() as *const u8, len) };

    // `options` controls lazy vs eager decoding. The default (see `AssemblyLoader::read_options`)
    // is fully lazy: a VM run typically executes only a fraction of the methods in a loaded
    // assembly (especially System.Private.CoreLib), so deferring IL decoding to first execution
    // via `Resolution::method_body` (see `MethodDescription::body`) avoids decoding bodies that
    // are never run.
    let res = Resolution::parse(byte_slice, options).map_err(AssemblyLoadError::from)?;

    #[cfg(feature = "metadata-validation")]
    crate::validation::validate_metadata(&res)?;

    let res_ptr = Box::into_raw(Box::new(res));
    // SAFETY: We manually track this leaked box to reclaim it later.
    unsafe {
        arena.add_resolution(res_ptr);
    }

    Ok(ResolutionS::new(res_ptr, arena.clone()))
}

fn find_latest_runtime_in_base(base: &Path) -> Option<PathBuf> {
    if !base.exists() {
        return None;
    }

    let entries = fs::read_dir(base).ok()?;
    let mut versions: Vec<_> = entries
        .flatten()
        .filter(|e| e.path().is_dir())
        .filter_map(|e| {
            let name = e.file_name().into_string().ok()?;
            if name.chars().next()?.is_ascii_digit() {
                Some((name, e.path()))
            } else {
                None
            }
        })
        .collect();

    versions.sort_by(|(a, _), (b, _)| {
        let parse_version = |s: &str| {
            s.split('.')
                .map(|part| part.parse::<u32>().unwrap_or(0))
                .collect::<Vec<_>>()
        };
        parse_version(b).cmp(&parse_version(a))
    });

    versions.first().map(|(_, path)| path.clone())
}

pub fn find_dotnet_app_path() -> Option<PathBuf> {
    let mut base_paths = vec![];
    if let Ok(dotnet_root) = std::env::var("DOTNET_ROOT") {
        base_paths.push(PathBuf::from(dotnet_root).join("shared/Microsoft.NETCore.App"));
    }

    if cfg!(target_os = "windows") {
        base_paths.push(PathBuf::from(
            "C:\\Program Files\\dotnet\\shared\\Microsoft.NETCore.App",
        ));
    } else if cfg!(target_os = "macos") {
        base_paths.push(PathBuf::from(
            "/usr/local/share/dotnet/shared/Microsoft.NETCore.App",
        ));
    } else {
        base_paths.push(PathBuf::from(
            "/usr/share/dotnet/shared/Microsoft.NETCore.App",
        ));
        base_paths.push(PathBuf::from(
            "/usr/lib/dotnet/shared/Microsoft.NETCore.App",
        ));
    }

    base_paths
        .into_iter()
        .find_map(|base| find_latest_runtime_in_base(&base))
}

pub fn find_dotnet_sdk_path() -> Option<PathBuf> {
    let base_paths = if cfg!(target_os = "windows") {
        vec![PathBuf::from(
            "C:\\Program Files\\dotnet\\shared\\Microsoft.NETCore.App",
        )]
    } else if cfg!(target_os = "macos") {
        vec![PathBuf::from(
            "/usr/local/share/dotnet/shared/Microsoft.NETCore.App",
        )]
    } else {
        vec![
            PathBuf::from("/usr/share/dotnet/shared/Microsoft.NETCore.App"),
            PathBuf::from("/usr/lib/dotnet/shared/Microsoft.NETCore.App"),
        ]
    };

    base_paths
        .into_iter()
        .find_map(|base| find_latest_runtime_in_base(&base))
}

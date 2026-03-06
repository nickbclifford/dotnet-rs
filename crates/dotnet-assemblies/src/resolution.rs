use crate::{AssemblyLoader, error::AssemblyLoadError, loader::SUPPORT_ASSEMBLY};
use dotnet_types::{
    TypeDescription, TypeResolver,
    error::TypeResolutionError,
    generics::{ConcreteType, GenericLookup},
    members::{FieldDescription, MethodDescription},
    resolution::ResolutionS,
};
use dotnet_utils::sync::Ordering;
use dotnetdll::prelude::*;
use std::{
    error::Error,
    fmt, fs,
    io::Read,
    path::{Path, PathBuf},
    ptr,
};

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
}

impl AssemblyLoader {
    fn find_exported_type(
        &self,
        resolution: ResolutionS,
        e: &ExportedType,
    ) -> Result<TypeDescription, TypeResolutionError> {
        match e.implementation {
            TypeImplementation::Nested(_) => todo!(),
            TypeImplementation::ModuleFile { .. } => todo!(),
            TypeImplementation::TypeForwarder(a) => {
                self.find_in_assembly(&resolution[a], &e.type_name())
            }
        }
    }

    pub fn find_in_assembly(
        &self,
        assembly: &ExternalAssemblyReference,
        full_name: &str,
    ) -> Result<TypeDescription, TypeResolutionError> {
        if let Some(t) = self.stubs.get(full_name) {
            return Ok(*t);
        }

        let res = self
            .get_assembly(assembly.name.as_ref())
            .map_err(|e| TypeResolutionError::AssemblyLoad(e.to_string()))?;

        let (namespace, name) = if let Some(idx) = full_name.rfind('.') {
            (&full_name[..idx], &full_name[idx + 1..])
        } else {
            ("", full_name)
        };

        match res
            .definition()
            .type_definitions
            .iter()
            .find(|t| t.name == name && t.namespace.as_deref().unwrap_or("") == namespace)
        {
            None => {
                for e in &res.definition().exported_types {
                    if e.name == name && e.namespace.as_deref().unwrap_or("") == namespace {
                        return self.find_exported_type(res, e);
                    }
                }
                Err(TypeResolutionError::TypeNotFound(format!(
                    "could not find type {} in assembly {}",
                    full_name, assembly.name
                )))
            }
            Some(t) => {
                let index = res
                    .definition()
                    .type_definitions
                    .iter()
                    .position(|td| ptr::eq(td, t))
                    .ok_or_else(|| {
                        TypeResolutionError::TypeNotFound(
                            "Internal error: type definition not found in its own assembly"
                                .to_string(),
                        )
                    })?;
                let type_index =
                    res.definition()
                        .type_definition_index(index)
                        .ok_or_else(|| {
                            TypeResolutionError::TypeNotFound(
                                "Internal error: invalid type definition index".to_string(),
                            )
                        })?;
                Ok(TypeDescription::new(res, t, type_index))
            }
        }
    }

    pub fn corlib_type(&self, name: &str) -> Result<TypeDescription, TypeResolutionError> {
        if let Some(t) = self.corlib_cache.get(name) {
            return Ok(*t);
        }

        let result = self.corlib_type_internal(name)?;
        self.corlib_cache.insert(name.to_string(), result);
        Ok(result)
    }

    fn corlib_type_internal(&self, name: &str) -> Result<TypeDescription, TypeResolutionError> {
        if let Some(t) = self.stubs.get(name) {
            return Ok(*t);
        }

        let mut tried_mscorlib = false;
        if self.external.read().contains_key("mscorlib") {
            let res = self
                .get_assembly("mscorlib")
                .map_err(|e| TypeResolutionError::AssemblyLoad(e.to_string()))?;
            if let Some(t) = self.try_find_in_assembly(res, name) {
                return Ok(t);
            }
            tried_mscorlib = true;
        }

        if self.external.read().contains_key("System.Private.CoreLib") {
            let res = self
                .get_assembly("System.Private.CoreLib")
                .map_err(|e| TypeResolutionError::AssemblyLoad(e.to_string()))?;
            if let Some(t) = self.try_find_in_assembly(res, name) {
                return Ok(t);
            }
        }

        if self.external.read().contains_key(SUPPORT_ASSEMBLY) {
            let res = self
                .get_assembly(SUPPORT_ASSEMBLY)
                .map_err(|e| TypeResolutionError::AssemblyLoad(e.to_string()))?;
            if let Some(t) = self.try_find_in_assembly(res, name) {
                return Ok(t);
            }
        }

        // Fallback to old behavior
        if tried_mscorlib {
            Err(TypeResolutionError::TypeNotFound(format!(
                "could not find type {} in corlib",
                name
            )))
        } else {
            self.find_in_assembly(&ExternalAssemblyReference::new("mscorlib"), name)
        }
    }

    fn try_find_in_assembly(
        &self,
        resolution: ResolutionS,
        full_name: &str,
    ) -> Option<TypeDescription> {
        let (namespace, name) = if let Some(idx) = full_name.rfind('.') {
            (&full_name[..idx], &full_name[idx + 1..])
        } else {
            ("", full_name)
        };

        if let Some(t) = resolution
            .definition()
            .type_definitions
            .iter()
            .find(|t| t.name == name && t.namespace.as_deref().unwrap_or("") == namespace)
        {
            let index = resolution
                .definition()
                .type_definitions
                .iter()
                .position(|td| ptr::eq(td, t))
                .unwrap();
            let type_index = resolution
                .definition()
                .type_definition_index(index)
                .unwrap();
            return Some(TypeDescription::new(resolution, t, type_index));
        }

        for e in &resolution.definition().exported_types {
            if e.name == name && e.namespace.as_deref().unwrap_or("") == namespace {
                return self.find_exported_type(resolution, e).ok();
            }
        }

        // Fallback for nested types or cases where namespace/name splitting is complex
        if full_name.contains('+') || full_name.contains('/') {
            let normalized_target = full_name.replace('/', "+");
            for (index, t) in resolution.definition().type_definitions.iter().enumerate() {
                if t.encloser.is_some() {
                    let type_index = resolution
                        .definition()
                        .type_definition_index(index)
                        .unwrap();
                    let td = TypeDescription::new(resolution, t, type_index);
                    if td.type_name().replace('/', "+") == normalized_target {
                        return Some(td);
                    }
                }
            }
        }

        None
    }

    pub fn locate_type(
        &self,
        resolution: ResolutionS,
        handle: UserType,
    ) -> Result<TypeDescription, TypeResolutionError> {
        let key = (resolution, handle);
        if let Some(cached) = self.type_cache.get(&key) {
            self.type_cache_hits.fetch_add(1, Ordering::Relaxed);
            return Ok(*cached);
        }

        self.type_cache_misses.fetch_add(1, Ordering::Relaxed);
        let result = match handle {
            UserType::Definition(d) => {
                let def = resolution.definition();
                let definition = &def[d];
                if let Some(t) = self.stubs.get(&definition.type_name()) {
                    *t
                } else {
                    TypeDescription::new(resolution, definition, d)
                }
            }
            UserType::Reference(r) => self.locate_type_ref(resolution, r)?,
        };

        self.type_cache.insert(key, result);
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
            ExternalModule(_) => todo!(),
            CurrentModule => todo!(),
            Assembly(a) => self.find_in_assembly(&resolution[*a], &type_ref.type_name()),
            Exported => todo!(),
            Nested(o) => {
                let td = self.locate_type_ref(resolution, *o)?;
                let res = td.resolution;
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
                                TypeResolutionError::TypeNotFound("Internal error".to_string())
                            })?;
                        let type_index =
                            res.definition()
                                .type_definition_index(index)
                                .ok_or_else(|| {
                                    TypeResolutionError::TypeNotFound("Internal error".to_string())
                                })?;
                        return Ok(TypeDescription::new(res, t, type_index));
                    }
                }

                Err(TypeResolutionError::TypeNotFound(format!(
                    "could not find type {} nested in {}",
                    type_ref.type_name(),
                    owner.type_name()
                )))
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
    ) -> Option<MethodDescription> {
        tracing::debug!(
            "find_method_in_type_with_substitution: desc={}, name={}, generics={:?}",
            desc.type_name(),
            name,
            generics
        );
        self.comparer()
            .find_method_in_type_with_substitution(desc, name, signature, sig_res, generics)
    }

    pub fn find_method_in_type_internal(
        &self,
        desc: TypeDescription,
        name: &str,
        signature: &ManagedMethod<MethodType>,
        sig_res: ResolutionS,
        sig_generics: Option<&GenericLookup>,
        type_generics: Option<&GenericLookup>,
    ) -> Option<MethodDescription> {
        self.comparer().find_method_in_type_internal(
            desc,
            name,
            signature,
            sig_res,
            sig_generics,
            type_generics,
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
            resolution,
            handle,
            generic_inst.clone(),
            pre_resolved_parent.clone(),
        );
        if let Some(cached) = self.method_cache.get(&key) {
            self.method_cache_hits.fetch_add(1, Ordering::Relaxed);
            return Ok(*cached);
        }

        self.method_cache_misses.fetch_add(1, Ordering::Relaxed);
        let result = match handle {
            UserMethod::Definition(d) => Ok(MethodDescription::new(
                self.locate_type(resolution, d.parent_type().into())?,
                resolution,
                &resolution.definition()[d],
            )),
            UserMethod::Reference(r) => {
                let method_ref = &resolution.definition()[r];

                use MethodReferenceParent::*;
                match &method_ref.parent {
                    Type(t) => {
                        let concrete = if let Some(p) = pre_resolved_parent {
                            p
                        } else {
                            generic_inst.make_concrete(resolution, t.clone())?
                        };

                        if method_ref.name == ".ctor"
                            && let BaseType::Array(_, _) = concrete.get()
                        {
                            let array_type = self.corlib_type("System.Array")?;
                            for method in &array_type.definition().methods {
                                if method.name == "CtorArraySentinel" {
                                    return Ok(MethodDescription::new(
                                        array_type,
                                        array_type.resolution,
                                        method,
                                    ));
                                }
                            }
                            return Err(TypeResolutionError::MethodNotFound(
                                "CtorArraySentinel not found in System.Array".to_string(),
                            ));
                        }

                        let parent_type: TypeDescription = self.find_concrete_type(concrete)?;
                        match self.find_method_in_type_with_substitution(
                            parent_type,
                            &method_ref.name,
                            &method_ref.signature,
                            resolution,
                            generic_inst,
                        ) {
                            None => Err(TypeResolutionError::MethodNotFound(format!(
                                "could not find {} in type {}",
                                method_ref
                                    .signature
                                    .show_with_name(resolution.definition(), &method_ref.name),
                                parent_type.type_name()
                            ))),
                            Some(method) => Ok(method),
                        }
                    }
                    Module(_) => todo!("method reference: module"),
                    VarargMethod(_) => todo!("method reference: vararg method"),
                }
            }
        };

        if let Ok(m) = result {
            self.method_cache.insert(key, m);
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
                let parent = self.locate_type(resolution, d.parent_type().into())?;
                let field = &resolution.definition()[d];
                let index = parent
                    .definition()
                    .fields
                    .iter()
                    .position(|f| ptr::eq(f, field))
                    .ok_or_else(|| TypeResolutionError::FieldNotFound(field.name.to_string()))?;
                Ok((
                    FieldDescription::new(parent, resolution, field, index),
                    generic_inst.clone(),
                ))
            }
            FieldSource::Reference(r) => {
                let field_ref = &resolution.definition()[r];

                use FieldReferenceParent::*;
                match &field_ref.parent {
                    Type(t) => {
                        let t = generic_inst.make_concrete(resolution, t.clone())?;
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
                                        parent_type,
                                        parent_type.resolution,
                                        field,
                                        i,
                                    ),
                                    GenericLookup::new(type_generics),
                                ));
                            }
                        }

                        Err(TypeResolutionError::FieldNotFound(format!(
                            "could not find {}::{}",
                            parent_type.type_name(),
                            field_ref.name
                        )))
                    }
                    _ => todo!("Field reference with non-type parent"),
                }
            }
        }
    }
}

#[derive(Debug)]
pub struct AttrResolveError;
impl Error for AttrResolveError {}
impl fmt::Display for AttrResolveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "could not resolve attribute")
    }
}

impl Resolver<'static> for AssemblyLoader {
    type Error = AttrResolveError;

    fn find_type(
        &self,
        name: &str,
    ) -> Result<(&TypeDefinition<'static>, &Resolution<'static>), Self::Error> {
        if name.contains("=") {
            todo!("fully qualified name {}", name)
        }
        let td = self.corlib_type(name).map_err(|_| AttrResolveError)?;
        Ok((td.definition(), td.resolution.definition()))
    }
}

pub fn static_res_from_file(path: impl AsRef<Path>) -> ResolutionS {
    match try_static_res_from_file(path) {
        Ok(r) => r,
        Err(e) => panic!("{}", e),
    }
}

pub fn try_static_res_from_file(path: impl AsRef<Path>) -> Result<ResolutionS, AssemblyLoadError> {
    let (res, _, _) = load_resolution_core(path)?;
    Ok(res)
}

pub(crate) fn load_resolution_core(
    path: impl AsRef<Path>,
) -> Result<(ResolutionS, *const Resolution<'static>, *mut [u64]), AssemblyLoadError> {
    let path_ref = path.as_ref();
    let mut file = fs::File::open(path_ref).map_err(|e| {
        AssemblyLoadError::Io(format!(
            "could not open file {} ({:?})",
            path_ref.display(),
            e
        ))
    })?;
    let mut buf = vec![];
    file.read_to_end(&mut buf)
        .map_err(|e| AssemblyLoadError::Io(format!("failed to read file: {}", e)))?;

    // Ensure alignment by copying to a Vec<u64> backing (8-byte alignment)
    let len = buf.len();
    let cap = len.div_ceil(8);
    let mut aligned: Vec<u64> = vec![0u64; cap];
    // SAFETY: source and destination are valid for 'len' bytes.
    // 'aligned' is newly allocated with sufficient capacity.
    unsafe {
        ptr::copy_nonoverlapping(buf.as_ptr(), aligned.as_mut_ptr() as *mut u8, len);
    }

    let aligned_boxed = aligned.into_boxed_slice();
    let aligned_ptr = Box::into_raw(aligned_boxed);
    // SAFETY: We manually track this leaked box to reclaim it later.
    let aligned_slice: &'static mut [u64] = unsafe { &mut *aligned_ptr };

    // Create the byte slice view
    let byte_slice =
        // SAFETY: 'aligned_slice' is valid for its entire length.
        // It contains 'len' bytes of data from 'buf'.
        unsafe { std::slice::from_raw_parts(aligned_slice.as_ptr() as *const u8, len) };

    let res = Resolution::parse(byte_slice, ReadOptions::default()).map_err(|e| {
        AssemblyLoadError::InvalidFormat(format!("failed to parse resolution: {}", e))
    })?;
    let res_ptr = Box::into_raw(Box::new(res));

    Ok((ResolutionS::new(res_ptr), res_ptr, aligned_ptr))
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

    for base in base_paths {
        if base.exists()
            && let Ok(entries) = fs::read_dir(base)
        {
            let mut versions: Vec<_> = entries
                .flatten()
                .filter(|e| e.path().is_dir())
                .filter_map(|e| {
                    let name = e.file_name().into_string().ok()?;
                    if name.chars().next()?.is_numeric() {
                        Some((name, e.path()))
                    } else {
                        None
                    }
                })
                .collect();

            // Sort versions to get the latest
            versions.sort_by(|(a, _), (b, _)| {
                let parse_version = |s: &str| {
                    s.split('.')
                        .map(|part| part.parse::<u32>().unwrap_or(0))
                        .collect::<Vec<_>>()
                };
                parse_version(b).cmp(&parse_version(a))
            });

            if let Some((_, path)) = versions.first() {
                return Some(path.clone());
            }
        }
    }
    None
}

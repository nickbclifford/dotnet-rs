//! # dotnet-assemblies
//!
//! Assembly loading and metadata resolution for the dotnet-rs VM.
//! This crate handles finding, loading, and caching .NET assemblies from the file system.
use dashmap::DashMap;
use dotnet_types::{
    TypeDescription, TypeResolver,
    comparer::TypeComparer,
    error::TypeResolutionError,
    generics::{ConcreteType, GenericLookup},
    members::{FieldDescription, MethodDescription},
    resolution::ResolutionS,
};
use dotnet_utils::sync::{AtomicU64, Ordering, RwLock};
use dotnetdll::prelude::*;
use gc_arena::{Collect, unsafe_empty_collect};
use std::{
    collections::HashMap,
    error::Error,
    fmt, fs,
    io::Read,
    path::{Path, PathBuf},
    ptr,
};

pub mod error;

use error::AssemblyLoadError;

pub struct AssemblyLoader {
    assembly_root: String,
    external: RwLock<HashMap<String, Option<ResolutionS>>>,
    /// Mapping of canonical BCL names (e.g., "System.Delegate") to their implementation
    /// in the support library (e.g., "DotnetRs.Delegate").
    stubs: HashMap<String, TypeDescription>,
    /// Reverse mapping from support library type names to their canonical BCL names.
    /// Used for name normalization during intrinsic dispatch.
    /// E.g., "DotnetRs.Delegate" → "System.Delegate"
    reverse_stubs: HashMap<String, String>,
    corlib_cache: DashMap<String, TypeDescription>,
    type_cache: DashMap<(ResolutionS, UserType), TypeDescription>,
    method_cache:
        DashMap<(ResolutionS, UserMethod, GenericLookup, Option<ConcreteType>), MethodDescription>,
    pub type_cache_hits: AtomicU64,
    pub type_cache_misses: AtomicU64,
    pub method_cache_hits: AtomicU64,
    pub method_cache_misses: AtomicU64,
}
unsafe_empty_collect!(AssemblyLoader);

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

const SUPPORT_LIBRARY: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/support.dll"));
pub const SUPPORT_ASSEMBLY: &str = "__dotnetrs_support";

impl AssemblyLoader {
    pub fn new(assembly_root: String) -> Result<Self, AssemblyLoadError> {
        let resolutions: HashMap<_, _> = fs::read_dir(&assembly_root)
            .map_err(|e| {
                AssemblyLoadError::Io(format!(
                    "could not read assembly root {}: {}",
                    assembly_root, e
                ))
            })?
            .filter_map(|e| {
                let path = e.ok()?.path();
                if path.extension()? == "dll" {
                    Some((path.file_stem()?.to_string_lossy().into_owned(), None))
                } else {
                    None
                }
            })
            .collect();

        Self::new_internal(assembly_root, resolutions)
    }

    /// Creates a new `AssemblyLoader` without scanning the assembly root for DLLs.
    /// This is useful for testing or when all assemblies are registered manually.
    pub fn new_bare(assembly_root: String) -> Result<Self, AssemblyLoadError> {
        Self::new_internal(assembly_root, HashMap::new())
    }

    fn new_internal(
        assembly_root: String,
        resolutions: HashMap<String, Option<ResolutionS>>,
    ) -> Result<Self, AssemblyLoadError> {
        let mut this = Self {
            assembly_root,
            external: RwLock::new(resolutions),
            stubs: HashMap::new(),
            reverse_stubs: HashMap::new(),
            corlib_cache: DashMap::new(),
            type_cache: DashMap::new(),
            method_cache: DashMap::new(),
            type_cache_hits: AtomicU64::new(0),
            type_cache_misses: AtomicU64::new(0),
            method_cache_hits: AtomicU64::new(0),
            method_cache_misses: AtomicU64::new(0),
        };

        this.add_support_library()?;
        Ok(this)
    }

    fn add_support_library(&mut self) -> Result<(), AssemblyLoadError> {
        // Ensure alignment for SUPPORT_LIBRARY
        let len = SUPPORT_LIBRARY.len();
        let cap = len.div_ceil(8);
        let mut aligned: Vec<u64> = vec![0u64; cap];
        // SAFETY: The source and destination are both valid for 'len' bytes.
        // The destination buffer 'aligned' has enough capacity as it's allocated with 'div_ceil(8)'.
        // Both buffers are non-overlapping as 'aligned' is newly allocated.
        unsafe {
            ptr::copy_nonoverlapping(
                SUPPORT_LIBRARY.as_ptr(),
                aligned.as_mut_ptr() as *mut u8,
                len,
            );
        }
        let aligned_slice = Box::leak(aligned.into_boxed_slice());
        let byte_slice =
            // SAFETY: 'aligned_slice' is a leaked Box<[u64]> which is valid for its entire length.
            // Converting it to a *const u8 slice of length 'len' is safe because it was initialized
            // with exactly 'len' bytes from SUPPORT_LIBRARY.
            unsafe { std::slice::from_raw_parts(aligned_slice.as_ptr() as *const u8, len) };

        let support_res_raw =
            Resolution::parse(byte_slice, ReadOptions::default()).map_err(|e| {
                AssemblyLoadError::InvalidFormat(format!("failed to parse support library: {}", e))
            })?;
        let support_res = Box::leak(Box::new(support_res_raw));
        {
            let mut external = self.external.write();
            external.insert(
                SUPPORT_ASSEMBLY.to_string(),
                Some(ResolutionS::new(support_res)),
            );
        }

        for (index, t) in support_res.type_definitions.iter().enumerate() {
            for a in &t.attributes {
                // the target stub attribute is internal to the support library,
                // so the constructor reference will always be a Definition variant
                let parent = match a.constructor {
                    UserMethod::Definition(d) => &support_res[d.parent_type()],
                    UserMethod::Reference(_) => {
                        continue;
                    }
                };
                if parent.type_name() == "DotnetRs.StubAttribute" {
                    let data = a.instantiation_data(self, &*support_res).map_err(|e| {
                        AssemblyLoadError::InvalidFormat(format!(
                            "failed to parse stub attribute data: {}",
                            e
                        ))
                    })?;
                    for n in data.named_args {
                        match n {
                            NamedArg::Field(name, FixedArg::String(Some(target)))
                                if name == "InPlaceOf" =>
                            {
                                let type_index =
                                    support_res.type_definition_index(index).ok_or_else(|| {
                                        AssemblyLoadError::InvalidFormat(
                                            "failed to find type definition index".to_string(),
                                        )
                                    })?;
                                let support_type_name = t.type_name();
                                self.stubs.insert(
                                    target.to_string(),
                                    TypeDescription::new(
                                        ResolutionS::new(support_res),
                                        t,
                                        type_index,
                                    ),
                                );
                                self.reverse_stubs
                                    .insert(support_type_name, target.to_string());
                            }
                            _ => {}
                        }
                    }
                }
            }
        }
        Ok(())
    }

    pub fn get_root(&self) -> &str {
        &self.assembly_root
    }

    /// Returns the canonical (System.*) name for a stubbed type,
    /// or the input unchanged for non-stub types.
    pub fn canonical_type_name<'a>(&'a self, name: &'a str) -> &'a str {
        self.reverse_stubs
            .get(name)
            .map(|s| s.as_str())
            .unwrap_or(name)
    }

    /// Returns true if the given name is an alias for a stubbed type.
    pub fn is_stub_alias(&self, name: &str) -> bool {
        self.reverse_stubs.contains_key(name)
    }

    pub fn get_assembly(&self, name: &str) -> Result<ResolutionS, AssemblyLoadError> {
        let res = { self.external.read().get(name).copied() };
        match res {
            None => {
                let mut file = PathBuf::from(&self.assembly_root);
                file.push(format!("{name}.dll"));
                if !file.exists() {
                    return Err(AssemblyLoadError::FileNotFound(format!(
                        "could not find assembly {name} in root {}",
                        self.assembly_root
                    )));
                }
                let resolution = try_static_res_from_file(file)?;
                match &resolution.assembly {
                    None => {
                        return Err(AssemblyLoadError::InvalidFormat(
                            "no assembly present in external module".to_string(),
                        ));
                    }
                    Some(a) => {
                        let mut external = self.external.write();
                        external.insert(a.name.to_string(), Some(resolution));
                        // Also cache under the name it was requested as, to prevent redundant loads
                        // when the filename doesn't match the assembly name (e.g. mscorlib -> System.Private.CoreLib)
                        if a.name.as_ref() != name {
                            external.insert(name.to_string(), Some(resolution));
                        }
                    }
                }
                Ok(resolution)
            }
            Some(None) => {
                let mut file = PathBuf::from(&self.assembly_root);
                file.push(format!("{name}.dll"));
                let resolution = try_static_res_from_file(file)?;
                match &resolution.assembly {
                    None => {
                        return Err(AssemblyLoadError::InvalidFormat(
                            "no assembly present in external module".to_string(),
                        ));
                    }
                    Some(a) => {
                        let mut external = self.external.write();
                        external.insert(a.name.to_string(), Some(resolution));
                        if a.name.as_ref() != name {
                            external.insert(name.to_string(), Some(resolution));
                        }
                    }
                }
                Ok(resolution)
            }
            Some(Some(res)) => Ok(res),
        }
    }

    pub fn assemblies(&self) -> Vec<ResolutionS> {
        self.external.read().values().flatten().copied().collect()
    }

    pub fn type_cache_size(&self) -> usize {
        self.type_cache.len()
    }

    pub fn method_cache_size(&self) -> usize {
        self.method_cache.len()
    }

    pub fn register_assembly(&self, resolution: ResolutionS) {
        if let Some(a) = &resolution.assembly {
            self.external
                .write()
                .insert(a.name.to_string(), Some(resolution));
        }
    }

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

    fn comparer(&self) -> TypeComparer<'_, Self> {
        TypeComparer::new(self)
    }

    pub fn find_method_in_type_with_substitution(
        &self,
        desc: TypeDescription,
        name: &str,
        signature: &ManagedMethod<MethodType>,
        sig_res: ResolutionS,
        generics: &GenericLookup,
    ) -> Option<MethodDescription> {
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
            UserMethod::Definition(d) => Ok(MethodDescription {
                parent: self.locate_type(resolution, d.parent_type().into())?,
                method_resolution: resolution,
                method: &resolution.definition()[d],
            }),
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
                                    return Ok(MethodDescription {
                                        parent: array_type,
                                        method_resolution: array_type.resolution,
                                        method,
                                    });
                                }
                            }
                            return Err(TypeResolutionError::MethodNotFound(
                                "CtorArraySentinel not found in System.Array".to_string(),
                            ));
                        }

                        let parent_type: TypeDescription = self.find_concrete_type(concrete)?;
                        match self.find_method_in_type(
                            parent_type,
                            &method_ref.name,
                            &method_ref.signature,
                            resolution,
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
                    FieldDescription {
                        parent,
                        field_resolution: resolution,
                        field,
                        index,
                    },
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
                                    FieldDescription {
                                        parent: parent_type,
                                        field_resolution: parent_type.resolution,
                                        field,
                                        index: i,
                                    },
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

    pub fn ancestors(&self, child: TypeDescription) -> impl Iterator<Item = Ancestor<'_>> + '_ {
        AncestorsImpl {
            assemblies: self,
            child: Some(child),
        }
    }
}

pub type Ancestor<'a> = (TypeDescription, Vec<&'a MemberType>);

struct AncestorsImpl<'a> {
    assemblies: &'a AssemblyLoader,
    child: Option<TypeDescription>,
}
impl<'a> Iterator for AncestorsImpl<'a> {
    type Item = Ancestor<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        let child = self.child?;

        self.child = match &child.definition().extends {
            None => None,
            Some(TypeSource::User(parent) | TypeSource::Generic { base: parent, .. }) => Some(
                self.assemblies
                    .locate_type(child.resolution, *parent)
                    .expect("Failed to locate parent type in ancestors"),
            ),
        };

        let generics = match &child.definition().extends {
            Some(TypeSource::Generic { parameters, .. }) => parameters.iter().collect(),
            _ => vec![],
        };

        Some((child, generics))
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
    type Error = AttrResolveError; // TODO: error handling

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

    // Leak the aligned buffer
    let aligned_slice = Box::leak(aligned.into_boxed_slice());
    // Create the byte slice view
    let byte_slice =
        // SAFETY: 'aligned_slice' is valid for its entire length.
        // It contains 'len' bytes of data from 'buf'.
        unsafe { std::slice::from_raw_parts(aligned_slice.as_ptr() as *const u8, len) };

    let resolution = Resolution::parse(byte_slice, ReadOptions::default())
        .map_err(|e| AssemblyLoadError::InvalidFormat(format!("failed to parse file: {}", e)))?;
    Ok(ResolutionS::new(Box::leak(Box::new(resolution)) as *const _))
}

pub fn find_dotnet_sdk_path() -> Option<PathBuf> {
    let mut search_paths = vec![PathBuf::from(
        "/usr/share/dotnet/shared/Microsoft.NETCore.App",
    )];

    if let Ok(home) = std::env::var("HOME") {
        search_paths.push(PathBuf::from(home).join(".dotnet/shared/Microsoft.NETCore.App"));
    }

    if let Ok(dotnet_root) = std::env::var("DOTNET_ROOT") {
        search_paths.insert(
            0,
            PathBuf::from(dotnet_root).join("shared/Microsoft.NETCore.App"),
        );
    }

    for base_path in search_paths {
        if !base_path.exists() {
            continue;
        }

        let mut versions: Vec<_> = fs::read_dir(base_path)
            .ok()?
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
            .map(|e| e.path())
            .collect();

        versions.sort();
        if let Some(latest) = versions.pop() {
            return Some(latest);
        }
    }

    None
}

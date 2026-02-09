use dashmap::DashMap;
use dotnet_types::{
    TypeDescription, TypeResolver,
    comparer::TypeComparer,
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

pub struct AssemblyLoader {
    assembly_root: String,
    external: RwLock<HashMap<String, Option<ResolutionS>>>,
    stubs: HashMap<String, TypeDescription>,
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
    fn corlib_type(&self, name: &str) -> TypeDescription {
        self.corlib_type(name)
    }

    fn locate_type(&self, resolution: ResolutionS, handle: UserType) -> TypeDescription {
        self.locate_type(resolution, handle)
    }
}

const SUPPORT_LIBRARY: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/support.dll"));
pub const SUPPORT_ASSEMBLY: &str = "__dotnetrs_support";

impl AssemblyLoader {
    pub fn new(assembly_root: String) -> Self {
        let mut resolutions: HashMap<_, _> = fs::read_dir(&assembly_root)
            .unwrap()
            .filter_map(|e| {
                let path = e.unwrap().path();
                if path.extension()? == "dll" {
                    Some((
                        path.file_stem().unwrap().to_string_lossy().into_owned(),
                        None,
                    ))
                } else {
                    None
                }
            })
            .collect();

        // Ensure alignment for SUPPORT_LIBRARY
        let len = SUPPORT_LIBRARY.len();
        let cap = len.div_ceil(8);
        let mut aligned: Vec<u64> = vec![0u64; cap];
        unsafe {
            ptr::copy_nonoverlapping(
                SUPPORT_LIBRARY.as_ptr(),
                aligned.as_mut_ptr() as *mut u8,
                len,
            );
        }
        let aligned_slice = Box::leak(aligned.into_boxed_slice());
        let byte_slice =
            unsafe { std::slice::from_raw_parts(aligned_slice.as_ptr() as *const u8, len) };

        let support_res = Box::leak(Box::new(
            Resolution::parse(byte_slice, ReadOptions::default()).unwrap(),
        ));
        resolutions.insert(
            SUPPORT_ASSEMBLY.to_string(),
            Some(ResolutionS::new(support_res)),
        );
        let mut this = Self {
            assembly_root,
            external: RwLock::new(resolutions),
            stubs: HashMap::new(),
            corlib_cache: DashMap::new(),
            type_cache: DashMap::new(),
            method_cache: DashMap::new(),
            type_cache_hits: AtomicU64::new(0),
            type_cache_misses: AtomicU64::new(0),
            method_cache_hits: AtomicU64::new(0),
            method_cache_misses: AtomicU64::new(0),
        };

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
                    let data = a.instantiation_data(&this, &*support_res).unwrap();
                    for n in data.named_args {
                        match n {
                            NamedArg::Field(name, FixedArg::String(Some(target)))
                                if name == "InPlaceOf" =>
                            {
                                let type_index = support_res.type_definition_index(index).unwrap();
                                this.stubs.insert(
                                    target.to_string(),
                                    TypeDescription::new(
                                        ResolutionS::new(support_res),
                                        t,
                                        type_index,
                                    ),
                                );
                            }
                            _ => {}
                        }
                    }
                }
            }
        }

        this
    }

    pub fn get_root(&self) -> &str {
        &self.assembly_root
    }

    pub fn get_assembly(&self, name: &str) -> ResolutionS {
        let res = { self.external.read().get(name).copied() };
        match res {
            None => {
                let mut file = PathBuf::from(&self.assembly_root);
                file.push(format!("{name}.dll"));
                if !file.exists() {
                    panic!(
                        "could not find assembly {name} in root {}",
                        self.assembly_root
                    );
                }
                let resolution = static_res_from_file(file);
                match &resolution.assembly {
                    None => panic!("no assembly present in external module"),
                    Some(a) => {
                        self.external
                            .write()
                            .insert(a.name.to_string(), Some(resolution));
                    }
                }
                resolution
            }
            Some(None) => {
                let mut file = PathBuf::from(&self.assembly_root);
                file.push(format!("{name}.dll"));
                let resolution = static_res_from_file(file);
                match &resolution.assembly {
                    None => panic!("no assembly present in external module"),
                    Some(a) => {
                        self.external
                            .write()
                            .insert(a.name.to_string(), Some(resolution));
                    }
                }
                resolution
            }
            Some(Some(res)) => res,
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

    fn find_exported_type(&self, resolution: ResolutionS, e: &ExportedType) -> TypeDescription {
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
    ) -> TypeDescription {
        if let Some(t) = self.stubs.get(full_name) {
            return *t;
        }

        let res = self.get_assembly(assembly.name.as_ref());

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
                panic!(
                    "could find type {} in assembly {}",
                    full_name, assembly.name
                )
            }
            Some(t) => {
                let index = res
                    .definition()
                    .type_definitions
                    .iter()
                    .position(|td| ptr::eq(td, t))
                    .unwrap();
                let type_index = res.definition().type_definition_index(index).unwrap();
                TypeDescription::new(res, t, type_index)
            }
        }
    }

    pub fn corlib_type(&self, name: &str) -> TypeDescription {
        if let Some(t) = self.corlib_cache.get(name) {
            return *t;
        }

        let result = self.corlib_type_internal(name);
        self.corlib_cache.insert(name.to_string(), result);
        result
    }

    fn corlib_type_internal(&self, name: &str) -> TypeDescription {
        if let Some(t) = self.stubs.get(name) {
            return *t;
        }

        let mut tried_mscorlib = false;
        if self.external.read().contains_key("mscorlib") {
            let res = self.get_assembly("mscorlib");
            if let Some(t) = self.try_find_in_assembly(res, name) {
                return t;
            }
            tried_mscorlib = true;
        }

        if self.external.read().contains_key("System.Private.CoreLib") {
            let res = self.get_assembly("System.Private.CoreLib");
            if let Some(t) = self.try_find_in_assembly(res, name) {
                return t;
            }
        }

        if self.external.read().contains_key(SUPPORT_ASSEMBLY) {
            let res = self.get_assembly(SUPPORT_ASSEMBLY);
            if let Some(t) = self.try_find_in_assembly(res, name) {
                return t;
            }
        }

        // Fallback to old behavior which panics if not found
        if tried_mscorlib {
            panic!("could not find type {} in corlib", name);
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
                return Some(self.find_exported_type(resolution, e));
            }
        }

        None
    }

    pub fn locate_type(&self, resolution: ResolutionS, handle: UserType) -> TypeDescription {
        let key = (resolution, handle);
        if let Some(cached) = self.type_cache.get(&key) {
            self.type_cache_hits.fetch_add(1, Ordering::Relaxed);
            return *cached;
        }

        self.type_cache_misses.fetch_add(1, Ordering::Relaxed);
        let result = match handle {
            UserType::Definition(d) => {
                let definition = &resolution.definition()[d];
                if let Some(t) = self.stubs.get(&definition.type_name()) {
                    *t
                } else {
                    TypeDescription::new(resolution, definition, d)
                }
            }
            UserType::Reference(r) => self.locate_type_ref(resolution, r),
        };

        self.type_cache.insert(key, result);
        result
    }

    fn locate_type_ref(&self, resolution: ResolutionS, r: TypeRefIndex) -> TypeDescription {
        let type_ref = &resolution[r];

        use ResolutionScope::*;
        match &type_ref.scope {
            ExternalModule(_) => todo!(),
            CurrentModule => todo!(),
            Assembly(a) => self.find_in_assembly(&resolution[*a], &type_ref.type_name()),
            Exported => todo!(),
            Nested(o) => {
                let td = self.locate_type_ref(resolution, *o);
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
                            .unwrap();
                        let type_index = res.definition().type_definition_index(index).unwrap();
                        return TypeDescription::new(res, t, type_index);
                    }
                }

                panic!(
                    "could not find type {} nested in {}",
                    type_ref.type_name(),
                    owner.type_name()
                )
            }
        }
    }

    pub fn find_concrete_type(&self, ty: ConcreteType) -> TypeDescription {
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
    ) -> MethodDescription {
        let key = (
            resolution,
            handle,
            generic_inst.clone(),
            pre_resolved_parent.clone(),
        );
        if let Some(cached) = self.method_cache.get(&key) {
            self.method_cache_hits.fetch_add(1, Ordering::Relaxed);
            return *cached;
        }

        self.method_cache_misses.fetch_add(1, Ordering::Relaxed);
        let result = match handle {
            UserMethod::Definition(d) => MethodDescription {
                parent: self.locate_type(resolution, d.parent_type().into()),
                method_resolution: resolution,
                method: &resolution.definition()[d],
            },
            UserMethod::Reference(r) => {
                let method_ref = &resolution.definition()[r];

                use MethodReferenceParent::*;
                match &method_ref.parent {
                    Type(t) => {
                        let concrete = if let Some(p) = pre_resolved_parent {
                            p
                        } else {
                            generic_inst.make_concrete(resolution, t.clone())
                        };

                        if method_ref.name == ".ctor"
                            && let BaseType::Array(_, _) = concrete.get()
                        {
                            let array_type = self.corlib_type("DotnetRs.Array");
                            for method in &array_type.definition().methods {
                                if method.name == "CtorArraySentinel" {
                                    return MethodDescription {
                                        parent: array_type,
                                        method_resolution: array_type.resolution,
                                        method,
                                    };
                                }
                            }
                            panic!("CtorArraySentinel not found in DotnetRs.Array");
                        }

                        let parent_type = self.find_concrete_type(concrete);
                        match self.find_method_in_type(
                            parent_type,
                            &method_ref.name,
                            &method_ref.signature,
                            resolution,
                        ) {
                            None => panic!(
                                "could not find {} in type {}",
                                method_ref
                                    .signature
                                    .show_with_name(resolution.definition(), &method_ref.name),
                                parent_type.type_name()
                            ),
                            Some(method) => method,
                        }
                    }
                    Module(_) => todo!("method reference: module"),
                    VarargMethod(_) => todo!("method reference: vararg method"),
                }
            }
        };

        self.method_cache.insert(key, result);
        result
    }

    pub fn locate_attribute(
        &self,
        resolution: ResolutionS,
        attribute: &Attribute,
    ) -> MethodDescription {
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
    ) -> (FieldDescription, GenericLookup) {
        match field {
            FieldSource::Definition(d) => (
                FieldDescription {
                    parent: self.locate_type(resolution, d.parent_type().into()),
                    field_resolution: resolution,
                    field: &resolution.definition()[d],
                },
                generic_inst.clone(),
            ),
            FieldSource::Reference(r) => {
                let field_ref = &resolution.definition()[r];

                use FieldReferenceParent::*;
                match &field_ref.parent {
                    Type(t) => {
                        let t = generic_inst.make_concrete(resolution, t.clone());
                        let parent_type = self.find_concrete_type(t.clone());

                        for field in &parent_type.definition().fields {
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

                                return (
                                    FieldDescription {
                                        parent: parent_type,
                                        field_resolution: parent_type.resolution,
                                        field,
                                    },
                                    GenericLookup::new(type_generics),
                                );
                            }
                        }

                        panic!(
                            "could not find {}::{}",
                            parent_type.type_name(),
                            field_ref.name
                        )
                    }
                    Module(_) => todo!("field reference: module"),
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
            Some(TypeSource::User(parent) | TypeSource::Generic { base: parent, .. }) => {
                Some(self.assemblies.locate_type(child.resolution, *parent))
            }
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
        let td = self.corlib_type(name);
        Ok((td.definition(), td.resolution.definition()))
    }
}

pub fn static_res_from_file(path: impl AsRef<Path>) -> ResolutionS {
    let path_ref = path.as_ref();
    let mut file = fs::File::open(path_ref)
        .unwrap_or_else(|e| panic!("could not open file {} ({:?})", path_ref.display(), e));
    let mut buf = vec![];
    file.read_to_end(&mut buf)
        .expect("failed to read_unchecked file");

    // Ensure alignment by copying to a Vec<u64> backing (8-byte alignment)
    let len = buf.len();
    let cap = len.div_ceil(8);
    let mut aligned: Vec<u64> = vec![0u64; cap];
    unsafe {
        ptr::copy_nonoverlapping(buf.as_ptr(), aligned.as_mut_ptr() as *mut u8, len);
    }

    // Leak the aligned buffer
    let aligned_slice = Box::leak(aligned.into_boxed_slice());
    // Create the byte slice view
    let byte_slice =
        unsafe { std::slice::from_raw_parts(aligned_slice.as_ptr() as *const u8, len) };

    let resolution = Resolution::parse(byte_slice, ReadOptions::default())
        .expect("failed to parse file as .NET metadata");
    ResolutionS::new(Box::leak(Box::new(resolution)) as *const _)
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

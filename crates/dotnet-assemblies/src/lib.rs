use dashmap::DashMap;
use dotnet_types::{
    TypeDescription, TypeResolver,
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
            BaseType::Int8 => self.corlib_type("System.Byte"),
            BaseType::UInt8 => self.corlib_type("System.SByte"),
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

    fn type_slices_equal(
        &self,
        res1: ResolutionS,
        a: &[MethodType],
        generics1: Option<&GenericLookup>,
        res2: ResolutionS,
        b: &[MethodType],
        generics2: Option<&GenericLookup>,
    ) -> bool {
        if a.len() != b.len() {
            return false;
        }
        for (a, b) in a.iter().zip(b.iter()) {
            if !self.types_equal(res1, a, generics1, res2, b, generics2) {
                return false;
            }
        }
        true
    }

    fn types_equal(
        &self,
        res1: ResolutionS,
        a: &MethodType,
        generics1: Option<&GenericLookup>,
        res2: ResolutionS,
        b: &MethodType,
        generics2: Option<&GenericLookup>,
    ) -> bool {
        if let Some(generics) = generics1 {
            match a {
                MethodType::TypeGeneric(idx) => {
                    if let Some(concrete) = generics.type_generics.get(*idx) {
                        return self.concrete_equals_method_type(concrete, res2, b, generics2);
                    }
                }
                MethodType::MethodGeneric(idx) => {
                    if let Some(concrete) = generics.method_generics.get(*idx) {
                        return self.concrete_equals_method_type(concrete, res2, b, generics2);
                    }
                }
                _ => {}
            }
        }

        match (a, b) {
            (MethodType::Base(l), MethodType::Base(r)) => match (l.as_ref(), r.as_ref()) {
                (BaseType::Type { source: ts1, .. }, BaseType::Type { source: ts2, .. }) => {
                    let (ut1, generics1_list) = decompose_type_source(ts1);
                    let (ut2, generics2_list) = decompose_type_source(ts2);
                    let td1 = self.locate_type(res1, ut1);
                    let td2 = self.locate_type(res2, ut2);
                    td1 == td2
                        && self.type_slices_equal(
                            res1,
                            &generics1_list,
                            generics1,
                            res2,
                            &generics2_list,
                            generics2,
                        )
                }
                (BaseType::Boolean, BaseType::Boolean) => true,
                (BaseType::Char, BaseType::Char) => true,
                (BaseType::Int8, BaseType::Int8) => true,
                (BaseType::UInt8, BaseType::UInt8) => true,
                (BaseType::Int16, BaseType::Int16) => true,
                (BaseType::UInt16, BaseType::UInt16) => true,
                (BaseType::Int32, BaseType::Int32) => true,
                (BaseType::UInt32, BaseType::UInt32) => true,
                (BaseType::Int64, BaseType::Int64) => true,
                (BaseType::UInt64, BaseType::UInt64) => true,
                (BaseType::Float32, BaseType::Float32) => true,
                (BaseType::Float64, BaseType::Float64) => true,
                (BaseType::IntPtr, BaseType::IntPtr) => true,
                (BaseType::UIntPtr, BaseType::UIntPtr) => true,
                (BaseType::Object, BaseType::Object) => true,
                (BaseType::String, BaseType::String) => true,
                (BaseType::Vector(_, l), BaseType::Vector(_, r)) => {
                    // TODO: CustomTypeModifiers
                    self.types_equal(res1, l, generics1, res2, r, generics2)
                }
                (BaseType::Array(l, _), BaseType::Array(r, _)) => {
                    // TODO: ArrayShapes
                    self.types_equal(res1, l, generics1, res2, r, generics2)
                }
                (BaseType::ValuePointer(_, l), BaseType::ValuePointer(_, r)) => {
                    // TODO: CustomTypeModifiers
                    match (l.as_ref(), r.as_ref()) {
                        (None, None) => true,
                        (Some(t1), Some(t2)) => {
                            self.types_equal(res1, t1, generics1, res2, t2, generics2)
                        }
                        _ => false,
                    }
                }
                (BaseType::FunctionPointer(_l), BaseType::FunctionPointer(_r)) => todo!(),
                _ => false,
            },
            (MethodType::TypeGeneric(i1), MethodType::TypeGeneric(i2)) => {
                if i1 == i2 {
                    return true;
                }
                // Check for substitution on both sides
                if let (Some(g1), Some(g2)) = (generics1, generics2)
                    && let (Some(c1), Some(c2)) =
                        (g1.type_generics.get(*i1), g2.type_generics.get(*i2))
                {
                    return c1 == c2;
                }
                false
            }
            (MethodType::MethodGeneric(i1), MethodType::MethodGeneric(i2)) => {
                if i1 == i2 {
                    return true;
                }
                // Check for substitution on both sides
                if let (Some(g1), Some(g2)) = (generics1, generics2)
                    && let (Some(c1), Some(c2)) =
                        (g1.method_generics.get(*i1), g2.method_generics.get(*i2))
                {
                    return c1 == c2;
                }
                false
            }
            _ => {
                // Check if one side is a substituted generic that matches the other side
                if let Some(generics) = generics2 {
                    match b {
                        MethodType::TypeGeneric(idx) => {
                            if let Some(concrete) = generics.type_generics.get(*idx) {
                                return self
                                    .concrete_equals_method_type(concrete, res1, a, generics1);
                            }
                        }
                        MethodType::MethodGeneric(idx) => {
                            if let Some(concrete) = generics.method_generics.get(*idx) {
                                return self
                                    .concrete_equals_method_type(concrete, res1, a, generics1);
                            }
                        }
                        _ => {}
                    }
                }
                false
            }
        }
    }

    fn param_types_equal(
        &self,
        res1: ResolutionS,
        a: &ParameterType<MethodType>,
        generics1: Option<&GenericLookup>,
        res2: ResolutionS,
        b: &ParameterType<MethodType>,
        generics2: Option<&GenericLookup>,
    ) -> bool {
        match (a, b) {
            (ParameterType::Value(l), ParameterType::Value(r))
            | (ParameterType::Ref(l), ParameterType::Ref(r)) => {
                self.types_equal(res1, l, generics1, res2, r, generics2)
            }
            (ParameterType::TypedReference, ParameterType::TypedReference) => true,
            _ => false,
        }
    }

    fn params_equal(
        &self,
        res1: ResolutionS,
        a: &[Parameter<MethodType>],
        generics1: Option<&GenericLookup>,
        res2: ResolutionS,
        b: &[Parameter<MethodType>],
        generics2: Option<&GenericLookup>,
    ) -> bool {
        if a.len() != b.len() {
            return false;
        }
        for (Parameter(_, a), Parameter(_, b)) in a.iter().zip(b.iter()) {
            if !self.param_types_equal(res1, a, generics1, res2, b, generics2) {
                return false;
            }
        }
        true
    }

    fn signatures_equal(
        &self,
        res1: ResolutionS,
        a: &ManagedMethod<MethodType>,
        generics1: Option<&GenericLookup>,
        res2: ResolutionS,
        b: &ManagedMethod<MethodType>,
        generics2: Option<&GenericLookup>,
    ) -> bool {
        if a.instance != b.instance {
            return false;
        }
        match (&a.return_type, &b.return_type) {
            (ReturnType(_, None), ReturnType(_, None)) => self.params_equal(
                res1,
                &a.parameters,
                generics1,
                res2,
                &b.parameters,
                generics2,
            ),
            (ReturnType(_, Some(l)), ReturnType(_, Some(r)))
                if self.param_types_equal(res1, l, generics1, res2, r, generics2) =>
            {
                self.params_equal(
                    res1,
                    &a.parameters,
                    generics1,
                    res2,
                    &b.parameters,
                    generics2,
                )
            }
            _ => false,
        }
    }

    // Compare a ConcreteType with a MethodType
    fn concrete_equals_method_type(
        &self,
        concrete: &ConcreteType,
        res2: ResolutionS,
        b: &MethodType,
        generics2: Option<&GenericLookup>,
    ) -> bool {
        if let Some(generics) = generics2 {
            match b {
                MethodType::TypeGeneric(idx) => {
                    if let Some(other) = generics.type_generics.get(*idx) {
                        return concrete == other;
                    }
                }
                MethodType::MethodGeneric(idx) => {
                    if let Some(other) = generics.method_generics.get(*idx) {
                        return concrete == other;
                    }
                }
                _ => {}
            }
        }
        match b {
            MethodType::Base(b_base) => {
                // Compare concrete type with a base type
                self.concrete_equals_base_type(concrete, res2, b_base, generics2)
            }
            _ => false,
        }
    }

    // Compare a ConcreteType with a BaseType<MethodType>
    fn concrete_equals_base_type(
        &self,
        concrete: &ConcreteType,
        res2: ResolutionS,
        b: &BaseType<MethodType>,
        generics2: Option<&GenericLookup>,
    ) -> bool {
        let res1 = concrete.resolution();
        let a = concrete.get();

        match (a, b) {
            (BaseType::Type { source: ts1, .. }, BaseType::Type { source: ts2, .. }) => {
                let (ut1, generics1) = decompose_type_source(ts1);
                let (ut2, generics2_list) = decompose_type_source(ts2);
                let td1 = self.locate_type(res1, ut1);
                let td2 = self.locate_type(res2, ut2);

                if td1 != td2 {
                    let d1 = td1.definition();
                    let d2 = td2.definition();
                    if d1.name != d2.name || d1.namespace != d2.namespace {
                        return false;
                    }
                }

                // Check that generic parameters match (ConcreteTypes on left, MethodTypes on right)
                if generics1.len() != generics2_list.len() {
                    return false;
                }

                for (g1, g2) in generics1.iter().zip(generics2_list.iter()) {
                    // g1 is ConcreteType, g2 is MethodType
                    // Compare the ConcreteType with the MethodType
                    if !self.concrete_equals_method_type(g1, res2, g2, generics2) {
                        return false;
                    }
                }
                true
            }
            (BaseType::Boolean, BaseType::Boolean) => true,
            (BaseType::Char, BaseType::Char) => true,
            (BaseType::Int8, BaseType::Int8) => true,
            (BaseType::UInt8, BaseType::UInt8) => true,
            (BaseType::Int16, BaseType::Int16) => true,
            (BaseType::UInt16, BaseType::UInt16) => true,
            (BaseType::Int32, BaseType::Int32) => true,
            (BaseType::UInt32, BaseType::UInt32) => true,
            (BaseType::Int64, BaseType::Int64) => true,
            (BaseType::UInt64, BaseType::UInt64) => true,
            (BaseType::Float32, BaseType::Float32) => true,
            (BaseType::Float64, BaseType::Float64) => true,
            (BaseType::IntPtr, BaseType::IntPtr) => true,
            (BaseType::UIntPtr, BaseType::UIntPtr) => true,
            (BaseType::Object, BaseType::Object) => true,
            (BaseType::String, BaseType::String) => true,
            (BaseType::Vector(_, l), BaseType::Vector(_, r)) => {
                self.concrete_equals_method_type(l, res2, r, generics2)
            }
            (BaseType::Array(l, _), BaseType::Array(r, _)) => {
                self.concrete_equals_method_type(l, res2, r, generics2)
            }
            (BaseType::ValuePointer(_, l), BaseType::ValuePointer(_, r)) => {
                match (l.as_ref(), r.as_ref()) {
                    (None, None) => true,
                    (Some(l_concrete), Some(r_method)) => {
                        self.concrete_equals_method_type(l_concrete, res2, r_method, generics2)
                    }
                    _ => false,
                }
            }
            (BaseType::FunctionPointer(_l), BaseType::FunctionPointer(_r)) => todo!(),
            _ => false,
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
        self.find_method_in_type_internal(desc, name, signature, sig_res, Some(generics))
    }

    pub fn find_method_in_type(
        &self,
        desc: TypeDescription,
        name: &str,
        signature: &ManagedMethod<MethodType>,
        sig_res: ResolutionS,
    ) -> Option<MethodDescription> {
        self.find_method_in_type_internal(desc, name, signature, sig_res, None)
    }

    fn find_method_in_type_internal(
        &self,
        desc: TypeDescription,
        name: &str,
        signature: &ManagedMethod<MethodType>,
        sig_res: ResolutionS,
        generics: Option<&GenericLookup>,
    ) -> Option<MethodDescription> {
        let def = desc.definition();

        macro_rules! check {
            ($method:expr) => {
                let m = $method;
                if m.name == name
                    && m.signature.parameters.len() == signature.parameters.len()
                    && self.signatures_equal(
                        sig_res,
                        signature,
                        generics,
                        desc.resolution,
                        &m.signature,
                        generics,
                    )
                {
                    return Some(MethodDescription {
                        parent: desc,
                        method_resolution: desc.resolution,
                        method: m,
                    });
                }
            };
        }

        // 1. Search regular methods
        for method in &def.methods {
            check!(method);
        }

        // 2. Optimized property search
        if let Some(prop_name) = name.strip_prefix("get_") {
            for prop in &def.properties {
                if prop.name == prop_name
                    && let Some(m) = &prop.getter
                {
                    check!(m);
                }
            }
        } else if let Some(prop_name) = name.strip_prefix("set_") {
            for prop in &def.properties {
                if prop.name == prop_name
                    && let Some(m) = &prop.setter
                {
                    check!(m);
                }
            }
        }

        // 3. Optimized event search
        if let Some(event_name) = name.strip_prefix("add_") {
            for event in &def.events {
                if event.name == event_name {
                    check!(&event.add_listener);
                }
            }
        } else if let Some(event_name) = name.strip_prefix("remove_") {
            for event in &def.events {
                if event.name == event_name {
                    check!(&event.remove_listener);
                }
            }
        } else if let Some(event_name) = name.strip_prefix("raise_") {
            for event in &def.events {
                if event.name == event_name
                    && let Some(m) = &event.raise_event
                {
                    check!(m);
                }
            }
        }

        // 4. Exhaustive search for property/event methods (including 'other' and non-standard names)

        for prop in &def.properties {
            for method in &prop.other {
                check!(method);
            }
            // Check getter/setter again if name didn't follow convention
            if let Some(m) = &prop.getter {
                check!(m);
            }
            if let Some(m) = &prop.setter {
                check!(m);
            }
        }

        for event in &def.events {
            for method in &event.other {
                check!(method);
            }
            // Check listeners again if name didn't follow convention
            check!(&event.add_listener);
            check!(&event.remove_listener);
            if let Some(m) = &event.raise_event {
                check!(m);
            }
        }

        None
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

pub fn decompose_type_source<T: Clone>(t: &TypeSource<T>) -> (UserType, Vec<T>) {
    let mut type_generics: &[T] = &[];
    let ut = match t {
        TypeSource::User(u) => *u,
        TypeSource::Generic { base, parameters } => {
            type_generics = parameters.as_slice();
            *base
        }
    };
    (ut, type_generics.to_vec())
}

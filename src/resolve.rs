use crate::utils::{decompose_type_source, static_res_from_file, ResolutionS};

use dotnetdll::prelude::*;
use gc_arena::{unsafe_empty_collect, Collect};
use std::{collections::HashMap, error::Error, path::PathBuf, sync::RwLock};
use crate::types::members::{FieldDescription, MethodDescription};
use crate::types::generics::{ConcreteType, GenericLookup};
use crate::types::TypeDescription;

pub struct Assemblies {
    assembly_root: String,
    external: RwLock<HashMap<String, Option<ResolutionS>>>,
    stubs: HashMap<String, TypeDescription>,
}
unsafe_empty_collect!(Assemblies);

const SUPPORT_LIBRARY: &[u8] = include_bytes!("support/bin/Debug/net10.0/support.dll");
pub const SUPPORT_ASSEMBLY: &str = "__dotnetrs_support";

impl Assemblies {
    pub fn new(assembly_root: String) -> Self {
        let mut resolutions: HashMap<_, _> = std::fs::read_dir(&assembly_root)
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

        let support_res = Box::leak(Box::new(
            Resolution::parse(SUPPORT_LIBRARY, ReadOptions::default()).unwrap(),
        ));
        resolutions.insert(SUPPORT_ASSEMBLY.to_string(), Some(ResolutionS(support_res)));
        let mut this = Self {
            assembly_root,
            external: RwLock::new(resolutions),
            stubs: HashMap::new(),
        };

        for t in &support_res.type_definitions {
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
                                this.stubs.insert(
                                    target.to_string(),
                                    TypeDescription {
                                        resolution: ResolutionS(support_res),
                                        definition: t,
                                    },
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
        let res = {
            self.external
                .read()
                .unwrap()
                .get(name)
                .copied()
                .unwrap_or_else(|| panic!("could not find assembly {name}"))
        };
        match res {
            None => {
                let mut file = PathBuf::from(&self.assembly_root);
                file.push(format!("{name}.dll"));
                let resolution = static_res_from_file(file);
                match &resolution.assembly {
                    None => panic!("no assembly present in external module"),
                    Some(a) => {
                        self.external
                            .write()
                            .unwrap()
                            .insert(a.name.to_string(), Some(resolution));
                    }
                }
                resolution
            }
            Some(res) => res,
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
        name: &str,
    ) -> TypeDescription {
        if let Some(t) = self.stubs.get(name) {
            return *t;
        }

        let ResolutionS(res) = self.get_assembly(assembly.name.as_ref());
        match res.type_definitions.iter().find(|t| t.type_name() == name) {
            None => {
                for e in &res.exported_types {
                    if e.type_name() == name {
                        return self.find_exported_type(ResolutionS(res), e);
                    }
                }
                panic!("could not find type {} in assembly {}", name, assembly.name)
            }
            Some(t) => TypeDescription {
                resolution: ResolutionS(res),
                definition: t,
            },
        }
    }

    pub fn corlib_type(&self, name: &str) -> TypeDescription {
        if let Some(t) = self.stubs.get(name) {
            return *t;
        }

        let mut tried_mscorlib = false;
        if self.external.read().unwrap().contains_key("mscorlib") {
            let res = self.get_assembly("mscorlib");
            if let Some(t) = self.try_find_in_assembly(res, name) {
                return t;
            }
            tried_mscorlib = true;
        }

        if self
            .external
            .read()
            .unwrap()
            .contains_key("System.Private.CoreLib")
        {
            let res = self.get_assembly("System.Private.CoreLib");
            if let Some(t) = self.try_find_in_assembly(res, name) {
                return t;
            }
        }

        if self.external.read().unwrap().contains_key(SUPPORT_ASSEMBLY) {
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

    fn try_find_in_assembly(&self, resolution: ResolutionS, name: &str) -> Option<TypeDescription> {
        let res = resolution.0;
        if let Some(t) = res.type_definitions.iter().find(|t| t.type_name() == name) {
            return Some(TypeDescription {
                resolution,
                definition: t,
            });
        }

        for e in &res.exported_types {
            if e.type_name() == name {
                return Some(self.find_exported_type(resolution, e));
            }
        }

        None
    }

    // TODO: cache
    pub fn locate_type(&self, resolution: ResolutionS, handle: UserType) -> TypeDescription {
        match handle {
            UserType::Definition(d) => {
                let definition = &resolution.0[d];
                if let Some(t) = self.stubs.get(&definition.type_name()) {
                    return *t;
                }

                TypeDescription {
                    resolution,
                    definition,
                }
            }
            UserType::Reference(r) => self.locate_type_ref(resolution, r),
        }
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
                let TypeDescription {
                    resolution: res,
                    definition: owner,
                } = self.locate_type_ref(resolution, *o);

                for t in &res.0.type_definitions {
                    if let Some(enc) = t.encloser {
                        if t.type_name() == type_ref.type_name() && std::ptr::eq(&res[enc], owner) {
                            return TypeDescription {
                                resolution: res,
                                definition: t,
                            };
                        }
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
        res2: ResolutionS,
        b: &[MethodType],
    ) -> bool {
        if a.len() != b.len() {
            return false;
        }
        for (a, b) in a.iter().zip(b.iter()) {
            if !self.types_equal(res1, a, res2, b) {
                return false;
            }
        }
        true
    }

    fn types_equal(
        &self,
        res1: ResolutionS,
        a: &MethodType,
        res2: ResolutionS,
        b: &MethodType,
    ) -> bool {
        match (a, b) {
            (MethodType::Base(l), MethodType::Base(r)) => match (l.as_ref(), r.as_ref()) {
                (BaseType::Type { source: ts1, .. }, BaseType::Type { source: ts2, .. }) => {
                    let (ut1, generics1) = decompose_type_source(ts1);
                    let (ut2, generics2) = decompose_type_source(ts2);
                    let td1 = self.locate_type(res1, ut1);
                    let td2 = self.locate_type(res2, ut2);
                    td1.type_name() == td2.type_name()
                        && self.type_slices_equal(res1, &generics1, res2, &generics2)
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
                    self.types_equal(res1, l, res2, r)
                }
                (BaseType::Array(l, _), BaseType::Array(r, _)) => {
                    // TODO: ArrayShapes
                    self.types_equal(res1, l, res2, r)
                }
                (BaseType::ValuePointer(_, l), BaseType::ValuePointer(_, r)) => {
                    // TODO: CustomTypeModifiers
                    match (l.as_ref(), r.as_ref()) {
                        (None, None) => true,
                        (Some(t1), Some(t2)) => self.types_equal(res1, t1, res2, t2),
                        _ => false,
                    }
                }
                (BaseType::FunctionPointer(_l), BaseType::FunctionPointer(_r)) => todo!(),
                _ => false,
            },
            (MethodType::TypeGeneric(i1), MethodType::TypeGeneric(i2)) => i1 == i2,
            (MethodType::MethodGeneric(i1), MethodType::MethodGeneric(i2)) => i1 == i2,
            _ => false,
        }
    }

    fn param_types_equal(
        &self,
        res1: ResolutionS,
        a: &ParameterType<MethodType>,
        res2: ResolutionS,
        b: &ParameterType<MethodType>,
    ) -> bool {
        match (a, b) {
            (ParameterType::Value(l), ParameterType::Value(r))
            | (ParameterType::Ref(l), ParameterType::Ref(r)) => self.types_equal(res1, l, res2, r),
            (ParameterType::TypedReference, ParameterType::TypedReference) => true,
            _ => false,
        }
    }

    fn params_equal(
        &self,
        res1: ResolutionS,
        a: &[Parameter<MethodType>],
        res2: ResolutionS,
        b: &[Parameter<MethodType>],
    ) -> bool {
        if a.len() != b.len() {
            return false;
        }
        for (Parameter(_, a), Parameter(_, b)) in a.iter().zip(b.iter()) {
            // TODO: CustomTypeModifiers
            if !self.param_types_equal(res1, a, res2, b) {
                return false;
            }
        }
        true
    }

    fn signatures_equal(
        &self,
        res1: ResolutionS,
        a: &ManagedMethod<MethodType>,
        res2: ResolutionS,
        b: &ManagedMethod<MethodType>,
    ) -> bool {
        if a.instance != b.instance {
            return false;
        }
        match (&a.return_type, &b.return_type) {
            (ReturnType(_, None), ReturnType(_, None)) => {
                self.params_equal(res1, &a.parameters, res2, &b.parameters)
            }
            (ReturnType(_, Some(l)), ReturnType(_, Some(r)))
                if self.param_types_equal(res1, l, res2, r) =>
            {
                self.params_equal(res1, &a.parameters, res2, &b.parameters)
            }
            _ => false,
        }
    }

    pub fn find_method_in_type(
        &self,
        desc: TypeDescription,
        name: &str,
        signature: &ManagedMethod<MethodType>,
        sig_res: ResolutionS,
    ) -> Option<MethodDescription> {
        let mut methods_to_search: Vec<_> = vec![];
        let def = &desc.definition;

        let filter = |n: &str| n.contains('_');

        let (has_underscore, rest): (Vec<_>, _) =
            def.methods.iter().partition(|m| filter(m.name.as_ref()));
        // prefixes required by the standard for properties and events:
        // get_, set_, add_, remove_, raise_
        if filter(name) {
            for p in &def.properties {
                if let Some(m) = &p.getter {
                    methods_to_search.push(m);
                }
                if let Some(m) = &p.setter {
                    methods_to_search.push(m);
                }
            }
            for e in &def.events {
                methods_to_search.push(&e.add_listener);
                methods_to_search.push(&e.remove_listener);
                if let Some(r) = &e.raise_event {
                    methods_to_search.push(r);
                }
            }
            methods_to_search.extend(has_underscore);
        } else {
            methods_to_search.extend(rest);
        }
        methods_to_search.extend(def.events.iter().flat_map(|e| &e.other));

        for method in &methods_to_search {
            if method.name == name
                && self.signatures_equal(sig_res, signature, desc.resolution, &method.signature)
            {
                return Some(MethodDescription {
                    parent: desc,
                    method,
                });
            }
        }

        None
    }

    // TODO: cache
    pub fn locate_method(
        &self,
        resolution: ResolutionS,
        handle: UserMethod,
        generic_inst: &GenericLookup,
    ) -> MethodDescription {
        match handle {
            UserMethod::Definition(d) => MethodDescription {
                parent: TypeDescription {
                    resolution,
                    definition: &resolution.0[d.parent_type()],
                },
                method: &resolution.0[d],
            },
            UserMethod::Reference(r) => {
                let method_ref = &resolution.0[r];

                use MethodReferenceParent::*;
                match &method_ref.parent {
                    Type(t) => {
                        let parent_type = self
                            .find_concrete_type(generic_inst.make_concrete(resolution, t.clone()));
                        match self.find_method_in_type(
                            parent_type,
                            &method_ref.name,
                            &method_ref.signature,
                            resolution,
                        ) {
                            None => panic!(
                                "could not find {}",
                                method_ref
                                    .signature
                                    .show_with_name(resolution.0, &method_ref.name)
                            ),
                            Some(method) => method,
                        }
                    }
                    Module(_) => todo!("method reference: module"),
                    VarargMethod(_) => todo!("method reference: vararg method"),
                }
            }
        }
    }

    pub fn locate_attribute(
        &self,
        resolution: ResolutionS,
        attribute: &Attribute,
    ) -> MethodDescription {
        self.locate_method(resolution, attribute.constructor, &GenericLookup::default())
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
                    parent: TypeDescription {
                        resolution,
                        definition: &resolution.0[d.parent_type()],
                    },
                    field: &resolution.0[d],
                },
                generic_inst.clone(),
            ),
            FieldSource::Reference(r) => {
                let field_ref = &resolution.0[r];

                use FieldReferenceParent::*;
                match &field_ref.parent {
                    Type(t) => {
                        let t = generic_inst.make_concrete(resolution, t.clone());
                        let parent_type = self.find_concrete_type(t.clone());

                        for field in &parent_type.definition.fields {
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
    assemblies: &'a Assemblies,
    child: Option<TypeDescription>,
}
impl<'a> Iterator for AncestorsImpl<'a> {
    type Item = Ancestor<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        let child = self.child?;

        self.child = match &child.definition.extends {
            None => None,
            Some(TypeSource::User(parent) | TypeSource::Generic { base: parent, .. }) => {
                Some(self.assemblies.locate_type(child.resolution, *parent))
            }
        };

        let generics = match &child.definition.extends {
            Some(TypeSource::Generic { parameters, .. }) => parameters.iter().collect(),
            _ => vec![],
        };

        Some((child, generics))
    }
}

#[derive(Debug)]
pub struct AttrResolveError;
impl Error for AttrResolveError {}
impl std::fmt::Display for AttrResolveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "could not resolve attribute")
    }
}

impl Resolver<'static> for Assemblies {
    type Error = AttrResolveError; // TODO: error handling

    fn find_type(
        &self,
        _name: &str,
    ) -> Result<(&TypeDefinition<'static>, &Resolution<'static>), Self::Error> {
        if _name.contains("=") {
            todo!("fully qualified name {}", _name)
        }
        let td = self.corlib_type(_name);
        Ok((td.definition, td.resolution.0))
    }
}

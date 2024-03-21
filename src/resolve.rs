use std::{cell::RefCell, collections::HashMap, path::PathBuf};

use clap::builder::OsStr;
use dotnetdll::prelude::*;

use crate::{
    utils::{static_res_from_file, ResolutionS},
    value::{GenericLookup, TypeDescription},
};

pub struct Assemblies {
    assembly_root: String,
    external: RefCell<HashMap<String, Option<ResolutionS>>>,
    pub entrypoint: ResolutionS,
}

pub type WithSource<T> = (ResolutionS, T);

impl Assemblies {
    pub fn new<'a>(entrypoint: ResolutionS, assembly_root: String) -> Self {
        let resolutions = std::fs::read_dir(&assembly_root)
            .unwrap()
            .filter_map(|e| {
                let path = e.unwrap().path();
                if path.extension()? == OsStr::from("dll") {
                    Some((
                        path.file_stem().unwrap().to_string_lossy().into_owned(),
                        None,
                    ))
                } else {
                    None
                }
            })
            .collect();
        Self {
            assembly_root,
            external: RefCell::new(resolutions),
            entrypoint,
        }
    }

    pub fn get_assembly(&self, name: &str) -> ResolutionS {
        let res = {
            self.external
                .borrow()
                .get(name)
                .copied()
                .unwrap_or_else(|| panic!("could not find assembly {name}"))
        };
        match res {
            None => {
                eprintln!("resolving {name}.dll");
                let mut file = PathBuf::from(&self.assembly_root);
                file.push(format!("{name}.dll"));
                let resolution = static_res_from_file(file);
                match &resolution.assembly {
                    None => todo!("no assembly present in external module"),
                    Some(a) => {
                        self.external
                            .borrow_mut()
                            .insert(a.name.to_string(), Some(resolution));
                    }
                }
                resolution
            }
            Some(res) => res,
        }
    }

    fn find_exported_type(
        &self,
        resolution: ResolutionS,
        e: &ExportedType,
    ) -> WithSource<TypeDescription> {
        match e.implementation {
            TypeImplementation::Nested(x) => todo!(),
            TypeImplementation::ModuleFile { .. } => todo!(),
            TypeImplementation::TypeForwarder(a) => {
                self.find_in_assembly(&resolution[a], &e.type_name())
            }
        }
    }

    fn find_in_assembly(
        &self,
        assembly: &ExternalAssemblyReference,
        name: &str,
    ) -> WithSource<TypeDescription> {
        let res = self.get_assembly(assembly.name.as_ref());
        println!("searching for type {} in assembly {}", name, assembly.name);
        match res.type_definitions.iter().find(|t| t.type_name() == name) {
            None => {
                for e in &res.exported_types {
                    if e.type_name() == name {
                        return self.find_exported_type(res, e);
                    }
                }
                panic!("could not find type {} in assembly {}", name, assembly.name)
            }
            Some(t) => (res, TypeDescription(t)),
        }
    }

    // TODO: cache
    pub fn locate_type(
        &self,
        resolution: ResolutionS,
        handle: UserType,
    ) -> WithSource<TypeDescription> {
        match handle {
            UserType::Definition(d) => (resolution, TypeDescription(&resolution[d])),
            UserType::Reference(r) => {
                let type_ref = &resolution[r];
                println!("looking for {}", type_ref.type_name());

                use ResolutionScope::*;
                match &type_ref.scope {
                    ExternalModule(_) => todo!(),
                    CurrentModule => todo!(),
                    Assembly(a) => {
                        let (res, t) =
                            self.find_in_assembly(&resolution[*a], &type_ref.type_name());
                        println!("found {}", t.0.type_name());
                        (res, t)
                    }
                    Exported => todo!(),
                    Nested(_) => todo!(),
                }
            }
        }
    }

    // TODO: cache
    pub fn locate_method(
        &self,
        resolution: ResolutionS,
        handle: UserMethod,
        generic_inst: &GenericLookup,
    ) -> WithSource<&'static Method<'static>> {
        match handle {
            UserMethod::Definition(d) => (self.entrypoint, &self.entrypoint[d]),
            UserMethod::Reference(r) => {
                let method_ref = &self.entrypoint[r];

                use MethodReferenceParent::*;
                match &method_ref.parent {
                    Type(t) => match generic_inst.make_concrete(t.clone()).get() {
                        BaseType::Type { source, .. } => {
                            let parent = match source {
                                TypeSource::User(base) | TypeSource::Generic { base, .. } => *base,
                            };

                            let (res, parent_type) = self.locate_type(resolution, parent);

                            for method in &parent_type.0.methods {
                                if method.name == method_ref.name {
                                    // TODO: properly check sigs instead of this horrifying hack lol
                                    if method_ref.signature.show(res) == method.signature.show(res)
                                    {
                                        println!(
                                            "found {}",
                                            method.signature.show_with_name(
                                                res,
                                                format!(
                                                    "{}::{}",
                                                    parent_type.0.type_name(),
                                                    method.name
                                                )
                                            )
                                        );
                                        return (res, method);
                                    }
                                }
                            }

                            panic!(
                                "could not find {}",
                                method_ref.signature.show_with_name(res, &method_ref.name)
                            )
                        }
                        BaseType::Boolean => todo!("System.Boolean"),
                        BaseType::Char => todo!("System.Char"),
                        BaseType::Int8 => todo!("System.Byte"),
                        BaseType::UInt8 => todo!("System.SByte"),
                        BaseType::Int16 => todo!("System.Int16"),
                        BaseType::UInt16 => todo!("System.UInt16"),
                        BaseType::Int32 => todo!("System.Int32"),
                        BaseType::UInt32 => todo!("System.UInt32"),
                        BaseType::Int64 => todo!("System.Int64"),
                        BaseType::UInt64 => todo!("System.UInt64"),
                        BaseType::Float32 => todo!("System.Single"),
                        BaseType::Float64 => todo!("System.Double"),
                        BaseType::IntPtr => todo!("System.IntPtr"),
                        BaseType::UIntPtr => todo!("System.UIntPtr"),
                        BaseType::Object => todo!("System.Object"),
                        BaseType::String => todo!("System.String"),
                        BaseType::Vector(_, _) | BaseType::Array(_, _) => todo!("System.Array"),
                        BaseType::ValuePointer(_, _) | BaseType::FunctionPointer(_) => {
                            todo!("pointer types cannot be parents of a method call")
                        }
                    },
                    Module(_) => todo!("method reference: module"),
                    VarargMethod(_) => todo!("method reference: vararg method"),
                }
            }
        }
    }

    pub fn ancestors(
        &self,
        resolution: ResolutionS,
        child: TypeDescription,
    ) -> Box<dyn Iterator<Item = TypeDescription>> {
        match &child.0.extends {
            None => Box::new(std::iter::empty()),
            Some(TypeSource::User(parent) | TypeSource::Generic { base: parent, .. }) => {
                let (res, parent) = self.locate_type(resolution, *parent);
                Box::new(std::iter::once(parent).chain(self.ancestors(res, parent)))
            }
        }
    }
}

impl Resolver<'static> for Assemblies {
    type Error = AlwaysFails; // TODO: error handling

    fn find_type(
        &self,
        name: &str,
    ) -> Result<(&TypeDefinition<'static>, &Resolution<'static>), Self::Error> {
        todo!()
    }
}

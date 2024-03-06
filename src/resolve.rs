use std::collections::HashMap;
use std::path::PathBuf;

use dotnetdll::prelude::*;

use crate::value::{ConcreteType, GenericLookup};
use crate::{
    utils::{static_res_from_file, ResolutionS},
    value::TypeDescription,
};

pub struct Assemblies {
    external: HashMap<String, ResolutionS>,
    pub root: ResolutionS,
}

impl Assemblies {
    pub fn new(root: ResolutionS, external_files: impl Iterator<Item = PathBuf>) -> Self {
        let mut resolutions = HashMap::new();
        for name in external_files {
            let resolution = static_res_from_file(name);
            match &resolution.assembly {
                None => todo!("no assembly present in external module"),
                Some(a) => {
                    resolutions.insert(a.name.to_string(), resolution);
                }
            }
        }
        Self {
            external: resolutions,
            root,
        }
    }

    pub fn locate_type(&self, handle: UserType) -> TypeDescription {
        match handle {
            UserType::Definition(d) => TypeDescription(&self.root[d]),
            UserType::Reference(r) => {
                let type_ref = &self.root[r];

                use ResolutionScope::*;
                match &type_ref.scope {
                    ExternalModule(_) => todo!(),
                    CurrentModule => todo!(),
                    Assembly(a) => {
                        let assembly = &self.root[*a];
                        match self.external.get(assembly.name.as_ref()) {
                            None => todo!("external assembly not provided ({})", assembly.name),
                            Some(res) => match res
                                .type_definitions
                                .iter()
                                .find(|t| t.type_name() == type_ref.type_name())
                            {
                                None => todo!("could not find type in corresponding assembly"),
                                Some(t) => TypeDescription(t),
                            },
                        }
                    }
                    Exported => todo!(),
                    Nested(_) => todo!(),
                }
            }
        }
    }

    pub fn locate_method(
        &self,
        handle: UserMethod,
        generic_inst: &GenericLookup,
    ) -> &'static Method<'static> {
        match handle {
            UserMethod::Definition(d) => &self.root[d],
            UserMethod::Reference(r) => {
                let method_ref = &self.root[r];

                use MethodReferenceParent::*;
                match &method_ref.parent {
                    Type(t) => match generic_inst.make_concrete(t.clone()).get() {
                        BaseType::Type { source, .. } => {
                            let parent = match source {
                                TypeSource::User(base) | TypeSource::Generic { base, .. } => *base,
                            };
                            let parent_type = self.locate_type(parent);
                            todo!("search through methods by signature")
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

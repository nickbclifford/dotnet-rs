use std::{cell::RefCell, collections::HashMap, ffi::OsString, path::PathBuf};

use dotnetdll::prelude::*;

use crate::{
    utils::{static_res_from_file, ResolutionS},
    value::{ConcreteType, FieldDescription, GenericLookup, MethodDescription, TypeDescription},
};

pub struct Assemblies {
    assembly_root: String,
    external: RefCell<HashMap<String, Option<ResolutionS>>>,
    pub entrypoint: ResolutionS,
}

impl Assemblies {
    pub fn new<'a>(entrypoint: ResolutionS, assembly_root: String) -> Self {
        let resolutions = std::fs::read_dir(&assembly_root)
            .unwrap()
            .filter_map(|e| {
                let path = e.unwrap().path();
                if path.extension()? == OsString::from("dll") {
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

    pub fn get_root(&self) -> &str {
        &self.assembly_root
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
        let res = self.get_assembly(assembly.name.as_ref());
        match res.type_definitions.iter().find(|t| t.type_name() == name) {
            None => {
                for e in &res.exported_types {
                    if e.type_name() == name {
                        return self.find_exported_type(res, e);
                    }
                }
                panic!("could not find type {} in assembly {}", name, assembly.name)
            }
            Some(t) => TypeDescription {
                resolution: res,
                definition: t,
            },
        }
    }

    pub fn corlib_type(&self, name: &str) -> TypeDescription {
        self.find_in_assembly(&ExternalAssemblyReference::new("mscorlib"), name)
    }

    // TODO: cache
    pub fn locate_type(&self, resolution: ResolutionS, handle: UserType) -> TypeDescription {
        match handle {
            UserType::Definition(d) => TypeDescription {
                resolution,
                definition: &resolution[d],
            },
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

                for t in &res.type_definitions {
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

    pub fn find_method_in_type(
        &self,
        desc: TypeDescription,
        name: &str,
        signature: &ManagedMethod<MethodType>,
    ) -> Option<MethodDescription> {
        for method in &desc.definition.methods {
            if method.name == name && signature == &method.signature {
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
                    definition: &resolution[d.parent_type()],
                },
                method: &resolution[d],
            },
            UserMethod::Reference(r) => {
                let method_ref = &resolution[r];

                use MethodReferenceParent::*;
                match &method_ref.parent {
                    Type(t) => {
                        let parent_type = self
                            .find_concrete_type(generic_inst.make_concrete(resolution, t.clone()));
                        match self.find_method_in_type(
                            parent_type,
                            &method_ref.name,
                            &method_ref.signature,
                        ) {
                            None => panic!(
                                "could not find {}",
                                method_ref
                                    .signature
                                    .show_with_name(parent_type.resolution, &method_ref.name)
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
    ) -> FieldDescription {
        match field {
            FieldSource::Definition(d) => FieldDescription {
                parent: TypeDescription {
                    resolution,
                    definition: &resolution[d.parent_type()],
                },
                field: &resolution[d],
            },
            FieldSource::Reference(r) => {
                let field_ref = &resolution[r];

                use FieldReferenceParent::*;
                match &field_ref.parent {
                    Type(t) => {
                        let parent_type = self
                            .find_concrete_type(generic_inst.make_concrete(resolution, t.clone()));

                        for field in &parent_type.definition.fields {
                            if field.name == field_ref.name {
                                return FieldDescription {
                                    parent: parent_type,
                                    field,
                                };
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

    pub fn ancestors(&self, child: TypeDescription) -> impl Iterator<Item = Ancestor> + '_ {
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

impl Resolver<'static> for Assemblies {
    type Error = AlwaysFails; // TODO: error handling

    fn find_type(
        &self,
        _name: &str,
    ) -> Result<(&TypeDefinition<'static>, &Resolution<'static>), Self::Error> {
        todo!()
    }
}

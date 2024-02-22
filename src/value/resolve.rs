use super::TypeDescription;
use crate::utils::static_res_from_file;
use dotnetdll::prelude::*;
use std::collections::HashMap;
use std::path::PathBuf;

pub struct Assemblies {
    external: HashMap<String, &'static Resolution<'static>>,
    root: &'static Resolution<'static>,
}

impl Assemblies {
    pub fn new(
        root: &'static Resolution<'static>,
        external_files: impl Iterator<Item = PathBuf>,
    ) -> Self {
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

    pub fn get_type(&self, handle: UserType) -> TypeDescription {
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
                            None => todo!("external assembly not provided"),
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
}

impl Resolver<'static> for Assemblies {
    type Error = (); // TODO: error handling

    fn find_type(
        &self,
        name: &str,
    ) -> Result<(&TypeDefinition<'static>, &Resolution<'static>), Self::Error> {
        todo!()
    }
}

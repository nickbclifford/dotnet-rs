use crate::AssemblyLoader;
use dotnet_types::TypeDescription;
use dotnetdll::prelude::*;

impl AssemblyLoader {
    pub fn ancestors(
        &self,
        child: TypeDescription,
    ) -> impl Iterator<Item = Ancestor<'static>> + '_ {
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
    type Item = Ancestor<'static>;

    fn next(&mut self) -> Option<Self::Item> {
        let child = self.child.take()?;

        self.child = match &child.definition().extends {
            None => None,
            Some(TypeSource::User(parent) | TypeSource::Generic { base: parent, .. }) => {
                let raw_parent_name = parent.type_name(child.resolution.definition());
                let parent_name = self.assemblies.canonical_type_name(&raw_parent_name);
                if matches!(parent_name, "System.Delegate" | "System.MulticastDelegate") {
                    Some(
                        self.assemblies
                            .corlib_type(parent_name)
                            .expect("Failed to locate delegate parent type in ancestors"),
                    )
                } else {
                    Some(
                        self.assemblies
                            .locate_type(child.resolution.clone(), *parent)
                            .expect("Failed to locate parent type in ancestors"),
                    )
                }
            }
        };

        let generics: Vec<&'static MemberType> = match &child.definition().extends {
            Some(TypeSource::Generic { parameters, .. }) => parameters.iter().collect(),
            _ => vec![],
        };

        Some((child, generics))
    }
}

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
        let child = self.child?;

        self.child = match &child.definition().extends {
            None => None,
            Some(TypeSource::User(parent) | TypeSource::Generic { base: parent, .. }) => Some(
                self.assemblies
                    .locate_type(child.resolution, *parent)
                    .expect("Failed to locate parent type in ancestors"),
            ),
        };

        let generics: Vec<&'static MemberType> = match &child.definition().extends {
            Some(TypeSource::Generic { parameters, .. }) => parameters.iter().collect(),
            _ => vec![],
        };

        Some((child, generics))
    }
}

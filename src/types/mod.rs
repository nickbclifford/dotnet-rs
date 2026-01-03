use std::fmt::{Debug, Formatter};
use std::hash::{Hash, Hasher};
use dotnetdll::prelude::{MemberType, ResolvedDebug, TypeDefinition, TypeSource};
use gc_arena::{Collect, unsafe_empty_collect};
use crate::types::members::MethodDescription;
use crate::utils::ResolutionS;
use crate::vm::context::ResolutionContext;

pub mod members;
pub mod generics;

#[derive(Clone, Copy)]
pub struct TypeDescription {
    pub resolution: ResolutionS,
    pub definition: &'static TypeDefinition<'static>,
}
unsafe_empty_collect!(TypeDescription);

impl Debug for TypeDescription {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.definition.show(self.resolution.0))
    }
}

impl PartialEq for TypeDescription {
    fn eq(&self, other: &Self) -> bool {
        std::ptr::eq(self.definition, other.definition)
    }
}

impl Eq for TypeDescription {}

impl Hash for TypeDescription {
    fn hash<H: Hasher>(&self, state: &mut H) {
        (self.definition as *const TypeDefinition).hash(state);
    }
}

impl TypeDescription {
    pub fn static_initializer(&self) -> Option<MethodDescription> {
        self.definition.methods.iter().find_map(|m| {
            if m.runtime_special_name
                && m.name == ".cctor"
                && !m.signature.instance
                && m.signature.parameters.is_empty()
            {
                Some(MethodDescription {
                    parent: *self,
                    method: m,
                })
            } else {
                None
            }
        })
    }

    pub fn type_name(&self) -> String {
        self.definition.nested_type_name(self.resolution.0)
    }

    pub fn is_enum(&self) -> Option<&MemberType> {
        match &self.definition.extends {
            Some(TypeSource::User(u))
                if matches!(u.type_name(self.resolution.0).as_str(), "System.Enum") =>
            {
                let inner = self.definition.fields.first()?;
                if inner.runtime_special_name && inner.name == "value__" {
                    Some(&inner.return_type)
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    pub fn is_value_type(&self, ctx: &ResolutionContext) -> bool {
        for (a, _) in ctx.get_ancestors(*self) {
            if matches!(a.type_name().as_str(), "System.Enum" | "System.ValueType") {
                return true;
            }
        }
        false
    }

    pub fn has_finalizer(&self, ctx: &ResolutionContext) -> bool {
        for (ancestor, _) in ctx.get_ancestors(*self) {
            let ns = ancestor.definition.namespace.as_deref().unwrap_or("");
            let name = &ancestor.definition.name;
            if ns == "System" && name == "Object" {
                continue;
            }
            if ns == "System" && name == "ValueType" {
                continue;
            }
            if ns == "System" && name == "Enum" {
                continue;
            }

            if ancestor.definition.methods.iter().any(|m| {
                m.name == "Finalize" && m.virtual_member && m.signature.parameters.is_empty()
            }) {
                return true;
            }
        }
        false
    }
}
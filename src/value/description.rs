use std::fmt::{Debug, Formatter};
use std::hash::{Hash, Hasher};
use dotnetdll::prelude::{Field, MemberType, Method, ResolvedDebug, TypeDefinition, TypeSource};
use gc_arena::{Collect, unsafe_empty_collect};
use crate::utils::ResolutionS;
use crate::value::ResolutionContext;

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

#[derive(Clone, Copy)]
pub struct MethodDescription {
    pub parent: TypeDescription,
    pub method: &'static Method<'static>,
}

impl MethodDescription {
    pub fn resolution(&self) -> ResolutionS {
        self.parent.resolution
    }
}

impl Debug for MethodDescription {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}",
            self.method.signature.show_with_name(
                self.resolution().0,
                format!("{}::{}", self.parent.type_name(), self.method.name)
            )
        )
    }
}

impl PartialEq for MethodDescription {
    fn eq(&self, other: &Self) -> bool {
        std::ptr::eq(self.method, other.method)
    }
}

impl Eq for MethodDescription {}

impl Hash for MethodDescription {
    fn hash<H: Hasher>(&self, state: &mut H) {
        (self.method as *const Method).hash(state);
    }
}

#[derive(Clone, Copy)]
pub struct FieldDescription {
    pub parent: TypeDescription,
    pub field: &'static Field<'static>,
}

impl Debug for FieldDescription {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        if self.field.static_member {
            write!(f, "static ")?;
        }

        write!(
            f,
            "{} {}::{}",
            self.field.return_type.show(self.parent.resolution.0),
            self.parent.type_name(),
            self.field.name
        )?;

        Ok(())
    }
}

impl PartialEq for FieldDescription {
    fn eq(&self, other: &Self) -> bool {
        std::ptr::eq(self.field, other.field)
    }
}

impl Eq for FieldDescription {}

impl Hash for FieldDescription {
    fn hash<H: Hasher>(&self, state: &mut H) {
        (self.field as *const Field).hash(state);
    }
}
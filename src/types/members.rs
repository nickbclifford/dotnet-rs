use crate::{types::TypeDescription, utils::ResolutionS};
use dotnetdll::prelude::{Field, Method, ResolvedDebug};
use std::{
    fmt::{Debug, Formatter},
    hash::{Hash, Hasher},
};

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

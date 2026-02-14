//! # dotnet-types
//!
//! Runtime representation of .NET types, methods, and fields.
//! This crate provides the descriptors and resolution logic used by the VM.
//!
//! ## Core Types
//!
//! - **[`TypeDescription`]**: Represents a resolved .NET type.
//! - **[`MethodDescription`]**: Represents a resolved .NET method.
//! - **[`FieldDescription`]**: Represents a resolved .NET field.
//! - **[`TypeComparer`](comparer::TypeComparer)**: Handles type equality and assignability.
use crate::{members::MethodDescription, resolution::ResolutionS};
use dotnetdll::prelude::{
    MemberType, ResolvedDebug, TypeDefinition, TypeIndex, TypeSource, UserType,
};
use gc_arena::{Collect, unsafe_empty_collect};
use std::{
    fmt::{Debug, Formatter},
    hash::{Hash, Hasher},
    ptr::NonNull,
};

#[macro_use]
mod macros;

pub mod comparer;
pub mod error;
pub mod generics;
pub mod members;
pub mod resolution;
pub mod runtime;

pub trait TypeResolver {
    fn corlib_type(&self, name: &str)
    -> Result<TypeDescription, error::TypeResolutionError>;
    fn locate_type(
        &self,
        resolution: ResolutionS,
        handle: UserType,
    ) -> Result<TypeDescription, error::TypeResolutionError>;
    fn find_concrete_type(
        &self,
        ty: generics::ConcreteType,
    ) -> Result<TypeDescription, error::TypeResolutionError>;
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct TypeDescription {
    pub resolution: ResolutionS,
    definition_ptr: Option<NonNull<TypeDefinition<'static>>>,
    pub index: TypeIndex,
}

unsafe_empty_collect!(TypeDescription);

// SAFETY: TypeDescription only contains a ResolutionS (which is a raw pointer wrapper)
// and a NonNull pointer to a TypeDefinition. Both point to data with 'static lifetime
// managed by the VM's assembly loader. Thread-safety is guaranteed by the read-only
// nature of metadata once loaded.
unsafe impl Send for TypeDescription {}
unsafe impl Sync for TypeDescription {}

impl TypeDescription {
    pub const NULL: Self = Self {
        resolution: ResolutionS::NULL,
        definition_ptr: None,
        index: unsafe {
            std::mem::transmute::<[u8; size_of::<TypeIndex>()], TypeIndex>(
                [0u8; size_of::<TypeIndex>()],
            )
        },
    };

    pub const fn new(
        resolution: ResolutionS,
        definition: &'static TypeDefinition<'static>,
        index: TypeIndex,
    ) -> Self {
        Self {
            resolution,
            definition_ptr: NonNull::new(definition as *const _ as *mut _),
            index,
        }
    }

    pub const fn from_raw(
        resolution: ResolutionS,
        definition_ptr: Option<NonNull<TypeDefinition<'static>>>,
        index: TypeIndex,
    ) -> Self {
        Self {
            resolution,
            definition_ptr,
            index,
        }
    }

    pub const fn definition_ptr(&self) -> Option<NonNull<TypeDefinition<'static>>> {
        self.definition_ptr
    }

    pub fn definition(&self) -> &'static TypeDefinition<'static> {
        match self.definition_ptr {
            Some(p) => {
                // SAFETY: definition_ptr is derived from a &'static reference during construction,
                // ensuring it points to valid metadata that persists for the program lifetime.
                unsafe { &*p.as_ptr() }
            }
            None => {
                panic!("Attempted to access definition of a null or uninitialized TypeDescription")
            }
        }
    }

    pub const fn is_null(&self) -> bool {
        self.definition_ptr.is_none()
    }
}

impl Debug for TypeDescription {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self.definition_ptr {
            None => write!(f, "NULL"),
            Some(_) => {
                if self.resolution.is_null() {
                    write!(f, "TypeDescription(No Resolution)")
                } else {
                    write!(
                        f,
                        "{}",
                        self.definition().show(self.resolution.definition())
                    )
                }
            }
        }
    }
}

impl PartialEq for TypeDescription {
    fn eq(&self, other: &Self) -> bool {
        self.definition_ptr == other.definition_ptr
    }
}

impl Eq for TypeDescription {}

impl Hash for TypeDescription {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.definition_ptr.hash(state);
    }
}

impl TypeDescription {
    pub fn static_initializer(&self) -> Option<MethodDescription> {
        self.definition().methods.iter().find_map(|m| {
            if m.runtime_special_name
                && m.name == ".cctor"
                && !m.signature.instance
                && m.signature.parameters.is_empty()
            {
                Some(MethodDescription {
                    parent: *self,
                    method_resolution: self.resolution,
                    method: m,
                })
            } else {
                None
            }
        })
    }

    pub fn type_name(&self) -> String {
        if self.is_null() {
            return "UNRESOLVED_TYPE_DEF".to_string();
        }
        if self.resolution.is_null() {
            return self.definition().name.to_string();
        }
        self.definition()
            .nested_type_name(self.resolution.definition())
    }

    pub fn is_enum(&self) -> Option<&MemberType> {
        match &self.definition().extends {
            Some(TypeSource::User(u))
                if matches!(
                    u.type_name(self.resolution.definition()).as_str(),
                    "System.Enum"
                ) =>
            {
                let inner = self.definition().fields.first()?;
                if inner.runtime_special_name && inner.name == "value__" {
                    Some(&inner.return_type)
                } else {
                    None
                }
            }
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn test_null_exists() {
        let _ = TypeDescription::NULL;
        let _ = ResolutionS::NULL;
    }
}

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
#![allow(clippy::mutable_key_type)]
#[cfg(doc)]
use crate::members::FieldDescription;
use crate::{generics::GenericLookup, members::MethodDescription, resolution::ResolutionS};
use dotnetdll::prelude::{
    MemberType, MethodMemberIndex, ResolvedDebug, TypeDefinition, TypeIndex, TypeSource, UserType,
};
use gc_arena::static_collect;
use std::{
    fmt::{Debug, Formatter},
    hash::{Hash, Hasher},
};

#[cfg(feature = "fuzzing")]
use arbitrary::Arbitrary;

#[macro_use]
mod macros;

pub mod comparer;
pub mod error;
pub mod generics;
pub mod members;
pub mod resolution;
pub mod runtime;

const _: [(); std::mem::size_of::<TypeIndex>()] = [(); std::mem::size_of::<usize>()];

#[inline]
pub(crate) const fn type_index_from_usize(index: usize) -> TypeIndex {
    // SAFETY: `dotnetdll` defines `TypeIndex` as a newtype around `usize`.
    unsafe { std::mem::transmute::<usize, TypeIndex>(index) }
}

#[inline]
pub(crate) const fn sentinel_type_index() -> TypeIndex {
    type_index_from_usize(0)
}

pub trait TypeResolver {
    fn corlib_type(&self, name: &str) -> Result<TypeDescription, error::TypeResolutionError>;
    fn locate_type(
        &self,
        resolution: ResolutionS,
        handle: UserType,
    ) -> Result<TypeDescription, error::TypeResolutionError>;
    fn find_concrete_type(
        &self,
        ty: generics::ConcreteType,
    ) -> Result<TypeDescription, error::TypeResolutionError>;

    fn canonical_type_name<'a>(&'a self, name: &'a str) -> &'a str {
        name
    }
}

#[repr(C)]
#[derive(Clone)]
pub struct TypeDescription {
    pub resolution: ResolutionS,
    pub index: TypeIndex,
}

#[cfg(feature = "fuzzing")]
impl<'a> Arbitrary<'a> for TypeDescription {
    fn arbitrary(u: &mut arbitrary::Unstructured<'a>) -> arbitrary::Result<Self> {
        let resolution: ResolutionS = u.arbitrary()?;
        let index = type_index_from_usize(u.arbitrary()?);

        Ok(Self { resolution, index })
    }
}

static_collect!(TypeDescription);

impl TypeDescription {
    pub const NULL: Self = Self {
        resolution: ResolutionS::NULL,
        index: sentinel_type_index(),
    };

    pub fn new(resolution: ResolutionS, index: TypeIndex) -> Self {
        Self { resolution, index }
    }

    pub fn definition(&self) -> &'static TypeDefinition<'static> {
        if self.is_null() {
            panic!("Attempted to access definition of a null or uninitialized TypeDescription")
        }
        &self.resolution.definition()[self.index]
    }

    pub const fn is_null(&self) -> bool {
        self.resolution.is_null()
    }
}

impl Debug for TypeDescription {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        if self.is_null() {
            write!(f, "NULL")
        } else {
            write!(
                f,
                "{}",
                self.definition().show(self.resolution.definition())
            )
        }
    }
}

impl PartialEq for TypeDescription {
    fn eq(&self, other: &Self) -> bool {
        self.index == other.index && self.resolution == other.resolution
    }
}

impl Eq for TypeDescription {}

impl Hash for TypeDescription {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.index.hash(state);
        self.resolution.hash(state);
    }
}

impl TypeDescription {
    pub fn before_field_init(&self) -> bool {
        self.definition().flags.before_field_init
    }

    pub fn static_initializer(&self) -> Option<MethodDescription> {
        self.definition()
            .methods
            .iter()
            .enumerate()
            .find_map(|(idx, m)| {
                if m.runtime_special_name
                    && m.name == ".cctor"
                    && !m.signature.instance
                    && m.signature.parameters.is_empty()
                {
                    Some(MethodDescription::new(
                        self.clone(),
                        GenericLookup::default(),
                        self.resolution.clone(),
                        MethodMemberIndex::Method(idx),
                    ))
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
    use dotnetdll::prelude::Resolution;

    fn assert_send<T: Send>() {}
    fn assert_sync<T: Sync>() {}

    #[test]
    fn test_null_exists() {
        let _ = TypeDescription::NULL;
        let _ = ResolutionS::NULL;
    }

    #[test]
    fn test_send_sync_traits() {
        assert_send::<Resolution<'static>>();
        assert_sync::<Resolution<'static>>();

        assert_send::<resolution::MetadataArena>();
        assert_sync::<resolution::MetadataArena>();

        assert_send::<resolution::ResolutionS>();
        assert_sync::<resolution::ResolutionS>();

        assert_send::<TypeDescription>();
        assert_sync::<TypeDescription>();

        assert_send::<members::MethodDescription>();
        assert_sync::<members::MethodDescription>();

        assert_send::<members::FieldDescription>();
        assert_sync::<members::FieldDescription>();

        assert_send::<generics::ConcreteType>();
        assert_sync::<generics::ConcreteType>();

        assert_send::<generics::GenericLookup>();
        assert_sync::<generics::GenericLookup>();
    }
}

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
use std::fmt::{Debug, Formatter};

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

const _: [(); size_of::<TypeIndex>()] = [(); size_of::<usize>()];

#[inline]
pub(crate) const fn type_index_from_usize(index: usize) -> TypeIndex {
    // SAFETY: `dotnetdll` defines `TypeIndex` as a newtype around `usize`.
    unsafe { std::mem::transmute::<usize, TypeIndex>(index) }
}

#[inline]
pub(crate) const fn sentinel_type_index() -> TypeIndex {
    type_index_from_usize(0)
}

/// Resolves metadata type references into runtime [`TypeDescription`] handles.
///
/// Implementors provide the core lookup contract used by the VM when converting
/// names, metadata tokens, and concrete generic signatures into resolved type
/// descriptors.
pub trait TypeResolver {
    /// Looks up a core-library type by its short metadata name.
    ///
    /// This is typically used for well-known framework types (for example,
    /// `System.Object`) that are expected to exist in corlib.
    fn corlib_type(&self, name: &str) -> Result<TypeDescription, error::TypeResolutionError>;

    /// Resolves a metadata [`UserType`] token within the provided resolution scope.
    fn locate_type(
        &self,
        resolution: ResolutionS,
        handle: UserType,
    ) -> Result<TypeDescription, error::TypeResolutionError>;

    /// Resolves an already-concrete generic type into a runtime type descriptor.
    fn find_concrete_type(
        &self,
        ty: generics::ConcreteType,
    ) -> Result<TypeDescription, error::TypeResolutionError>;

    /// Normalizes incoming type names before resolution.
    ///
    /// The default implementation returns `name` unchanged.
    fn canonical_type_name<'a>(&'a self, name: &'a str) -> &'a str {
        name
    }
}

/// A resolved reference to a .NET type definition.
///
/// The descriptor combines an assembly resolution scope with an index into that
/// scope's type-definition table. [`TypeDescription::NULL`] is the sentinel for
/// unresolved or intentionally absent type handles.
#[repr(C)]
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct TypeDescription {
    /// Assembly or module scope that owns the resolved type definition.
    pub resolution: ResolutionS,
    /// Index of the type definition within [`Self::resolution`].
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
    /// Sentinel descriptor used when a type has not been resolved from metadata.
    pub const NULL: Self = Self {
        resolution: ResolutionS::NULL,
        index: sentinel_type_index(),
    };

    /// Creates a new runtime type descriptor from a resolution scope and type index.
    pub fn new(resolution: ResolutionS, index: TypeIndex) -> Self {
        Self { resolution, index }
    }

    /// Returns the metadata type definition for this resolved type descriptor.
    ///
    /// # Panics
    ///
    /// Panics if this handle is null or uninitialized (for example,
    /// [`TypeDescription::NULL`]).
    pub fn definition(&self) -> &'static TypeDefinition<'static> {
        if self.is_null() {
            // invariant: TypeDescription::definition() must only be called on resolved, non-null metadata handles.
            panic!("Attempted to access definition of a null or uninitialized TypeDescription")
        }
        &self.resolution.definition()[self.index]
    }

    /// Returns `true` when this value is the null/sentinel type descriptor.
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

impl TypeDescription {
    /// Returns whether this type is marked with the metadata `beforefieldinit` flag.
    pub fn before_field_init(&self) -> bool {
        self.definition().flags.before_field_init
    }

    /// Returns the static type initializer (`.cctor`) if one is defined.
    pub fn static_initializer(&self) -> Option<MethodDescription> {
        self.definition()
            .methods
            .iter()
            .enumerate()
            .find_map(|(idx, m)| {
                if !m.runtime_special_name || m.name != ".cctor" {
                    return None;
                }
                let desc = MethodDescription::new(
                    self.clone(),
                    GenericLookup::default(),
                    self.resolution.clone(),
                    MethodMemberIndex::Method(idx),
                );
                let sig = desc.signature();
                if !sig.instance && sig.parameters.is_empty() {
                    Some(desc)
                } else {
                    None
                }
            })
    }

    /// Returns a display name for this type, including containing type names when nested.
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

    /// Returns the underlying primitive member type when this type is an enum.
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

        assert_send::<ResolutionS>();
        assert_sync::<ResolutionS>();

        assert_send::<TypeDescription>();
        assert_sync::<TypeDescription>();

        assert_send::<MethodDescription>();
        assert_sync::<MethodDescription>();

        assert_send::<members::FieldDescription>();
        assert_sync::<members::FieldDescription>();

        assert_send::<generics::ConcreteType>();
        assert_sync::<generics::ConcreteType>();

        assert_send::<GenericLookup>();
        assert_sync::<GenericLookup>();
    }
}

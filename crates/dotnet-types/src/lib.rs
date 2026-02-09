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

pub mod comparer;
pub mod generics;
#[macro_use]
mod macros;
pub mod members;
pub mod resolution;
pub mod runtime;

pub trait TypeResolver {
    fn corlib_type(&self, name: &str) -> TypeDescription;
    fn locate_type(&self, resolution: ResolutionS, handle: UserType) -> TypeDescription;
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct TypeDescription {
    pub resolution: ResolutionS,
    definition_ptr: Option<NonNull<TypeDefinition<'static>>>,
    pub index: TypeIndex,
}

unsafe_empty_collect!(TypeDescription);

unsafe impl Send for TypeDescription {}
unsafe impl Sync for TypeDescription {}

impl TypeDescription {
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
            Some(p) => unsafe { &*p.as_ptr() },
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

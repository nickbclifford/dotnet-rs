use crate::{types::members::MethodDescription, utils::ResolutionS};
use dotnetdll::prelude::{MemberType, ResolvedDebug, TypeDefinition, TypeSource};
use gc_arena::{Collect, unsafe_empty_collect};
use std::{
    fmt::{Debug, Formatter},
    hash::{Hash, Hasher},
    ptr::NonNull,
};

pub mod generics;
pub mod members;
pub mod runtime;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct TypeDescription {
    pub resolution: ResolutionS,
    definition_ptr: Option<NonNull<TypeDefinition<'static>>>,
}

unsafe_empty_collect!(TypeDescription);

unsafe impl Send for TypeDescription {}
unsafe impl Sync for TypeDescription {}

impl TypeDescription {
    pub fn new(resolution: ResolutionS, definition: &'static TypeDefinition<'static>) -> Self {
        Self {
            resolution,
            definition_ptr: NonNull::new(definition as *const _ as *mut _),
        }
    }

    pub fn from_raw(
        resolution: ResolutionS,
        definition_ptr: Option<NonNull<TypeDefinition<'static>>>,
    ) -> Self {
        Self {
            resolution,
            definition_ptr,
        }
    }

    pub fn definition_ptr(&self) -> Option<NonNull<TypeDefinition<'static>>> {
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

    pub fn is_null(&self) -> bool {
        self.definition_ptr.is_none()
    }
}

impl Debug for TypeDescription {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self.definition_ptr {
            None => write!(f, "NULL"),
            Some(_) => write!(
                f,
                "{}",
                self.definition().show(self.resolution.definition())
            ),
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

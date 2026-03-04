use crate::{TypeDescription, resolution::ResolutionS};
use dotnetdll::prelude::{Field, Method, ResolvedDebug};
use gc_arena::static_collect;
use std::{
    fmt::{Debug, Formatter},
    hash::{Hash, Hasher},
    ptr::NonNull,
};

#[derive(Clone, Copy)]
pub struct MethodDescription {
    pub parent: TypeDescription,
    pub method_resolution: ResolutionS,
    method_ptr: NonNull<Method<'static>>,
}

static_collect!(MethodDescription);

unsafe impl Send for MethodDescription {}
unsafe impl Sync for MethodDescription {}

impl MethodDescription {
    pub const fn new(
        parent: TypeDescription,
        method_resolution: ResolutionS,
        method: &'static Method<'static>,
    ) -> Self {
        Self {
            parent,
            method_resolution,
            method_ptr: unsafe { NonNull::new_unchecked(method as *const _ as *mut _) },
        }
    }

    pub fn method(&self) -> &'static Method<'static> {
        unsafe { self.method_ptr.as_ref() }
    }

    pub const fn resolution(&self) -> ResolutionS {
        self.method_resolution
    }
}

impl Debug for MethodDescription {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        if self.resolution().is_null() {
            return write!(
                f,
                "{}::{} (No Resolution)",
                self.parent.type_name(),
                self.method().name
            );
        }
        write!(
            f,
            "{}",
            self.method().signature.show_with_name(
                self.resolution().definition(),
                format!("{}::{}", self.parent.type_name(), self.method().name)
            )
        )
    }
}

impl PartialEq for MethodDescription {
    fn eq(&self, other: &Self) -> bool {
        std::ptr::eq(self.method(), other.method())
    }
}

impl Eq for MethodDescription {}

impl Hash for MethodDescription {
    fn hash<H: Hasher>(&self, state: &mut H) {
        (self.method() as *const Method).hash(state);
    }
}

#[derive(Clone, Copy)]
pub struct FieldDescription {
    pub parent: TypeDescription,
    pub field_resolution: ResolutionS,
    field_ptr: NonNull<Field<'static>>,
    pub index: usize,
}

static_collect!(FieldDescription);

unsafe impl Send for FieldDescription {}
unsafe impl Sync for FieldDescription {}

impl FieldDescription {
    pub const fn new(
        parent: TypeDescription,
        field_resolution: ResolutionS,
        field: &'static Field<'static>,
        index: usize,
    ) -> Self {
        Self {
            parent,
            field_resolution,
            field_ptr: unsafe { NonNull::new_unchecked(field as *const _ as *mut _) },
            index,
        }
    }

    pub fn field(&self) -> &'static Field<'static> {
        unsafe { self.field_ptr.as_ref() }
    }

    pub const fn resolution(&self) -> ResolutionS {
        self.field_resolution
    }
}

impl Debug for FieldDescription {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        if self.field().static_member {
            write!(f, "static ")?;
        }

        write!(
            f,
            "{} {}::{}",
            self.field()
                .return_type
                .show(self.resolution().definition()),
            self.parent.type_name(),
            self.field().name
        )?;

        Ok(())
    }
}

impl PartialEq for FieldDescription {
    fn eq(&self, other: &Self) -> bool {
        std::ptr::eq(self.field(), other.field())
    }
}

impl Eq for FieldDescription {}

impl Hash for FieldDescription {
    fn hash<H: Hasher>(&self, state: &mut H) {
        (self.field() as *const Field).hash(state);
    }
}

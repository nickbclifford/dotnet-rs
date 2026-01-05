use crate::{types::TypeDescription, utils::ResolutionS};
use dotnetdll::prelude::{BaseType, MethodType, Resolution, ResolvedDebug, TypeSource, UserType};
use gc_arena::{Collect, Collection};
use std::fmt::{Debug, Formatter};

#[derive(Clone, PartialEq, Eq, Hash)]
pub struct ConcreteType {
    source: ResolutionS,
    base: Box<BaseType<Self>>,
}

unsafe impl Collect for ConcreteType {
    fn trace(&self, _cc: &Collection) {}
}

impl From<TypeDescription> for ConcreteType {
    fn from(td: TypeDescription) -> Self {
        let index = td
            .resolution
            .definition()
            .type_definitions
            .iter()
            .position(|t| std::ptr::eq(t, td.definition()))
            .expect("TypeDescription has invalid definition pointer");
        Self::new(
            td.resolution,
            BaseType::Type {
                source: TypeSource::User(UserType::Definition(
                    td.resolution
                        .definition()
                        .type_definition_index(index)
                        .unwrap(),
                )),
                value_kind: None,
            },
        )
    }
}

impl ConcreteType {
    pub fn new(source: ResolutionS, base: BaseType<Self>) -> Self {
        ConcreteType {
            source,
            base: Box::new(base),
        }
    }

    pub fn get(&self) -> &BaseType<Self> {
        &self.base
    }

    pub fn get_mut(&mut self) -> &mut BaseType<Self> {
        &mut self.base
    }

    pub fn resolution(&self) -> ResolutionS {
        self.source
    }
}

impl Debug for ConcreteType {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "ConcreteType")
    }
}

impl ResolvedDebug for ConcreteType {
    fn show(&self, _res: &Resolution) -> String {
        format!("{:?}", self)
    }
}

#[derive(Clone, Default, PartialEq, Eq, Hash)]
pub struct GenericLookup {
    pub type_generics: Vec<ConcreteType>,
    pub method_generics: Vec<ConcreteType>,
}

unsafe impl Collect for GenericLookup {
    fn trace(&self, cc: &Collection) {
        self.type_generics.trace(cc);
        self.method_generics.trace(cc);
    }
}

impl GenericLookup {
    pub fn new(type_generics: Vec<ConcreteType>) -> Self {
        Self {
            type_generics,
            method_generics: vec![],
        }
    }

    pub fn make_concrete(&self, res: ResolutionS, t: impl Into<MethodType>) -> ConcreteType {
        match t.into() {
            MethodType::Base(b) => ConcreteType::new(res, b.map(|t| self.make_concrete(res, t))),
            MethodType::TypeGeneric(i) => self.type_generics[i].clone(),
            MethodType::MethodGeneric(i) => self.method_generics[i].clone(),
        }
    }
}

impl Debug for GenericLookup {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        struct GenericIndexFormatter(char, usize);
        impl Debug for GenericIndexFormatter {
            fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
                write!(f, "{}{}", self.0, self.1)
            }
        }

        f.debug_map()
            .entries(
                self.type_generics
                    .iter()
                    .enumerate()
                    .map(|(i, t)| (GenericIndexFormatter('T', i), t)),
            )
            .entries(
                self.method_generics
                    .iter()
                    .enumerate()
                    .map(|(i, t)| (GenericIndexFormatter('M', i), t)),
            )
            .finish()
    }
}

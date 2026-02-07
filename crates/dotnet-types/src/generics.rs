use crate::{TypeDescription, resolution::ResolutionS};
use dotnetdll::prelude::{BaseType, MethodType, Resolution, ResolvedDebug, TypeSource, UserType};
use gc_arena::{Collect, Collection};
use std::{
    fmt::{Debug, Formatter},
    sync::Arc,
};

#[derive(Clone, PartialEq, Eq, Hash)]
pub struct ConcreteType {
    source: ResolutionS,
    base: Arc<BaseType<Self>>,
}

unsafe impl Collect for ConcreteType {
    fn trace(&self, _cc: &Collection) {}
}

impl From<TypeDescription> for ConcreteType {
    fn from(td: TypeDescription) -> Self {
        Self::new(
            td.resolution,
            BaseType::Type {
                source: TypeSource::User(UserType::Definition(td.index)),
                value_kind: None,
            },
        )
    }
}

impl ConcreteType {
    pub fn new(source: ResolutionS, base: BaseType<Self>) -> Self {
        ConcreteType {
            source,
            base: Arc::new(base),
        }
    }

    pub fn get(&self) -> &BaseType<Self> {
        &self.base
    }

    pub fn get_mut(&mut self) -> &mut BaseType<Self> {
        Arc::make_mut(&mut self.base)
    }

    pub fn resolution(&self) -> ResolutionS {
        self.source
    }
}

impl Debug for ConcreteType {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        if self.source.is_null() {
            write!(f, "ConcreteType(No Resolution)")
        } else {
            write!(f, "{}", self.show(&self.source))
        }
    }
}

impl ResolvedDebug for ConcreteType {
    fn show(&self, res: &Resolution) -> String {
        self.base.show(res)
    }
}

#[derive(Clone, Default, PartialEq, Eq, Hash)]
pub struct GenericLookup {
    pub type_generics: Arc<[ConcreteType]>,
    pub method_generics: Arc<[ConcreteType]>,
}

unsafe impl Collect for GenericLookup {
    fn trace(&self, _cc: &Collection) {}
}

impl GenericLookup {
    pub fn cache_key_hash(&self) -> u64 {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        let mut hasher = DefaultHasher::new();
        self.hash(&mut hasher);
        hasher.finish()
    }

    pub fn new(type_generics: Vec<ConcreteType>) -> Self {
        Self {
            type_generics: type_generics.into(),
            method_generics: Arc::new([]),
        }
    }

    pub fn make_concrete(&self, res: ResolutionS, t: impl Into<MethodType>) -> ConcreteType {
        match t.into() {
            MethodType::Base(b) => ConcreteType::new(res, b.map(|t| self.make_concrete(res, t))),
            MethodType::TypeGeneric(i) => self
                .type_generics
                .get(i)
                .unwrap_or_else(|| {
                    panic!(
                        "Type generic index !{} out of bounds (current context only has {})",
                        i,
                        self.type_generics.len()
                    );
                })
                .clone(),
            MethodType::MethodGeneric(i) => self
                .method_generics
                .get(i)
                .unwrap_or_else(|| {
                    panic!(
                        "Method generic index !!{} out of bounds (current context only has {})",
                        i,
                        self.method_generics.len()
                    );
                })
                .clone(),
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

use crate::{TypeDescription, TypeResolver, error::TypeResolutionError, resolution::ResolutionS};
use dotnetdll::prelude::{
    Accessibility, BaseType, Kind, MemberAccessibility, MethodType, Resolution, ResolvedDebug,
    TypeSource, UserType,
};
#[cfg(feature = "generic-constraint-validation")]
use dotnetdll::resolved::generic::Generic;
use gc_arena::{Collect, collect::Trace};
use std::{
    fmt::{Debug, Formatter},
    sync::Arc,
};

#[cfg(feature = "fuzzing")]
use arbitrary::Arbitrary;

#[derive(Clone, PartialEq, Eq, Hash)]
pub struct ConcreteType {
    source: ResolutionS,
    base: Arc<BaseType<Self>>,
}

#[cfg(feature = "fuzzing")]
impl<'a> Arbitrary<'a> for ConcreteType {
    fn arbitrary(u: &mut arbitrary::Unstructured<'a>) -> arbitrary::Result<Self> {
        let source: ResolutionS = u.arbitrary()?;
        // Use a simple base type to avoid complex dotnetdll dependencies
        Ok(Self::new(source, BaseType::Int32))
    }
}

unsafe impl<'gc> Collect<'gc> for ConcreteType {
    fn trace<Tr: Trace<'gc>>(&self, _cc: &mut Tr) {}
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

    pub fn is_class(&self, loader: &impl TypeResolver) -> bool {
        if self.is_value_type(loader) {
            return false;
        }
        matches!(
            self.base.as_ref(),
            dotnetdll::prelude::BaseType::Type { .. }
                | dotnetdll::prelude::BaseType::Object
                | dotnetdll::prelude::BaseType::String
                | dotnetdll::prelude::BaseType::Vector(_, _)
                | dotnetdll::prelude::BaseType::Array(_, _)
        )
    }

    pub fn is_value_type(&self, loader: &impl TypeResolver) -> bool {
        match self.base.as_ref() {
            dotnetdll::prelude::BaseType::Type {
                value_kind: Some(dotnetdll::prelude::ValueKind::ValueType),
                ..
            } => true,
            dotnetdll::prelude::BaseType::Boolean
            | dotnetdll::prelude::BaseType::Char
            | dotnetdll::prelude::BaseType::Int8
            | dotnetdll::prelude::BaseType::UInt8
            | dotnetdll::prelude::BaseType::Int16
            | dotnetdll::prelude::BaseType::UInt16
            | dotnetdll::prelude::BaseType::Int32
            | dotnetdll::prelude::BaseType::UInt32
            | dotnetdll::prelude::BaseType::Int64
            | dotnetdll::prelude::BaseType::UInt64
            | dotnetdll::prelude::BaseType::Float32
            | dotnetdll::prelude::BaseType::Float64
            | dotnetdll::prelude::BaseType::IntPtr
            | dotnetdll::prelude::BaseType::UIntPtr => true,
            dotnetdll::prelude::BaseType::Type { .. } => {
                let mut curr = self.clone();
                while let Ok(td) = loader.find_concrete_type(curr.clone()) {
                    let def = td.definition();
                    if def.name == "ValueType" && def.namespace.as_deref() == Some("System") {
                        return true;
                    }
                    if let Some(base_source) = &def.extends {
                        let lookup = curr.make_lookup();
                        if let Ok(base) = lookup.make_concrete(
                            td.resolution,
                            member_to_method_type(base_source),
                            loader,
                        ) {
                            curr = base;
                            continue;
                        }
                    }
                    break;
                }
                false
            }
            _ => false,
        }
    }

    pub fn resolution(&self) -> ResolutionS {
        self.source
    }

    pub fn is_interface(&self, loader: &impl TypeResolver) -> bool {
        loader
            .find_concrete_type(self.clone())
            .map(|td| matches!(td.definition().flags.kind, Kind::Interface))
            .unwrap_or(false)
    }

    pub fn is_nullable(&self, loader: &impl TypeResolver) -> bool {
        loader
            .find_concrete_type(self.clone())
            .map(|td| {
                let def = td.definition();
                def.name == "Nullable`1" && def.namespace.as_deref() == Some("System")
            })
            .unwrap_or(false)
    }

    pub fn has_default_constructor(&self, loader: &impl TypeResolver) -> bool {
        if self.is_value_type(loader) {
            return true;
        }

        if let Ok(td) = loader.find_concrete_type(self.clone()) {
            let def = td.definition();
            if def.flags.abstract_type {
                return false;
            }

            for method in def.methods.iter() {
                if method.name == ".ctor"
                    && method.signature.parameters.is_empty()
                    && method.accessibility == MemberAccessibility::Access(Accessibility::Public)
                {
                    return true;
                }
            }
        }
        false
    }

    pub fn make_lookup(&self) -> GenericLookup {
        if let BaseType::Type {
            source: TypeSource::Generic { parameters, .. },
            ..
        } = self.get()
        {
            GenericLookup {
                type_generics: parameters.clone().into(),
                method_generics: Arc::new([]),
            }
        } else {
            GenericLookup::default()
        }
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

pub(crate) fn member_to_method_type(
    src: &dotnetdll::prelude::TypeSource<dotnetdll::prelude::MemberType>,
) -> MethodType {
    match src {
        dotnetdll::prelude::TypeSource::User(h) => MethodType::Base(Box::new(BaseType::Type {
            source: dotnetdll::prelude::TypeSource::User(*h),
            value_kind: None,
        })),
        dotnetdll::prelude::TypeSource::Generic { base, parameters } => {
            MethodType::Base(Box::new(BaseType::Type {
                source: dotnetdll::prelude::TypeSource::Generic {
                    base: *base,
                    parameters: parameters.iter().cloned().map(MethodType::from).collect(),
                },
                value_kind: None,
            }))
        }
    }
}

#[derive(Clone, Default, PartialEq, Eq, Hash)]
pub struct GenericLookup {
    pub type_generics: Arc<[ConcreteType]>,
    pub method_generics: Arc<[ConcreteType]>,
}

#[cfg(feature = "fuzzing")]
impl<'a> Arbitrary<'a> for GenericLookup {
    fn arbitrary(u: &mut arbitrary::Unstructured<'a>) -> arbitrary::Result<Self> {
        let type_generics: Vec<ConcreteType> = u.arbitrary()?;
        let method_generics: Vec<ConcreteType> = u.arbitrary()?;
        Ok(Self {
            type_generics: type_generics.into(),
            method_generics: method_generics.into(),
        })
    }
}

unsafe impl<'gc> Collect<'gc> for GenericLookup {
    fn trace<Tr: Trace<'gc>>(&self, _cc: &mut Tr) {}
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

    pub fn make_concrete(
        &self,
        res: ResolutionS,
        t: impl Into<MethodType>,
        _loader: &impl TypeResolver,
    ) -> Result<ConcreteType, TypeResolutionError> {
        let t = t.into();

        match t {
            MethodType::Base(b) => {
                let mut err = None;
                let concrete_base = b.map(|t| match self.make_concrete(res, t, _loader) {
                    Ok(c) => c,
                    Err(e) => {
                        err = Some(e);
                        ConcreteType::new(res, BaseType::Boolean)
                    }
                });
                if let Some(e) = err {
                    Err(e)
                } else {
                    let concrete = ConcreteType::new(res, concrete_base);

                    #[cfg(feature = "generic-constraint-validation")]
                    {
                        if let BaseType::Type {
                            source: TypeSource::Generic { base, parameters },
                            ..
                        } = concrete.get()
                        {
                            let td = _loader.locate_type(res, *base)?;
                            let lookup = GenericLookup {
                                type_generics: parameters.clone().into(),
                                method_generics: Arc::new([]),
                            };
                            lookup.validate_constraints(td.resolution, _loader, &td.definition().generic_parameters, false)?;
                        }
                    }

                    Ok(concrete)
                }
            }
            MethodType::TypeGeneric(i) => self.type_generics.get(i).cloned().ok_or(
                TypeResolutionError::GenericIndexOutOfBounds {
                    index: i,
                    length: self.type_generics.len(),
                },
            ),
            MethodType::MethodGeneric(i) => self.method_generics.get(i).cloned().ok_or(
                TypeResolutionError::GenericIndexOutOfBounds {
                    index: i,
                    length: self.method_generics.len(),
                },
            ),
        }
    }

    #[cfg(feature = "generic-constraint-validation")]
    pub fn validate_constraints<T: ResolvedDebug + Clone + Into<MethodType>>(
        &self,
        res: ResolutionS,
        loader: &impl TypeResolver,
        generic_parameters: &[Generic<'static, T>],
        is_method: bool,
    ) -> Result<(), TypeResolutionError> {
        let args = if is_method {
            &self.method_generics
        } else {
            &self.type_generics
        };

        if args.len() != generic_parameters.len() {
            return Err(TypeResolutionError::GenericIndexOutOfBounds {
                index: generic_parameters.len(),
                length: args.len(),
            });
        }

        let comparer = crate::comparer::TypeComparer::new(loader);

        for (arg, param) in args.iter().zip(generic_parameters.iter()) {
            // Special constraints
            if param.special_constraint.reference_type && !arg.is_class(loader) {
                return Err(TypeResolutionError::GenericConstraintViolation(format!(
                    "Type {} must be a reference type to satisfy constraint on {}",
                    arg.show(&arg.resolution().definition()),
                    param.name
                )));
            }
            if param.special_constraint.value_type
                && (!arg.is_value_type(loader) || arg.is_nullable(loader))
            {
                return Err(TypeResolutionError::GenericConstraintViolation(format!(
                    "Type {} must be a non-nullable value type to satisfy constraint on {}",
                    arg.show(&arg.resolution().definition()),
                    param.name
                )));
            }
            if param.special_constraint.has_default_constructor
                && !arg.has_default_constructor(loader)
            {
                return Err(TypeResolutionError::GenericConstraintViolation(format!(
                    "Type {} must have a public default constructor to satisfy constraint on {}",
                    arg.show(&arg.resolution().definition()),
                    param.name
                )));
            }

            // Type constraints
            for constraint in &param.type_constraints {
                let constraint_type =
                    self.make_concrete(res, constraint.constraint_type.clone(), loader)?;
                if !comparer.is_assignable_to(arg, &constraint_type) {
                    return Err(TypeResolutionError::GenericConstraintViolation(format!(
                        "Type {} must be assignable to {} to satisfy constraint on {}",
                        arg.show(&arg.resolution().definition()),
                        constraint_type.show(&constraint_type.resolution().definition()),
                        param.name
                    )));
                }
            }
        }

        Ok(())
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

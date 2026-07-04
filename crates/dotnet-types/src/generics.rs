use crate::{TypeDescription, TypeResolver, error::TypeResolutionError, resolution::ResolutionS};
use dotnetdll::prelude::{
    Accessibility, BaseType, Kind, MemberAccessibility, MethodType, Resolution, ResolvedDebug,
    TypeSource, UserType,
};
use gc_arena::{Collect, collect::Trace};
use lru::LruCache;
use std::{
    cell::RefCell,
    collections::HashSet,
    fmt::{Debug, Formatter},
    num::NonZeroUsize,
    sync::Arc,
};

#[cfg(feature = "generic-constraint-validation")]
use dotnetdll::resolved::generic::Generic;

#[cfg(feature = "fuzzing")]
use arbitrary::Arbitrary;

// Cap per-thread cache at 512 entries — enough to cover commonly recurring closed generics.
const MAKE_LOOKUP_CACHE_CAPACITY: usize = 512;

thread_local! {
    static MAKE_LOOKUP_CACHE: RefCell<LruCache<ConcreteType, Arc<[ConcreteType]>>> =
        RefCell::new(LruCache::new(
            NonZeroUsize::new(MAKE_LOOKUP_CACHE_CAPACITY).unwrap(),
        ));
}

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
            BaseType::Type { .. }
                | BaseType::Object
                | BaseType::String
                | BaseType::Vector(_, _)
                | BaseType::Array(_, _)
        )
    }

    #[allow(clippy::mutable_key_type)]
    pub fn is_value_type(&self, loader: &impl TypeResolver) -> bool {
        match self.base.as_ref() {
            BaseType::Type {
                value_kind: Some(dotnetdll::prelude::ValueKind::ValueType),
                ..
            } => true,
            BaseType::Boolean
            | BaseType::Char
            | BaseType::Int8
            | BaseType::UInt8
            | BaseType::Int16
            | BaseType::UInt16
            | BaseType::Int32
            | BaseType::UInt32
            | BaseType::Int64
            | BaseType::UInt64
            | BaseType::Float32
            | BaseType::Float64
            | BaseType::IntPtr
            | BaseType::UIntPtr => true,
            BaseType::Type { .. } => {
                let mut curr = self.clone();
                let mut seen = HashSet::new();
                while seen.insert(curr.clone()) {
                    let Ok(td) = loader.find_concrete_type(curr.clone()) else {
                        break;
                    };
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
        self.source.clone()
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

            for (idx, method) in def.methods.iter().enumerate() {
                if method.name == ".ctor"
                    && method.accessibility == MemberAccessibility::Access(Accessibility::Public)
                {
                    let desc = crate::members::MethodDescription::new(
                        td.clone(),
                        crate::generics::GenericLookup::default(),
                        td.resolution.clone(),
                        dotnetdll::prelude::MethodMemberIndex::Method(idx),
                    );
                    if desc.signature().parameters.is_empty() {
                        return true;
                    }
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
            let type_generics = MAKE_LOOKUP_CACHE.with(|cache| {
                let mut cache = cache.borrow_mut();
                if let Some(cached) = cache.get(self) {
                    return cached.clone();
                }
                let arc: Arc<[ConcreteType]> = parameters.clone().into();
                cache.put(self.clone(), arc.clone());
                arc
            });
            GenericLookup::from_type_arc(type_generics)
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

pub fn member_to_method_type(src: &TypeSource<dotnetdll::prelude::MemberType>) -> MethodType {
    match src {
        TypeSource::User(h) => MethodType::Base(Box::new(BaseType::Type {
            source: TypeSource::User(*h),
            value_kind: None,
        })),
        TypeSource::Generic { base, parameters } => MethodType::Base(Box::new(BaseType::Type {
            source: TypeSource::Generic {
                base: *base,
                parameters: parameters.iter().cloned().map(MethodType::from).collect(),
            },
            value_kind: None,
        })),
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
    pub fn new(type_generics: Vec<ConcreteType>) -> Self {
        Self {
            type_generics: type_generics.into(),
            method_generics: Arc::new([]),
        }
    }

    /// Construct a lookup from an already-owned `Arc<[ConcreteType]>`, avoiding the
    /// `Vec → Arc` allocation that `new` incurs.
    pub fn from_type_arc(type_generics: Arc<[ConcreteType]>) -> Self {
        Self {
            type_generics,
            method_generics: Arc::new([]),
        }
    }

    /// Returns a bounded reference to a type generic argument.
    ///
    /// Use this when the generic slot comes from metadata/runtime binding. If the slot is
    /// malformed or missing, this returns `TypeResolutionError::GenericIndexOutOfBounds`
    /// instead of panicking, preserving the host-error (`VmError`) path at callers.
    pub fn type_arg(&self, index: usize) -> Result<&ConcreteType, TypeResolutionError> {
        Self::bounded_generic_arg(&self.type_generics, index)
    }

    pub fn cloned_type_arg(&self, index: usize) -> Result<ConcreteType, TypeResolutionError> {
        self.type_arg(index).cloned()
    }

    /// Returns a bounded reference to a method generic argument.
    ///
    /// Like `type_arg`, this accessor classifies out-of-bounds generic slots as
    /// `TypeResolutionError::GenericIndexOutOfBounds` rather than a panic.
    pub fn method_arg(&self, index: usize) -> Result<&ConcreteType, TypeResolutionError> {
        Self::bounded_generic_arg(&self.method_generics, index)
    }

    pub fn cloned_method_arg(&self, index: usize) -> Result<ConcreteType, TypeResolutionError> {
        self.method_arg(index).cloned()
    }

    fn bounded_generic_arg(
        generics: &[ConcreteType],
        index: usize,
    ) -> Result<&ConcreteType, TypeResolutionError> {
        generics
            .get(index)
            .ok_or(TypeResolutionError::GenericIndexOutOfBounds {
                index,
                length: generics.len(),
            })
    }

    fn lookup_method_generic_with_fallback_indices(&self, i: usize) -> Option<ConcreteType> {
        let type_arity = self.type_generics.len();
        let method_arity = self.method_generics.len();
        let method_candidate_indices = [
            Some(i),
            i.checked_sub(type_arity),
            i.checked_sub(type_arity + 1),
            i.checked_sub(method_arity),
            i.checked_sub(method_arity + 1),
            i.checked_sub(1),
        ];
        for method_index in method_candidate_indices.into_iter().flatten() {
            if let Some(ty) = self.method_generics.get(method_index).cloned() {
                return Some(ty);
            }
        }

        None
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
                let concrete_base = b.map(|t| match self.make_concrete(res.clone(), t, _loader) {
                    Ok(c) => c,
                    Err(e) => {
                        err = Some(e);
                        ConcreteType::new(res.clone(), BaseType::Boolean)
                    }
                });
                if let Some(e) = err {
                    Err(e)
                } else {
                    let concrete = ConcreteType::new(res.clone(), concrete_base);

                    #[cfg(feature = "generic-constraint-validation")]
                    {
                        if let BaseType::Type {
                            source: TypeSource::Generic { base, parameters },
                            ..
                        } = concrete.get()
                        {
                            // Copy/clone before releasing the borrow on `concrete`.
                            let base = *base;
                            let parameters = parameters.clone();
                            let cache_key = concrete.clone();

                            if let Some(cached_result) =
                                cached_constraint_validation_result(&cache_key)
                            {
                                cached_result?;
                                return Ok(concrete);
                            }

                            // Cycle guard: if this (resolution, type) pair is already
                            // being validated on this thread, we have a constraint cycle.
                            // Short-circuit permissively to avoid infinite recursion on
                            // valid recursive BCL constraints.
                            let key = (res.clone(), base);
                            let already_visiting =
                                CONSTRAINT_VALIDATION_VISITING.with(|v| v.borrow().contains(&key));
                            if already_visiting {
                                cache_constraint_validation_result(cache_key, Ok(()));
                                return Ok(concrete);
                            }

                            let td = _loader.locate_type(res.clone(), base)?;
                            let lookup = GenericLookup {
                                type_generics: parameters.into(),
                                method_generics: Arc::new([]),
                            };
                            let generic_parameters = &td.definition().generic_parameters;

                            // A zero-arity definition has nothing to validate; this occurs on
                            // some BCL canonicalization paths where a parent lookup carries
                            // generic arguments but the resolved method/type itself is non-generic.
                            if lookup.type_generics.len() != generic_parameters.len() {
                                if generic_parameters.is_empty() {
                                    cache_constraint_validation_result(cache_key, Ok(()));
                                    return Ok(concrete);
                                }
                                let err = TypeResolutionError::GenericIndexOutOfBounds {
                                    index: generic_parameters.len(),
                                    length: lookup.type_generics.len(),
                                };
                                cache_constraint_validation_result(cache_key, Err(err.clone()));
                                return Err(err);
                            }

                            // Most generic parameters in BCL shapes are unconstrained. Skip
                            // expensive assignability traversal when there is nothing to check.
                            let has_constraints = generic_parameters.iter().any(|param| {
                                param.special_constraint.reference_type
                                    || param.special_constraint.value_type
                                    || param.special_constraint.has_default_constructor
                                    || !param.type_constraints.is_empty()
                            });
                            if !has_constraints {
                                cache_constraint_validation_result(cache_key, Ok(()));
                                return Ok(concrete);
                            }

                            // Depth guard: if the nesting depth exceeds the limit,
                            // short-circuit permissively to avoid unbounded work on
                            // wide BCL interface hierarchies.
                            let depth = CONSTRAINT_VALIDATION_DEPTH.with(|d| *d.borrow());
                            if depth >= CONSTRAINT_VALIDATION_DEPTH_LIMIT {
                                cache_constraint_validation_result(cache_key, Ok(()));
                                return Ok(concrete);
                            }

                            // Mark in-progress, validate, then always clear the mark.
                            CONSTRAINT_VALIDATION_VISITING
                                .with(|v| v.borrow_mut().insert(key.clone()));
                            CONSTRAINT_VALIDATION_DEPTH.with(|d| *d.borrow_mut() += 1);
                            let validate_result = lookup.validate_constraints(
                                td.resolution.clone(),
                                _loader,
                                generic_parameters,
                                false,
                            );
                            CONSTRAINT_VALIDATION_DEPTH.with(|d| *d.borrow_mut() -= 1);
                            CONSTRAINT_VALIDATION_VISITING.with(|v| v.borrow_mut().remove(&key));
                            cache_constraint_validation_result(cache_key, validate_result.clone());
                            validate_result?;
                        }
                    }

                    Ok(concrete)
                }
            }
            MethodType::TypeGeneric(i) => {
                if let Some(ty) = self.type_generics.get(i).cloned() {
                    Ok(ty)
                } else {
                    // Some runtime paths surface generic arguments for non-generic receiver
                    // types through method generic context even when metadata uses TypeGeneric.
                    // Mixed canonical forms observed in BCL paths can shift method indices by
                    // type arity and/or one-based offsets, so probe the small set of known
                    // encodings before failing.
                    if let Some(ty) = self.lookup_method_generic_with_fallback_indices(i) {
                        return Ok(ty);
                    }

                    let length = self.type_generics.len();
                    Err(TypeResolutionError::GenericIndexOutOfBounds { index: i, length })
                }
            }
            MethodType::MethodGeneric(i) => {
                if let Some(ty) = self.method_generics.get(i).cloned() {
                    Ok(ty)
                } else {
                    // Some BCL canonicalization paths encode method generic slots using
                    // mixed type+method numbering schemes. Probe common offset variants
                    // before failing hard.
                    if let Some(ty) = self.lookup_method_generic_with_fallback_indices(i) {
                        return Ok(ty);
                    }

                    // Some call paths carry type generic arguments while metadata reports
                    // MethodGeneric slots. Allow a fallback into type_generics for those
                    // mixed canonical forms.
                    let method_arity = self.method_generics.len();
                    let type_candidate_indices = [
                        Some(i),
                        i.checked_sub(method_arity),
                        i.checked_sub(method_arity + 1),
                        i.checked_sub(1),
                    ];
                    for type_index in type_candidate_indices.into_iter().flatten() {
                        if let Some(ty) = self.type_generics.get(type_index).cloned() {
                            return Ok(ty);
                        }
                    }

                    let length = self.method_generics.len();
                    Err(TypeResolutionError::GenericIndexOutOfBounds { index: i, length })
                }
            }
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

        // Zero-arity generic parameter lists require no validation. Keep strict
        // arity checks for all other cases so invalid bindings still fail fast.
        if args.len() != generic_parameters.len() {
            if generic_parameters.is_empty() {
                return Ok(());
            }
            return Err(TypeResolutionError::GenericIndexOutOfBounds {
                index: generic_parameters.len(),
                length: args.len(),
            });
        }

        let comparer = crate::comparer::TypeComparer::new(loader);

        for (arg, param) in args.iter().zip(generic_parameters.iter()) {
            // Special constraints
            if param.special_constraint.reference_type && !arg.is_class(loader) {
                return Err(TypeResolutionError::GenericConstraintViolation(
                    format!(
                        "Type {} must be a reference type to satisfy constraint on {}",
                        arg.show(arg.resolution().definition()),
                        param.name
                    )
                    .into(),
                ));
            }
            if param.special_constraint.value_type
                && (!arg.is_value_type(loader) || arg.is_nullable(loader))
            {
                return Err(TypeResolutionError::GenericConstraintViolation(
                    format!(
                        "Type {} must be a non-nullable value type to satisfy constraint on {}",
                        arg.show(arg.resolution().definition()),
                        param.name
                    )
                    .into(),
                ));
            }
            if param.special_constraint.has_default_constructor
                && !arg.has_default_constructor(loader)
            {
                return Err(TypeResolutionError::GenericConstraintViolation(
                    format!(
                        "Type {} must have a public default constructor to satisfy constraint on {}",
                        arg.show(arg.resolution().definition()),
                        param.name
                    )
                    .into(),
                ));
            }

            // Type constraints
            for constraint in &param.type_constraints {
                let constraint_type =
                    self.make_concrete(res.clone(), constraint.constraint_type.clone(), loader)?;
                if !comparer.is_assignable_to(arg, &constraint_type) {
                    return Err(TypeResolutionError::GenericConstraintViolation(
                        format!(
                            "Type {} must be assignable to {} to satisfy constraint on {}",
                            arg.show(arg.resolution().definition()),
                            constraint_type.show(constraint_type.resolution().definition()),
                            param.name
                        )
                        .into(),
                    ));
                }
            }
        }

        Ok(())
    }
}

// Thread-local visited set used by `make_concrete` to detect and break
// cyclic generic constraint chains
// (`validate_constraints` → `make_concrete` → `validate_constraints` → …).
//
// Each entry is a `(ResolutionS, UserType)` pair representing a
// (resolution, type-handle) whose constraint validation is currently
// in progress on this thread.  Before calling `validate_constraints`
// for a resolved generic type, `make_concrete` checks whether the pair
// is already present; if so it short-circuits without re-entering
// `validate_constraints`.
//
// `CONSTRAINT_VALIDATION_DEPTH` is a secondary guard that caps the total
// nesting depth of constraint validation (across all types on the current
// call stack) so that wide BCL interface hierarchies (e.g. the many
// IEquatable<T>/IComparable<T> chains on primitive types) cannot produce
// an unbounded work explosion even when the per-type cycle guard does not
// fire.  When the depth exceeds the cap we return `Ok(concrete)`
// permissively, consistent with the cycle-guard short-circuit.
#[cfg(feature = "generic-constraint-validation")]
const CONSTRAINT_VALIDATION_DEPTH_LIMIT: usize = 64;
#[cfg(feature = "generic-constraint-validation")]
const CONSTRAINT_VALIDATION_CACHE_LIMIT: usize = 8192;

#[cfg(feature = "generic-constraint-validation")]
thread_local! {
    static CONSTRAINT_VALIDATION_VISITING: RefCell<HashSet<(ResolutionS, UserType)>> =
        RefCell::new(HashSet::new());
    static CONSTRAINT_VALIDATION_DEPTH: RefCell<usize> = const { RefCell::new(0) };
    static CONSTRAINT_VALIDATION_CACHE:
        RefCell<std::collections::HashMap<ConcreteType, Result<(), TypeResolutionError>>> =
            RefCell::new(std::collections::HashMap::new());
}

#[cfg(feature = "generic-constraint-validation")]
fn cached_constraint_validation_result(
    key: &ConcreteType,
) -> Option<Result<(), TypeResolutionError>> {
    CONSTRAINT_VALIDATION_CACHE.with(|cache| cache.borrow().get(key).cloned())
}

#[cfg(feature = "generic-constraint-validation")]
fn cache_constraint_validation_result(key: ConcreteType, result: Result<(), TypeResolutionError>) {
    CONSTRAINT_VALIDATION_CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        if cache.len() >= CONSTRAINT_VALIDATION_CACHE_LIMIT {
            cache.clear();
        }
        cache.insert(key, result);
    });
}

/// Regression tests for generic constraint cycle/self-reference behavior.
///
/// These tests target the mutual recursion between `validate_constraints` and
/// `make_concrete` that is triggered under the `generic-constraint-validation`
/// feature when a type parameter's constraint is itself a generic instantiation
/// of a type that has constraints (e.g. `T : ICyclicConstraint<T>`).
///
#[cfg(all(test, feature = "generic-constraint-validation"))]
mod constraint_cycle_tests {
    use super::*;
    use crate::error::TypeResolutionError;
    use crate::resolution::MetadataArena;
    use crate::resolution::ResolutionS;
    use dotnetdll::prelude::{
        BaseType, MemberType, Module, Resolution, TypeDefinition, TypeIndex, TypeSource, UserType,
    };
    use dotnetdll::resolved::generic::{Constraint, Generic};
    use std::sync::{Arc, Mutex};

    fn one_type_index() -> TypeIndex {
        crate::type_index_from_usize(1)
    }

    fn make_resolution_with_type(type_def: TypeDefinition<'static>) -> (ResolutionS, TypeIndex) {
        let mut resolution = Resolution::new(Module::new("test.dll"));
        let type_index = resolution.push_type_definition(type_def);
        let ptr = Box::into_raw(Box::new(resolution)) as *const Resolution<'static>;
        let arena = Arc::new(MetadataArena::new());
        unsafe { arena.add_resolution(ptr) };
        (ResolutionS::new(ptr, arena), type_index)
    }

    /// Builds a `TypeDefinition<'static>` representing `ICyclicConstraint<T>` where
    /// the single generic parameter `T` carries a type constraint `ICyclicConstraint<T>`.
    /// This is the minimal structure that triggers infinite recursion in
    /// `validate_constraints` → `make_concrete` → `validate_constraints` …
    fn make_cyclic_type_def() -> TypeDefinition<'static> {
        let mut def = TypeDefinition::new(None, "ICyclicConstraint");
        let mut param: Generic<'static, MemberType> = Generic::new("T");
        // Constraint: T must implement ICyclicConstraint<T> (self-referential).
        param.type_constraints.push(Constraint {
            attributes: vec![],
            custom_modifiers: vec![],
            constraint_type: MemberType::Base(Box::new(BaseType::Type {
                value_kind: None,
                source: TypeSource::Generic {
                    base: UserType::Definition(one_type_index()),
                    parameters: vec![MemberType::TypeGeneric(0)],
                },
            })),
        });
        def.generic_parameters.push(param);
        def
    }

    fn make_cyclic_constraint_params(type_index: TypeIndex) -> Vec<Generic<'static, MemberType>> {
        let mut param: Generic<'static, MemberType> = Generic::new("T");
        param.type_constraints.push(Constraint {
            attributes: vec![],
            custom_modifiers: vec![],
            constraint_type: MemberType::Base(Box::new(BaseType::Type {
                value_kind: None,
                source: TypeSource::Generic {
                    base: UserType::Definition(type_index),
                    parameters: vec![MemberType::TypeGeneric(0)],
                },
            })),
        });

        vec![param]
    }

    /// A mock `TypeResolver` that counts every `locate_type` call and terminates
    /// the recursion by returning `Err` once `max_calls` is exceeded.  This lets
    /// the tests observe unbounded recursion without causing an actual stack overflow.
    struct CountingMockResolver {
        call_count: Arc<Mutex<usize>>,
        max_calls: usize,
        type_index: TypeIndex,
    }

    impl TypeResolver for CountingMockResolver {
        fn corlib_type(&self, _name: &str) -> Result<TypeDescription, TypeResolutionError> {
            Err(TypeResolutionError::TypeNotFound(
                "corlib_type not supported in CountingMockResolver".into(),
            ))
        }

        fn locate_type(
            &self,
            res: ResolutionS,
            _handle: UserType,
        ) -> Result<TypeDescription, TypeResolutionError> {
            let mut count = self.call_count.lock().unwrap();
            *count += 1;
            if *count > self.max_calls {
                return Err(TypeResolutionError::TypeNotFound(
                    format!(
                        "locate_type call limit {} exceeded — \
                     unbounded recursion detected in constraint validation",
                        self.max_calls
                    )
                    .into(),
                ));
            }
            drop(count);
            Ok(TypeDescription::new(res, self.type_index))
        }

        fn find_concrete_type(
            &self,
            _ty: ConcreteType,
        ) -> Result<TypeDescription, TypeResolutionError> {
            Err(TypeResolutionError::TypeNotFound(
                "find_concrete_type not supported in CountingMockResolver".into(),
            ))
        }
    }

    // -------------------------------------------------------------------------
    // Baseline: no constraints on a single generic parameter → Ok(())
    // -------------------------------------------------------------------------

    /// Baseline sanity check: a generic parameter with no type constraints should
    /// pass validation without ever calling `locate_type`.
    #[test]
    fn test_validate_constraints_no_type_constraints_returns_ok() {
        let (res, type_index) = make_resolution_with_type(TypeDefinition::new(None, "SimpleType"));
        let call_count = Arc::new(Mutex::new(0usize));
        let resolver = CountingMockResolver {
            call_count: Arc::clone(&call_count),
            max_calls: 0,
            type_index,
        };

        let arg = ConcreteType::new(res.clone(), BaseType::Boolean);
        let lookup = GenericLookup::new(vec![arg]);

        let param: Generic<'static, MemberType> = Generic::new("T");
        let params: Vec<Generic<'static, MemberType>> = vec![param];

        let result = lookup.validate_constraints(res, &resolver, &params, false);

        assert!(
            result.is_ok(),
            "validate_constraints with no constraints must return Ok(()), got {:?}",
            result
        );
        assert_eq!(
            *call_count.lock().unwrap(),
            0,
            "locate_type must not be called when there are no type constraints"
        );
    }

    #[test]
    fn test_validate_constraints_zero_arity_mismatch_is_ignored() {
        let (res, type_index) = make_resolution_with_type(TypeDefinition::new(None, "SimpleType"));
        let call_count = Arc::new(Mutex::new(0usize));
        let resolver = CountingMockResolver {
            call_count: Arc::clone(&call_count),
            max_calls: 0,
            type_index,
        };

        let arg = ConcreteType::new(res.clone(), BaseType::Boolean);
        let lookup = GenericLookup::new(vec![arg]);
        let params: Vec<Generic<'static, MemberType>> = vec![];

        let result = lookup.validate_constraints(res, &resolver, &params, false);
        assert!(
            result.is_ok(),
            "zero-arity generic parameter lists should skip validation, got {:?}",
            result
        );
        assert_eq!(
            *call_count.lock().unwrap(),
            0,
            "zero-arity validation skip must not resolve any types"
        );
    }

    #[test]
    fn test_validate_constraints_non_zero_arity_mismatch_errors() {
        let (res, type_index) = make_resolution_with_type(TypeDefinition::new(None, "SimpleType"));
        let call_count = Arc::new(Mutex::new(0usize));
        let resolver = CountingMockResolver {
            call_count: Arc::clone(&call_count),
            max_calls: 0,
            type_index,
        };

        let arg = ConcreteType::new(res.clone(), BaseType::Boolean);
        let lookup = GenericLookup::new(vec![arg]);
        let params: Vec<Generic<'static, MemberType>> = vec![Generic::new("T"), Generic::new("U")];

        let result = lookup.validate_constraints(res, &resolver, &params, false);
        assert_eq!(
            result,
            Err(TypeResolutionError::GenericIndexOutOfBounds {
                index: 2,
                length: 1,
            })
        );
        assert_eq!(
            *call_count.lock().unwrap(),
            0,
            "arity mismatch should fail before any type-resolution work"
        );
    }

    // -------------------------------------------------------------------------
    // Self-reference via TypeGeneric — does NOT trigger recursion
    // -------------------------------------------------------------------------

    /// A direct `TypeGeneric(0)` constraint on param 0 resolves immediately in
    /// `make_concrete` (returns `type_generics[0]` without entering the
    /// `TypeSource::Generic` branch).  `locate_type` must never be called.
    #[test]
    fn test_direct_type_generic_self_reference_does_not_recurse() {
        let (res, type_index) = make_resolution_with_type(TypeDefinition::new(None, "FlatType"));
        let call_count = Arc::new(Mutex::new(0usize));
        let resolver = CountingMockResolver {
            call_count: Arc::clone(&call_count),
            max_calls: 1,
            type_index,
        };

        let arg = ConcreteType::new(res.clone(), BaseType::Boolean);
        let lookup = GenericLookup::new(vec![arg]);

        // Constraint: T must implement T itself (TypeGeneric(0) — direct self-reference).
        let mut param: Generic<'static, MemberType> = Generic::new("T");
        param.type_constraints.push(Constraint {
            attributes: vec![],
            custom_modifiers: vec![],
            constraint_type: MemberType::TypeGeneric(0),
        });
        let params: Vec<Generic<'static, MemberType>> = vec![param];

        let _result = lookup.validate_constraints(res, &resolver, &params, false);

        assert_eq!(
            *call_count.lock().unwrap(),
            0,
            "locate_type must not be called for a direct TypeGeneric constraint; \
             make_concrete resolves TypeGeneric immediately from type_generics[]"
        );
    }

    // -------------------------------------------------------------------------
    // Cyclic constraint via TypeSource::Generic — triggers unbounded recursion
    // -------------------------------------------------------------------------

    /// Regression test for the fixed behavior:
    ///
    /// After the visited-set guard was added to `make_concrete`, a cyclic
    /// constraint `T : ICyclicConstraint<T>` must be detected on the *first*
    /// re-entry and must **not** call `locate_type` more than once.
    ///
    /// Previously this test documented the bug (`locate_type` called > 1 time);
    /// now it documents the correct fixed behaviour.
    #[test]
    fn test_cyclic_generic_constraint_causes_unbounded_recursion_regression() {
        let (res, type_index) = make_resolution_with_type(make_cyclic_type_def());
        let call_count = Arc::new(Mutex::new(0usize));
        let max_calls = 5usize;
        let resolver = CountingMockResolver {
            call_count: Arc::clone(&call_count),
            max_calls,
            type_index,
        };

        let arg = ConcreteType::new(res.clone(), BaseType::Boolean);
        let lookup = GenericLookup::new(vec![arg]);

        // Outer param T with constraint ICyclicConstraint<T> — creates the cycle.
        let params = make_cyclic_constraint_params(type_index);

        let result = lookup.validate_constraints(res, &resolver, &params, false);
        let final_count = *call_count.lock().unwrap();

        // Cycle guard should terminate quickly; current implementation may perform
        // one extra locate_type before short-circuiting.
        assert!(
            final_count <= 2,
            "After cycle-detection fix, locate_type should be called at most twice, \
             but was called {} times",
            final_count
        );

        // The cycle detector must prevent the dedicated cycle error from escaping.
        assert!(
            !matches!(
                &result,
                Err(TypeResolutionError::GenericConstraintViolation(msg))
                    if msg.contains("Cyclic generic constraint detected")
            ),
            "cycle detection must not return the cyclic-recursion error, got {:?}",
            result
        );
    }

    /// Regression test: cycle detection terminates quickly and returns `Ok(())`.
    ///
    /// This duplicates the core cycle scenario with a larger locate-type budget
    /// to ensure recursion does not grow with a permissive cycle fallback.
    #[test]
    fn test_cyclic_generic_constraint_detection_terminates_quickly() {
        let (res, type_index) = make_resolution_with_type(make_cyclic_type_def());
        let call_count = Arc::new(Mutex::new(0usize));
        let resolver = CountingMockResolver {
            call_count: Arc::clone(&call_count),
            max_calls: 50, // high cap; the fix should never get close to it
            type_index,
        };

        let arg = ConcreteType::new(res.clone(), BaseType::Boolean);
        let lookup = GenericLookup::new(vec![arg]);

        let params = make_cyclic_constraint_params(type_index);

        let result = lookup.validate_constraints(res, &resolver, &params, false);
        let final_count = *call_count.lock().unwrap();

        // After fix: cycle detected early, locate_type called at most twice.
        assert!(
            final_count <= 2,
            "After cycle-detection fix, locate_type should be called at most twice, \
             but was called {} times",
            final_count
        );

        // The cycle detector must prevent the dedicated cycle error from escaping.
        assert!(
            !matches!(
                &result,
                Err(TypeResolutionError::GenericConstraintViolation(msg))
                    if msg.contains("Cyclic generic constraint detected")
            ),
            "cycle detection must not return the cyclic-recursion error, got {:?}",
            result
        );
    }

    #[test]
    fn test_non_cyclic_constraint_violations_still_error() {
        let (res, type_index) = make_resolution_with_type(TypeDefinition::new(None, "RefTypeOnly"));
        let call_count = Arc::new(Mutex::new(0usize));
        let resolver = CountingMockResolver {
            call_count: Arc::clone(&call_count),
            max_calls: 1,
            type_index,
        };

        // bool is a value type; it should fail a reference-type constraint.
        let arg = ConcreteType::new(res.clone(), BaseType::Boolean);
        let lookup = GenericLookup::new(vec![arg]);

        let mut param: Generic<'static, MemberType> = Generic::new("T");
        param.special_constraint.reference_type = true;
        let params: Vec<Generic<'static, MemberType>> = vec![param];

        let result = lookup.validate_constraints(res, &resolver, &params, false);
        assert!(
            matches!(
                result,
                Err(TypeResolutionError::GenericConstraintViolation(_))
            ),
            "non-cyclic constraint violations must still return GenericConstraintViolation, got {:?}",
            result
        );
        assert_eq!(
            *call_count.lock().unwrap(),
            0,
            "special-constraint validation should not require locate_type calls"
        );
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

#[cfg(test)]
mod generic_lookup_arg_tests {
    use super::*;

    fn sample_type() -> ConcreteType {
        ConcreteType::new(ResolutionS::NULL, BaseType::Int32)
    }

    fn sample_method_type() -> ConcreteType {
        ConcreteType::new(ResolutionS::NULL, BaseType::UInt32)
    }

    #[test]
    fn type_arg_returns_requested_slot() {
        let ty = sample_type();
        let lookup = GenericLookup {
            type_generics: vec![ty.clone()].into(),
            method_generics: Arc::new([]),
        };

        assert_eq!(lookup.type_arg(0), Ok(&ty));
        assert_eq!(lookup.cloned_type_arg(0), Ok(ty));
    }

    #[test]
    fn method_arg_returns_requested_slot() {
        let ty = sample_method_type();
        let lookup = GenericLookup {
            type_generics: Arc::new([]),
            method_generics: vec![ty.clone()].into(),
        };

        assert_eq!(lookup.method_arg(0), Ok(&ty));
        assert_eq!(lookup.cloned_method_arg(0), Ok(ty));
    }

    #[test]
    fn bounded_accessors_return_generic_index_out_of_bounds() {
        let lookup = GenericLookup::default();

        assert_eq!(
            lookup.type_arg(0),
            Err(TypeResolutionError::GenericIndexOutOfBounds {
                index: 0,
                length: 0,
            })
        );
        assert_eq!(
            lookup.method_arg(1),
            Err(TypeResolutionError::GenericIndexOutOfBounds {
                index: 1,
                length: 0,
            })
        );
    }
}

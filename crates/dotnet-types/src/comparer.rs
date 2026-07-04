use crate::{
    TypeDescription, TypeResolver,
    generics::{ConcreteType, GenericLookup, member_to_method_type},
    members::MethodDescription,
    resolution::ResolutionS,
};
use dotnetdll::prelude::*;
use std::{
    cell::RefCell,
    collections::{HashMap, HashSet, VecDeque},
};

pub struct TypeComparer<'a, R: TypeResolver> {
    loader: &'a R,
    assignability_cache: RefCell<HashMap<(ConcreteType, ConcreteType), bool>>,
    assignability_in_progress: RefCell<HashSet<(ConcreteType, ConcreteType)>>,
}

#[derive(Clone, Copy)]
enum TypeIdentityComparison {
    CanonicalNameFallback,
    DefinitionNameFallback,
}

#[derive(Clone, Copy)]
enum TypeResolutionFailureBehavior {
    ReturnFalse,
    PanicOnFailure,
    CompareResultsThenReturnFalse,
}

#[derive(Clone, Copy)]
struct TypeComparisonPolicy {
    type_identity: TypeIdentityComparison,
    resolution_failure: TypeResolutionFailureBehavior,
}

impl TypeComparisonPolicy {
    const TYPES_EQUAL: Self = Self {
        type_identity: TypeIdentityComparison::CanonicalNameFallback,
        resolution_failure: TypeResolutionFailureBehavior::ReturnFalse,
    };

    const CONCRETE_EQUALS_BASE_TYPE: Self = Self {
        type_identity: TypeIdentityComparison::DefinitionNameFallback,
        resolution_failure: TypeResolutionFailureBehavior::PanicOnFailure,
    };

    const CONCRETE_TYPES_EQUAL: Self = Self {
        type_identity: TypeIdentityComparison::DefinitionNameFallback,
        resolution_failure: TypeResolutionFailureBehavior::CompareResultsThenReturnFalse,
    };
}

#[derive(Clone, Copy)]
enum NormalizedTypeRef<'a> {
    Method {
        resolution: &'a ResolutionS,
        ty: &'a MethodType,
        generics: Option<&'a GenericLookup>,
    },
    Concrete(&'a ConcreteType),
}

#[derive(Clone, Copy)]
enum SignatureCompareMode {
    Invariant,
    Variant,
}

impl<'a, R: TypeResolver> TypeComparer<'a, R> {
    pub fn new(loader: &'a R) -> Self {
        Self {
            loader,
            assignability_cache: RefCell::new(HashMap::new()),
            assignability_in_progress: RefCell::new(HashSet::new()),
        }
    }

    pub fn type_slices_equal(
        &self,
        res1: &ResolutionS,
        a: &[MethodType],
        generics1: Option<&GenericLookup>,
        res2: &ResolutionS,
        b: &[MethodType],
        generics2: Option<&GenericLookup>,
    ) -> bool {
        if a.len() != b.len() {
            return false;
        }
        for (a, b) in a.iter().zip(b.iter()) {
            if !self.types_equal(res1, a, generics1, res2, b, generics2) {
                return false;
            }
        }
        true
    }

    pub fn types_equal(
        &self,
        res1: &ResolutionS,
        a: &MethodType,
        generics1: Option<&GenericLookup>,
        res2: &ResolutionS,
        b: &MethodType,
        generics2: Option<&GenericLookup>,
    ) -> bool {
        self.normalized_types_equal(
            NormalizedTypeRef::Method {
                resolution: res1,
                ty: a,
                generics: generics1,
            },
            NormalizedTypeRef::Method {
                resolution: res2,
                ty: b,
                generics: generics2,
            },
            TypeComparisonPolicy::TYPES_EQUAL,
        )
    }

    fn resolve_generic_type<'g>(
        generics: Option<&'g GenericLookup>,
        ty: &MethodType,
    ) -> Option<&'g ConcreteType> {
        let generics = generics?;
        match ty {
            MethodType::TypeGeneric(idx) => generics.type_generics.get(*idx),
            MethodType::MethodGeneric(idx) => generics.method_generics.get(*idx),
            _ => None,
        }
    }

    fn same_generic_placeholder(left: &MethodType, right: &MethodType) -> bool {
        match (left, right) {
            (MethodType::TypeGeneric(i1), MethodType::TypeGeneric(i2)) if i1 == i2 => true,
            (MethodType::MethodGeneric(i1), MethodType::MethodGeneric(i2)) if i1 == i2 => true,
            _ => false,
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn normalized_method_types_equal(
        &self,
        left_resolution: &ResolutionS,
        left: &MethodType,
        left_generics: Option<&GenericLookup>,
        right_resolution: &ResolutionS,
        right: &MethodType,
        right_generics: Option<&GenericLookup>,
        policy: TypeComparisonPolicy,
    ) -> bool {
        // Preserve identity of matching generic placeholders before any concrete substitution.
        // If we substitute first, cross-resolution concrete equality can spuriously fail even
        // when both signatures reference the same generic slot (e.g. T0 vs T0).
        if Self::same_generic_placeholder(left, right) {
            return true;
        }

        if let Some(concrete) = Self::resolve_generic_type(left_generics, left) {
            return self.normalized_concrete_equals_method_type(
                concrete,
                right_resolution,
                right,
                right_generics,
                policy,
            );
        }

        if let Some(concrete) = Self::resolve_generic_type(right_generics, right) {
            return self.normalized_concrete_equals_method_type(
                concrete,
                left_resolution,
                left,
                left_generics,
                policy,
            );
        }

        match (left, right) {
            (MethodType::Base(left_base), MethodType::Base(right_base)) => self
                .normalized_base_types_equal(
                    ResolutionS::clone(left_resolution),
                    left_base,
                    ResolutionS::clone(right_resolution),
                    right_base,
                    policy,
                    |left_child, right_child| {
                        self.normalized_types_equal(
                            NormalizedTypeRef::Method {
                                resolution: left_resolution,
                                ty: left_child,
                                generics: left_generics,
                            },
                            NormalizedTypeRef::Method {
                                resolution: right_resolution,
                                ty: right_child,
                                generics: right_generics,
                            },
                            policy,
                        )
                    },
                ),
            _ => false,
        }
    }

    fn normalized_concrete_equals_method_type(
        &self,
        concrete: &ConcreteType,
        method_resolution: &ResolutionS,
        method_type: &MethodType,
        method_generics: Option<&GenericLookup>,
        policy: TypeComparisonPolicy,
    ) -> bool {
        if let Some(other) = Self::resolve_generic_type(method_generics, method_type) {
            return concrete == other;
        }

        match method_type {
            MethodType::Base(base_type) => self.normalized_base_types_equal(
                concrete.resolution(),
                concrete.get(),
                ResolutionS::clone(method_resolution),
                base_type,
                policy,
                |left_child, right_child| {
                    self.normalized_types_equal(
                        NormalizedTypeRef::Concrete(left_child),
                        NormalizedTypeRef::Method {
                            resolution: method_resolution,
                            ty: right_child,
                            generics: method_generics,
                        },
                        policy,
                    )
                },
            ),
            _ => false,
        }
    }

    fn normalized_types_equal(
        &self,
        left: NormalizedTypeRef<'_>,
        right: NormalizedTypeRef<'_>,
        policy: TypeComparisonPolicy,
    ) -> bool {
        match (left, right) {
            (
                NormalizedTypeRef::Method {
                    resolution: left_resolution,
                    ty: left_type,
                    generics: left_generics,
                },
                NormalizedTypeRef::Method {
                    resolution: right_resolution,
                    ty: right_type,
                    generics: right_generics,
                },
            ) => self.normalized_method_types_equal(
                left_resolution,
                left_type,
                left_generics,
                right_resolution,
                right_type,
                right_generics,
                policy,
            ),
            (
                NormalizedTypeRef::Concrete(left_concrete),
                NormalizedTypeRef::Method {
                    resolution,
                    ty,
                    generics,
                },
            ) => self.normalized_concrete_equals_method_type(
                left_concrete,
                resolution,
                ty,
                generics,
                policy,
            ),
            (
                NormalizedTypeRef::Method {
                    resolution,
                    ty,
                    generics,
                },
                NormalizedTypeRef::Concrete(right_concrete),
            ) => self.normalized_concrete_equals_method_type(
                right_concrete,
                resolution,
                ty,
                generics,
                policy,
            ),
            (
                NormalizedTypeRef::Concrete(left_concrete),
                NormalizedTypeRef::Concrete(right_concrete),
            ) => {
                if left_concrete == right_concrete {
                    return true;
                }

                self.normalized_base_types_equal(
                    left_concrete.resolution(),
                    left_concrete.get(),
                    right_concrete.resolution(),
                    right_concrete.get(),
                    policy,
                    |left_child, right_child| {
                        self.normalized_types_equal(
                            NormalizedTypeRef::Concrete(left_child),
                            NormalizedTypeRef::Concrete(right_child),
                            policy,
                        )
                    },
                )
            }
        }
    }

    fn resolve_type_with_policy(
        &self,
        resolution: ResolutionS,
        user_type: UserType,
        policy: TypeComparisonPolicy,
    ) -> Option<TypeDescription> {
        match policy.resolution_failure {
            TypeResolutionFailureBehavior::ReturnFalse => {
                self.loader.locate_type(resolution, user_type).ok()
            }
            TypeResolutionFailureBehavior::PanicOnFailure => Some(
                self.loader
                    .locate_type(resolution, user_type)
                    .expect("Type resolution failed during comparison"),
            ),
            TypeResolutionFailureBehavior::CompareResultsThenReturnFalse => {
                self.loader.locate_type(resolution, user_type).ok()
            }
        }
    }

    fn resolved_types_equal(
        &self,
        left: &TypeDescription,
        right: &TypeDescription,
        policy: TypeComparisonPolicy,
    ) -> bool {
        if left == right {
            return true;
        }

        match policy.type_identity {
            TypeIdentityComparison::CanonicalNameFallback => {
                !left.is_null()
                    && !right.is_null()
                    && self.loader.canonical_type_name(&left.type_name())
                        == self.loader.canonical_type_name(&right.type_name())
            }
            TypeIdentityComparison::DefinitionNameFallback => {
                let left_definition = left.definition();
                let right_definition = right.definition();
                left_definition.name == right_definition.name
                    && left_definition.namespace == right_definition.namespace
            }
        }
    }

    fn normalized_base_types_equal<L: Clone, Rhs: Clone, F>(
        &self,
        left_resolution: ResolutionS,
        left: &BaseType<L>,
        right_resolution: ResolutionS,
        right: &BaseType<Rhs>,
        policy: TypeComparisonPolicy,
        mut child_equal: F,
    ) -> bool
    where
        F: FnMut(&L, &Rhs) -> bool,
    {
        match (left, right) {
            (BaseType::Type { source: ts1, .. }, BaseType::Type { source: ts2, .. }) => {
                let (ut1, generics1) = decompose_type_source(ts1);
                let (ut2, generics2) = decompose_type_source(ts2);

                let same_type = match policy.resolution_failure {
                    TypeResolutionFailureBehavior::CompareResultsThenReturnFalse => {
                        let left_result = self.loader.locate_type(left_resolution, ut1);
                        let right_result = self.loader.locate_type(right_resolution, ut2);

                        if left_result == right_result {
                            true
                        } else {
                            let Ok(left_type) = left_result else {
                                return false;
                            };
                            let Ok(right_type) = right_result else {
                                return false;
                            };
                            self.resolved_types_equal(&left_type, &right_type, policy)
                        }
                    }
                    _ => {
                        let Some(left_type) =
                            self.resolve_type_with_policy(left_resolution, ut1, policy)
                        else {
                            return false;
                        };
                        let Some(right_type) =
                            self.resolve_type_with_policy(right_resolution, ut2, policy)
                        else {
                            return false;
                        };
                        self.resolved_types_equal(&left_type, &right_type, policy)
                    }
                };

                if !same_type {
                    return false;
                }

                if generics1.len() != generics2.len() {
                    return false;
                }

                for (g1, g2) in generics1.iter().zip(generics2.iter()) {
                    if !child_equal(g1, g2) {
                        return false;
                    }
                }
                true
            }
            (BaseType::Boolean, BaseType::Boolean) => true,
            (BaseType::Char, BaseType::Char) => true,
            (BaseType::Int8, BaseType::Int8) => true,
            (BaseType::UInt8, BaseType::UInt8) => true,
            (BaseType::Int16, BaseType::Int16) => true,
            (BaseType::UInt16, BaseType::UInt16) => true,
            (BaseType::Int32, BaseType::Int32) => true,
            (BaseType::UInt32, BaseType::UInt32) => true,
            (BaseType::Int64, BaseType::Int64) => true,
            (BaseType::UInt64, BaseType::UInt64) => true,
            (BaseType::Float32, BaseType::Float32) => true,
            (BaseType::Float64, BaseType::Float64) => true,
            (BaseType::IntPtr, BaseType::IntPtr) => true,
            (BaseType::UIntPtr, BaseType::UIntPtr) => true,
            (BaseType::Object, BaseType::Object) => true,
            (BaseType::String, BaseType::String) => true,
            (BaseType::Vector(_, l), BaseType::Vector(_, r)) => child_equal(l, r),
            (BaseType::Array(l, _), BaseType::Array(r, _)) => child_equal(l, r),
            (BaseType::ValuePointer(_, l), BaseType::ValuePointer(_, r)) => {
                match (l.as_ref(), r.as_ref()) {
                    (None, None) => true,
                    (Some(l_element), Some(r_element)) => child_equal(l_element, r_element),
                    _ => false,
                }
            }
            // Function pointer signature comparison is not implemented yet.
            // Conservatively treat funcptr types as not equal.
            (BaseType::FunctionPointer(_), BaseType::FunctionPointer(_)) => false,
            _ => false,
        }
    }

    pub fn param_types_equal(
        &self,
        res1: &ResolutionS,
        a: &ParameterType<MethodType>,
        generics1: Option<&GenericLookup>,
        res2: &ResolutionS,
        b: &ParameterType<MethodType>,
        generics2: Option<&GenericLookup>,
    ) -> bool {
        match (a, b) {
            (ParameterType::Value(l), ParameterType::Value(r))
            | (ParameterType::Ref(l), ParameterType::Ref(r)) => {
                self.types_equal(res1, l, generics1, res2, r, generics2)
            }
            (ParameterType::TypedReference, ParameterType::TypedReference) => true,
            _ => false,
        }
    }

    pub fn params_equal(
        &self,
        res1: &ResolutionS,
        a: &[Parameter<MethodType>],
        generics1: Option<&GenericLookup>,
        res2: &ResolutionS,
        b: &[Parameter<MethodType>],
        generics2: Option<&GenericLookup>,
    ) -> bool {
        if a.len() != b.len() {
            return false;
        }
        for (Parameter(_, a), Parameter(_, b)) in a.iter().zip(b.iter()) {
            if !self.param_types_equal(res1, a, generics1, res2, b, generics2) {
                return false;
            }
        }
        true
    }

    #[allow(clippy::too_many_arguments)]
    fn signatures_match(
        &self,
        mode: SignatureCompareMode,
        res1: &ResolutionS,
        a: &ManagedMethod<MethodType>,
        generics1: Option<&GenericLookup>,
        res2: &ResolutionS,
        b: &ManagedMethod<MethodType>,
        generics2: Option<&GenericLookup>,
    ) -> bool {
        if a.instance != b.instance {
            return false;
        }
        if a.parameters.len() != b.parameters.len() {
            return false;
        }

        let variance_lookups = match mode {
            SignatureCompareMode::Invariant => None,
            SignatureCompareMode::Variant => Some((
                generics1.cloned().unwrap_or_default(),
                generics2.cloned().unwrap_or_default(),
            )),
        };

        if !self.signature_return_types_match(
            mode,
            res1,
            &a.return_type,
            generics1,
            res2,
            &b.return_type,
            generics2,
            variance_lookups.as_ref().map(|(l, r)| (l, r)),
        ) {
            return false;
        }

        for (Parameter(_, a_parameter), Parameter(_, b_parameter)) in
            a.parameters.iter().zip(b.parameters.iter())
        {
            if !self.signature_parameter_types_match(
                mode,
                res1,
                a_parameter,
                generics1,
                res2,
                b_parameter,
                generics2,
                variance_lookups.as_ref().map(|(l, r)| (l, r)),
            ) {
                return false;
            }
        }

        true
    }

    #[allow(clippy::too_many_arguments)]
    fn signature_return_types_match(
        &self,
        mode: SignatureCompareMode,
        res1: &ResolutionS,
        left: &ReturnType<MethodType>,
        generics1: Option<&GenericLookup>,
        res2: &ResolutionS,
        right: &ReturnType<MethodType>,
        generics2: Option<&GenericLookup>,
        variance_lookups: Option<(&GenericLookup, &GenericLookup)>,
    ) -> bool {
        let ReturnType(_, left_return) = left;
        let ReturnType(_, right_return) = right;

        match (left_return, right_return) {
            (None, None) => true,
            (Some(left_type), Some(right_type)) => match mode {
                SignatureCompareMode::Invariant => {
                    self.param_types_equal(res1, left_type, generics1, res2, right_type, generics2)
                }
                SignatureCompareMode::Variant => {
                    let Some((lookup1, lookup2)) = variance_lookups else {
                        unreachable!("variance lookups should exist in variant mode");
                    };

                    let left_method_type = match left_type {
                        ParameterType::Value(ty) | ParameterType::Ref(ty) => ty,
                        ParameterType::TypedReference => return false,
                    };
                    let right_method_type = match right_type {
                        ParameterType::Value(ty) | ParameterType::Ref(ty) => ty,
                        ParameterType::TypedReference => return false,
                    };

                    // Return type covariance: candidate (`right`) must be assignable to target (`left`).
                    self.variance_assignable_method_types(
                        res2,
                        right_method_type,
                        lookup2,
                        res1,
                        left_method_type,
                        lookup1,
                    )
                }
            },
            _ => false,
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn signature_parameter_types_match(
        &self,
        mode: SignatureCompareMode,
        res1: &ResolutionS,
        left: &ParameterType<MethodType>,
        generics1: Option<&GenericLookup>,
        res2: &ResolutionS,
        right: &ParameterType<MethodType>,
        generics2: Option<&GenericLookup>,
        variance_lookups: Option<(&GenericLookup, &GenericLookup)>,
    ) -> bool {
        match mode {
            SignatureCompareMode::Invariant => {
                self.param_types_equal(res1, left, generics1, res2, right, generics2)
            }
            SignatureCompareMode::Variant => match (left, right) {
                (ParameterType::Value(left_type), ParameterType::Value(right_type)) => {
                    let Some((lookup1, lookup2)) = variance_lookups else {
                        unreachable!("variance lookups should exist in variant mode");
                    };

                    // Parameter contravariance: target (`left`) must be assignable to candidate (`right`).
                    self.variance_assignable_method_types(
                        res1, left_type, lookup1, res2, right_type, lookup2,
                    )
                }
                // ByRef and TypedReference parameters remain exact in variant mode.
                _ => self.param_types_equal(res1, left, generics1, res2, right, generics2),
            },
        }
    }

    fn variance_assignable_method_types(
        &self,
        source_resolution: &ResolutionS,
        source: &MethodType,
        source_lookup: &GenericLookup,
        target_resolution: &ResolutionS,
        target: &MethodType,
        target_lookup: &GenericLookup,
    ) -> bool {
        let Ok(source_concrete) = source_lookup.make_concrete(
            ResolutionS::clone(source_resolution),
            source.clone(),
            self.loader,
        ) else {
            return false;
        };
        let Ok(target_concrete) = target_lookup.make_concrete(
            ResolutionS::clone(target_resolution),
            target.clone(),
            self.loader,
        ) else {
            return false;
        };

        self.is_assignable_to(&source_concrete, &target_concrete)
    }

    pub fn signatures_compatible_with_variance(
        &self,
        res1: &ResolutionS,
        a: &ManagedMethod<MethodType>,
        generics1: Option<&GenericLookup>,
        res2: &ResolutionS,
        b: &ManagedMethod<MethodType>,
        generics2: Option<&GenericLookup>,
    ) -> bool {
        self.signatures_match(
            SignatureCompareMode::Variant,
            res1,
            a,
            generics1,
            res2,
            b,
            generics2,
        )
    }

    pub fn signatures_equal(
        &self,
        res1: &ResolutionS,
        a: &ManagedMethod<MethodType>,
        generics1: Option<&GenericLookup>,
        res2: &ResolutionS,
        b: &ManagedMethod<MethodType>,
        generics2: Option<&GenericLookup>,
    ) -> bool {
        self.signatures_match(
            SignatureCompareMode::Invariant,
            res1,
            a,
            generics1,
            res2,
            b,
            generics2,
        )
    }

    pub fn concrete_equals_method_type(
        &self,
        concrete: &ConcreteType,
        res2: &ResolutionS,
        b: &MethodType,
        generics2: Option<&GenericLookup>,
    ) -> bool {
        self.normalized_types_equal(
            NormalizedTypeRef::Concrete(concrete),
            NormalizedTypeRef::Method {
                resolution: res2,
                ty: b,
                generics: generics2,
            },
            TypeComparisonPolicy::CONCRETE_EQUALS_BASE_TYPE,
        )
    }

    pub fn concrete_equals_base_type(
        &self,
        concrete: &ConcreteType,
        res2: &ResolutionS,
        b: &BaseType<MethodType>,
        generics2: Option<&GenericLookup>,
    ) -> bool {
        self.normalized_base_types_equal(
            concrete.resolution(),
            concrete.get(),
            ResolutionS::clone(res2),
            b,
            TypeComparisonPolicy::CONCRETE_EQUALS_BASE_TYPE,
            |left, right| self.concrete_equals_method_type(left, res2, right, generics2),
        )
    }

    pub fn is_assignable_to(&self, source: &ConcreteType, target: &ConcreteType) -> bool {
        let key = (source.clone(), target.clone());
        if let Some(cached) = self.assignability_cache.borrow().get(&key) {
            return *cached;
        }

        // Recursive generic variance checks can re-enter assignability with the
        // same `(source, target)` before the outer call completes. Treat the
        // cycle as assignable to fail-open under GCV rather than spinning.
        if !self
            .assignability_in_progress
            .borrow_mut()
            .insert(key.clone())
        {
            return true;
        }

        let result = self.is_assignable_to_impl(source, target);

        self.assignability_in_progress.borrow_mut().remove(&key);
        self.assignability_cache.borrow_mut().insert(key, result);
        result
    }

    fn is_assignable_to_impl(&self, source: &ConcreteType, target: &ConcreteType) -> bool {
        if self.concrete_types_equal(source, target) {
            return true;
        }

        if self.compatible_with_variance(source, target) {
            return true;
        }

        if matches!(target.get(), BaseType::Object) {
            return true;
        }

        if self.is_system_enum(target) {
            return self.is_subclass_of(source, target);
        }

        if self.is_system_value_type(target) {
            return source.is_value_type(self.loader);
        }

        if target.is_interface(self.loader) {
            return self.implements_interface(source, target);
        }

        if target.is_class(self.loader) {
            return self.is_subclass_of(source, target);
        }

        false
    }

    fn is_system_enum(&self, ty: &ConcreteType) -> bool {
        self.has_canonical_metadata_name(ty, "System.Enum")
    }

    fn is_system_value_type(&self, ty: &ConcreteType) -> bool {
        self.has_canonical_metadata_name(ty, "System.ValueType")
    }

    fn has_canonical_metadata_name(&self, ty: &ConcreteType, name: &str) -> bool {
        let BaseType::Type { source, .. } = ty.get() else {
            return false;
        };
        let (ut, _) = decompose_type_source(source);
        let resolution = ty.resolution();
        if resolution.is_null() {
            return false;
        }
        let ty_name = ut.type_name(resolution.definition());
        self.loader.canonical_type_name(&ty_name) == name
    }

    #[allow(clippy::mutable_key_type)]
    pub fn is_subclass_of(&self, source: &ConcreteType, target: &ConcreteType) -> bool {
        let mut curr = source.clone();
        let mut seen = HashSet::new();

        while seen.insert(curr.clone()) {
            let Ok(td) = self.loader.find_concrete_type(curr.clone()) else {
                break;
            };
            if let Some(base_source) = &td.definition().extends {
                let lookup = curr.make_lookup();
                if let Ok(base) = lookup.make_concrete(
                    td.resolution,
                    member_to_method_type(base_source),
                    self.loader,
                ) {
                    if self.concrete_types_equal(&base, target) {
                        return true;
                    }
                    curr = base;
                } else {
                    break;
                }
            } else {
                break;
            }
        }
        false
    }

    #[allow(clippy::mutable_key_type)]
    pub fn implements_interface(&self, source: &ConcreteType, target: &ConcreteType) -> bool {
        let mut queue = VecDeque::new();
        let mut seen = HashSet::new();

        queue.push_back(source.clone());
        seen.insert(source.clone());

        while let Some(curr) = queue.pop_front() {
            if let Ok(td) = self.loader.find_concrete_type(curr.clone()) {
                let lookup = curr.make_lookup();

                // Check direct interfaces
                for (_, itf_source) in &td.definition().implements {
                    if let Ok(itf) = lookup.make_concrete(
                        td.resolution.clone(),
                        member_to_method_type(itf_source),
                        self.loader,
                    ) {
                        if self.compatible_with_variance(&itf, target) {
                            return true;
                        }
                        if seen.insert(itf.clone()) {
                            queue.push_back(itf);
                        }
                    }
                }

                // Check base class interfaces
                if let Some(base_source) = &td.definition().extends
                    && let Ok(base) = lookup.make_concrete(
                        td.resolution.clone(),
                        member_to_method_type(base_source),
                        self.loader,
                    )
                    && seen.insert(base.clone())
                {
                    queue.push_back(base);
                }
            }
        }
        false
    }

    pub fn find_method_in_type_with_substitution(
        &self,
        desc: TypeDescription,
        name: &str,
        signature: &ManagedMethod<MethodType>,
        sig_res: ResolutionS,
        generics: &GenericLookup,
        allow_variance: bool,
    ) -> Option<MethodDescription> {
        self.find_method_in_type_internal(
            desc,
            name,
            signature,
            sig_res,
            Some(generics),
            Some(generics),
            allow_variance,
        )
    }

    pub fn find_method_in_type(
        &self,
        desc: TypeDescription,
        name: &str,
        signature: &ManagedMethod<MethodType>,
        sig_res: ResolutionS,
    ) -> Option<MethodDescription> {
        self.find_method_in_type_internal(desc, name, signature, sig_res, None, None, false)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn find_method_in_type_internal(
        &self,
        desc: TypeDescription,
        name: &str,
        signature: &ManagedMethod<MethodType>,
        sig_res: ResolutionS,
        sig_generics: Option<&GenericLookup>,
        type_generics: Option<&GenericLookup>,
        allow_variance: bool,
    ) -> Option<MethodDescription> {
        let def = desc.definition();
        let def_res = desc.resolution.definition();

        macro_rules! check {
            ($method:expr, $member_index:expr) => {
                let m = $method;
                if m.name == name {
                    let m_idx = MethodDescription::index_for(def_res, desc.index, $member_index);
                    let m_sig = def_res
                        .method_signature(m_idx)
                        .expect("failed to decode method signature");
                    if m_sig.parameters.len() == signature.parameters.len()
                        && (if allow_variance {
                            self.signatures_compatible_with_variance(
                                &sig_res,
                                signature,
                                sig_generics,
                                &desc.resolution,
                                m_sig,
                                type_generics,
                            )
                        } else {
                            self.signatures_equal(
                                &sig_res,
                                signature,
                                sig_generics,
                                &desc.resolution,
                                m_sig,
                                type_generics,
                            )
                        })
                    {
                        return Some(MethodDescription::new(
                            desc.clone(),
                            type_generics.cloned().unwrap_or_default(),
                            desc.resolution.clone(),
                            $member_index,
                        ));
                    }
                }
            };
        }

        for (i, method) in def.methods.iter().enumerate().rev() {
            check!(method, MethodMemberIndex::Method(i));
        }

        if let Some(prop_name) = name.strip_prefix("get_") {
            for (prop_idx, prop) in def.properties.iter().enumerate() {
                if let (true, Some(getter)) = (prop.name == prop_name, &prop.getter) {
                    check!(getter, MethodMemberIndex::PropertyGetter(prop_idx));
                }
            }
        } else if let Some(prop_name) = name.strip_prefix("set_") {
            for (prop_idx, prop) in def.properties.iter().enumerate() {
                if let (true, Some(setter)) = (prop.name == prop_name, &prop.setter) {
                    check!(setter, MethodMemberIndex::PropertySetter(prop_idx));
                }
            }
        }

        if let Some(event_name) = name.strip_prefix("add_") {
            for (event_idx, event) in def.events.iter().enumerate() {
                if event.name == event_name {
                    check!(&event.add_listener, MethodMemberIndex::EventAdd(event_idx));
                }
            }
        } else if let Some(event_name) = name.strip_prefix("remove_") {
            for (event_idx, event) in def.events.iter().enumerate() {
                if event.name == event_name {
                    check!(
                        &event.remove_listener,
                        MethodMemberIndex::EventRemove(event_idx)
                    );
                }
            }
        } else if let Some(event_name) = name.strip_prefix("raise_") {
            for (event_idx, event) in def.events.iter().enumerate() {
                if let (true, Some(raise_event)) = (event.name == event_name, &event.raise_event) {
                    check!(raise_event, MethodMemberIndex::EventRaise(event_idx));
                }
            }
        }

        for (prop_idx, prop) in def.properties.iter().enumerate() {
            for (other_idx, method) in prop.other.iter().enumerate() {
                check!(
                    method,
                    MethodMemberIndex::PropertyOther {
                        property: prop_idx,
                        other: other_idx
                    }
                );
            }
            if let Some(m) = &prop.getter {
                check!(m, MethodMemberIndex::PropertyGetter(prop_idx));
            }
            if let Some(m) = &prop.setter {
                check!(m, MethodMemberIndex::PropertySetter(prop_idx));
            }
        }

        for (event_idx, event) in def.events.iter().enumerate() {
            for (other_idx, method) in event.other.iter().enumerate() {
                check!(
                    method,
                    MethodMemberIndex::EventOther {
                        event: event_idx,
                        other: other_idx
                    }
                );
            }
            check!(&event.add_listener, MethodMemberIndex::EventAdd(event_idx));
            check!(
                &event.remove_listener,
                MethodMemberIndex::EventRemove(event_idx)
            );
            if let Some(m) = &event.raise_event {
                check!(m, MethodMemberIndex::EventRaise(event_idx));
            }
        }

        None
    }

    pub fn compatible_with_variance(&self, source: &ConcreteType, target: &ConcreteType) -> bool {
        if self.concrete_types_equal(source, target) {
            return true;
        }

        match (source.get(), target.get()) {
            (BaseType::Type { source: ts1, .. }, BaseType::Type { source: ts2, .. }) => {
                let (ut1, generics1) = decompose_type_source(ts1);
                let (ut2, generics2) = decompose_type_source(ts2);

                let Ok(td1) = self.loader.locate_type(source.resolution(), ut1) else {
                    return false;
                };
                let Ok(td2) = self.loader.locate_type(target.resolution(), ut2) else {
                    return false;
                };

                if td1 != td2 {
                    return false;
                }

                if generics1.len() != generics2.len() {
                    return false;
                }

                let def = td1.definition();
                for (i, (g1, g2)) in generics1.iter().zip(generics2.iter()).enumerate() {
                    let param = &def.generic_parameters[i];
                    // variance is likely an enum with None, Covariant, Contravariant
                    match param.variance {
                        generic::Variance::Invariant => {
                            if !self.concrete_types_equal(g1, g2) {
                                return false;
                            }
                        }
                        generic::Variance::Covariant => {
                            // Covariant: source must be assignable to target
                            if !self.is_assignable_to(g1, g2) {
                                return false;
                            }
                        }
                        generic::Variance::Contravariant => {
                            // Contravariant: target must be assignable to source
                            if !self.is_assignable_to(g2, g1) {
                                return false;
                            }
                        }
                    }
                }
                true
            }
            _ => false,
        }
    }

    pub fn concrete_types_equal(&self, c1: &ConcreteType, c2: &ConcreteType) -> bool {
        self.normalized_types_equal(
            NormalizedTypeRef::Concrete(c1),
            NormalizedTypeRef::Concrete(c2),
            TypeComparisonPolicy::CONCRETE_TYPES_EQUAL,
        )
    }
}
pub fn decompose_type_source<T: Clone>(t: &TypeSource<T>) -> (UserType, Vec<T>) {
    let mut type_generics: &[T] = &[];
    let ut = match t {
        TypeSource::User(u) => *u,
        TypeSource::Generic { base, parameters } => {
            type_generics = parameters.as_slice();
            *base
        }
    };
    (ut, type_generics.to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::TypeResolutionError;
    use crate::resolution::{MetadataArena, ResolutionS};
    use std::sync::Arc;

    struct MockResolver;
    impl TypeResolver for MockResolver {
        fn corlib_type(&self, _name: &str) -> Result<TypeDescription, TypeResolutionError> {
            Ok(TypeDescription::new(
                ResolutionS::NULL,
                crate::sentinel_type_index(),
            ))
        }
        fn locate_type(
            &self,
            res: ResolutionS,
            handle: UserType,
        ) -> Result<TypeDescription, TypeResolutionError> {
            match handle {
                UserType::Definition(d) => Ok(TypeDescription::new(res, d)),
                UserType::Reference(_) => {
                    Ok(TypeDescription::new(res, crate::sentinel_type_index()))
                }
            }
        }
        fn find_concrete_type(
            &self,
            _ty: ConcreteType,
        ) -> Result<TypeDescription, TypeResolutionError> {
            Ok(TypeDescription::new(
                ResolutionS::NULL,
                crate::sentinel_type_index(),
            ))
        }
    }

    struct SelectiveFailResolver {
        failing_definitions: Vec<TypeIndex>,
        fail_all_definitions: bool,
    }

    impl SelectiveFailResolver {
        fn for_definitions(failing_definitions: Vec<TypeIndex>) -> Self {
            Self {
                failing_definitions,
                fail_all_definitions: false,
            }
        }

        fn fail_all() -> Self {
            Self {
                failing_definitions: Vec::new(),
                fail_all_definitions: true,
            }
        }

        fn should_fail(&self, definition: TypeIndex) -> bool {
            self.fail_all_definitions || self.failing_definitions.contains(&definition)
        }
    }

    impl TypeResolver for SelectiveFailResolver {
        fn corlib_type(&self, _name: &str) -> Result<TypeDescription, TypeResolutionError> {
            Ok(TypeDescription::new(
                ResolutionS::NULL,
                crate::sentinel_type_index(),
            ))
        }

        fn locate_type(
            &self,
            res: ResolutionS,
            handle: UserType,
        ) -> Result<TypeDescription, TypeResolutionError> {
            match handle {
                UserType::Definition(definition) => {
                    if self.should_fail(definition) {
                        Err(TypeResolutionError::TypeNotFound("missing".into()))
                    } else {
                        Ok(TypeDescription::new(res, definition))
                    }
                }
                UserType::Reference(_) => {
                    Ok(TypeDescription::new(res, crate::sentinel_type_index()))
                }
            }
        }

        fn find_concrete_type(
            &self,
            _ty: ConcreteType,
        ) -> Result<TypeDescription, TypeResolutionError> {
            Ok(TypeDescription::new(
                ResolutionS::NULL,
                crate::sentinel_type_index(),
            ))
        }
    }

    fn method_user_type(definition: TypeIndex) -> MethodType {
        MethodType::Base(Box::new(BaseType::Type {
            value_kind: None,
            source: TypeSource::User(UserType::Definition(definition)),
        }))
    }

    fn concrete_user_type(resolution: ResolutionS, definition: TypeIndex) -> ConcreteType {
        ConcreteType::new(
            resolution,
            BaseType::Type {
                value_kind: None,
                source: TypeSource::User(UserType::Definition(definition)),
            },
        )
    }

    fn method_signature(
        return_type: Option<ParameterType<MethodType>>,
        parameters: Vec<ParameterType<MethodType>>,
    ) -> ManagedMethod<MethodType> {
        ManagedMethod {
            instance: true,
            explicit_this: false,
            calling_convention: CallingConvention::Default,
            parameters: parameters
                .into_iter()
                .map(|parameter| Parameter(vec![], parameter))
                .collect(),
            return_type: ReturnType(vec![], return_type),
            varargs: None,
        }
    }

    fn primitive(ty: BaseType<MethodType>) -> MethodType {
        MethodType::Base(Box::new(ty))
    }

    #[test]
    fn test_types_equal_primitive() {
        let resolver = MockResolver;
        let comparer = TypeComparer::new(&resolver);
        let res = ResolutionS::NULL;

        let t1 = MethodType::Base(Box::new(BaseType::Int32));
        let t2 = MethodType::Base(Box::new(BaseType::Int32));
        let t3 = MethodType::Base(Box::new(BaseType::Int64));

        assert!(comparer.types_equal(&res, &t1, None, &res, &t2, None));
        assert!(!comparer.types_equal(&res, &t1, None, &res, &t3, None));
    }

    #[test]
    fn test_param_types_equal() {
        let resolver = MockResolver;
        let comparer = TypeComparer::new(&resolver);
        let res = ResolutionS::NULL;

        let t1 = ParameterType::Value(MethodType::Base(Box::new(BaseType::Int32)));
        let t2 = ParameterType::Value(MethodType::Base(Box::new(BaseType::Int32)));
        let t3 = ParameterType::Ref(MethodType::Base(Box::new(BaseType::Int32)));

        assert!(comparer.param_types_equal(&res, &t1, None, &res, &t2, None));
        assert!(!comparer.param_types_equal(&res, &t1, None, &res, &t3, None));
    }

    #[test]
    fn signatures_variance_keeps_return_covariance_and_parameter_contravariance() {
        let resolver = MockResolver;
        let comparer = TypeComparer::new(&resolver);
        let res = ResolutionS::NULL;

        let invariant_target = method_signature(
            Some(ParameterType::Value(primitive(BaseType::Object))),
            vec![ParameterType::Value(primitive(BaseType::String))],
        );
        let variant_candidate = method_signature(
            Some(ParameterType::Value(primitive(BaseType::String))),
            vec![ParameterType::Value(primitive(BaseType::Object))],
        );

        assert!(comparer.signatures_compatible_with_variance(
            &res,
            &invariant_target,
            None,
            &res,
            &variant_candidate,
            None,
        ));
        assert!(!comparer.signatures_equal(
            &res,
            &invariant_target,
            None,
            &res,
            &variant_candidate,
            None,
        ));
    }

    #[test]
    fn signatures_variance_rejects_typed_reference_return_type() {
        let resolver = MockResolver;
        let comparer = TypeComparer::new(&resolver);
        let res = ResolutionS::NULL;

        let left = method_signature(Some(ParameterType::TypedReference), vec![]);
        let right = method_signature(Some(ParameterType::TypedReference), vec![]);

        assert!(
            !comparer.signatures_compatible_with_variance(&res, &left, None, &res, &right, None,)
        );
        assert!(comparer.signatures_equal(&res, &left, None, &res, &right, None));
    }

    #[test]
    fn signatures_variance_keeps_byref_parameters_exact() {
        let resolver = MockResolver;
        let comparer = TypeComparer::new(&resolver);
        let res = ResolutionS::NULL;

        let left = method_signature(
            Some(ParameterType::Value(primitive(BaseType::Object))),
            vec![ParameterType::Ref(primitive(BaseType::Object))],
        );
        let right = method_signature(
            Some(ParameterType::Value(primitive(BaseType::String))),
            vec![ParameterType::Ref(primitive(BaseType::String))],
        );

        assert!(
            !comparer.signatures_compatible_with_variance(&res, &left, None, &res, &right, None,)
        );
    }

    #[test]
    fn types_equal_preserves_same_placeholder_identity_before_substitution() {
        let resolver = MockResolver;
        let comparer = TypeComparer::new(&resolver);
        let res = ResolutionS::NULL;

        let lookup_left = GenericLookup::new(vec![ConcreteType::new(res.clone(), BaseType::Int32)]);
        let lookup_right =
            GenericLookup::new(vec![ConcreteType::new(res.clone(), BaseType::Int64)]);

        assert!(comparer.types_equal(
            &res,
            &MethodType::TypeGeneric(0),
            Some(&lookup_left),
            &res,
            &MethodType::TypeGeneric(0),
            Some(&lookup_right),
        ));
    }

    #[test]
    fn types_equal_returns_false_on_resolution_failure_after_generic_substitution() {
        let failing_definition = crate::type_index_from_usize(42);
        let resolver = SelectiveFailResolver::for_definitions(vec![failing_definition]);
        let comparer = TypeComparer::new(&resolver);
        let res = ResolutionS::NULL;

        let concrete = concrete_user_type(res.clone(), failing_definition);
        let lookup = GenericLookup::new(vec![concrete]);
        let target = method_user_type(failing_definition);

        assert!(
            !comparer.types_equal(
                &res,
                &MethodType::TypeGeneric(0),
                Some(&lookup),
                &res,
                &target,
                None,
            ),
            "types_equal must return false (not panic) when substitution hits locate_type failure"
        );
    }

    #[test]
    fn concrete_equals_method_type_panics_on_resolution_failure() {
        let failing_definition = crate::type_index_from_usize(7);
        let resolver = SelectiveFailResolver::for_definitions(vec![failing_definition]);
        let comparer = TypeComparer::new(&resolver);
        let res = ResolutionS::NULL;

        let concrete = concrete_user_type(res.clone(), failing_definition);
        let target = method_user_type(failing_definition);

        let panic_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            comparer.concrete_equals_method_type(&concrete, &res, &target, None)
        }));

        assert!(
            panic_result.is_err(),
            "concrete_equals_method_type should preserve panic-on-resolution-failure behavior"
        );
    }

    #[test]
    fn concrete_types_equal_keeps_equal_error_result_behavior() {
        let resolver = SelectiveFailResolver::fail_all();
        let comparer = TypeComparer::new(&resolver);
        let res = ResolutionS::NULL;

        let c1 = concrete_user_type(res.clone(), crate::type_index_from_usize(1));
        let c2 = concrete_user_type(res.clone(), crate::type_index_from_usize(2));

        assert_ne!(c1, c2);
        assert!(
            comparer.concrete_types_equal(&c1, &c2),
            "legacy CompareResultsThenReturnFalse mode treats identical locate_type errors as equal"
        );
    }

    // Regression test for property getter method lookup.
    //
    // `find_method_in_type` must return `Some(MethodDescription)` when the target
    // method is a property getter that lives in `def.properties[i].getter`, even
    // though the getter is NOT present in `TypeDefinition.methods`.
    //
    // This previously panicked when the lookup path only scanned
    // `def.methods` and missed accessor-backed methods.
    #[test]
    fn property_getter_lookup_panics_on_ptr_eq_unwrap() {
        use crate::TypeDescription;

        // Build an instance void() getter method named "get_Value".
        let getter_method: Method<'static> = Method {
            name: "get_Value".into(),
            signature: ManagedMethod {
                instance: true,
                explicit_this: false,
                calling_convention: CallingConvention::Default,
                parameters: vec![],
                return_type: ReturnType(vec![], None),
                varargs: None,
            },
            attributes: vec![],
            body: None,
            accessibility: MemberAccessibility::Access(Accessibility::Public),
            generic_parameters: vec![],
            return_type_metadata: None,
            parameter_metadata: vec![],
            sealed: false,
            virtual_member: false,
            hide_by_sig: true,
            vtable_layout: VtableLayout::ReuseSlot,
            strict: false,
            abstract_member: false,
            special_name: true,
            runtime_special_name: false,
            pinvoke: None,
            security: None,
            require_sec_object: false,
            body_format: BodyFormat::IL,
            body_management: BodyManagement::Managed,
            forward_ref: false,
            preserve_sig: false,
            internal_call: false,
            synchronized: false,
            no_inlining: false,
            no_optimization: false,
        };

        // TypeDefinition "MyClass": property "Value" with a getter, but methods is EMPTY.
        // This is the key pre-condition: accessor methods are NOT in def.methods.
        let mut type_def = TypeDefinition::new(None, "MyClass");
        type_def.properties = vec![Property {
            attributes: vec![],
            name: "Value".into(),
            getter: Some(getter_method),
            setter: None,
            other: vec![],
            static_member: false,
            property_type: Parameter(
                vec![],
                ParameterType::Value(MemberType::Base(Box::new(BaseType::Int32))),
            ),
            parameters: vec![],
            special_name: false,
            runtime_special_name: false,
            default: None,
        }];
        assert!(
            type_def.methods.is_empty(),
            "pre-condition: getter must not be in def.methods"
        );

        // Resolution::new inserts <Module> at index 0; our type is at index 1.
        let mut resolution = Resolution::new(Module::new("test.dll"));
        resolution.type_definitions.push(type_def);

        // SAFETY: pointer comes from Box::into_raw; MetadataArena::drop calls
        // Box::from_raw on the same pointer, so ownership is correctly transferred.
        let ptr = Box::into_raw(Box::new(resolution)) as *const Resolution<'static>;
        let arena = Arc::new(MetadataArena::new());
        unsafe { arena.add_resolution(ptr) };
        let res_s = ResolutionS::new(ptr, arena);

        // TypeIndex 1 = "MyClass" (the <Module> sentinel is at index 0).
        let type_index: TypeIndex = crate::type_index_from_usize(1usize);
        let type_desc = TypeDescription::new(res_s.clone(), type_index);

        // Search signature: instance void() — matches the getter exactly.
        let search_sig: ManagedMethod<MethodType> = ManagedMethod {
            instance: true,
            explicit_this: false,
            calling_convention: CallingConvention::Default,
            parameters: vec![],
            return_type: ReturnType(vec![], None),
            varargs: None,
        };

        let resolver = MockResolver;
        let comparer = TypeComparer::new(&resolver);

        // Accessor methods may live outside `def.methods`; lookup must still return Some(_).
        let result = comparer.find_method_in_type(type_desc, "get_Value", &search_sig, res_s);
        assert!(
            result.is_some(),
            "find_method_in_type must return Some for a property getter"
        );
    }

    #[test]
    fn types_equal_matches_same_name_across_resolutions() {
        fn build_resolution_with_type(name: &'static str) -> (ResolutionS, TypeIndex) {
            let mut resolution = Resolution::new(Module::new("test.dll"));
            let type_index =
                resolution.push_type_definition(TypeDefinition::new(Some("System".into()), name));

            let ptr = Box::into_raw(Box::new(resolution)) as *const Resolution<'static>;
            let arena = Arc::new(MetadataArena::new());
            unsafe { arena.add_resolution(ptr) };
            (ResolutionS::new(ptr, arena), type_index)
        }

        let (res_left, type_left) = build_resolution_with_type("ReadOnlySpan`1");
        let (res_right, type_right) = build_resolution_with_type("ReadOnlySpan`1");

        let left = MethodType::Base(Box::new(BaseType::Type {
            value_kind: Some(ValueKind::ValueType),
            source: TypeSource::User(UserType::Definition(type_left)),
        }));
        let right = MethodType::Base(Box::new(BaseType::Type {
            value_kind: Some(ValueKind::ValueType),
            source: TypeSource::User(UserType::Definition(type_right)),
        }));

        let resolver = MockResolver;
        let comparer = TypeComparer::new(&resolver);
        assert!(
            comparer.types_equal(&res_left, &left, None, &res_right, &right, None),
            "matching full names across different resolutions should compare equal"
        );
    }
}

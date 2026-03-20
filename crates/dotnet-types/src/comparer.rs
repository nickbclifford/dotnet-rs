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
        res1: ResolutionS,
        a: &[MethodType],
        generics1: Option<&GenericLookup>,
        res2: ResolutionS,
        b: &[MethodType],
        generics2: Option<&GenericLookup>,
    ) -> bool {
        if a.len() != b.len() {
            return false;
        }
        for (a, b) in a.iter().zip(b.iter()) {
            if !self.types_equal(res1.clone(), a, generics1, res2.clone(), b, generics2) {
                return false;
            }
        }
        true
    }

    pub fn types_equal(
        &self,
        res1: ResolutionS,
        a: &MethodType,
        generics1: Option<&GenericLookup>,
        res2: ResolutionS,
        b: &MethodType,
        generics2: Option<&GenericLookup>,
    ) -> bool {
        if let Some(concrete) = Self::resolve_generic_type(generics1, a) {
            return self.concrete_equals_method_type(concrete, res2, b, generics2);
        }

        match (a, b) {
            (MethodType::Base(l), MethodType::Base(r)) => match (l.as_ref(), r.as_ref()) {
                (BaseType::Type { source: ts1, .. }, BaseType::Type { source: ts2, .. }) => {
                    let (ut1, generics1_list) = decompose_type_source(ts1);
                    let (ut2, generics2_list) = decompose_type_source(ts2);
                    let td1 = self.loader.locate_type(res1.clone(), ut1);
                    let td2 = self.loader.locate_type(res2.clone(), ut2);
                    let same_type = match (td1, td2) {
                        (Ok(left), Ok(right)) => {
                            let left_type_name = left.type_name();
                            let right_type_name = right.type_name();
                            left == right
                                || (!left.is_null()
                                    && !right.is_null()
                                    && self.loader.canonical_type_name(&left_type_name)
                                        == self.loader.canonical_type_name(&right_type_name))
                        }
                        _ => false,
                    };

                    same_type
                        && self.type_slices_equal(
                            res1.clone(),
                            &generics1_list,
                            generics1,
                            res2.clone(),
                            &generics2_list,
                            generics2,
                        )
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
                (BaseType::Vector(_, l), BaseType::Vector(_, r)) => {
                    self.types_equal(res1, l, generics1, res2, r, generics2)
                }
                (BaseType::Array(l, _), BaseType::Array(r, _)) => {
                    self.types_equal(res1, l, generics1, res2, r, generics2)
                }
                (BaseType::ValuePointer(_, l), BaseType::ValuePointer(_, r)) => {
                    match (l.as_ref(), r.as_ref()) {
                        (None, None) => true,
                        (Some(t1), Some(t2)) => {
                            self.types_equal(res1, t1, generics1, res2, t2, generics2)
                        }
                        _ => false,
                    }
                }
                (BaseType::FunctionPointer(_l), BaseType::FunctionPointer(_r)) => todo!(),
                _ => false,
            },
            (MethodType::TypeGeneric(i1), MethodType::TypeGeneric(i2)) => {
                if i1 == i2 {
                    return true;
                }
                if let (Some(c1), Some(c2)) = (
                    generics1.and_then(|g| g.type_generics.get(*i1)),
                    generics2.and_then(|g| g.type_generics.get(*i2)),
                ) {
                    return c1 == c2;
                }
                false
            }
            (MethodType::MethodGeneric(i1), MethodType::MethodGeneric(i2)) => {
                if i1 == i2 {
                    return true;
                }
                if let (Some(c1), Some(c2)) = (
                    generics1.and_then(|g| g.method_generics.get(*i1)),
                    generics2.and_then(|g| g.method_generics.get(*i2)),
                ) {
                    return c1 == c2;
                }
                false
            }
            _ => {
                if let Some(concrete) = Self::resolve_generic_type(generics2, b) {
                    return self.concrete_equals_method_type(concrete, res1, a, generics1);
                }
                false
            }
        }
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

    pub fn param_types_equal(
        &self,
        res1: ResolutionS,
        a: &ParameterType<MethodType>,
        generics1: Option<&GenericLookup>,
        res2: ResolutionS,
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
        res1: ResolutionS,
        a: &[Parameter<MethodType>],
        generics1: Option<&GenericLookup>,
        res2: ResolutionS,
        b: &[Parameter<MethodType>],
        generics2: Option<&GenericLookup>,
    ) -> bool {
        if a.len() != b.len() {
            return false;
        }
        for (Parameter(_, a), Parameter(_, b)) in a.iter().zip(b.iter()) {
            if !self.param_types_equal(res1.clone(), a, generics1, res2.clone(), b, generics2) {
                return false;
            }
        }
        true
    }

    pub fn signatures_compatible_with_variance(
        &self,
        res1: ResolutionS,
        a: &ManagedMethod<MethodType>,
        generics1: Option<&GenericLookup>,
        res2: ResolutionS,
        b: &ManagedMethod<MethodType>,
        generics2: Option<&GenericLookup>,
    ) -> bool {
        if a.instance != b.instance {
            return false;
        }
        if a.parameters.len() != b.parameters.len() {
            return false;
        }

        let lookup1 = generics1.cloned().unwrap_or_default();
        let lookup2 = generics2.cloned().unwrap_or_default();

        // Return type: Covariant (b's return type must be assignable to a's)
        match (&a.return_type, &b.return_type) {
            (ReturnType(_, None), ReturnType(_, None)) => {}
            (ReturnType(_, Some(l_p)), ReturnType(_, Some(r_p))) => {
                let l_m = match l_p {
                    ParameterType::Value(v) | ParameterType::Ref(v) => v,
                    ParameterType::TypedReference => return false, // Not variant
                };
                let r_m = match r_p {
                    ParameterType::Value(v) | ParameterType::Ref(v) => v,
                    ParameterType::TypedReference => return false, // Not variant
                };
                let Ok(l_concrete) = lookup1.make_concrete(res1.clone(), l_m.clone(), self.loader)
                else {
                    return false;
                };
                let Ok(r_concrete) = lookup2.make_concrete(res2.clone(), r_m.clone(), self.loader)
                else {
                    return false;
                };
                if !self.is_assignable_to(&r_concrete, &l_concrete) {
                    return false;
                }
            }
            _ => return false,
        }

        // Parameters: Contravariant (a's parameters must be assignable to b's)
        for (Parameter(_, a_p), Parameter(_, b_p)) in a.parameters.iter().zip(b.parameters.iter()) {
            match (a_p, b_p) {
                (ParameterType::Value(a_v), ParameterType::Value(b_v)) => {
                    let Ok(a_concrete) =
                        lookup1.make_concrete(res1.clone(), a_v.clone(), self.loader)
                    else {
                        return false;
                    };
                    let Ok(b_concrete) =
                        lookup2.make_concrete(res2.clone(), b_v.clone(), self.loader)
                    else {
                        return false;
                    };
                    if !self.is_assignable_to(&a_concrete, &b_concrete) {
                        return false;
                    }
                }
                _ => {
                    // Non-value parameters (ByRef, Pinned) must match exactly
                    if !self.param_types_equal(
                        res1.clone(),
                        a_p,
                        generics1,
                        res2.clone(),
                        b_p,
                        generics2,
                    ) {
                        return false;
                    }
                }
            }
        }
        true
    }

    pub fn signatures_equal(
        &self,
        res1: ResolutionS,
        a: &ManagedMethod<MethodType>,
        generics1: Option<&GenericLookup>,
        res2: ResolutionS,
        b: &ManagedMethod<MethodType>,
        generics2: Option<&GenericLookup>,
    ) -> bool {
        if a.instance != b.instance {
            return false;
        }
        match (&a.return_type, &b.return_type) {
            (ReturnType(_, None), ReturnType(_, None)) => self.params_equal(
                res1,
                &a.parameters,
                generics1,
                res2,
                &b.parameters,
                generics2,
            ),
            (ReturnType(_, Some(l)), ReturnType(_, Some(r))) => {
                if self.param_types_equal(res1.clone(), l, generics1, res2.clone(), r, generics2) {
                    self.params_equal(
                        res1,
                        &a.parameters,
                        generics1,
                        res2,
                        &b.parameters,
                        generics2,
                    )
                } else {
                    false
                }
            }
            _ => false,
        }
    }

    pub fn concrete_equals_method_type(
        &self,
        concrete: &ConcreteType,
        res2: ResolutionS,
        b: &MethodType,
        generics2: Option<&GenericLookup>,
    ) -> bool {
        if let Some(generics) = generics2 {
            match b {
                MethodType::TypeGeneric(idx) => {
                    if let Some(other) = generics.type_generics.get(*idx) {
                        return concrete == other;
                    }
                }
                MethodType::MethodGeneric(idx) => {
                    if let Some(other) = generics.method_generics.get(*idx) {
                        return concrete == other;
                    }
                }
                _ => {}
            }
        }
        match b {
            MethodType::Base(b_base) => {
                self.concrete_equals_base_type(concrete, res2, b_base, generics2)
            }
            _ => false,
        }
    }

    pub fn concrete_equals_base_type(
        &self,
        concrete: &ConcreteType,
        res2: ResolutionS,
        b: &BaseType<MethodType>,
        generics2: Option<&GenericLookup>,
    ) -> bool {
        let res1 = concrete.resolution();
        let a = concrete.get();

        match (a, b) {
            (BaseType::Type { source: ts1, .. }, BaseType::Type { source: ts2, .. }) => {
                let (ut1, generics1) = decompose_type_source(ts1);
                let (ut2, generics2_list) = decompose_type_source(ts2);
                let td1 = self
                    .loader
                    .locate_type(res1, ut1)
                    .expect("Type resolution failed during comparison");
                let td2 = self
                    .loader
                    .locate_type(res2.clone(), ut2)
                    .expect("Type resolution failed during comparison");

                if td1 != td2 {
                    let d1 = td1.definition();
                    let d2 = td2.definition();
                    if d1.name != d2.name || d1.namespace != d2.namespace {
                        return false;
                    }
                }

                if generics1.len() != generics2_list.len() {
                    return false;
                }

                for (g1, g2) in generics1.iter().zip(generics2_list.iter()) {
                    if !self.concrete_equals_method_type(g1, res2.clone(), g2, generics2) {
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
            (BaseType::Vector(_, l), BaseType::Vector(_, r)) => {
                self.concrete_equals_method_type(l, res2, r, generics2)
            }
            (BaseType::Array(l, _), BaseType::Array(r, _)) => {
                self.concrete_equals_method_type(l, res2, r, generics2)
            }
            (BaseType::ValuePointer(_, l), BaseType::ValuePointer(_, r)) => {
                match (l.as_ref(), r.as_ref()) {
                    (None, None) => true,
                    (Some(l_concrete), Some(r_method)) => {
                        self.concrete_equals_method_type(l_concrete, res2, r_method, generics2)
                    }
                    _ => false,
                }
            }
            (BaseType::FunctionPointer(_l), BaseType::FunctionPointer(_r)) => todo!(),
            _ => false,
        }
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

    fn is_system_value_type(&self, ty: &ConcreteType) -> bool {
        let BaseType::Type { source, .. } = ty.get() else {
            return false;
        };
        let (ut, _) = decompose_type_source(source);
        let Ok(td) = self.loader.locate_type(ty.resolution(), ut) else {
            return false;
        };
        let td_name = td.type_name();
        self.loader.canonical_type_name(&td_name) == "System.ValueType"
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

        macro_rules! check {
            ($method:expr, $member_index:expr) => {
                let m = $method;
                if m.name == name
                    && m.signature.parameters.len() == signature.parameters.len()
                    && (if allow_variance {
                        self.signatures_compatible_with_variance(
                            sig_res.clone(),
                            signature,
                            sig_generics,
                            desc.resolution.clone(),
                            &m.signature,
                            type_generics,
                        )
                    } else {
                        self.signatures_equal(
                            sig_res.clone(),
                            signature,
                            sig_generics,
                            desc.resolution.clone(),
                            &m.signature,
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
                        dotnetdll::resolved::generic::Variance::Invariant => {
                            if !self.concrete_types_equal(g1, g2) {
                                return false;
                            }
                        }
                        dotnetdll::resolved::generic::Variance::Covariant => {
                            // Covariant: source must be assignable to target
                            if !self.is_assignable_to(g1, g2) {
                                return false;
                            }
                        }
                        dotnetdll::resolved::generic::Variance::Contravariant => {
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
        if c1 == c2 {
            return true;
        }

        let res1 = c1.resolution();
        let a = c1.get();
        let res2 = c2.resolution();
        let b = c2.get();

        match (a, b) {
            (BaseType::Type { source: ts1, .. }, BaseType::Type { source: ts2, .. }) => {
                let (ut1, generics1) = decompose_type_source(ts1);
                let (ut2, generics2) = decompose_type_source(ts2);
                let td1 = self.loader.locate_type(res1, ut1);
                let td2 = self.loader.locate_type(res2, ut2);

                if td1 != td2 {
                    let td1 = match td1 {
                        Ok(t) => t,
                        Err(_) => return false,
                    };
                    let td2 = match td2 {
                        Ok(t) => t,
                        Err(_) => return false,
                    };
                    let d1 = td1.definition();
                    let d2 = td2.definition();
                    if d1.name != d2.name || d1.namespace != d2.namespace {
                        return false;
                    }
                }

                if generics1.len() != generics2.len() {
                    return false;
                }

                for (g1, g2) in generics1.iter().zip(generics2.iter()) {
                    if !self.concrete_types_equal(g1, g2) {
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
            (BaseType::Vector(_, l), BaseType::Vector(_, r)) => self.concrete_types_equal(l, r),
            (BaseType::Array(l, _), BaseType::Array(r, _)) => self.concrete_types_equal(l, r),
            (BaseType::ValuePointer(_, l), BaseType::ValuePointer(_, r)) => {
                match (l.as_ref(), r.as_ref()) {
                    (None, None) => true,
                    (Some(l_concrete), Some(r_concrete)) => {
                        self.concrete_types_equal(l_concrete, r_concrete)
                    }
                    _ => false,
                }
            }
            (BaseType::FunctionPointer(_l), BaseType::FunctionPointer(_r)) => todo!(),
            _ => false,
        }
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

    #[test]
    fn test_types_equal_primitive() {
        let resolver = MockResolver;
        let comparer = TypeComparer::new(&resolver);
        let res = ResolutionS::NULL;

        let t1 = MethodType::Base(Box::new(BaseType::Int32));
        let t2 = MethodType::Base(Box::new(BaseType::Int32));
        let t3 = MethodType::Base(Box::new(BaseType::Int64));

        assert!(comparer.types_equal(res.clone(), &t1, None, res.clone(), &t2, None));
        assert!(!comparer.types_equal(res.clone(), &t1, None, res.clone(), &t3, None));
    }

    #[test]
    fn test_param_types_equal() {
        let resolver = MockResolver;
        let comparer = TypeComparer::new(&resolver);
        let res = ResolutionS::NULL;

        let t1 = ParameterType::Value(MethodType::Base(Box::new(BaseType::Int32)));
        let t2 = ParameterType::Value(MethodType::Base(Box::new(BaseType::Int32)));
        let t3 = ParameterType::Ref(MethodType::Base(Box::new(BaseType::Int32)));

        assert!(comparer.param_types_equal(res.clone(), &t1, None, res.clone(), &t2, None));
        assert!(!comparer.param_types_equal(res.clone(), &t1, None, res.clone(), &t3, None));
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
            value_kind: Some(dotnetdll::resolved::types::ValueKind::ValueType),
            source: TypeSource::User(UserType::Definition(type_left)),
        }));
        let right = MethodType::Base(Box::new(BaseType::Type {
            value_kind: Some(dotnetdll::resolved::types::ValueKind::ValueType),
            source: TypeSource::User(UserType::Definition(type_right)),
        }));

        let resolver = MockResolver;
        let comparer = TypeComparer::new(&resolver);
        assert!(
            comparer.types_equal(res_left, &left, None, res_right, &right, None),
            "matching full names across different resolutions should compare equal"
        );
    }
}

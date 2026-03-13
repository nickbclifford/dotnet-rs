use crate::{
    TypeDescription, TypeResolver,
    generics::{ConcreteType, GenericLookup, member_to_method_type},
    members::MethodDescription,
    resolution::ResolutionS,
};
use dotnetdll::prelude::*;
use std::collections::{HashSet, VecDeque};

pub struct TypeComparer<'a, R: TypeResolver> {
    loader: &'a R,
}

impl<'a, R: TypeResolver> TypeComparer<'a, R> {
    pub fn new(loader: &'a R) -> Self {
        Self { loader }
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
            if !self.types_equal(res1, a, generics1, res2, b, generics2) {
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
                    let td1 = self.loader.locate_type(res1, ut1);
                    let td2 = self.loader.locate_type(res2, ut2);
                    td1 == td2
                        && self.type_slices_equal(
                            res1,
                            &generics1_list,
                            generics1,
                            res2,
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
            if !self.param_types_equal(res1, a, generics1, res2, b, generics2) {
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
                let Ok(l_concrete) = lookup1.make_concrete(res1, l_m.clone(), self.loader)
                else {
                    return false;
                };
                let Ok(r_concrete) = lookup2.make_concrete(res2, r_m.clone(), self.loader)
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
                    let Ok(a_concrete) = lookup1.make_concrete(res1, a_v.clone(), self.loader) else {
                        return false;
                    };
                    let Ok(b_concrete) = lookup2.make_concrete(res2, b_v.clone(), self.loader) else {
                        return false;
                    };
                    if !self.is_assignable_to(&a_concrete, &b_concrete) {
                        return false;
                    }
                }
                _ => {
                    // Non-value parameters (ByRef, Pinned) must match exactly
                    if !self.param_types_equal(res1, a_p, generics1, res2, b_p, generics2) {
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
                if self.param_types_equal(res1, l, generics1, res2, r, generics2) {
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
                    .locate_type(res2, ut2)
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
                    if !self.concrete_equals_method_type(g1, res2, g2, generics2) {
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
        if self.concrete_types_equal(source, target) {
            return true;
        }

        if self.compatible_with_variance(source, target) {
            return true;
        }

        if matches!(target.get(), BaseType::Object) {
            return true;
        }

        if target.is_interface(self.loader) {
            return self.implements_interface(source, target);
        }

        if target.is_class(self.loader) {
            return self.is_subclass_of(source, target);
        }

        false
    }

    pub fn is_subclass_of(&self, source: &ConcreteType, target: &ConcreteType) -> bool {
        let mut curr = source.clone();
        while let Ok(td) = self.loader.find_concrete_type(curr.clone()) {
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
                        td.resolution,
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
                        td.resolution,
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
            ($method:expr) => {
                let m = $method;
                if m.name == name
                    && m.signature.parameters.len() == signature.parameters.len()
                    && (if allow_variance {
                        self.signatures_compatible_with_variance(
                            sig_res,
                            signature,
                            sig_generics,
                            desc.resolution,
                            &m.signature,
                            type_generics,
                        )
                    } else {
                        self.signatures_equal(
                            sig_res,
                            signature,
                            sig_generics,
                            desc.resolution,
                            &m.signature,
                            type_generics,
                        )
                    })
                {
                    return Some(MethodDescription::new(
                        desc,
                        type_generics.cloned().unwrap_or_default(),
                        desc.resolution,
                        m,
                    ));
                }
            };
        }

        for method in def.methods.iter().rev() {
            check!(method);
        }

        if let Some(prop_name) = name.strip_prefix("get_") {
            for prop in &def.properties {
                if let (true, Some(getter)) = (prop.name == prop_name, &prop.getter) {
                    check!(getter);
                }
            }
        } else if let Some(prop_name) = name.strip_prefix("set_") {
            for prop in &def.properties {
                if let (true, Some(setter)) = (prop.name == prop_name, &prop.setter) {
                    check!(setter);
                }
            }
        }

        if let Some(event_name) = name.strip_prefix("add_") {
            for event in &def.events {
                if event.name == event_name {
                    check!(&event.add_listener);
                }
            }
        } else if let Some(event_name) = name.strip_prefix("remove_") {
            for event in &def.events {
                if event.name == event_name {
                    check!(&event.remove_listener);
                }
            }
        } else if let Some(event_name) = name.strip_prefix("raise_") {
            for event in &def.events {
                if let (true, Some(raise_event)) = (event.name == event_name, &event.raise_event) {
                    check!(raise_event);
                }
            }
        }

        for prop in &def.properties {
            for method in &prop.other {
                check!(method);
            }
            if let Some(m) = &prop.getter {
                check!(m);
            }
            if let Some(m) = &prop.setter {
                check!(m);
            }
        }

        for event in &def.events {
            for method in &event.other {
                check!(method);
            }
            check!(&event.add_listener);
            check!(&event.remove_listener);
            if let Some(m) = &event.raise_event {
                check!(m);
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
    use crate::resolution::ResolutionS;

    struct MockResolver;
    impl TypeResolver for MockResolver {
        fn corlib_type(&self, _name: &str) -> Result<TypeDescription, TypeResolutionError> {
            Ok(TypeDescription::from_raw(
                ResolutionS::new(std::ptr::null()),
                None,
                unsafe { std::mem::transmute::<usize, TypeIndex>(0usize) },
            ))
        }
        fn locate_type(
            &self,
            res: ResolutionS,
            _handle: UserType,
        ) -> Result<TypeDescription, TypeResolutionError> {
            Ok(TypeDescription::from_raw(res, None, unsafe {
                std::mem::transmute::<usize, TypeIndex>(0usize)
            }))
        }
        fn find_concrete_type(
            &self,
            _ty: ConcreteType,
        ) -> Result<TypeDescription, TypeResolutionError> {
            Ok(TypeDescription::from_raw(
                ResolutionS::new(std::ptr::null()),
                None,
                unsafe { std::mem::transmute::<usize, TypeIndex>(0usize) },
            ))
        }
    }

    #[test]
    fn test_types_equal_primitive() {
        let resolver = MockResolver;
        let comparer = TypeComparer::new(&resolver);
        let res = ResolutionS::new(std::ptr::null());

        let t1 = MethodType::Base(Box::new(BaseType::Int32));
        let t2 = MethodType::Base(Box::new(BaseType::Int32));
        let t3 = MethodType::Base(Box::new(BaseType::Int64));

        assert!(comparer.types_equal(res, &t1, None, res, &t2, None));
        assert!(!comparer.types_equal(res, &t1, None, res, &t3, None));
    }

    #[test]
    fn test_param_types_equal() {
        let resolver = MockResolver;
        let comparer = TypeComparer::new(&resolver);
        let res = ResolutionS::new(std::ptr::null());

        let t1 = ParameterType::Value(MethodType::Base(Box::new(BaseType::Int32)));
        let t2 = ParameterType::Value(MethodType::Base(Box::new(BaseType::Int32)));
        let t3 = ParameterType::Ref(MethodType::Base(Box::new(BaseType::Int32)));

        assert!(comparer.param_types_equal(res, &t1, None, res, &t2, None));
        assert!(!comparer.param_types_equal(res, &t1, None, res, &t3, None));
    }
}

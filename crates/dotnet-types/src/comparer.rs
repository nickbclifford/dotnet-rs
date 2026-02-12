use crate::{
    TypeDescription, TypeResolver,
    generics::{ConcreteType, GenericLookup},
    members::MethodDescription,
    resolution::ResolutionS,
};
use dotnetdll::prelude::*;

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
        if let Some(generics) = generics1 {
            match a {
                MethodType::TypeGeneric(idx) => {
                    if let Some(concrete) = generics.type_generics.get(*idx) {
                        return self.concrete_equals_method_type(concrete, res2, b, generics2);
                    }
                }
                MethodType::MethodGeneric(idx) => {
                    if let Some(concrete) = generics.method_generics.get(*idx) {
                        return self.concrete_equals_method_type(concrete, res2, b, generics2);
                    }
                }
                _ => {}
            }
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
                if let Some(generics) = generics2 {
                    match b {
                        MethodType::TypeGeneric(idx) => {
                            if let Some(concrete) = generics.type_generics.get(*idx) {
                                return self
                                    .concrete_equals_method_type(concrete, res1, a, generics1);
                            }
                        }
                        MethodType::MethodGeneric(idx) => {
                            if let Some(concrete) = generics.method_generics.get(*idx) {
                                return self
                                    .concrete_equals_method_type(concrete, res1, a, generics1);
                            }
                        }
                        _ => {}
                    }
                }
                false
            }
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

    pub fn find_method_in_type_with_substitution(
        &self,
        desc: TypeDescription,
        name: &str,
        signature: &ManagedMethod<MethodType>,
        sig_res: ResolutionS,
        generics: &GenericLookup,
    ) -> Option<MethodDescription> {
        self.find_method_in_type_internal(
            desc,
            name,
            signature,
            sig_res,
            Some(generics),
            Some(generics),
        )
    }

    pub fn find_method_in_type(
        &self,
        desc: TypeDescription,
        name: &str,
        signature: &ManagedMethod<MethodType>,
        sig_res: ResolutionS,
    ) -> Option<MethodDescription> {
        self.find_method_in_type_internal(desc, name, signature, sig_res, None, None)
    }

    pub fn find_method_in_type_internal(
        &self,
        desc: TypeDescription,
        name: &str,
        signature: &ManagedMethod<MethodType>,
        sig_res: ResolutionS,
        sig_generics: Option<&GenericLookup>,
        type_generics: Option<&GenericLookup>,
    ) -> Option<MethodDescription> {
        let def = desc.definition();

        macro_rules! check {
            ($method:expr) => {
                let m = $method;
                if m.name == name
                    && m.signature.parameters.len() == signature.parameters.len()
                    && self.signatures_equal(
                        sig_res,
                        signature,
                        sig_generics,
                        desc.resolution,
                        &m.signature,
                        type_generics,
                    )
                {
                    return Some(MethodDescription {
                        parent: desc,
                        method_resolution: desc.resolution,
                        method: m,
                    });
                }
            };
        }

        for method in &def.methods {
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
                unsafe { std::mem::transmute::<usize, dotnetdll::resolution::TypeIndex>(0usize) },
            ))
        }
        fn locate_type(
            &self,
            res: ResolutionS,
            _handle: UserType,
        ) -> Result<TypeDescription, TypeResolutionError> {
            Ok(TypeDescription::from_raw(res, None, unsafe {
                std::mem::transmute::<usize, dotnetdll::resolution::TypeIndex>(0usize)
            }))
        }
        fn find_concrete_type(
            &self,
            _ty: crate::generics::ConcreteType,
        ) -> Result<TypeDescription, TypeResolutionError> {
            Ok(TypeDescription::from_raw(
                ResolutionS::new(std::ptr::null()),
                None,
                unsafe { std::mem::transmute::<usize, dotnetdll::resolution::TypeIndex>(0usize) },
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

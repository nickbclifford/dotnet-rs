use crate::{
    assemblies::{Ancestor, AssemblyLoader},
    types::{
        generics::{ConcreteType, GenericLookup},
        members::{FieldDescription, MethodDescription},
        TypeDescription,
    },
    utils::{decompose_type_source, ResolutionS},
    value::object::ObjectHandle,
};
use dotnetdll::prelude::{
    BaseType, FieldSource, MemberType, MethodType, TypeSource, UserMethod, UserType, ValueKind,
};
use std::{
    collections::{HashSet, VecDeque},
    sync::Arc,
};

#[derive(Clone)]
pub struct ResolutionContext<'a, 'm> {
    pub generics: &'a GenericLookup,
    pub loader: &'m AssemblyLoader,
    pub resolution: ResolutionS,
    pub type_owner: Option<TypeDescription>,
    pub method_owner: Option<MethodDescription>,
    pub shared: Arc<crate::vm::stack::SharedGlobalState<'m>>,
}

impl<'a, 'm> ResolutionContext<'a, 'm> {
    pub fn new(
        generics: &'a GenericLookup,
        loader: &'m AssemblyLoader,
        resolution: ResolutionS,
        shared: Arc<crate::vm::stack::SharedGlobalState<'m>>,
    ) -> Self {
        Self {
            generics,
            loader,
            resolution,
            type_owner: None,
            method_owner: None,
            shared,
        }
    }

    pub fn for_method(
        method: MethodDescription,
        loader: &'m AssemblyLoader,
        generics: &'a GenericLookup,
        shared: Arc<crate::vm::stack::SharedGlobalState<'m>>,
    ) -> Self {
        Self {
            generics,
            loader,
            resolution: method.resolution(),
            type_owner: Some(method.parent),
            method_owner: Some(method),
            shared,
        }
    }

    pub fn with_generics(&self, generics: &'a GenericLookup) -> ResolutionContext<'a, 'm> {
        ResolutionContext {
            generics,
            loader: self.loader,
            resolution: self.resolution,
            type_owner: self.type_owner,
            method_owner: self.method_owner,
            shared: self.shared.clone(),
        }
    }

    pub fn for_type(&self, td: TypeDescription) -> ResolutionContext<'a, 'm> {
        self.for_type_with_generics(td, self.generics)
    }

    pub fn for_type_with_generics(
        &self,
        td: TypeDescription,
        generics: &'a GenericLookup,
    ) -> ResolutionContext<'a, 'm> {
        ResolutionContext {
            resolution: td.resolution,
            generics,
            loader: self.loader,
            type_owner: Some(td),
            method_owner: None,
            shared: self.shared.clone(),
        }
    }

    pub fn locate_type(&self, handle: UserType) -> TypeDescription {
        self.loader.locate_type(self.resolution, handle)
    }

    pub fn locate_method(
        &self,
        handle: UserMethod,
        generic_inst: &GenericLookup,
    ) -> MethodDescription {
        self.loader
            .locate_method(self.resolution, handle, generic_inst)
    }

    pub fn locate_field(&self, field: FieldSource) -> (FieldDescription, GenericLookup) {
        self.loader
            .locate_field(self.resolution, field, self.generics)
    }

    pub fn get_ancestors(
        &self,
        child_type: TypeDescription,
    ) -> impl Iterator<Item = Ancestor<'m>> + 'm {
        self.loader.ancestors(child_type)
    }

    pub fn is_a(&self, value: ConcreteType, ancestor: ConcreteType) -> bool {
        let value = self.normalize_type(value);
        let ancestor = self.normalize_type(ancestor);

        if value == ancestor {
            return true;
        }

        let cache_key = (value.clone(), ancestor.clone());
        if let Some(cached) = self.shared.hierarchy_cache.get(&cache_key) {
            return *cached;
        }

        let value_td = self.loader.find_concrete_type(value);
        let ancestor_td = self.loader.find_concrete_type(ancestor);

        let mut seen = HashSet::new();
        let mut queue = VecDeque::new();

        for (a, _) in self.get_ancestors(value_td) {
            queue.push_back(a);
        }

        let mut result = false;
        while let Some(current) = queue.pop_front() {
            if current == ancestor_td {
                result = true;
                break;
            }
            if !seen.insert(current) {
                continue;
            }

            for (_, interface_source) in &current.definition().implements {
                let handle = match interface_source {
                    TypeSource::User(h) | TypeSource::Generic { base: h, .. } => *h,
                };
                let interface = self.loader.locate_type(current.resolution, handle);
                queue.push_back(interface);
            }
        }

        self.shared.hierarchy_cache.insert(cache_key, result);
        result
    }

    pub fn get_heap_description(&self, object: ObjectHandle) -> TypeDescription {
        use crate::value::object::HeapStorage::*;
        match &object.as_ref().borrow().storage {
            Obj(o) => o.description,
            Vec(_) => self.loader.corlib_type("System.Array"),
            Str(_) => self.loader.corlib_type("System.String"),
            Boxed(v) => v.description(self),
        }
    }

    pub fn make_concrete<T: Clone + Into<MethodType>>(&self, t: &T) -> ConcreteType {
        self.generics.make_concrete(self.resolution, t.clone())
    }

    pub fn get_field_type(&self, field: FieldDescription) -> ConcreteType {
        let return_type = &field.field.return_type;
        if field.field.by_ref {
            let by_ref_t: MemberType = BaseType::pointer(return_type.clone()).into();
            self.make_concrete(&by_ref_t)
        } else {
            self.make_concrete(return_type)
        }
    }

    pub fn get_field_desc(&self, field: FieldDescription) -> TypeDescription {
        self.loader.find_concrete_type(self.get_field_type(field))
    }

    pub fn normalize_type(&self, mut t: ConcreteType) -> ConcreteType {
        let (ut, res) = match t.get() {
            BaseType::Type { source, .. } => (decompose_type_source(source).0, t.resolution()),
            _ => return t,
        };

        let name = ut.type_name(res.definition());
        let base = match name.as_ref() {
            "System.Boolean" => Some(BaseType::Boolean),
            "System.Char" => Some(BaseType::Char),
            "System.Byte" => Some(BaseType::UInt8),
            "System.SByte" => Some(BaseType::Int8),
            "System.Int16" => Some(BaseType::Int16),
            "System.UInt16" => Some(BaseType::UInt16),
            "System.Int32" => Some(BaseType::Int32),
            "System.UInt32" => Some(BaseType::UInt32),
            "System.Int64" => Some(BaseType::Int64),
            "System.UInt64" => Some(BaseType::UInt64),
            "System.Single" => Some(BaseType::Float32),
            "System.Double" => Some(BaseType::Float64),
            "System.IntPtr" => Some(BaseType::IntPtr),
            "System.UIntPtr" => Some(BaseType::UIntPtr),
            "System.Object" => Some(BaseType::Object),
            "System.String" => Some(BaseType::String),
            _ => None,
        };

        if let Some(base) = base {
            ConcreteType::new(res, base)
        } else {
            if let BaseType::Type { source, value_kind } = t.get_mut() {
                if value_kind.is_none() {
                    let (ut, _) = decompose_type_source(source);
                    let td = self.loader.locate_type(res, ut);
                    if td.is_value_type(self) {
                        *value_kind = Some(ValueKind::ValueType);
                    }
                }
            }
            t
        }
    }
}

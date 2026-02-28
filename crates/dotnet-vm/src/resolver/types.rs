use crate::resolver::ResolverService;
use dotnet_types::{
    TypeDescription,
    comparer::decompose_type_source,
    error::TypeResolutionError,
    generics::{ConcreteType, GenericLookup},
    members::FieldDescription,
    resolution::ResolutionS,
};
use dotnet_value::{
    StackValue,
    object::{ObjectHandle, ValueType},
};
use dotnetdll::prelude::*;
use std::collections::{HashSet, VecDeque};

impl<'m> ResolverService<'m> {
    pub fn normalize_type(&self, mut t: ConcreteType) -> Result<ConcreteType, TypeResolutionError> {
        let (ut, res) = match t.get() {
            BaseType::Type { source, .. } => (
                decompose_type_source::<ConcreteType>(source).0,
                t.resolution(),
            ),
            _ => return Ok(t),
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
            "System.IntPtr" => Some(BaseType::IntPtr),
            "System.UIntPtr" => Some(BaseType::UIntPtr),
            "System.Single" => Some(BaseType::Float32),
            "System.Double" => Some(BaseType::Float64),
            "System.String" => Some(BaseType::String),
            "System.Object" => Some(BaseType::Object),
            _ => None,
        };

        if let Some(base) = base {
            Ok(ConcreteType::new(res, base))
        } else {
            if let BaseType::Type { source, value_kind } = t.get_mut()
                && value_kind.is_none()
            {
                let (ut, _) = decompose_type_source::<ConcreteType>(source);
                let td = self.loader.locate_type(res, ut)?;
                if self.is_value_type(td)? {
                    *value_kind = Some(ValueKind::ValueType);
                }
            }
            Ok(t)
        }
    }

    pub fn is_a(
        &self,
        value: ConcreteType,
        ancestor: ConcreteType,
    ) -> Result<bool, TypeResolutionError> {
        let value = self.normalize_type(value)?;
        let ancestor = self.normalize_type(ancestor)?;

        if value == ancestor {
            return Ok(true);
        }

        let cache_key = (value.clone(), ancestor.clone());
        if let Some(cached) = self.caches.hierarchy_cache.get(&cache_key) {
            if let Some(metrics) = self.metrics() {
                metrics.record_hierarchy_cache_hit();
            }
            return Ok(*cached);
        }

        if let Some(metrics) = self.metrics() {
            metrics.record_hierarchy_cache_miss();
        }

        let value_td = self.loader.find_concrete_type(value)?;
        let ancestor_td = self.loader.find_concrete_type(ancestor)?;

        let mut seen = HashSet::new();
        let mut queue = VecDeque::new();

        for (a, _) in self.loader.ancestors(value_td) {
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
                let interface = self.loader.locate_type(current.resolution, handle)?;
                queue.push_back(interface);
            }
        }

        self.caches.hierarchy_cache.insert(cache_key, result);
        Ok(result)
    }

    pub fn locate_type(
        &self,
        resolution: ResolutionS,
        handle: UserType,
    ) -> Result<TypeDescription, TypeResolutionError> {
        self.loader.locate_type(resolution, handle)
    }

    pub fn make_concrete<T: Clone + Into<MethodType>>(
        &self,
        resolution: ResolutionS,
        generics: &GenericLookup,
        t: &T,
    ) -> Result<ConcreteType, TypeResolutionError> {
        generics.make_concrete(resolution, t.clone())
    }

    pub fn is_value_type(&self, td: TypeDescription) -> Result<bool, TypeResolutionError> {
        if let Some(cached) = self.caches.value_type_cache.get(&td) {
            if let Some(m) = self.metrics() {
                m.record_value_type_cache_hit();
            }
            return Ok(*cached);
        }
        if let Some(m) = self.metrics() {
            m.record_value_type_cache_miss();
        }

        let enum_type = self.loader.corlib_type("System.Enum")?;
        let value_type = self.loader.corlib_type("System.ValueType")?;

        for (a, _) in self.loader.ancestors(td) {
            if a == enum_type || a == value_type {
                self.caches.value_type_cache.insert(td, true);
                return Ok(true);
            }
        }
        self.caches.value_type_cache.insert(td, false);
        Ok(false)
    }

    pub fn has_finalizer(&self, td: TypeDescription) -> Result<bool, TypeResolutionError> {
        if let Some(cached) = self.caches.has_finalizer_cache.get(&td) {
            if let Some(m) = self.metrics() {
                m.record_has_finalizer_cache_hit();
            }
            return Ok(*cached);
        }
        if let Some(m) = self.metrics() {
            m.record_has_finalizer_cache_miss();
        }

        let check_type = |td: TypeDescription| {
            let def = td.definition();
            let ns = def.namespace.as_deref().unwrap_or("");
            let name = &def.name;

            if ns == "System" && (name == "Object" || name == "ValueType" || name == "Enum") {
                return false;
            }

            def.methods.iter().any(|m| {
                m.name == "Finalize" && m.virtual_member && m.signature.parameters.is_empty()
            })
        };

        if check_type(td) {
            self.caches.has_finalizer_cache.insert(td, true);
            return Ok(true);
        }

        for (ancestor, _) in self.loader.ancestors(td) {
            if check_type(ancestor) {
                self.caches.has_finalizer_cache.insert(td, true);
                return Ok(true);
            }
        }
        self.caches.has_finalizer_cache.insert(td, false);
        Ok(false)
    }

    pub fn get_field_type(
        &self,
        resolution: ResolutionS,
        generics: &GenericLookup,
        field: FieldDescription,
    ) -> Result<ConcreteType, TypeResolutionError> {
        let return_type = &field.field.return_type;
        if field.field.by_ref {
            let by_ref_t: MemberType = BaseType::pointer(return_type.clone()).into();
            self.make_concrete(resolution, generics, &by_ref_t)
        } else {
            self.make_concrete(resolution, generics, return_type)
        }
    }

    pub fn get_field_desc(
        &self,
        resolution: ResolutionS,
        generics: &GenericLookup,
        field: FieldDescription,
    ) -> Result<TypeDescription, TypeResolutionError> {
        self.loader
            .find_concrete_type(self.get_field_type(resolution, generics, field)?)
    }

    pub fn get_heap_description(
        &self,
        object: ObjectHandle,
    ) -> Result<TypeDescription, TypeResolutionError> {
        let inner = object.as_ref().borrow();
        self.get_heap_description_inner(&inner)
    }

    pub(crate) fn get_heap_description_inner(
        &self,
        inner: &dotnet_value::object::ObjectInner<'_>,
    ) -> Result<TypeDescription, TypeResolutionError> {
        use dotnet_value::object::HeapStorage::*;
        match &inner.storage {
            Obj(o) => Ok(o.description),
            Vec(_) => self.loader.corlib_type("System.Array"),
            Str(_) => self.loader.corlib_type("System.String"),
            Boxed(o) => Ok(o.description),
        }
    }

    pub fn value_type_description<'gc>(
        &self,
        vt: &ValueType<'gc>,
    ) -> Result<TypeDescription, TypeResolutionError> {
        let asms = self.loader();
        match vt {
            ValueType::Bool(_) => asms.corlib_type("System.Boolean"),
            ValueType::Char(_) => asms.corlib_type("System.Char"),
            ValueType::Int8(_) => asms.corlib_type("System.SByte"),
            ValueType::UInt8(_) => asms.corlib_type("System.Byte"),
            ValueType::Int16(_) => asms.corlib_type("System.Int16"),
            ValueType::UInt16(_) => asms.corlib_type("System.UInt16"),
            ValueType::Int32(_) => asms.corlib_type("System.Int32"),
            ValueType::UInt32(_) => asms.corlib_type("System.UInt32"),
            ValueType::Int64(_) => asms.corlib_type("System.Int64"),
            ValueType::UInt64(_) => asms.corlib_type("System.UInt64"),
            ValueType::NativeInt(_) => asms.corlib_type("System.IntPtr"),
            ValueType::NativeUInt(_) => asms.corlib_type("System.UIntPtr"),
            ValueType::Pointer(_) => asms.corlib_type("System.IntPtr"),
            ValueType::Float32(_) => asms.corlib_type("System.Single"),
            ValueType::Float64(_) => asms.corlib_type("System.Double"),
            ValueType::TypedRef(_, _) => asms.corlib_type("System.TypedReference"),
            ValueType::Struct(s) => Ok(s.description),
        }
    }

    pub fn stack_value_type<'gc>(
        &self,
        val: &StackValue<'gc>,
    ) -> Result<TypeDescription, TypeResolutionError> {
        use dotnet_value::object::ObjectRef;
        match val {
            StackValue::Int32(_) => self.loader.corlib_type("System.Int32"),
            StackValue::Int64(_) => self.loader.corlib_type("System.Int64"),
            StackValue::NativeInt(_) | StackValue::UnmanagedPtr(_) => {
                self.loader.corlib_type("System.IntPtr")
            }
            StackValue::NativeFloat(_) => self.loader.corlib_type("System.Double"),
            StackValue::ObjectRef(ObjectRef(Some(o))) => self.get_heap_description(*o),
            StackValue::ObjectRef(ObjectRef(None)) => self.loader.corlib_type("System.Object"),
            StackValue::ManagedPtr(m) => Ok(m.inner_type()),
            StackValue::ValueType(o) => Ok(o.description),
            StackValue::TypedRef(_, _) => self.loader.corlib_type("System.TypedReference"),
            #[cfg(feature = "multithreading")]
            StackValue::CrossArenaObjectRef(ptr, _) => {
                let lock = unsafe { &*ptr.as_ptr() };
                let guard = lock.borrow();
                self.get_heap_description_inner(&guard)
            }
        }
    }
}

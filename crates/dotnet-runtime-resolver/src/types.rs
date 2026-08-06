use crate::ResolverService;
use dotnet_types::{
    TypeDescription, WellKnown,
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

impl<C, L> ResolverService<C, L>
where
    C: crate::ResolverCacheAdapter,
    L: crate::ResolverLayoutAdapter,
{
    pub fn normalize_type(&self, t: ConcreteType) -> Result<ConcreteType, TypeResolutionError> {
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
            if let BaseType::Type {
                source,
                value_kind: None,
            } = t.get()
            {
                let (ut, _) = decompose_type_source::<ConcreteType>(source);
                let td = self.loader.locate_type(res.clone(), ut)?;
                if self.is_value_type(td)? {
                    return Ok(ConcreteType::new(
                        res,
                        BaseType::Type {
                            source: source.clone(),
                            value_kind: Some(ValueKind::ValueType),
                        },
                    ));
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

        if let Some(cached) = self.caches.get_hierarchy_cached(&value, &ancestor) {
            return Ok(cached);
        }

        let result = self.loader.comparer().is_assignable_to(&value, &ancestor);

        self.caches.set_hierarchy_cached(value, ancestor, result);
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
        generics.make_concrete(resolution, t.clone(), self.loader())
    }

    pub fn is_value_type(&self, td: TypeDescription) -> Result<bool, TypeResolutionError> {
        if let Some(cached) = self.caches.get_value_type_cached(&td) {
            return Ok(cached);
        }

        let enum_type = self.loader.corlib_wkt(WellKnown::Enum)?;
        let value_type = self.loader.corlib_wkt(WellKnown::ValueType)?;

        for (a, _) in self.loader.ancestors(td.clone()) {
            if a == enum_type || a == value_type {
                self.caches.set_value_type_cached(td.clone(), true);
                return Ok(true);
            }
        }
        self.caches.set_value_type_cached(td, false);
        Ok(false)
    }

    pub fn has_finalizer(&self, td: TypeDescription) -> Result<bool, TypeResolutionError> {
        if let Some(cached) = self.caches.get_has_finalizer_cached(&td) {
            return Ok(cached);
        }

        let check_type = |td: TypeDescription| {
            let def = td.definition();
            let ns = def.namespace.as_deref().unwrap_or("");
            let name = &def.name;

            if ns == "System" && (name == "Object" || name == "ValueType" || name == "Enum") {
                return false;
            }

            def.methods.iter().enumerate().any(|(i, m)| {
                if !m.virtual_member || m.name != "Finalize" {
                    return false;
                }
                let d = dotnet_types::members::MethodDescription::new(
                    td.clone(),
                    dotnet_types::generics::GenericLookup::default(),
                    td.resolution.clone(),
                    dotnetdll::prelude::MethodMemberIndex::Method(i),
                );
                d.signature().parameters.is_empty()
            })
        };

        if check_type(td.clone()) {
            self.caches.set_has_finalizer_cached(td.clone(), true);
            return Ok(true);
        }

        for (ancestor, _) in self.loader.ancestors(td.clone()) {
            if check_type(ancestor) {
                self.caches.set_has_finalizer_cached(td.clone(), true);
                return Ok(true);
            }
        }
        self.caches.set_has_finalizer_cached(td, false);
        Ok(false)
    }

    pub fn get_field_type(
        &self,
        resolution: ResolutionS,
        generics: &GenericLookup,
        field: FieldDescription,
    ) -> Result<ConcreteType, TypeResolutionError> {
        let return_type = &field.field().return_type;
        if field.field().by_ref {
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
            Obj(o) => Ok(o.description.clone()),
            Vec(_) => self.loader.corlib_wkt(WellKnown::Array),
            Str(_) => self.loader.corlib_wkt(WellKnown::String),
            Boxed(o) => Ok(o.description.clone()),
        }
    }

    pub fn value_type_description<'gc>(
        &self,
        vt: &ValueType<'gc>,
    ) -> Result<TypeDescription, TypeResolutionError> {
        let asms = self.loader();
        match vt {
            ValueType::Bool(_) => asms.corlib_wkt(WellKnown::Boolean),
            ValueType::Char(_) => asms.corlib_wkt(WellKnown::Char),
            ValueType::Int8(_) => asms.corlib_wkt(WellKnown::SByte),
            ValueType::UInt8(_) => asms.corlib_wkt(WellKnown::Byte),
            ValueType::Int16(_) => asms.corlib_wkt(WellKnown::Int16),
            ValueType::UInt16(_) => asms.corlib_wkt(WellKnown::UInt16),
            ValueType::Int32(_) => asms.corlib_wkt(WellKnown::Int32),
            ValueType::UInt32(_) => asms.corlib_wkt(WellKnown::UInt32),
            ValueType::Int64(_) => asms.corlib_wkt(WellKnown::Int64),
            ValueType::UInt64(_) => asms.corlib_wkt(WellKnown::UInt64),
            ValueType::NativeInt(_) => asms.corlib_wkt(WellKnown::IntPtr),
            ValueType::NativeUInt(_) => asms.corlib_wkt(WellKnown::UIntPtr),
            ValueType::Pointer(_) => asms.corlib_wkt(WellKnown::IntPtr),
            ValueType::Float32(_) => asms.corlib_wkt(WellKnown::Single),
            ValueType::Float64(_) => asms.corlib_wkt(WellKnown::Double),
            ValueType::Struct(s) => Ok(s.description.clone()),
        }
    }

    pub fn stack_value_type<'gc>(
        &self,
        val: &StackValue<'gc>,
    ) -> Result<TypeDescription, TypeResolutionError> {
        use dotnet_value::object::ObjectRef;
        match val {
            StackValue::Int32(_) => self.loader.corlib_wkt(WellKnown::Int32),
            StackValue::Int64(_) => self.loader.corlib_wkt(WellKnown::Int64),
            StackValue::NativeInt(_) | StackValue::UnmanagedPtr(_) => {
                self.loader.corlib_wkt(WellKnown::IntPtr)
            }
            StackValue::NativeFloat(_) => self.loader.corlib_wkt(WellKnown::Double),
            StackValue::ObjectRef(ObjectRef(Some(o))) => self.get_heap_description(*o),
            StackValue::ObjectRef(ObjectRef(None)) => self.loader.corlib_wkt(WellKnown::Object),
            StackValue::ManagedPtr(m) => Ok(m.inner_type()),
            StackValue::ValueType(o) => Ok(o.description.clone()),
            StackValue::TypedRef(_) | StackValue::UninitializedTypedRef => {
                self.loader.corlib_wkt(WellKnown::TypedReference)
            }
            #[cfg(feature = "multithreading")]
            StackValue::CrossArenaObjectRef(ptr, _) => {
                // SAFETY: Cross-arena object pointers are live GC handles maintained by the
                // thread manager; borrowing their lock yields the stable shared object view.
                let lock = unsafe { &*ptr.as_ptr() };
                let guard = lock.borrow();
                self.get_heap_description_inner(&guard)
            }
        }
    }
}

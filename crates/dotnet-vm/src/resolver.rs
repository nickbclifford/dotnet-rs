//! Type, method, and field resolution with caching.
//!
//! This module provides the [`ResolverService`] for resolving metadata tokens to
//! their corresponding type definitions, method signatures, and field descriptors.
//! Resolution involves looking up entities in loaded assemblies, handling generic
//! instantiation, and caching results for performance.
//!
//! # Architecture
//!
//! The [`ResolverService`] acts as a facade over several resolution subsystems:
//!
//! - **Assembly Loading**: Delegates to [`AssemblyLoader`] for loading and parsing
//!   .NET assemblies from the filesystem.
//!
//! - **Generic Instantiation**: Uses [`GenericLookup`] to substitute type parameters
//!   with concrete types in generic methods and types.
//!
//! - **Caching**: Maintains several caches in [`GlobalCaches`]:
//!   - Intrinsic method cache (is a method implemented natively?)
//!   - Intrinsic field cache (is a field implemented natively?)
//!   - Type layout cache (computed memory layouts for value types)
//!   - Virtual dispatch cache (resolved virtual method targets)
//!
//! - **Type Layouts**: Computes memory layouts for value types, including field
//!   offsets, alignment, and total size.
//!
//! - **Interface Resolution**: Resolves interface methods to their concrete
//!   implementations in classes that implement those interfaces.
//!
//! # Resolution Process
//!
//! ## Type Resolution
//!
//! Types are resolved from metadata tokens (`TypeDefOrRef`, `TypeSpec`) to
//! [`TypeDescription`] instances. For generic types, the resolver substitutes
//! type parameters with concrete arguments from the current generic context.
//!
//! ## Method Resolution
//!
//! Methods are resolved in several phases:
//! 1. **Token to descriptor**: Map metadata token to [`MethodDescription`]
//! 2. **Virtual dispatch**: For `callvirt`, resolve to most-derived implementation
//! 3. **Generic instantiation**: Substitute type/method generic parameters
//! 4. **Intrinsic check**: Determine if method has native implementation
//!
//! ## Field Resolution
//!
//! Fields are resolved to [`FieldDescription`] with computed offsets within their
//! containing type. Value type field access requires layout calculation.
//!
//! # Caching Strategy
//!
//! The resolver maintains several caches to avoid redundant work:
//!
//! - **Intrinsic caches**: Method and field intrinsic checks are cached, as these
//!   involve string comparisons and registry lookups.
//!
//! - **Layout cache**: Type layouts are expensive to compute (requires recursive
//!   field traversal) and are cached by concrete type.
//!
//! - **Virtual dispatch cache**: Resolved virtual methods are cached by (method, type)
//!   pair to avoid repeated inheritance chain traversal.
//!
//! Cache eviction is not currently implemented, as the cache sizes are bounded by
//! the number of unique types/methods in the loaded assemblies.
//!
//! # Example
//!
//! ```ignore
//! let resolver = ResolverService::new(shared_state);
//!
//! // Resolve a type from a metadata token
//! let type_desc = resolver.resolve_type(type_token, generic_context)?;
//!
//! // Check if a method is intrinsic (natively implemented)
//! if resolver.is_intrinsic_cached(method_desc) {
//!     // Call intrinsic implementation
//! } else {
//!     // Interpret CIL bytecode
//! }
//!
//! // Compute type layout for value type
//! let layout = resolver.compute_layout(value_type_desc)?;
//! ```
use crate::{
    context::ResolutionContext,
    layout::type_layout_with_metrics,
    metrics::RuntimeMetrics,
    resolution::TypeResolutionExt,
    state::{GlobalCaches, SharedGlobalState},
};
use dotnet_assemblies::AssemblyLoader;
use dotnet_types::{
    TypeDescription,
    comparer::decompose_type_source,
    error::TypeResolutionError,
    generics::{ConcreteType, GenericLookup},
    members::{FieldDescription, MethodDescription},
    resolution::ResolutionS,
};
use dotnet_utils::gc::GCHandle;
use dotnet_value::{
    StackValue,
    layout::{HasLayout, LayoutManager},
    object::{CTSValue, HeapStorage, Object, ObjectHandle, ObjectRef, ValueType, Vector},
    pointer::ManagedPtr,
    storage::FieldStorage,
};
use dotnetdll::prelude::*;
use sptr::Strict;
use std::{
    any,
    collections::{HashSet, VecDeque},
    error::Error,
    ptr::NonNull,
    sync::Arc,
};

/// Service for resolving types, methods, and fields with caching.
pub struct ResolverService<'m> {
    pub loader: &'m AssemblyLoader,
    pub caches: Arc<GlobalCaches>,
    pub shared: Option<Arc<SharedGlobalState<'m>>>,
}

impl<'m> ResolverService<'m> {
    pub fn new(shared: Arc<SharedGlobalState<'m>>) -> Self {
        Self {
            loader: shared.loader,
            caches: shared.caches.clone(),
            shared: Some(shared),
        }
    }

    pub fn from_parts(loader: &'m AssemblyLoader, caches: Arc<GlobalCaches>) -> Self {
        Self {
            loader,
            caches,
            shared: None,
        }
    }

    fn metrics(&self) -> Option<&RuntimeMetrics> {
        self.shared.as_ref().map(|s| &s.metrics)
    }

    pub fn loader(&self) -> &'m AssemblyLoader {
        self.loader
    }

    pub fn is_intrinsic_cached(&self, method: MethodDescription) -> bool {
        if let Some(cached) = self.caches.intrinsic_cache.get(&method) {
            if let Some(metrics) = self.metrics() {
                metrics.record_intrinsic_cache_hit();
            }
            return *cached;
        }

        if let Some(metrics) = self.metrics() {
            metrics.record_intrinsic_cache_miss();
        }
        let result =
            crate::intrinsics::is_intrinsic(method, self.loader(), &self.caches.intrinsic_registry);
        self.caches.intrinsic_cache.insert(method, result);
        result
    }

    pub fn is_intrinsic_field_cached(&self, field: FieldDescription) -> bool {
        if let Some(cached) = self.caches.intrinsic_field_cache.get(&field) {
            if let Some(metrics) = self.metrics() {
                metrics.record_intrinsic_field_cache_hit();
            }
            return *cached;
        }

        if let Some(metrics) = self.metrics() {
            metrics.record_intrinsic_field_cache_miss();
        }
        let result = crate::intrinsics::is_intrinsic_field(
            field,
            self.loader(),
            &self.caches.intrinsic_registry,
        );
        self.caches.intrinsic_field_cache.insert(field, result);
        result
    }

    pub fn find_generic_method(
        &self,
        source: &MethodSource,
        ctx: &ResolutionContext<'_, 'm>,
    ) -> Result<(MethodDescription, GenericLookup), TypeResolutionError> {
        let mut new_lookup = ctx.generics.clone();

        let method = match source {
            MethodSource::User(u) => *u,
            MethodSource::Generic(g) => {
                let params: Vec<_> = g
                    .parameters
                    .iter()
                    .map(|t| ctx.make_concrete(t))
                    .collect::<Result<Vec<_>, _>>()?;
                new_lookup.method_generics = params.into();
                g.base
            }
        };

        let mut parent_type = None;
        if let UserMethod::Reference(r) = method
            && let MethodReferenceParent::Type(t) = &ctx.resolution[r].parent
        {
            let parent = ctx.make_concrete(t)?;
            if let BaseType::Type {
                source: TypeSource::Generic { parameters, .. },
                ..
            } = parent.get()
            {
                new_lookup.type_generics = parameters.clone().into();
            }
            parent_type = Some(parent);
        }

        Ok((
            ctx.locate_method(method, &new_lookup, parent_type)?,
            new_lookup,
        ))
    }

    pub fn resolve_virtual_method(
        &self,
        base_method: MethodDescription,
        this_type: TypeDescription,
        generics: &GenericLookup,
        ctx: &ResolutionContext<'_, 'm>,
    ) -> Result<MethodDescription, TypeResolutionError> {
        let mut normalized_generics = generics.clone();
        if this_type.definition().generic_parameters.is_empty() {
            normalized_generics.type_generics = Arc::new([]);
        }
        if base_method.method.generic_parameters.is_empty() {
            normalized_generics.method_generics = Arc::new([]);
        }

        let key = (base_method, this_type, normalized_generics);
        if let Some(cached) = self.caches.vmt_cache.get(&key) {
            if let Some(metrics) = self.metrics() {
                metrics.record_vmt_cache_hit();
            }
            return Ok(*cached);
        }

        if let Some(metrics) = self.metrics() {
            metrics.record_vmt_cache_miss();
        }

        // Standard virtual method resolution: search ancestors
        for (parent, _) in ctx.get_ancestors(this_type) {
            if let Some(this_method) = self.find_and_cache_method(parent, base_method, &key.2)? {
                self.caches.vmt_cache.insert(key, this_method);
                return Ok(this_method);
            }
        }

        Err(TypeResolutionError::MethodNotFound(format!(
            "could not find virtual method implementation of {:?} in {:?} with generics {:?}",
            base_method, this_type, generics
        )))
    }

    fn find_and_cache_method(
        &self,
        this_type: TypeDescription,
        method: MethodDescription,
        generics: &GenericLookup,
    ) -> Result<Option<MethodDescription>, TypeResolutionError> {
        let def = this_type.definition();
        if !def.overrides.is_empty() {
            let cache_key = (this_type, generics.clone());
            let overrides = if let Some(map) = self.caches.overrides_cache.get(&cache_key) {
                map.clone()
            } else {
                let mut map = std::collections::HashMap::new();
                for ovr in def.overrides.iter() {
                    let decl = self.loader.locate_method(
                        this_type.resolution,
                        ovr.declaration,
                        generics,
                        None,
                    )?;
                    let impl_m = self.loader.locate_method(
                        this_type.resolution,
                        ovr.implementation,
                        generics,
                        Some(this_type.into()),
                    )?;
                    map.insert(decl.method as *const _ as usize, impl_m);
                }
                let arc_map = Arc::new(map);
                self.caches
                    .overrides_cache
                    .insert(cache_key, arc_map.clone());
                arc_map
            };

            if let Some(impl_m) = overrides.get(&(method.method as *const _ as usize)) {
                self.caches
                    .vmt_cache
                    .insert((method, this_type, generics.clone()), *impl_m);
                return Ok(Some(*impl_m));
            }
        }

        if let Some(this_method) = self.loader.find_method_in_type_with_substitution(
            this_type,
            &method.method.name,
            &method.method.signature,
            method.resolution(),
            generics,
        ) {
            self.caches
                .vmt_cache
                .insert((method, this_type, generics.clone()), this_method);
            return Ok(Some(this_method));
        }

        Ok(None)
    }

    pub fn type_layout_cached(
        &self,
        t: ConcreteType,
        ctx: &ResolutionContext<'_, 'm>,
    ) -> Result<Arc<LayoutManager>, TypeResolutionError> {
        type_layout_with_metrics(t, ctx, self.metrics())
    }

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

    pub fn locate_method(
        &self,
        resolution: ResolutionS,
        handle: UserMethod,
        generic_inst: &GenericLookup,
        pre_resolved_parent: Option<ConcreteType>,
    ) -> Result<MethodDescription, TypeResolutionError> {
        self.loader
            .locate_method(resolution, handle, generic_inst, pre_resolved_parent)
    }

    pub fn locate_field(
        &self,
        resolution: ResolutionS,
        field: FieldSource,
        generics: &GenericLookup,
    ) -> Result<(FieldDescription, GenericLookup), TypeResolutionError> {
        self.loader.locate_field(resolution, field, generics)
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

    fn get_heap_description_inner(
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
            StackValue::ManagedPtr(m) => Ok(m.inner_type),
            StackValue::ValueType(o) => Ok(o.description),
            StackValue::TypedRef(_, _) => self.loader.corlib_type("System.TypedReference"),
            #[cfg(feature = "multithreaded-gc")]
            StackValue::CrossArenaObjectRef(ptr, _) => {
                let lock = unsafe { ptr.0.as_ref() };
                let guard = lock.borrow();
                self.get_heap_description_inner(&guard)
            }
        }
    }

    pub fn new_object<'gc>(
        &self,
        td: TypeDescription,
        ctx: &ResolutionContext<'_, 'm>,
    ) -> Result<Object<'gc>, TypeResolutionError> {
        Ok(Object::new(
            td,
            ctx.generics.clone(),
            self.new_instance_fields(td, ctx)?,
        ))
    }

    pub fn box_value<'gc>(
        &self,
        t: &ConcreteType,
        data: StackValue<'gc>,
        gc: GCHandle<'gc>,
        ctx: &ResolutionContext<'_, 'm>,
    ) -> Result<ObjectRef<'gc>, TypeResolutionError> {
        let t = self.normalize_type(t.clone())?;
        match self.new_cts_value(&t, data, ctx)? {
            CTSValue::Value(v) => {
                let td = self.loader.find_concrete_type(t)?;
                let obj_instance = self.new_object(td, ctx)?;
                let size = v.size_bytes();
                CTSValue::Value(v).write(&mut obj_instance.instance_storage.get_mut()[..size]);
                Ok(ObjectRef::new(gc, HeapStorage::Boxed(obj_instance)))
            }
            CTSValue::Ref(r) => Ok(r),
        }
    }

    pub fn new_instance_fields(
        &self,
        td: TypeDescription,
        ctx: &ResolutionContext<'_, 'm>,
    ) -> Result<FieldStorage, TypeResolutionError> {
        let layout =
            crate::layout::LayoutFactory::instance_field_layout_cached(td, ctx, self.metrics())?;
        let size = layout.size();
        Ok(FieldStorage::new(layout, vec![0; size.as_usize()]))
    }

    pub fn new_static_fields(
        &self,
        td: TypeDescription,
        ctx: &ResolutionContext<'_, 'm>,
    ) -> Result<FieldStorage, TypeResolutionError> {
        let layout = Arc::new(crate::layout::LayoutFactory::static_fields_with_metrics(
            td,
            ctx,
            self.metrics(),
        )?);
        let size = layout.size();
        Ok(FieldStorage::new(layout, vec![0; size.as_usize()]))
    }

    pub fn new_value_type<'gc>(
        &self,
        t: &ConcreteType,
        data: StackValue<'gc>,
        ctx: &ResolutionContext<'_, 'm>,
    ) -> Result<ValueType<'gc>, TypeResolutionError> {
        match self.new_cts_value(t, data, ctx)? {
            CTSValue::Value(v) => Ok(v),
            CTSValue::Ref(r) => {
                panic!(
                    "tried to instantiate value type, received object reference ({:?})",
                    r
                )
            }
        }
    }

    pub fn new_cts_value<'gc>(
        &self,
        t: &ConcreteType,
        data: StackValue<'gc>,
        ctx: &ResolutionContext<'_, 'm>,
    ) -> Result<CTSValue<'gc>, TypeResolutionError> {
        use ValueType::*;
        let t = self.normalize_type(t.clone())?;
        match t.get() {
            BaseType::Boolean => Ok(CTSValue::Value(Bool(convert_num::<u8>(data) != 0))),
            BaseType::Char => Ok(CTSValue::Value(Char(convert_num(data)))),
            BaseType::Int8 => Ok(CTSValue::Value(Int8(convert_num(data)))),
            BaseType::UInt8 => Ok(CTSValue::Value(UInt8(convert_num(data)))),
            BaseType::Int16 => Ok(CTSValue::Value(Int16(convert_num(data)))),
            BaseType::UInt16 => Ok(CTSValue::Value(UInt16(convert_num(data)))),
            BaseType::Int32 => Ok(CTSValue::Value(Int32(convert_num(data)))),
            BaseType::UInt32 => Ok(CTSValue::Value(UInt32(convert_num(data)))),
            BaseType::Int64 => Ok(CTSValue::Value(Int64(convert_i64(data)))),
            BaseType::UInt64 => Ok(CTSValue::Value(UInt64(reinterpret_i64_as_u64(data)))),
            BaseType::Float32 => Ok(CTSValue::Value(Float32(match data {
                StackValue::NativeFloat(f) => f as f32,
                other => panic!("invalid stack value {:?} for float conversion", other),
            }))),
            BaseType::Float64 => Ok(CTSValue::Value(Float64(match data {
                StackValue::NativeFloat(f) => f,
                other => panic!("invalid stack value {:?} for float conversion", other),
            }))),
            BaseType::IntPtr => Ok(CTSValue::Value(NativeInt(convert_num(data)))),
            BaseType::UIntPtr | BaseType::FunctionPointer(_) => {
                Ok(CTSValue::Value(NativeUInt(convert_num(data))))
            }
            BaseType::ValuePointer(_modifiers, inner) => match data {
                StackValue::ManagedPtr(p) => Ok(CTSValue::Value(Pointer(p))),
                _ => {
                    let ptr = convert_num::<usize>(data);
                    let inner_type = if let Some(source) = inner {
                        self.loader.find_concrete_type(source.clone())?
                    } else {
                        self.loader.corlib_type("System.Void")?
                    };
                    Ok(CTSValue::Value(Pointer(ManagedPtr::new(
                        NonNull::new(ptr as *mut u8),
                        inner_type,
                        None,
                        false,
                        Some(dotnet_value::ByteOffset(ptr)),
                    ))))
                }
            },
            BaseType::Object
            | BaseType::String
            | BaseType::Vector(_, _)
            | BaseType::Array(_, _) => {
                if let StackValue::ObjectRef(o) = data {
                    Ok(CTSValue::Ref(o))
                } else {
                    panic!("expected ObjectRef, got {:?}", data)
                }
            }
            BaseType::Type {
                value_kind: Some(ValueKind::Class),
                ..
            } => {
                if let StackValue::ObjectRef(o) = data {
                    Ok(CTSValue::Ref(o))
                } else {
                    panic!("expected ObjectRef, got {:?}", data)
                }
            }
            BaseType::Type {
                value_kind: None | Some(ValueKind::ValueType),
                source,
            } => {
                let (ut, type_generics) = decompose_type_source(source);
                let new_lookup = GenericLookup::new(type_generics);
                let new_ctx = ctx.with_generics(&new_lookup);
                let td = new_ctx.locate_type(ut)?;

                if !td.is_value_type(&new_ctx)? {
                    return Ok(CTSValue::Ref(if let StackValue::ObjectRef(r) = data {
                        r
                    } else {
                        panic!("expected ObjectRef, got {:?}", data)
                    }));
                }

                if let Some(e) = td.is_enum() {
                    let enum_type = new_ctx.make_concrete(e)?;
                    return self.new_cts_value(&enum_type, data, &new_ctx);
                }

                if td.type_name() == "System.TypedReference" {
                    let StackValue::TypedRef(p, t) = data else {
                        panic!("expected TypedRef, got {:?}", data);
                    };
                    return Ok(CTSValue::Value(TypedRef(p, t)));
                }

                if let StackValue::ValueType(mut o) = data {
                    if o.description != td {
                        o.description = td;
                    }
                    Ok(CTSValue::Value(Struct(o)))
                } else {
                    let mut instance = self.new_object(td, &new_ctx)?;
                    if let StackValue::ObjectRef(o) = data
                        && let Some(handle) = o.0
                    {
                        let borrowed = handle.borrow();
                        match &borrowed.storage {
                            HeapStorage::Obj(obj) => {
                                instance.instance_storage = obj.instance_storage.clone();
                            }
                            _ => panic!("cannot unbox from non-object storage"),
                        }
                    }
                    Ok(CTSValue::Value(Struct(instance)))
                }
            }
        }
    }

    pub fn read_cts_value<'gc>(
        &self,
        t: &ConcreteType,
        data: &[u8],
        gc: GCHandle<'gc>,
        ctx: &ResolutionContext<'_, 'm>,
    ) -> Result<CTSValue<'gc>, TypeResolutionError> {
        use ValueType::*;
        let t = self.normalize_type(t.clone())?;
        match t.get() {
            BaseType::Boolean => Ok(CTSValue::Value(Bool(data[0] != 0))),
            BaseType::Char => Ok(CTSValue::Value(Char(u16::from_ne_bytes(
                data.try_into().unwrap(),
            )))),
            BaseType::Int8 => Ok(CTSValue::Value(Int8(data[0] as i8))),
            BaseType::UInt8 => Ok(CTSValue::Value(UInt8(data[0]))),
            BaseType::Int16 => Ok(CTSValue::Value(Int16(i16::from_ne_bytes(
                data.try_into().unwrap(),
            )))),
            BaseType::UInt16 => Ok(CTSValue::Value(UInt16(u16::from_ne_bytes(
                data.try_into().unwrap(),
            )))),
            BaseType::Int32 => Ok(CTSValue::Value(Int32(i32::from_ne_bytes(
                data.try_into().unwrap(),
            )))),
            BaseType::UInt32 => Ok(CTSValue::Value(UInt32(u32::from_ne_bytes(
                data.try_into().unwrap(),
            )))),
            BaseType::Int64 => Ok(CTSValue::Value(Int64(i64::from_ne_bytes(
                data.try_into().unwrap(),
            )))),
            BaseType::UInt64 => Ok(CTSValue::Value(UInt64(u64::from_ne_bytes(
                data.try_into().unwrap(),
            )))),
            BaseType::Float32 => Ok(CTSValue::Value(Float32(f32::from_ne_bytes(
                data.try_into().unwrap(),
            )))),
            BaseType::Float64 => Ok(CTSValue::Value(Float64(f64::from_ne_bytes(
                data.try_into().unwrap(),
            )))),
            BaseType::IntPtr => Ok(CTSValue::Value(NativeInt(isize::from_ne_bytes(
                data.try_into().unwrap(),
            )))),
            BaseType::UIntPtr | BaseType::FunctionPointer(_) => Ok(CTSValue::Value(NativeUInt(
                usize::from_ne_bytes(data.try_into().unwrap()),
            ))),
            BaseType::ValuePointer(_modifiers, inner) => {
                let inner_type = if let Some(source) = inner {
                    self.loader.find_concrete_type(source.clone())?
                } else {
                    self.loader.corlib_type("System.Void")?
                };

                if data.len() >= ManagedPtr::SIZE {
                    let info = unsafe { ManagedPtr::read_branded(&data[..ManagedPtr::SIZE], &gc) }
                        .expect("read_cts_value: ManagedPtr deserialization failed");
                    let m = ManagedPtr::from_info_full(info, inner_type, false);
                    Ok(CTSValue::Value(Pointer(m)))
                } else {
                    let mut ptr_bytes = [0u8; ObjectRef::SIZE];
                    ptr_bytes.copy_from_slice(&data[0..ObjectRef::SIZE]);
                    let ptr = usize::from_ne_bytes(ptr_bytes);
                    Ok(CTSValue::Value(Pointer(ManagedPtr::new(
                        NonNull::new(sptr::from_exposed_addr_mut(ptr)),
                        inner_type,
                        None,
                        false,
                        Some(dotnet_value::ByteOffset(ptr)),
                    ))))
                }
            }
            BaseType::Object
            | BaseType::String
            | BaseType::Vector(_, _)
            | BaseType::Array(_, _) => {
                Ok(CTSValue::Ref(unsafe { ObjectRef::read_branded(data, &gc) }))
            }
            BaseType::Type {
                value_kind: Some(ValueKind::Class),
                ..
            } => Ok(CTSValue::Ref(unsafe { ObjectRef::read_branded(data, &gc) })),
            BaseType::Type {
                value_kind: None | Some(ValueKind::ValueType),
                source,
            } => {
                let (ut, type_generics) = decompose_type_source(source);
                let new_lookup = GenericLookup::new(type_generics);
                let new_ctx = ctx.with_generics(&new_lookup);
                let td = new_ctx.locate_type(ut)?;

                if !td.is_value_type(&new_ctx)? {
                    return Ok(CTSValue::Ref(unsafe { ObjectRef::read_branded(data, &gc) }));
                }

                if let Some(e) = td.is_enum() {
                    let enum_type = new_ctx.make_concrete(e)?;
                    return self.read_cts_value(&enum_type, data, gc, &new_ctx);
                }

                if td.type_name() == "System.TypedReference" {
                    let mut buf = ManagedPtr::serialization_buffer();
                    buf.copy_from_slice(&data[..ManagedPtr::SIZE]);
                    let addr_bytes = buf[0..ObjectRef::SIZE].try_into().unwrap();
                    let type_bytes = buf[ObjectRef::SIZE..ManagedPtr::SIZE].try_into().unwrap();
                    let addr = usize::from_ne_bytes(addr_bytes);
                    let type_ptr = sptr::from_exposed_addr::<TypeDescription>(
                        usize::from_ne_bytes(type_bytes),
                    );

                    if type_ptr.is_null() {
                        return Err(TypeResolutionError::InvalidHandle);
                    }

                    // SAFETY: Reconstructing Arc from raw pointer stored in memory.
                    let type_desc = unsafe {
                        let arc = Arc::from_raw(type_ptr);
                        let clone = arc.clone();
                        let _ = Arc::into_raw(arc);
                        clone
                    };

                    let m = ManagedPtr::new(
                        NonNull::new(sptr::from_exposed_addr_mut(addr)),
                        *type_desc.clone(),
                        None,
                        false,
                        Some(dotnet_utils::ByteOffset(0)),
                    );
                    return Ok(CTSValue::Value(TypedRef(m, type_desc)));
                }

                let instance = self.new_object(td, &new_ctx)?;
                let layout = instance.instance_storage.layout().clone();
                let mut storage = instance.instance_storage.get_mut();

                if layout.has_ref_fields {
                    for (key, field_layout) in &layout.fields {
                        let pos = field_layout.position.as_usize();
                        let size = field_layout.layout.size().as_usize();
                        let field_data = &data[pos..pos + size];

                        if field_layout.layout.has_managed_ptrs() {
                            let field_info = td
                                .definition()
                                .fields
                                .iter()
                                .find(|f| f.name == key.name)
                                .expect("field not found during read_cts_value patching");

                            let field_desc = FieldDescription {
                                parent: td,
                                field_resolution: td.resolution,
                                field: field_info,
                                index: 0,
                            };
                            let field_type =
                                self.get_field_type(td.resolution, new_ctx.generics, field_desc)?;

                            let val = self.read_cts_value(&field_type, field_data, gc, &new_ctx)?;
                            val.write(&mut storage[pos..pos + size]);
                        } else {
                            storage[pos..pos + size].copy_from_slice(field_data);
                        }
                    }
                } else {
                    storage.copy_from_slice(data);
                }

                drop(storage);
                Ok(CTSValue::Value(Struct(instance)))
            }
        }
    }

    pub fn new_vector<'gc>(
        &self,
        element: ConcreteType,
        size: usize,
        ctx: &ResolutionContext<'_, 'm>,
    ) -> Result<Vector<'gc>, TypeResolutionError> {
        let layout = crate::layout::LayoutFactory::create_array_layout_with_metrics(
            element.clone(),
            size,
            ctx,
            self.metrics(),
        )?;
        let total_size = layout.element_layout.size() * size;
        if total_size.as_usize() > 0x7FFF_FFFF {
            return Err(TypeResolutionError::MassiveAllocation(format!(
                "attempted to allocate massive vector of {} bytes (element: {:?}, length: {})",
                total_size, element, size
            )));
        }

        Ok(Vector::new(
            element,
            layout,
            vec![0; total_size.as_usize()],
            vec![size],
        ))
    }
}

fn convert_num<T: TryFrom<i32> + TryFrom<isize> + TryFrom<usize>>(data: StackValue<'_>) -> T {
    match data {
        StackValue::Int32(i) => i
            .try_into()
            .unwrap_or_else(|_| panic!("failed to convert from i32")),
        StackValue::NativeInt(i) => i
            .try_into()
            .unwrap_or_else(|_| panic!("failed to convert from isize")),
        StackValue::UnmanagedPtr(p) => {
            p.0.as_ptr()
                .expose_addr()
                .try_into()
                .unwrap_or_else(|_| panic!("failed to convert from pointer"))
        }
        StackValue::ManagedPtr(p) => {
            let ptr = unsafe { p.with_data(0, |data| data.as_ptr()) };
            ptr.expose_addr()
                .try_into()
                .unwrap_or_else(|_| panic!("failed to convert from pointer"))
        }
        other => panic!(
            "invalid stack value {:?} for conversion into {}",
            other,
            any::type_name::<T>()
        ),
    }
}

fn convert_i64<T: TryFrom<i64>>(data: StackValue<'_>) -> T
where
    T::Error: Error,
{
    match data {
        StackValue::Int64(i) => i.try_into().unwrap_or_else(|e| {
            panic!(
                "failed to convert from i64 to {} ({})",
                any::type_name::<T>(),
                e
            )
        }),
        other => panic!("invalid stack value {:?} for integer conversion", other),
    }
}

fn reinterpret_i64_as_u64(data: StackValue<'_>) -> u64 {
    match data {
        StackValue::Int64(i) => i as u64,
        other => panic!("invalid stack value {:?} for u64 reinterpretation", other),
    }
}

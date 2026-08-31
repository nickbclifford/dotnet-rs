use crate::{ResolverExecutionContext, ResolverService};
use dotnet_types::{
    TypeDescription, WellKnown,
    comparer::decompose_type_source,
    error::TypeResolutionError,
    generics::{ConcreteType, GenericLookup},
    members::FieldDescription,
    resolution::ResolutionS,
};
use dotnet_utils::{ByteOffset, gc::GCHandle};
use dotnet_value::{
    StackValue,
    cts_cli_conversion::{CliToCts, CtsScalarKind},
    layout::{FieldLayoutManager, HasLayout},
    object::{CTSValue, HeapStorage, Object, ObjectRef, ValueType, Vector},
    pointer::{ManagedPtr, NoManagedPtrResolver, unmanaged_ptr_from_addr},
    storage::FieldStorage,
};
use dotnetdll::prelude::*;
use std::{ptr::NonNull, sync::Arc};

fn copy_overlapping_field_storage(src: &FieldStorage, dst: &FieldStorage) {
    src.with_data(|src| {
        dst.with_data_mut(|dst| {
            let copy_len = src.len().min(dst.len());
            dst[..copy_len].copy_from_slice(&src[..copy_len]);
        });
    });
}

impl<C, L> ResolverService<C, L>
where
    C: crate::ResolverCacheAdapter,
    L: crate::ResolverLayoutAdapter,
{
    pub fn new_object<'gc, Ctx: ResolverExecutionContext>(
        &self,
        td: TypeDescription,
        ctx: &Ctx,
    ) -> Result<Object<'gc>, TypeResolutionError> {
        self.new_object_with_lookup(td, ctx.resolution().clone(), ctx.generics())
    }

    pub fn box_value<'gc, Ctx: ResolverExecutionContext>(
        &self,
        t: &ConcreteType,
        data: StackValue<'gc>,
        gc: GCHandle<'gc>,
        ctx: &Ctx,
    ) -> Result<ObjectRef<'gc>, TypeResolutionError> {
        let t = self.normalize_type(t.clone())?;

        if self.loader.find_concrete_type(t.clone())?.type_name() == "System.TypedReference" {
            return Err(TypeResolutionError::InvalidLayout(
                "ECMA-335 typedref values cannot be boxed".into(),
            ));
        }

        // ECMA-335 §I.8.2.4: Boxing Nullable<T> produces null if HasValue=false, or boxed T if HasValue=true.
        if t.is_nullable(self.loader.as_ref())
            && let StackValue::ValueType(obj) = &data
        {
            let layout = obj.instance_storage.layout();
            let has_value_field = layout
                .fields
                .iter()
                .find(|(k, _)| k.name == "hasValue" || k.name == "_hasValue")
                .map(|(_, v)| v);

            if let Some(field_layout) = has_value_field {
                let pos = field_layout.position.as_usize();
                let has_value = obj.instance_storage.with_data(|d| d[pos]) != 0;

                if !has_value {
                    return Ok(ObjectRef(None));
                }

                // HasValue is true, box the 'value' field.
                // BCL-dynamic layout probe — see REVIEW.md §4 (F-SCOPE-001)
                let value_field = layout
                    .fields
                    .iter()
                    .find(|(k, _)| k.name == "value" || k.name == "_value")
                    .expect("Nullable<T> must have a value field");

                let (_, value_field_layout) = value_field;
                let value_pos = value_field_layout.position.as_usize();
                let value_size = value_field_layout.layout.size().as_usize();

                // T is the first generic argument of Nullable<T>
                let source = t.get();
                let inner_t = if let BaseType::Type {
                    source: TypeSource::Generic { parameters, .. },
                    ..
                } = source
                {
                    parameters
                        .first()
                        .ok_or(TypeResolutionError::InvalidHandle)?
                } else {
                    return Err(TypeResolutionError::InvalidHandle);
                };

                let cts_value = obj.instance_storage.with_data(|d| {
                    let value_data = &d[value_pos..value_pos + value_size];
                    self.read_cts_value_with_lookup(
                        inner_t,
                        value_data,
                        gc,
                        inner_t.resolution(),
                        ctx.generics(),
                    )
                })?;
                return self.box_value(inner_t, cts_value.into_stack(), gc, ctx);
            }
        }

        match self.new_cts_value_with_lookup(&t, data, t.resolution(), ctx.generics())? {
            CTSValue::Value(v) => {
                let td = self.loader.find_concrete_type(t.clone())?;
                let boxed_lookup = t.make_lookup();
                let obj_instance =
                    self.new_object_with_lookup(td, ctx.resolution().clone(), &boxed_lookup)?;
                let dest_len = obj_instance.instance_storage.with_data(|data| data.len());
                match v {
                    ValueType::Struct(source) => {
                        obj_instance.instance_storage.with_data_mut(|dest| {
                            source.instance_storage.with_data(|src| {
                                let copy_len = src.len().min(dest.len());
                                dest[..copy_len].copy_from_slice(&src[..copy_len]);
                            });
                        });
                    }
                    scalar => {
                        let scalar_size = scalar.size_bytes();
                        if scalar_size != dest_len {
                            return Err(TypeResolutionError::InvalidLayout(
                                format!(
                                    "box_value size mismatch for {}: source={}, destination={}",
                                    obj_instance.description.type_name(),
                                    scalar_size,
                                    dest_len
                                )
                                .into(),
                            ));
                        }
                        obj_instance.instance_storage.with_data_mut(|dest| {
                            CTSValue::Value(scalar).write(dest);
                        });
                    }
                }
                Ok(ObjectRef::new(
                    gc,
                    HeapStorage::Boxed(Box::new(obj_instance)),
                ))
            }
            CTSValue::Ref(r) => Ok(r),
        }
    }

    pub fn new_instance_fields<Ctx: ResolverExecutionContext>(
        &self,
        td: TypeDescription,
        ctx: &Ctx,
    ) -> Result<FieldStorage, TypeResolutionError> {
        self.new_instance_fields_with_lookup(td, ctx.resolution().clone(), ctx.generics())
    }

    pub fn new_static_fields<Ctx: ResolverExecutionContext>(
        &self,
        td: TypeDescription,
        ctx: &Ctx,
    ) -> Result<FieldStorage, TypeResolutionError> {
        self.new_static_fields_with_lookup(td, ctx.resolution().clone(), ctx.generics())
    }

    pub fn new_value_type<'gc, Ctx: ResolverExecutionContext>(
        &self,
        t: &ConcreteType,
        data: StackValue<'gc>,
        ctx: &Ctx,
    ) -> Result<ValueType<'gc>, TypeResolutionError> {
        self.new_value_type_with_lookup(t, data, t.resolution(), ctx.generics())
    }

    pub fn new_cts_value<'gc, Ctx: ResolverExecutionContext>(
        &self,
        t: &ConcreteType,
        data: StackValue<'gc>,
        ctx: &Ctx,
    ) -> Result<CTSValue<'gc>, TypeResolutionError> {
        self.new_cts_value_with_lookup(t, data, t.resolution(), ctx.generics())
    }

    pub fn read_cts_value<'gc, Ctx: ResolverExecutionContext>(
        &self,
        t: &ConcreteType,
        data: &[u8],
        gc: GCHandle<'gc>,
        ctx: &Ctx,
    ) -> Result<CTSValue<'gc>, TypeResolutionError> {
        self.read_cts_value_with_lookup(t, data, gc, t.resolution(), ctx.generics())
    }

    pub fn new_vector<'gc, Ctx: ResolverExecutionContext>(
        &self,
        element: ConcreteType,
        size: usize,
        ctx: &Ctx,
    ) -> Result<Vector<'gc>, TypeResolutionError> {
        self.new_vector_with_lookup(element.clone(), size, element.resolution(), ctx.generics())
    }

    fn new_object_with_lookup<'gc>(
        &self,
        td: TypeDescription,
        resolution: ResolutionS,
        generics: &GenericLookup,
    ) -> Result<Object<'gc>, TypeResolutionError> {
        let has_finalizer = self.has_finalizer(td.clone())?;
        Ok(Object::new_with_finalizer_flag(
            td.clone(),
            generics.clone(),
            self.new_instance_fields_with_lookup(td, resolution, generics)?,
            has_finalizer,
        ))
    }

    fn new_instance_fields_with_lookup(
        &self,
        td: TypeDescription,
        _resolution: ResolutionS,
        generics: &GenericLookup,
    ) -> Result<FieldStorage, TypeResolutionError> {
        let layout = self.instance_field_layout_cached_with_lookup(td, generics)?;
        Ok(self.new_field_storage(layout))
    }

    fn new_static_fields_with_lookup(
        &self,
        td: TypeDescription,
        _resolution: ResolutionS,
        generics: &GenericLookup,
    ) -> Result<FieldStorage, TypeResolutionError> {
        let layout = Arc::new(self.static_fields_with_lookup(td, generics)?);
        Ok(self.new_field_storage(layout))
    }

    fn new_field_storage(&self, layout: Arc<FieldLayoutManager>) -> FieldStorage {
        let size = layout.size();
        FieldStorage::new(layout, vec![0; size.as_usize()])
    }

    fn resolve_pointer_inner_type(
        &self,
        inner: &Option<ConcreteType>,
    ) -> Result<TypeDescription, TypeResolutionError> {
        if let Some(source) = inner {
            self.loader.find_concrete_type(source.clone())
        } else {
            self.loader.corlib_wkt(WellKnown::Void)
        }
    }

    fn new_value_type_with_lookup<'gc>(
        &self,
        t: &ConcreteType,
        data: StackValue<'gc>,
        resolution: ResolutionS,
        generics: &GenericLookup,
    ) -> Result<ValueType<'gc>, TypeResolutionError> {
        match self.new_cts_value_with_lookup(t, data, resolution, generics)? {
            CTSValue::Value(v) => Ok(v),
            CTSValue::Ref(r) => Err(TypeResolutionError::InvalidLayout(
                format!(
                    "tried to instantiate value type, received object reference ({:?})",
                    r
                )
                .into(),
            )),
        }
    }

    fn new_cts_value_with_lookup<'gc>(
        &self,
        t: &ConcreteType,
        data: StackValue<'gc>,
        resolution: ResolutionS,
        _generics: &GenericLookup,
    ) -> Result<CTSValue<'gc>, TypeResolutionError> {
        use ValueType::*;
        let t = self.normalize_type(t.clone())?;
        match t.get() {
            BaseType::Boolean => Ok(CTSValue::Value(CliToCts::narrow_scalar_value(
                data,
                CtsScalarKind::Boolean,
            )?)),
            BaseType::Char => Ok(CTSValue::Value(CliToCts::narrow_scalar_value(
                data,
                CtsScalarKind::Char,
            )?)),
            BaseType::Int8 => Ok(CTSValue::Value(CliToCts::narrow_scalar_value(
                data,
                CtsScalarKind::Int8,
            )?)),
            BaseType::UInt8 => Ok(CTSValue::Value(CliToCts::narrow_scalar_value(
                data,
                CtsScalarKind::UInt8,
            )?)),
            BaseType::Int16 => Ok(CTSValue::Value(CliToCts::narrow_scalar_value(
                data,
                CtsScalarKind::Int16,
            )?)),
            BaseType::UInt16 => Ok(CTSValue::Value(CliToCts::narrow_scalar_value(
                data,
                CtsScalarKind::UInt16,
            )?)),
            BaseType::Int32 => Ok(CTSValue::Value(CliToCts::narrow_scalar_value(
                data,
                CtsScalarKind::Int32,
            )?)),
            BaseType::UInt32 => Ok(CTSValue::Value(CliToCts::narrow_scalar_value(
                data,
                CtsScalarKind::UInt32,
            )?)),
            BaseType::Int64 => Ok(CTSValue::Value(CliToCts::narrow_scalar_value(
                data,
                CtsScalarKind::Int64,
            )?)),
            BaseType::UInt64 => Ok(CTSValue::Value(CliToCts::narrow_scalar_value(
                data,
                CtsScalarKind::UInt64,
            )?)),
            BaseType::Float32 => Ok(CTSValue::Value(CliToCts::narrow_float32(data)?)),
            BaseType::Float64 => Ok(CTSValue::Value(CliToCts::narrow_float64(data)?)),
            BaseType::IntPtr => Ok(CTSValue::Value(CliToCts::narrow_scalar_value(
                data,
                CtsScalarKind::NativeInt,
            )?)),
            BaseType::UIntPtr | BaseType::FunctionPointer(_) => Ok(CTSValue::Value(
                CliToCts::narrow_scalar_value(data, CtsScalarKind::NativeUInt)?,
            )),
            BaseType::ValuePointer(_modifiers, inner) => match data {
                StackValue::ManagedPtr(p) => Ok(CTSValue::Value(Pointer(p.into_inner()))),
                StackValue::UnmanagedPtr(p) => {
                    let inner_type = self.resolve_pointer_inner_type(inner)?;
                    Ok(CTSValue::Value(Pointer(ManagedPtr::new(
                        Some(p.0),
                        inner_type,
                        None,
                        false,
                        Some(ByteOffset::new(p.0.as_ptr().addr())),
                    ))))
                }
                _ => {
                    let ptr = CliToCts::convert_num::<usize>(data)?;
                    let inner_type = self.resolve_pointer_inner_type(inner)?;
                    Ok(CTSValue::Value(Pointer(ManagedPtr::new(
                        // SAFETY: F10.RawMemoryAccessValid — Scalar-to-ValuePointer conversion is the CLI
                        // unmanaged-address boundary; no managed origin exists, so the
                        // producer supplies the resulting pointer's validity contract.
                        NonNull::new(unsafe { unmanaged_ptr_from_addr(ptr) }),
                        inner_type,
                        None,
                        false,
                        Some(ByteOffset::new(ptr)),
                    ))))
                }
            },
            BaseType::Object
            | BaseType::String
            | BaseType::Vector(_, _)
            | BaseType::Array(_, _)
            | BaseType::Type {
                value_kind: Some(ValueKind::Class),
                ..
            } => {
                if let StackValue::ObjectRef(o) = data {
                    Ok(CTSValue::Ref(o))
                } else {
                    Err(TypeResolutionError::InvalidLayout(
                        format!("expected ObjectRef, got {:?}", data).into(),
                    ))
                }
            }
            BaseType::Type {
                value_kind: None | Some(ValueKind::ValueType),
                source,
            } => {
                let (ut, type_generics) = decompose_type_source(source);
                let new_lookup = GenericLookup::new(type_generics);
                let td = self.locate_type(resolution.clone(), ut)?;

                if !self.is_value_type(td.clone())? {
                    return Ok(CTSValue::Ref(if let StackValue::ObjectRef(r) = data {
                        r
                    } else {
                        return Err(TypeResolutionError::InvalidLayout(
                            format!("expected ObjectRef, got {:?}", data).into(),
                        ));
                    }));
                }

                if let Some(e) = td.is_enum() {
                    let enum_type = self.make_concrete(resolution.clone(), &new_lookup, e)?;
                    return self.new_cts_value_with_lookup(
                        &enum_type,
                        data,
                        resolution.clone(),
                        &new_lookup,
                    );
                }

                if td.type_name() == "System.TypedReference" {
                    return Err(TypeResolutionError::InvalidLayout(
                        "ECMA-335 typedref values may only occupy parameters and locals".into(),
                    ));
                }

                if let StackValue::ValueType(mut o) = data {
                    let expected_layout =
                        self.instance_field_layout_cached_with_lookup(td.clone(), &new_lookup)?;

                    let needs_canonicalization = o.description != td
                        || o.generics != new_lookup
                        || !Arc::ptr_eq(o.instance_storage.layout(), &expected_layout);

                    if needs_canonicalization {
                        // Keep description/generics/storage layout in sync when coercing value types.
                        // Generic instantiation can change GC descriptors even when the type
                        // definition is the same (e.g., Nullable<T>), so we canonicalize by
                        // target context and preserve overlapping payload bytes.
                        let replacement = FieldStorage::new(
                            expected_layout.clone(),
                            vec![0; expected_layout.size().as_usize()],
                        );
                        copy_overlapping_field_storage(&o.instance_storage, &replacement);
                        o.description = td.clone();
                        o.generics = new_lookup.clone();
                        o.instance_storage = replacement;
                    }
                    Ok(CTSValue::Value(Struct(o)))
                } else {
                    let mut instance =
                        self.new_object_with_lookup(td.clone(), resolution.clone(), &new_lookup)?;
                    if let StackValue::ObjectRef(o) = data
                        && let Some(handle) = o.0
                    {
                        let borrowed = handle.borrow();
                        match &borrowed.storage {
                            HeapStorage::Obj(obj) | HeapStorage::Boxed(obj) => {
                                // Unboxing: keep description/storage layout in sync.
                                // The source obj may have a different type than td, so rebuild
                                // storage with the target layout and copy overlapping data.
                                let replacement = self.new_instance_fields_with_lookup(
                                    td,
                                    resolution,
                                    &new_lookup,
                                )?;
                                copy_overlapping_field_storage(&obj.instance_storage, &replacement);
                                instance.instance_storage = replacement;
                            }
                            _ => {
                                return Err(TypeResolutionError::InvalidLayout(
                                    "cannot unbox from non-object storage".into(),
                                ));
                            }
                        }
                    }
                    Ok(CTSValue::Value(Struct(instance)))
                }
            }
        }
    }

    fn read_cts_value_with_lookup<'gc>(
        &self,
        t: &ConcreteType,
        data: &[u8],
        gc: GCHandle<'gc>,
        resolution: ResolutionS,
        _generics: &GenericLookup,
    ) -> Result<CTSValue<'gc>, TypeResolutionError> {
        use ValueType::*;
        let t = self.normalize_type(t.clone())?;
        match t.get() {
            BaseType::Boolean => Ok(CTSValue::Value(CliToCts::read_scalar_storage(
                CtsScalarKind::Boolean,
                data,
            ))),
            BaseType::Char => Ok(CTSValue::Value(CliToCts::read_scalar_storage(
                CtsScalarKind::Char,
                data,
            ))),
            BaseType::Int8 => Ok(CTSValue::Value(CliToCts::read_scalar_storage(
                CtsScalarKind::Int8,
                data,
            ))),
            BaseType::UInt8 => Ok(CTSValue::Value(CliToCts::read_scalar_storage(
                CtsScalarKind::UInt8,
                data,
            ))),
            BaseType::Int16 => Ok(CTSValue::Value(CliToCts::read_scalar_storage(
                CtsScalarKind::Int16,
                data,
            ))),
            BaseType::UInt16 => Ok(CTSValue::Value(CliToCts::read_scalar_storage(
                CtsScalarKind::UInt16,
                data,
            ))),
            BaseType::Int32 => Ok(CTSValue::Value(CliToCts::read_scalar_storage(
                CtsScalarKind::Int32,
                data,
            ))),
            BaseType::UInt32 => Ok(CTSValue::Value(CliToCts::read_scalar_storage(
                CtsScalarKind::UInt32,
                data,
            ))),
            BaseType::Int64 => Ok(CTSValue::Value(CliToCts::read_scalar_storage(
                CtsScalarKind::Int64,
                data,
            ))),
            BaseType::UInt64 => Ok(CTSValue::Value(CliToCts::read_scalar_storage(
                CtsScalarKind::UInt64,
                data,
            ))),
            BaseType::Float32 => Ok(CTSValue::Value(Float32(f32::from_ne_bytes(
                data.try_into()
                    .expect("caller slices data to this scalar's layout-computed width"),
            )))),
            BaseType::Float64 => Ok(CTSValue::Value(Float64(f64::from_ne_bytes(
                data.try_into()
                    .expect("caller slices data to this scalar's layout-computed width"),
            )))),
            BaseType::IntPtr => Ok(CTSValue::Value(CliToCts::read_scalar_storage(
                CtsScalarKind::NativeInt,
                data,
            ))),
            BaseType::UIntPtr | BaseType::FunctionPointer(_) => Ok(CTSValue::Value(
                CliToCts::read_scalar_storage(CtsScalarKind::NativeUInt, data),
            )),
            BaseType::ValuePointer(_modifiers, inner) => {
                let inner_type = self.resolve_pointer_inner_type(inner)?;

                if data.len() >= ManagedPtr::SIZE {
                    // SAFETY: F3.InteriorPointerRebased; F6.NoEscapeAcrossArena —
                    // `data.len() >= ManagedPtr::SIZE` is checked above, and VM-managed storage
                    // supplies a complete ManagedPtr encoding whose recorded origin and offset
                    // are resolved under `gc`'s arena brand.
                    let info = unsafe {
                        ManagedPtr::read_resolved_branded(
                            &data[..ManagedPtr::SIZE],
                            &gc,
                            &NoManagedPtrResolver,
                        )
                    }
                    .expect("read_cts_value: ManagedPtr deserialization failed");
                    let m = ManagedPtr::from_info_full(info, inner_type, false);
                    Ok(CTSValue::Value(Pointer(m)))
                } else {
                    let mut ptr_bytes = [0u8; ObjectRef::SIZE];
                    ptr_bytes.copy_from_slice(&data[0..ObjectRef::SIZE]);
                    let ptr = usize::from_ne_bytes(ptr_bytes);
                    // SAFETY: F10.RawMemoryAccessValid — This legacy `ValuePointer`
                    // representation is a raw-address storage boundary. `data` contains no
                    // owner or origin handle from which provenance could be recovered, so its
                    // producer must ensure the address remains valid for the resulting unmanaged
                    // pointer's use.
                    Ok(CTSValue::Value(Pointer(ManagedPtr::new(
                        // SAFETY: F10.RawMemoryAccessValid — The preceding raw-storage boundary
                        // contract supplies the unmanaged pointer validity premise.
                        NonNull::new(unsafe { unmanaged_ptr_from_addr(ptr) }),
                        inner_type,
                        None,
                        false,
                        Some(ByteOffset::new(ptr)),
                    ))))
                }
            }
            BaseType::Object
            | BaseType::String
            | BaseType::Vector(_, _)
            | BaseType::Array(_, _) => {
                // SAFETY: F2.DescriptorMatchesEcmaLayout; F1.GcHandleRooted — These
                // reference-like CLR types select the `ObjectRef` layout, and `data` is provided
                // by managed storage with the expected object-reference width under `gc`'s arena
                // brand.
                Ok(CTSValue::Ref(unsafe { ObjectRef::read_branded(data, &gc) }))
            }
            BaseType::Type {
                value_kind: Some(ValueKind::Class),
                ..
            } => {
                // SAFETY: F2.DescriptorMatchesEcmaLayout; F1.GcHandleRooted — Class-typed values
                // select `ObjectRef` payloads in managed storage and use the same `gc`-branded
                // deserialization contract as other reference types.
                Ok(CTSValue::Ref(unsafe { ObjectRef::read_branded(data, &gc) }))
            }
            BaseType::Type {
                value_kind: None | Some(ValueKind::ValueType),
                source,
            } => {
                let (ut, type_generics) = decompose_type_source(source);
                let new_lookup = GenericLookup::new(type_generics);
                let td = self.locate_type(resolution.clone(), ut)?;

                if !self.is_value_type(td.clone())? {
                    // SAFETY: F2.DescriptorMatchesEcmaLayout; F1.GcHandleRooted — Non-value
                    // `Type` instances select runtime object references serialized as `ObjectRef`
                    // bytes in managed storage and decoded under `gc`'s arena brand.
                    return Ok(CTSValue::Ref(unsafe { ObjectRef::read_branded(data, &gc) }));
                }

                if let Some(e) = td.is_enum() {
                    let enum_type = self.make_concrete(resolution.clone(), &new_lookup, e)?;
                    return self.read_cts_value_with_lookup(
                        &enum_type,
                        data,
                        gc,
                        resolution.clone(),
                        &new_lookup,
                    );
                }

                if td.type_name() == "System.TypedReference" {
                    return Err(TypeResolutionError::InvalidLayout(
                        "ECMA-335 typedref values may only occupy parameters and locals; they cannot be read from managed storage".into(),
                    ));
                }

                let instance =
                    self.new_object_with_lookup(td.clone(), resolution.clone(), &new_lookup)?;
                let layout = instance.instance_storage.layout().clone();

                instance.instance_storage.with_data_mut(|storage| {
                    if layout.has_ref_fields {
                        for (key, field_layout) in &layout.fields {
                            let pos = field_layout.position.as_usize();
                            let size = field_layout.layout.size().as_usize();
                            let field_data = &data[pos..pos + size];

                            if field_layout.layout.has_managed_ptrs() {
                                let index = td
                                    .definition()
                                    .fields
                                    .iter()
                                    .position(|f| f.name == key.name)
                                    .expect("field not found during read_cts_value patching");

                                let field_desc =
                                    FieldDescription::new(td.clone(), td.resolution.clone(), index);
                                let field_type = self.get_field_type(
                                    resolution.clone(),
                                    &new_lookup,
                                    field_desc,
                                )?;

                                let val = self.read_cts_value_with_lookup(
                                    &field_type,
                                    field_data,
                                    gc,
                                    resolution.clone(),
                                    &new_lookup,
                                )?;
                                val.write(&mut storage[pos..pos + size]);
                            } else {
                                storage[pos..pos + size].copy_from_slice(field_data);
                            }
                        }
                    } else {
                        storage.copy_from_slice(data);
                    }
                    Ok::<_, TypeResolutionError>(())
                })?;

                Ok(CTSValue::Value(Struct(instance)))
            }
        }
    }

    fn new_vector_with_lookup<'gc>(
        &self,
        element: ConcreteType,
        size: usize,
        resolution: ResolutionS,
        generics: &GenericLookup,
    ) -> Result<Vector<'gc>, TypeResolutionError> {
        let layout =
            self.create_array_layout_with_lookup(element.clone(), size, resolution, generics)?;
        let total_size = layout.element_layout.size() * size;
        if total_size.as_usize() > 0x7FFF_FFFF {
            return Err(TypeResolutionError::MassiveAllocation(
                format!(
                    "attempted to allocate massive vector of {} bytes (element: {:?}, length: {})",
                    total_size, element, size
                )
                .into(),
            ));
        }

        Ok(Vector::new(
            element,
            layout,
            vec![0; total_size.as_usize()],
            vec![size],
        ))
    }
}

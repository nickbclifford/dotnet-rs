use crate::{ResolverExecutionContext, ResolverService};
use dotnet_types::{
    TypeDescription,
    comparer::decompose_type_source,
    error::TypeResolutionError,
    generics::{ConcreteType, GenericLookup},
    members::FieldDescription,
    resolution::ResolutionS,
};
use dotnet_utils::gc::GCHandle;
use dotnet_value::{
    ByteOffset, StackValue,
    layout::HasLayout,
    object::{CTSValue, HeapStorage, Object, ObjectRef, ValueType, Vector},
    pointer::ManagedPtr,
    storage::FieldStorage,
};
use dotnetdll::prelude::*;
use sptr::Strict;
use std::{any, error::Error, ptr::NonNull, sync::Arc};

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

        // ECMA-335 §I.8.2.4: Boxing Nullable<T> produces null if HasValue=false, or boxed T if HasValue=true.
        if t.is_nullable(self.loader.as_ref()) && matches!(data, StackValue::ValueType(_)) {
            let StackValue::ValueType(obj) = &data else {
                unreachable!()
            };
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
                        ctx.resolution().clone(),
                        ctx.generics(),
                    )
                })?;
                return self.box_value(inner_t, cts_value.into_stack(), gc, ctx);
            }
        }

        match self.new_cts_value_with_lookup(&t, data, ctx.resolution().clone(), ctx.generics())? {
            CTSValue::Value(v) => {
                let td = self.loader.find_concrete_type(t)?;
                let obj_instance =
                    self.new_object_with_lookup(td, ctx.resolution().clone(), ctx.generics())?;
                let size = v.size_bytes();
                obj_instance.instance_storage.with_data_mut(|data| {
                    CTSValue::Value(v).write(&mut data[..size]);
                });
                Ok(ObjectRef::new(gc, HeapStorage::Boxed(obj_instance)))
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
        self.new_value_type_with_lookup(t, data, ctx.resolution().clone(), ctx.generics())
    }

    pub fn new_cts_value<'gc, Ctx: ResolverExecutionContext>(
        &self,
        t: &ConcreteType,
        data: StackValue<'gc>,
        ctx: &Ctx,
    ) -> Result<CTSValue<'gc>, TypeResolutionError> {
        self.new_cts_value_with_lookup(t, data, ctx.resolution().clone(), ctx.generics())
    }

    pub fn read_cts_value<'gc, Ctx: ResolverExecutionContext>(
        &self,
        t: &ConcreteType,
        data: &[u8],
        gc: GCHandle<'gc>,
        ctx: &Ctx,
    ) -> Result<CTSValue<'gc>, TypeResolutionError> {
        self.read_cts_value_with_lookup(t, data, gc, ctx.resolution().clone(), ctx.generics())
    }

    pub fn new_vector<'gc, Ctx: ResolverExecutionContext>(
        &self,
        element: ConcreteType,
        size: usize,
        ctx: &Ctx,
    ) -> Result<Vector<'gc>, TypeResolutionError> {
        self.new_vector_with_lookup(element, size, ctx.resolution().clone(), ctx.generics())
    }

    fn new_object_with_lookup<'gc>(
        &self,
        td: TypeDescription,
        resolution: ResolutionS,
        generics: &GenericLookup,
    ) -> Result<Object<'gc>, TypeResolutionError> {
        Ok(Object::new(
            td.clone(),
            generics.clone(),
            self.new_instance_fields_with_lookup(td, resolution, generics)?,
        ))
    }

    fn new_instance_fields_with_lookup(
        &self,
        td: TypeDescription,
        resolution: ResolutionS,
        generics: &GenericLookup,
    ) -> Result<FieldStorage, TypeResolutionError> {
        let layout = self.instance_field_layout_cached_with_lookup(td, resolution, generics)?;
        let size = layout.size();
        Ok(FieldStorage::new(layout, vec![0; size.as_usize()]))
    }

    fn new_static_fields_with_lookup(
        &self,
        td: TypeDescription,
        resolution: ResolutionS,
        generics: &GenericLookup,
    ) -> Result<FieldStorage, TypeResolutionError> {
        let layout = Arc::new(self.static_fields_with_lookup(td, resolution, generics)?);
        let size = layout.size();
        Ok(FieldStorage::new(layout, vec![0; size.as_usize()]))
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
            CTSValue::Ref(r) => {
                panic!(
                    "tried to instantiate value type, received object reference ({:?})",
                    r
                )
            }
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
                        NonNull::new(sptr::from_exposed_addr_mut(ptr)),
                        inner_type,
                        None,
                        false,
                        Some(ByteOffset(ptr)),
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
                let td = self.locate_type(resolution.clone(), ut)?;

                if !self.is_value_type(td.clone())? {
                    return Ok(CTSValue::Ref(if let StackValue::ObjectRef(r) = data {
                        r
                    } else {
                        panic!("expected ObjectRef, got {:?}", data)
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
                    let StackValue::TypedRef(p, t) = data else {
                        panic!("expected TypedRef, got {:?}", data);
                    };
                    return Ok(CTSValue::Value(TypedRef(p, t)));
                }

                if let StackValue::ValueType(mut o) = data {
                    let expected_layout = self.instance_field_layout_cached_with_lookup(
                        td.clone(),
                        resolution.clone(),
                        &new_lookup,
                    )?;

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
                        o.instance_storage.with_data(|src| {
                            replacement.with_data_mut(|dst| {
                                let copy_len = src.len().min(dst.len());
                                dst[..copy_len].copy_from_slice(&src[..copy_len]);
                            });
                        });
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
                                obj.instance_storage.with_data(|src| {
                                    replacement.with_data_mut(|dst| {
                                        let copy_len = src.len().min(dst.len());
                                        dst[..copy_len].copy_from_slice(&src[..copy_len]);
                                    });
                                });
                                instance.instance_storage = replacement;
                            }
                            _ => panic!("cannot unbox from non-object storage"),
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
                        Some(ByteOffset(ptr)),
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
                let td = self.locate_type(resolution.clone(), ut)?;

                if !self.is_value_type(td.clone())? {
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
                        (*type_desc).clone(),
                        None,
                        false,
                        Some(ByteOffset(0)),
                    );
                    return Ok(CTSValue::Value(TypedRef(m, type_desc)));
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

use crate::{context::ResolutionContext, layout::LayoutFactory};
use dotnet_assemblies::decompose_type_source;
use dotnet_types::{
    generics::{ConcreteType, GenericLookup},
    TypeDescription,
};
use dotnet_utils::gc::GCHandle;
use dotnet_value::{
    layout::HasLayout,
    object::{CTSValue, Object, ValueType, Vector},
    pointer::ManagedPtr,
    storage::FieldStorage,
    StackValue,
};
use dotnetdll::prelude::{BaseType, ValueKind};
use std::{any, error::Error, ptr::NonNull, sync::Arc};

pub trait TypeResolutionExt {
    fn is_value_type(&self, ctx: &ResolutionContext) -> bool;
    fn has_finalizer(&self, ctx: &ResolutionContext) -> bool;
}

pub trait ValueResolution {
    fn stack_value_type(&self, val: &StackValue) -> TypeDescription;
    fn new_object<'gc>(&self, td: TypeDescription) -> Object<'gc>;
    fn new_instance_fields(&self, td: TypeDescription) -> FieldStorage;
    fn new_static_fields(&self, td: TypeDescription) -> FieldStorage;
    fn new_value_type<'gc>(&self, t: &ConcreteType, data: StackValue<'gc>) -> ValueType<'gc>;
    fn value_type_description<'gc>(&self, vt: &ValueType<'gc>) -> TypeDescription;
    fn new_cts_value<'gc>(&self, t: &ConcreteType, data: StackValue<'gc>) -> CTSValue<'gc>;
    fn read_cts_value<'gc>(
        &self,
        t: &ConcreteType,
        data: &[u8],
        gc: GCHandle<'gc>,
    ) -> CTSValue<'gc>;
    fn new_vector<'gc>(&self, element: ConcreteType, size: usize) -> Vector<'gc>;
}

impl<'a, 'm> ValueResolution for ResolutionContext<'a, 'm> {
    fn stack_value_type(&self, val: &StackValue) -> TypeDescription {
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
            StackValue::ManagedPtr(m) => m.inner_type,
            StackValue::ValueType(o) => o.description,
            #[cfg(feature = "multithreaded-gc")]
            StackValue::CrossArenaObjectRef(_, _) => {
                todo!("handle CrossArenaObjectRef in contains_type")
            }
        }
    }

    fn new_object<'gc>(&self, td: TypeDescription) -> Object<'gc> {
        Object::new(td, self.new_instance_fields(td))
    }

    fn new_instance_fields(&self, td: TypeDescription) -> FieldStorage {
        let layout = LayoutFactory::instance_field_layout_cached(td, self, None);
        let size = layout.size();
        FieldStorage::new(layout, vec![0; size])
    }

    fn new_static_fields(&self, td: TypeDescription) -> FieldStorage {
        let layout = Arc::new(LayoutFactory::static_fields(td, self));
        let size = layout.size();
        FieldStorage::new(layout, vec![0; size])
    }

    fn new_value_type<'gc>(&self, t: &ConcreteType, data: StackValue<'gc>) -> ValueType<'gc> {
        match self.new_cts_value(t, data) {
            CTSValue::Value(v) => v,
            CTSValue::Ref(r) => {
                panic!(
                    "tried to instantiate value type, received object reference ({:?})",
                    r
                )
            }
        }
    }

    fn value_type_description<'gc>(&self, vt: &ValueType<'gc>) -> TypeDescription {
        let asms = &self.loader;
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
            ValueType::TypedRef => asms.corlib_type("System.TypedReference"),
            ValueType::Struct(s) => s.description,
        }
    }

    fn new_cts_value<'gc>(&self, t: &ConcreteType, data: StackValue<'gc>) -> CTSValue<'gc> {
        use ValueType::*;
        let t = self.normalize_type(t.clone());
        match t.get() {
            BaseType::Boolean => CTSValue::Value(Bool(convert_num::<u8>(data) != 0)),
            BaseType::Char => CTSValue::Value(Char(convert_num(data))),
            BaseType::Int8 => CTSValue::Value(Int8(convert_num(data))),
            BaseType::UInt8 => CTSValue::Value(UInt8(convert_num(data))),
            BaseType::Int16 => CTSValue::Value(Int16(convert_num(data))),
            BaseType::UInt16 => CTSValue::Value(UInt16(convert_num(data))),
            BaseType::Int32 => CTSValue::Value(Int32(convert_num(data))),
            BaseType::UInt32 => CTSValue::Value(UInt32(convert_num(data))),
            BaseType::Int64 => CTSValue::Value(Int64(convert_i64(data))),
            BaseType::UInt64 => CTSValue::Value(UInt64(reinterpret_i64_as_u64(data))),
            BaseType::Float32 => CTSValue::Value(Float32(match data {
                StackValue::NativeFloat(f) => f as f32,
                other => panic!("invalid stack value {:?} for float conversion", other),
            })),
            BaseType::Float64 => CTSValue::Value(Float64(match data {
                StackValue::NativeFloat(f) => f,
                other => panic!("invalid stack value {:?} for float conversion", other),
            })),
            BaseType::IntPtr => CTSValue::Value(NativeInt(convert_num(data))),
            BaseType::UIntPtr | BaseType::FunctionPointer(_) => {
                CTSValue::Value(NativeUInt(convert_num(data)))
            }
            BaseType::ValuePointer(_modifiers, inner) => match data {
                StackValue::ManagedPtr(p) => CTSValue::Value(Pointer(p)),
                _ => {
                    let ptr = convert_num::<usize>(data);
                    let inner_type = if let Some(source) = inner {
                        self.loader.find_concrete_type(source.clone())
                    } else {
                        self.loader.corlib_type("System.Void")
                    };
                    CTSValue::Value(Pointer(ManagedPtr::new(
                        NonNull::new(ptr as *mut u8),
                        inner_type,
                        None,
                        false,
                    )))
                }
            },
            BaseType::Object
            | BaseType::String
            | BaseType::Vector(_, _)
            | BaseType::Array(_, _) => {
                if let StackValue::ObjectRef(o) = data {
                    CTSValue::Ref(o)
                } else {
                    panic!("expected ObjectRef, got {:?}", data)
                }
            }
            BaseType::Type {
                value_kind: Some(ValueKind::Class),
                ..
            } => {
                if let StackValue::ObjectRef(o) = data {
                    CTSValue::Ref(o)
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
                let new_ctx = self.with_generics(&new_lookup);
                let td = new_ctx.locate_type(ut);

                if let Some(e) = td.is_enum() {
                    let enum_type = new_ctx.make_concrete(e);
                    return self.new_cts_value(&enum_type, data);
                }

                if td.type_name() == "System.TypedReference" {
                    return CTSValue::Value(TypedRef);
                }

                if let StackValue::ValueType(mut o) = data {
                    if o.description != td {
                        // This can happen with unboxing or other conversions.
                        // We should probably check for compatibility here.
                        o.description = td;
                    }
                    CTSValue::Value(Struct(o))
                } else {
                    let mut instance = self.new_object(td);
                    if let StackValue::ObjectRef(o) = data {
                        // Unboxing
                        if let Some(handle) = o.0 {
                            let borrowed = handle.borrow();
                            match &borrowed.storage {
                                dotnet_value::object::HeapStorage::Obj(obj) => {
                                    instance.instance_storage = obj.instance_storage.clone();
                                }
                                _ => panic!("cannot unbox from non-object storage"),
                            }
                        }
                    }
                    CTSValue::Value(Struct(instance))
                }
            }
        }
    }

    fn read_cts_value<'gc>(
        &self,
        t: &ConcreteType,
        data: &[u8],
        gc: GCHandle<'gc>,
    ) -> CTSValue<'gc> {
        use ValueType::*;
        let t = self.normalize_type(t.clone());
        match t.get() {
            BaseType::Boolean => CTSValue::Value(Bool(data[0] != 0)),
            BaseType::Char => CTSValue::Value(Char(u16::from_ne_bytes(data.try_into().unwrap()))),
            BaseType::Int8 => CTSValue::Value(Int8(data[0] as i8)),
            BaseType::UInt8 => CTSValue::Value(UInt8(data[0])),
            BaseType::Int16 => CTSValue::Value(Int16(i16::from_ne_bytes(data.try_into().unwrap()))),
            BaseType::UInt16 => {
                CTSValue::Value(UInt16(u16::from_ne_bytes(data.try_into().unwrap())))
            }
            BaseType::Int32 => CTSValue::Value(Int32(i32::from_ne_bytes(data.try_into().unwrap()))),
            BaseType::UInt32 => {
                CTSValue::Value(UInt32(u32::from_ne_bytes(data.try_into().unwrap())))
            }
            BaseType::Int64 => CTSValue::Value(Int64(i64::from_ne_bytes(data.try_into().unwrap()))),
            BaseType::UInt64 => {
                CTSValue::Value(UInt64(u64::from_ne_bytes(data.try_into().unwrap())))
            }
            BaseType::Float32 => {
                CTSValue::Value(Float32(f32::from_ne_bytes(data.try_into().unwrap())))
            }
            BaseType::Float64 => {
                CTSValue::Value(Float64(f64::from_ne_bytes(data.try_into().unwrap())))
            }
            BaseType::IntPtr => {
                CTSValue::Value(NativeInt(isize::from_ne_bytes(data.try_into().unwrap())))
            }
            BaseType::UIntPtr | BaseType::FunctionPointer(_) => {
                CTSValue::Value(NativeUInt(usize::from_ne_bytes(data.try_into().unwrap())))
            }
            BaseType::ValuePointer(_modifiers, inner) => {
                let inner_type = if let Some(source) = inner {
                    self.loader.find_concrete_type(source.clone())
                } else {
                    self.loader.corlib_type("System.Void")
                };

                if data.len() >= 16 {
                    let (ptr, owner, _offset) = unsafe { ManagedPtr::read_from_bytes(data) };
                    CTSValue::Value(Pointer(ManagedPtr::new(
                        ptr,
                        inner_type,
                        Some(owner),
                        false,
                    )))
                } else {
                    let mut ptr_bytes = [0u8; 8];
                    ptr_bytes.copy_from_slice(&data[0..8]);
                    let ptr = usize::from_ne_bytes(ptr_bytes);
                    CTSValue::Value(Pointer(ManagedPtr::new(
                        NonNull::new(ptr as *mut u8),
                        inner_type,
                        None,
                        false,
                    )))
                }
            }
            BaseType::Object
            | BaseType::String
            | BaseType::Vector(_, _)
            | BaseType::Array(_, _) => {
                CTSValue::Ref(unsafe { dotnet_value::object::ObjectRef::read_branded(data, gc) })
            }
            BaseType::Type {
                value_kind: Some(ValueKind::Class),
                ..
            } => CTSValue::Ref(unsafe { dotnet_value::object::ObjectRef::read_branded(data, gc) }),
            BaseType::Type {
                value_kind: None | Some(ValueKind::ValueType),
                source,
            } => {
                let (ut, type_generics) = decompose_type_source(source);
                let new_lookup = GenericLookup::new(type_generics);
                let new_ctx = self.with_generics(&new_lookup);
                let td = new_ctx.locate_type(ut);

                if let Some(e) = td.is_enum() {
                    let enum_type = new_ctx.make_concrete(e);
                    return self.read_cts_value(&enum_type, data, gc);
                }

                if td.type_name() == "System.TypedReference" {
                    return CTSValue::Value(TypedRef);
                }

                let instance = self.new_object(td);
                instance.instance_storage.get_mut().copy_from_slice(data);
                CTSValue::Value(Struct(instance))
            }
        }
    }

    fn new_vector<'gc>(&self, element: ConcreteType, size: usize) -> Vector<'gc> {
        let layout = LayoutFactory::create_array_layout(element.clone(), size, self);
        let total_size = layout.element_layout.size() * size;
        if total_size > 0x7FFF_FFFF {
            panic!(
                "attempted to allocate massive vector of {} bytes (element: {:?}, length: {})",
                total_size, element, size
            );
        }

        Vector::new(element, layout, vec![0; total_size], vec![size])
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
        StackValue::UnmanagedPtr(p) => (p.0.as_ptr() as usize)
            .try_into()
            .unwrap_or_else(|_| panic!("failed to convert from pointer")),
        StackValue::ManagedPtr(p) => (p.pointer().map_or(0, |x| x.as_ptr() as usize))
            .try_into()
            .unwrap_or_else(|_| panic!("failed to convert from pointer")),
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

impl TypeResolutionExt for TypeDescription {
    fn is_value_type(&self, ctx: &ResolutionContext) -> bool {
        for (a, _) in ctx.get_ancestors(*self) {
            if matches!(a.type_name().as_str(), "System.Enum" | "System.ValueType") {
                return true;
            }
        }
        false
    }

    fn has_finalizer(&self, ctx: &ResolutionContext) -> bool {
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

        if check_type(*self) {
            return true;
        }

        for (ancestor, _) in ctx.get_ancestors(*self) {
            if check_type(ancestor) {
                return true;
            }
        }
        false
    }
}

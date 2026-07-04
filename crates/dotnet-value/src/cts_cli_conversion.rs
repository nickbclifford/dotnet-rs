use crate::{
    StackValue,
    object::{CTSValue, ObjectInner, ObjectRef, ValueType},
};
use dotnet_types::error::TypeResolutionError;
use dotnet_utils::gc::ThreadSafeLock;
use dotnetdll::prelude::{BaseType, LoadType};
use gc_arena::Gc;
use std::{any, error::Error, mem::size_of};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CtsScalarKind {
    Boolean,
    Char,
    Int8,
    UInt8,
    Int16,
    UInt16,
    Int32,
    UInt32,
    Int64,
    UInt64,
    NativeInt,
    NativeUInt,
}

impl CtsScalarKind {
    pub fn from_enum_underlying<T>(base: &BaseType<T>) -> Option<Self> {
        Some(match base {
            BaseType::Boolean => Self::Boolean,
            BaseType::Char => Self::Char,
            BaseType::Int8 => Self::Int8,
            BaseType::UInt8 => Self::UInt8,
            BaseType::Int16 => Self::Int16,
            BaseType::UInt16 => Self::UInt16,
            BaseType::Int32 => Self::Int32,
            BaseType::UInt32 => Self::UInt32,
            BaseType::Int64 => Self::Int64,
            BaseType::UInt64 => Self::UInt64,
            BaseType::IntPtr => Self::NativeInt,
            BaseType::UIntPtr => Self::NativeUInt,
            _ => return None,
        })
    }

    pub fn from_load_type(load: LoadType) -> Option<Self> {
        Some(match load {
            LoadType::Int8 => Self::Int8,
            LoadType::UInt8 => Self::UInt8,
            LoadType::Int16 => Self::Int16,
            LoadType::UInt16 => Self::UInt16,
            LoadType::Int32 => Self::Int32,
            LoadType::UInt32 => Self::UInt32,
            LoadType::Int64 => Self::Int64,
            LoadType::IntPtr => Self::NativeInt,
            _ => return None,
        })
    }

    pub const fn storage_size(self) -> usize {
        match self {
            Self::Boolean | Self::Int8 | Self::UInt8 => 1,
            Self::Char | Self::Int16 | Self::UInt16 => 2,
            Self::Int32 | Self::UInt32 => 4,
            Self::Int64 | Self::UInt64 => 8,
            Self::NativeInt | Self::NativeUInt => size_of::<usize>(),
        }
    }

    fn widen_raw<'gc>(self, raw: u64) -> StackValue<'gc> {
        match self {
            Self::Boolean | Self::UInt8 => StackValue::Int32(raw as u8 as i32),
            Self::Int8 => StackValue::Int32(raw as i8 as i32),
            Self::Char | Self::UInt16 => StackValue::Int32(raw as u16 as i32),
            Self::Int16 => StackValue::Int32(raw as i16 as i32),
            Self::Int32 => StackValue::Int32(raw as i32),
            Self::UInt32 => StackValue::Int32(raw as u32 as i32),
            Self::Int64 | Self::UInt64 => StackValue::Int64(raw as i64),
            Self::NativeInt | Self::NativeUInt => StackValue::NativeInt(raw as isize),
        }
    }

    fn read_storage_value<'gc>(self, data: &[u8]) -> ValueType<'gc> {
        match self {
            Self::Boolean => ValueType::Bool(data[0] != 0),
            Self::Char => ValueType::Char(u16::from_ne_bytes(data.try_into().unwrap())),
            Self::Int8 => ValueType::Int8(data[0] as i8),
            Self::UInt8 => ValueType::UInt8(data[0]),
            Self::Int16 => ValueType::Int16(i16::from_ne_bytes(data.try_into().unwrap())),
            Self::UInt16 => ValueType::UInt16(u16::from_ne_bytes(data.try_into().unwrap())),
            Self::Int32 => ValueType::Int32(i32::from_ne_bytes(data.try_into().unwrap())),
            Self::UInt32 => ValueType::UInt32(u32::from_ne_bytes(data.try_into().unwrap())),
            Self::Int64 => ValueType::Int64(i64::from_ne_bytes(data.try_into().unwrap())),
            Self::UInt64 => ValueType::UInt64(u64::from_ne_bytes(data.try_into().unwrap())),
            Self::NativeInt => ValueType::NativeInt(isize::from_ne_bytes(data.try_into().unwrap())),
            Self::NativeUInt => {
                ValueType::NativeUInt(usize::from_ne_bytes(data.try_into().unwrap()))
            }
        }
    }
}

pub struct CtsToCli;

impl CtsToCli {
    pub fn widen<'gc>(value: CTSValue<'gc>) -> StackValue<'gc> {
        match value {
            CTSValue::Value(value) => Self::widen_value_type(value),
            CTSValue::Ref(obj) => StackValue::ObjectRef(obj),
        }
    }

    pub fn widen_value_type<'gc>(value: ValueType<'gc>) -> StackValue<'gc> {
        match value {
            ValueType::Bool(v) => CtsScalarKind::Boolean.widen_raw(v as u64),
            ValueType::Char(v) => CtsScalarKind::Char.widen_raw(v as u64),
            ValueType::Int8(v) => CtsScalarKind::Int8.widen_raw(v as u8 as u64),
            ValueType::UInt8(v) => CtsScalarKind::UInt8.widen_raw(v as u64),
            ValueType::Int16(v) => CtsScalarKind::Int16.widen_raw(v as u16 as u64),
            ValueType::UInt16(v) => CtsScalarKind::UInt16.widen_raw(v as u64),
            ValueType::Int32(v) => CtsScalarKind::Int32.widen_raw(v as u32 as u64),
            ValueType::UInt32(v) => CtsScalarKind::UInt32.widen_raw(v as u64),
            ValueType::Int64(v) => CtsScalarKind::Int64.widen_raw(v as u64),
            ValueType::UInt64(v) => CtsScalarKind::UInt64.widen_raw(v),
            ValueType::NativeInt(v) => CtsScalarKind::NativeInt.widen_raw(v as usize as u64),
            ValueType::NativeUInt(v) => CtsScalarKind::NativeUInt.widen_raw(v as u64),
            ValueType::Pointer(v) => StackValue::ManagedPtr(v.into()),
            ValueType::Float32(v) => StackValue::NativeFloat(v as f64),
            ValueType::Float64(v) => StackValue::NativeFloat(v),
            ValueType::TypedRef(ptr, ty) => StackValue::TypedRef(ptr.into(), ty),
            ValueType::Struct(obj) => StackValue::ValueType(obj),
        }
    }

    pub fn widen_enum_underlying_bytes<'gc, T>(
        base: &BaseType<T>,
        data: &[u8],
    ) -> Option<StackValue<'gc>> {
        let scalar = CtsScalarKind::from_enum_underlying(base)?;
        Some(scalar.widen_raw(read_le_u64_prefix(data, scalar.storage_size())))
    }

    pub fn widen_load_atomic_raw<'gc>(load: LoadType, raw: u64) -> StackValue<'gc> {
        if let Some(scalar) = CtsScalarKind::from_load_type(load) {
            return scalar.widen_raw(raw);
        }

        match load {
            LoadType::Float32 => StackValue::NativeFloat(f32::from_bits(raw as u32) as f64),
            LoadType::Float64 => StackValue::NativeFloat(f64::from_bits(raw)),
            LoadType::Object => {
                let ptr = std::ptr::with_exposed_provenance::<ThreadSafeLock<ObjectInner<'gc>>>(
                    raw as usize,
                );
                let obj = if ptr.is_null() {
                    None
                } else {
                    Some(unsafe { Gc::from_ptr(ptr) })
                };
                StackValue::ObjectRef(ObjectRef(obj))
            }
            _ => unreachable!("unsupported load conversion for {:?}", load),
        }
    }
}

pub struct CliToCts;

impl CliToCts {
    pub fn narrow_scalar_value<'gc>(
        data: StackValue<'gc>,
        target: CtsScalarKind,
    ) -> Result<ValueType<'gc>, TypeResolutionError> {
        use ValueType::*;
        Ok(match target {
            CtsScalarKind::Boolean => Bool(Self::convert_num::<u8>(data)? != 0),
            CtsScalarKind::Char => Char(Self::convert_num(data)?),
            CtsScalarKind::Int8 => Int8(Self::convert_num(data)?),
            CtsScalarKind::UInt8 => UInt8(Self::convert_num(data)?),
            CtsScalarKind::Int16 => Int16(Self::convert_num(data)?),
            CtsScalarKind::UInt16 => UInt16(Self::convert_num(data)?),
            CtsScalarKind::Int32 => Int32(Self::convert_num(data)?),
            CtsScalarKind::UInt32 => UInt32(Self::convert_u32(data)?),
            CtsScalarKind::Int64 => Int64(Self::convert_i64(data)?),
            CtsScalarKind::UInt64 => UInt64(Self::reinterpret_i64_as_u64(data)?),
            CtsScalarKind::NativeInt => NativeInt(Self::convert_num(data)?),
            CtsScalarKind::NativeUInt => NativeUInt(Self::convert_num(data)?),
        })
    }

    pub fn read_scalar_storage<'gc>(target: CtsScalarKind, data: &[u8]) -> ValueType<'gc> {
        target.read_storage_value(data)
    }

    pub fn narrow_float32<'gc>(
        data: StackValue<'gc>,
    ) -> Result<ValueType<'gc>, TypeResolutionError> {
        match data {
            StackValue::NativeFloat(v) => Ok(ValueType::Float32(v as f32)),
            other => Err(TypeResolutionError::InvalidLayout(
                format!("invalid stack value {:?} for conversion into f32", other).into(),
            )),
        }
    }

    pub fn narrow_float64<'gc>(
        data: StackValue<'gc>,
    ) -> Result<ValueType<'gc>, TypeResolutionError> {
        match data {
            StackValue::NativeFloat(v) => Ok(ValueType::Float64(v)),
            other => Err(TypeResolutionError::InvalidLayout(
                format!("invalid stack value {:?} for conversion into f64", other).into(),
            )),
        }
    }

    pub fn convert_u32(data: StackValue<'_>) -> Result<u32, TypeResolutionError> {
        let data = data.coerce_enum_to_underlying();
        match data {
            StackValue::Int32(i) => Ok(i as u32),
            StackValue::NativeInt(i) => Ok((i as usize) as u32),
            StackValue::UnmanagedPtr(p) => Ok(p.0.as_ptr().expose_provenance() as u32),
            StackValue::ManagedPtr(p) => {
                let ptr = unsafe { p.with_data(0, |data| data.as_ptr()) };
                Ok(ptr.expose_provenance() as u32)
            }
            other => Err(TypeResolutionError::InvalidLayout(
                format!("invalid stack value {:?} for conversion into u32", other).into(),
            )),
        }
    }

    pub fn convert_num<T: TryFrom<i32> + TryFrom<isize> + TryFrom<usize>>(
        data: StackValue<'_>,
    ) -> Result<T, TypeResolutionError> {
        let data = data.coerce_enum_to_underlying();
        match data {
            StackValue::Int32(i) => i.try_into().map_err(|_| {
                TypeResolutionError::InvalidLayout(
                    format!("failed to convert from i32 into {}", any::type_name::<T>()).into(),
                )
            }),
            StackValue::NativeInt(i) => i.try_into().map_err(|_| {
                TypeResolutionError::InvalidLayout(
                    format!(
                        "failed to convert from isize into {}",
                        any::type_name::<T>()
                    )
                    .into(),
                )
            }),
            StackValue::UnmanagedPtr(p) => {
                p.0.as_ptr().expose_provenance().try_into().map_err(|_| {
                    TypeResolutionError::InvalidLayout(
                        format!(
                            "failed to convert unmanaged pointer into {}",
                            any::type_name::<T>()
                        )
                        .into(),
                    )
                })
            }
            StackValue::ManagedPtr(p) => {
                let ptr = unsafe { p.with_data(0, |data| data.as_ptr()) };
                ptr.expose_provenance().try_into().map_err(|_| {
                    TypeResolutionError::InvalidLayout(
                        format!(
                            "failed to convert managed pointer into {}",
                            any::type_name::<T>()
                        )
                        .into(),
                    )
                })
            }
            other => Err(TypeResolutionError::InvalidLayout(
                format!(
                    "invalid stack value {:?} for conversion into {}",
                    other,
                    any::type_name::<T>()
                )
                .into(),
            )),
        }
    }

    pub fn convert_i64<T: TryFrom<i64>>(data: StackValue<'_>) -> Result<T, TypeResolutionError>
    where
        T::Error: Error,
    {
        let data = data.coerce_enum_to_underlying();
        match data {
            StackValue::Int64(i) => i.try_into().map_err(|e| {
                TypeResolutionError::InvalidLayout(
                    format!(
                        "failed to convert from i64 to {} ({})",
                        any::type_name::<T>(),
                        e
                    )
                    .into(),
                )
            }),
            other => Err(TypeResolutionError::InvalidLayout(
                format!("invalid stack value {:?} for integer conversion", other).into(),
            )),
        }
    }

    pub fn reinterpret_i64_as_u64(data: StackValue<'_>) -> Result<u64, TypeResolutionError> {
        let data = data.coerce_enum_to_underlying();
        match data {
            StackValue::Int64(i) => Ok(i as u64),
            other => Err(TypeResolutionError::InvalidLayout(
                format!("invalid stack value {:?} for u64 reinterpretation", other).into(),
            )),
        }
    }
}

fn read_le_u64_prefix(data: &[u8], width: usize) -> u64 {
    let mut buf = [0u8; 8];
    buf[..width].copy_from_slice(&data[..width]);
    u64::from_le_bytes(buf)
}

#[cfg(test)]
mod tests {
    use super::{CliToCts, CtsToCli};
    use crate::{
        StackValue,
        object::{CTSValue, HeapStorage, ObjectRef, ValueType},
        pointer::{ManagedPtr, UnmanagedPtr},
        test_helpers::with_test_gc_context,
    };
    use dotnet_types::{TypeDescription, error::TypeResolutionError};
    use dotnetdll::prelude::{BaseType, LoadType};
    use gc_arena::Gc;
    use std::ptr::NonNull;

    #[test]
    fn convert_i64_rejects_non_int64_input() {
        let result = CliToCts::convert_i64::<i64>(StackValue::Int32(7));
        match result {
            Err(TypeResolutionError::InvalidLayout(message)) => {
                assert!(message.contains("invalid stack value"));
                assert!(message.contains("integer conversion"));
            }
            other => panic!("expected InvalidLayout error, got {:?}", other),
        }
    }

    #[test]
    fn enum_underlying_widening_preserves_sign_and_zero_extension() {
        let int8 = CtsToCli::widen_enum_underlying_bytes(&BaseType::<()>::Int8, &[0xFF]);
        assert_eq!(int8, Some(StackValue::Int32(-1)));

        let uint8 = CtsToCli::widen_enum_underlying_bytes(&BaseType::<()>::UInt8, &[0xFF]);
        assert_eq!(uint8, Some(StackValue::Int32(255)));

        let int16_data = (-2i16).to_le_bytes();
        let int16 = CtsToCli::widen_enum_underlying_bytes(&BaseType::<()>::Int16, &int16_data);
        assert_eq!(int16, Some(StackValue::Int32(-2)));

        let uint16_data = 0xFFFEu16.to_le_bytes();
        let uint16 = CtsToCli::widen_enum_underlying_bytes(&BaseType::<()>::UInt16, &uint16_data);
        assert_eq!(uint16, Some(StackValue::Int32(0xFFFE)));

        let uint64_data = u64::MAX.to_le_bytes();
        let uint64 = CtsToCli::widen_enum_underlying_bytes(&BaseType::<()>::UInt64, &uint64_data);
        assert_eq!(uint64, Some(StackValue::Int64(-1)));
    }

    #[test]
    fn load_widening_small_integers_preserves_sign_and_zero_extension() {
        assert_eq!(
            CtsToCli::widen_load_atomic_raw(LoadType::Int8, 0xFF),
            StackValue::Int32(-1)
        );
        assert_eq!(
            CtsToCli::widen_load_atomic_raw(LoadType::UInt8, 0xFF),
            StackValue::Int32(255)
        );
        assert_eq!(
            CtsToCli::widen_load_atomic_raw(LoadType::Int16, 0xFFFE),
            StackValue::Int32(-2)
        );
        assert_eq!(
            CtsToCli::widen_load_atomic_raw(LoadType::UInt16, 0xFFFE),
            StackValue::Int32(0xFFFE)
        );
    }

    #[test]
    fn widening_preserves_pointer_and_reference_payloads() {
        let ptr = ManagedPtr::new(
            NonNull::new(0x1234usize as *mut u8),
            TypeDescription::NULL,
            None,
            false,
            None,
        );
        assert_eq!(
            CtsToCli::widen_value_type(ValueType::Pointer(ptr.clone())),
            StackValue::ManagedPtr(ptr.into())
        );

        let null_ref = ObjectRef(None);
        assert_eq!(
            CtsToCli::widen(CTSValue::Ref(null_ref)),
            StackValue::ObjectRef(null_ref)
        );

        with_test_gc_context(|gc| {
            let obj = ObjectRef::new(gc, HeapStorage::Str(crate::string::CLRString::from("x")));
            let raw = obj
                .0
                .map(|h| Gc::as_ptr(h).expose_provenance() as u64)
                .expect("object reference should be non-null");
            assert_eq!(
                CtsToCli::widen_load_atomic_raw(LoadType::Object, raw),
                StackValue::ObjectRef(obj)
            );
        });

        assert_eq!(
            CtsToCli::widen_load_atomic_raw(LoadType::Object, 0),
            StackValue::ObjectRef(ObjectRef(None))
        );

        let raw_ptr = NonNull::new(0x5678usize as *mut u8).unwrap();
        let narrowed =
            CliToCts::convert_num::<usize>(StackValue::UnmanagedPtr(UnmanagedPtr(raw_ptr)))
                .expect("unmanaged pointer should narrow to native uint");
        assert_eq!(narrowed, 0x5678usize);
    }
}

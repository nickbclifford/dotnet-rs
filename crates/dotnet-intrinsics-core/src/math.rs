use dotnet_macros::{dotnet_intrinsic, dotnet_intrinsic_field};
use dotnet_types::{
    WellKnown,
    error::{ExecutionError, VmError},
    generics::{ConcreteType, GenericLookup},
    members::{FieldDescription, MethodDescription},
};
use dotnet_utils::ByteOffset;
use dotnet_value::{
    StackValue,
    object::{HeapStorage, ObjectRef},
    pointer::PointerOrigin,
};
use dotnet_vm_data::StepResult;
use dotnet_vm_ops::ops::{
    ExceptionOps, LoaderOps, MemoryOps, RawMemoryOps, TypeLayoutOps, TypedStackOps,
};
use dotnetdll::prelude::BaseType;
use std::sync::Arc;

fn instantiate_comparer<'gc, T: LoaderOps + MemoryOps<'gc> + TypedStackOps<'gc>>(
    ctx: &mut T,
    target_type: ConcreteType,
    comparer_type: WellKnown,
) -> Result<ObjectRef<'gc>, VmError> {
    let comparer_td = ctx.loader().corlib_wkt(comparer_type)?;
    let comparer_lookup = GenericLookup::new(vec![target_type]);

    // Keep the comparer as a generic type-definition + lookup pair.
    // Rebuilding it via MethodType erases each ConcreteType's source resolution,
    // which can make generic-constraint validation index the wrong resolution.
    Ok(ObjectRef::new(
        ctx.gc_with_token(&ctx.no_active_borrows_token()),
        HeapStorage::Obj(Box::new(
            ctx.new_object_with_lookup(comparer_td, &comparer_lookup)?,
        )),
    ))
}

#[dotnet_intrinsic("static bool System.Runtime.Intrinsics.Vector512::get_IsHardwareAccelerated()")]
#[dotnet_intrinsic("static bool System.Numerics.Vector::get_IsHardwareAccelerated()")]
pub fn intrinsic_vector_is_hardware_accelerated<'gc, T: TypedStackOps<'gc>>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    ctx.push_i32(0);
    StepResult::Continue
}

#[dotnet_intrinsic(
    "static System.Collections.Generic.EqualityComparer<T> System.Collections.Generic.EqualityComparer<T>::get_Default()"
)]
pub fn intrinsic_equality_comparer_get_default<
    'gc,
    T: LoaderOps + MemoryOps<'gc> + TypedStackOps<'gc>,
>(
    ctx: &mut T,
    _method: MethodDescription,
    generics: &GenericLookup,
) -> StepResult {
    let target_type = dotnet_vm_ops::vm_try!(generics.cloned_type_arg(0));
    let instance = dotnet_vm_ops::vm_try!(instantiate_comparer(
        ctx,
        target_type,
        WellKnown::SupportComparersEqualityComparer1,
    ));
    ctx.push_obj(instance);
    StepResult::Continue
}

#[dotnet_intrinsic(
    "static System.Collections.Generic.Comparer<T> System.Collections.Generic.Comparer<T>::get_Default()"
)]
pub fn intrinsic_comparer_get_default<'gc, T: LoaderOps + MemoryOps<'gc> + TypedStackOps<'gc>>(
    ctx: &mut T,
    _method: MethodDescription,
    generics: &GenericLookup,
) -> StepResult {
    let target_type = dotnet_vm_ops::vm_try!(generics.cloned_type_arg(0));
    let instance = dotnet_vm_ops::vm_try!(instantiate_comparer(
        ctx,
        target_type,
        WellKnown::SupportComparersOrderingComparer1,
    ));
    ctx.push_obj(instance);
    StepResult::Continue
}

#[dotnet_intrinsic("static byte System.Byte::CreateTruncating<T>(T)")]
#[dotnet_intrinsic("static sbyte System.SByte::CreateTruncating<T>(T)")]
#[dotnet_intrinsic("static ushort System.UInt16::CreateTruncating<T>(T)")]
#[dotnet_intrinsic("static short System.Int16::CreateTruncating<T>(T)")]
#[dotnet_intrinsic("static uint System.UInt32::CreateTruncating<T>(T)")]
#[dotnet_intrinsic("static int System.Int32::CreateTruncating<T>(T)")]
#[dotnet_intrinsic("static ulong System.UInt64::CreateTruncating<T>(T)")]
#[dotnet_intrinsic("static long System.Int64::CreateTruncating<T>(T)")]
#[dotnet_intrinsic("static nuint System.UIntPtr::CreateTruncating<T>(T)")]
#[dotnet_intrinsic("static nint System.IntPtr::CreateTruncating<T>(T)")]
pub fn intrinsic_numeric_create_truncating<'gc, T: TypedStackOps<'gc>>(
    ctx: &mut T,
    method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let value = ctx.pop();
    let target_type = method.parent.definition().type_name();

    macro_rules! convert {
        ($val:expr) => {
            match target_type.as_str() {
                "Byte" | "System.Byte" => ctx.push_i32(($val as u8) as i32),
                "SByte" | "System.SByte" => ctx.push_i32(($val as i8) as i32),
                "UInt16" | "System.UInt16" => ctx.push_i32(($val as u16) as i32),
                "Int16" | "System.Int16" => ctx.push_i32(($val as i16) as i32),
                "UInt32" | "System.UInt32" => ctx.push_i32(($val as u32) as i32),
                "Int32" | "System.Int32" => ctx.push_i32($val as i32),
                "UInt64" | "System.UInt64" => ctx.push_i64(($val as u64) as i64),
                "Int64" | "System.Int64" => ctx.push_i64($val as i64),
                "UIntPtr" | "System.UIntPtr" => ctx.push_isize(($val as usize) as isize),
                "IntPtr" | "System.IntPtr" => ctx.push_isize($val as isize),
                _ => {
                    return StepResult::Error(VmError::Execution(ExecutionError::TypeMismatch {
                        expected: "Byte/SByte/UInt16/Int16/UInt32/Int32/UInt64/Int64/UIntPtr/IntPtr",
                        actual: target_type.clone().into(),
                    }));
                }
            }
        };
    }

    match value {
        StackValue::Int32(v) => convert!(v),
        StackValue::Int64(v) => convert!(v),
        StackValue::NativeInt(v) => convert!(v),
        actual => {
            return StepResult::Error(VmError::Execution(ExecutionError::TypeMismatch {
                expected: "Int32 or Int64 or NativeInt",
                actual: format!("{actual:?}").into(),
            }));
        }
    }

    StepResult::Continue
}

#[dotnet_intrinsic("static uint System.UInt32::CreateChecked<T>(T)")]
pub fn intrinsic_uint32_create_checked<'gc, T: TypedStackOps<'gc>>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let value = ctx.pop();
    match value {
        StackValue::Int32(v) => ctx.push_i32((v as u32) as i32),
        StackValue::Int64(v) => ctx.push_i32((v as u32) as i32),
        StackValue::NativeInt(v) => ctx.push_i32((v as u32) as i32),
        other => {
            return StepResult::not_implemented(format!(
                "UInt32.CreateChecked<T> unsupported source type: {:?}",
                other
            ));
        }
    }
    StepResult::Continue
}

#[dotnet_intrinsic("static int System.Int32::CreateChecked<T>(T)")]
pub fn intrinsic_int32_create_checked<'gc, T: TypedStackOps<'gc>>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let value = ctx.pop();
    match value {
        StackValue::Int32(v) => ctx.push_i32(v),
        StackValue::Int64(v) => ctx.push_i32(v as i32),
        StackValue::NativeInt(v) => ctx.push_i32(v as i32),
        other => {
            return StepResult::not_implemented(format!(
                "Int32.CreateChecked<T> unsupported source type: {:?}",
                other
            ));
        }
    }
    StepResult::Continue
}

#[dotnet_intrinsic("static M0 System.Numerics.INumberBase<M0>::get_Zero()")]
#[dotnet_intrinsic("static M1 System.Numerics.IAdditiveIdentity<M0, M1>::get_AdditiveIdentity()")]
pub fn intrinsic_numeric_zero<'gc, T: TypedStackOps<'gc>>(
    ctx: &mut T,
    _method: MethodDescription,
    generics: &GenericLookup,
) -> StepResult {
    let Some(target_type) = generics.type_generics.first() else {
        return StepResult::not_implemented(
            "numeric zero intrinsic called without type generic".to_string(),
        );
    };

    match target_type.get() {
        BaseType::Boolean
        | BaseType::Char
        | BaseType::UInt8
        | BaseType::Int8
        | BaseType::Int16
        | BaseType::UInt16
        | BaseType::Int32
        | BaseType::UInt32 => ctx.push_i32(0),
        BaseType::Int64 | BaseType::UInt64 => ctx.push_i64(0),
        BaseType::IntPtr | BaseType::UIntPtr => ctx.push_isize(0),
        BaseType::Float32 => ctx.push_f64(0.0),
        BaseType::Float64 => ctx.push_f64(0.0),
        other => {
            return StepResult::not_implemented(format!(
                "numeric zero intrinsic unsupported generic type: {:?}",
                other
            ));
        }
    }

    StepResult::Continue
}

#[dotnet_intrinsic("static M2 System.Numerics.IAdditionOperators<M0, M1, M2>::op_Addition(M0, M1)")]
pub fn intrinsic_numeric_addition<'gc, T: TypedStackOps<'gc>>(
    ctx: &mut T,
    _method: MethodDescription,
    generics: &GenericLookup,
) -> StepResult {
    let rhs = ctx.pop();
    let lhs = ctx.pop();

    let Some(result_type) = generics.type_generics.get(2) else {
        return StepResult::not_implemented(
            "numeric addition intrinsic called without result generic".to_string(),
        );
    };

    match result_type.get() {
        BaseType::Int32 | BaseType::UInt32 | BaseType::Int16 | BaseType::UInt16 => {
            match (lhs, rhs) {
                (StackValue::Int32(a), StackValue::Int32(b)) => ctx.push_i32(a.wrapping_add(b)),
                (a, b) => {
                    return StepResult::not_implemented(format!(
                        "numeric addition intrinsic unsupported operands for i32 result: {:?}, {:?}",
                        a, b
                    ));
                }
            }
        }
        BaseType::Int64 | BaseType::UInt64 => match (lhs, rhs) {
            (StackValue::Int64(a), StackValue::Int64(b)) => ctx.push_i64(a.wrapping_add(b)),
            (StackValue::Int32(a), StackValue::Int32(b)) => {
                ctx.push_i64((a as i64).wrapping_add(b as i64))
            }
            (a, b) => {
                return StepResult::not_implemented(format!(
                    "numeric addition intrinsic unsupported operands for i64 result: {:?}, {:?}",
                    a, b
                ));
            }
        },
        BaseType::IntPtr | BaseType::UIntPtr => match (lhs, rhs) {
            (StackValue::NativeInt(a), StackValue::NativeInt(b)) => {
                ctx.push_isize(a.wrapping_add(b))
            }
            (StackValue::Int32(a), StackValue::Int32(b)) => {
                ctx.push_isize((a as isize).wrapping_add(b as isize))
            }
            (a, b) => {
                return StepResult::not_implemented(format!(
                    "numeric addition intrinsic unsupported operands for native int result: {:?}, {:?}",
                    a, b
                ));
            }
        },
        BaseType::Float32 | BaseType::Float64 => match (lhs, rhs) {
            (StackValue::NativeFloat(a), StackValue::NativeFloat(b)) => ctx.push_f64(a + b),
            (a, b) => {
                return StepResult::not_implemented(format!(
                    "numeric addition intrinsic unsupported operands for floating result: {:?}, {:?}",
                    a, b
                ));
            }
        },
        other => {
            return StepResult::not_implemented(format!(
                "numeric addition intrinsic unsupported result type: {:?}",
                other
            ));
        }
    }

    StepResult::Continue
}

#[dotnet_intrinsic(
    "static M2 System.Numerics.IComparisonOperators<M0, M1, M2>::op_GreaterThan(M0, M1)"
)]
pub fn intrinsic_numeric_greater_than<'gc, T: TypedStackOps<'gc>>(
    ctx: &mut T,
    _method: MethodDescription,
    generics: &GenericLookup,
) -> StepResult {
    let rhs = ctx.pop();
    let lhs = ctx.pop();

    let Some(target_type) = generics.type_generics.first() else {
        return StepResult::not_implemented(
            "numeric greater-than intrinsic called without type generic".to_string(),
        );
    };

    let result = match target_type.get() {
        BaseType::Int8 | BaseType::Int16 | BaseType::Int32 => match (lhs, rhs) {
            (StackValue::Int32(a), StackValue::Int32(b)) => a > b,
            (a, b) => {
                return StepResult::not_implemented(format!(
                    "numeric greater-than intrinsic unsupported signed i32 operands: {:?}, {:?}",
                    a, b
                ));
            }
        },
        BaseType::UInt8 | BaseType::UInt16 | BaseType::UInt32 => match (lhs, rhs) {
            (StackValue::Int32(a), StackValue::Int32(b)) => (a as u32) > (b as u32),
            (a, b) => {
                return StepResult::not_implemented(format!(
                    "numeric greater-than intrinsic unsupported unsigned i32 operands: {:?}, {:?}",
                    a, b
                ));
            }
        },
        BaseType::Int64 => match (lhs, rhs) {
            (StackValue::Int64(a), StackValue::Int64(b)) => a > b,
            (StackValue::Int32(a), StackValue::Int32(b)) => a > b,
            (a, b) => {
                return StepResult::not_implemented(format!(
                    "numeric greater-than intrinsic unsupported signed i64 operands: {:?}, {:?}",
                    a, b
                ));
            }
        },
        BaseType::UInt64 => match (lhs, rhs) {
            (StackValue::Int64(a), StackValue::Int64(b)) => (a as u64) > (b as u64),
            (StackValue::Int32(a), StackValue::Int32(b)) => (a as u32) > (b as u32),
            (a, b) => {
                return StepResult::not_implemented(format!(
                    "numeric greater-than intrinsic unsupported unsigned i64 operands: {:?}, {:?}",
                    a, b
                ));
            }
        },
        BaseType::IntPtr => match (lhs, rhs) {
            (StackValue::NativeInt(a), StackValue::NativeInt(b)) => a > b,
            (StackValue::Int32(a), StackValue::Int32(b)) => a > b,
            (a, b) => {
                return StepResult::not_implemented(format!(
                    "numeric greater-than intrinsic unsupported native int operands: {:?}, {:?}",
                    a, b
                ));
            }
        },
        BaseType::UIntPtr => match (lhs, rhs) {
            (StackValue::NativeInt(a), StackValue::NativeInt(b)) => (a as usize) > (b as usize),
            (StackValue::Int32(a), StackValue::Int32(b)) => (a as u32) > (b as u32),
            (a, b) => {
                return StepResult::not_implemented(format!(
                    "numeric greater-than intrinsic unsupported native uint operands: {:?}, {:?}",
                    a, b
                ));
            }
        },
        BaseType::Float32 | BaseType::Float64 => match (lhs, rhs) {
            (StackValue::NativeFloat(a), StackValue::NativeFloat(b)) => a > b,
            (a, b) => {
                return StepResult::not_implemented(format!(
                    "numeric greater-than intrinsic unsupported floating operands: {:?}, {:?}",
                    a, b
                ));
            }
        },
        other => {
            return StepResult::not_implemented(format!(
                "numeric greater-than intrinsic unsupported target type: {:?}",
                other
            ));
        }
    };

    ctx.push_i32(i32::from(result));
    StepResult::Continue
}

#[dotnet_intrinsic("static M0 System.Numerics.IIncrementOperators<M0>::op_Increment(M0)")]
pub fn intrinsic_numeric_increment<'gc, T: TypedStackOps<'gc>>(
    ctx: &mut T,
    _method: MethodDescription,
    generics: &GenericLookup,
) -> StepResult {
    let value = ctx.pop();

    let Some(target_type) = generics.type_generics.first() else {
        return StepResult::not_implemented(
            "numeric increment intrinsic called without type generic".to_string(),
        );
    };

    match target_type.get() {
        BaseType::Int8
        | BaseType::UInt8
        | BaseType::Int16
        | BaseType::UInt16
        | BaseType::Int32
        | BaseType::UInt32 => match value {
            StackValue::Int32(v) => ctx.push_i32(v.wrapping_add(1)),
            other => {
                return StepResult::not_implemented(format!(
                    "numeric increment intrinsic unsupported operand for i32 result: {:?}",
                    other
                ));
            }
        },
        BaseType::Int64 | BaseType::UInt64 => match value {
            StackValue::Int64(v) => ctx.push_i64(v.wrapping_add(1)),
            StackValue::Int32(v) => ctx.push_i64((v as i64).wrapping_add(1)),
            other => {
                return StepResult::not_implemented(format!(
                    "numeric increment intrinsic unsupported operand for i64 result: {:?}",
                    other
                ));
            }
        },
        BaseType::IntPtr | BaseType::UIntPtr => match value {
            StackValue::NativeInt(v) => ctx.push_isize(v.wrapping_add(1)),
            StackValue::Int32(v) => ctx.push_isize((v as isize).wrapping_add(1)),
            other => {
                return StepResult::not_implemented(format!(
                    "numeric increment intrinsic unsupported operand for native int result: {:?}",
                    other
                ));
            }
        },
        BaseType::Float32 | BaseType::Float64 => match value {
            StackValue::NativeFloat(v) => ctx.push_f64(v + 1.0),
            other => {
                return StepResult::not_implemented(format!(
                    "numeric increment intrinsic unsupported operand for floating result: {:?}",
                    other
                ));
            }
        },
        other => {
            return StepResult::not_implemented(format!(
                "numeric increment intrinsic unsupported result type: {:?}",
                other
            ));
        }
    }

    StepResult::Continue
}

#[dotnet_intrinsic("static bool System.Double::IsInfinity(double)")]
pub fn intrinsic_double_is_infinity<'gc, T: TypedStackOps<'gc>>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let val = ctx.pop_f64();
    ctx.push_i32(if val.is_infinite() { 1 } else { 0 });
    StepResult::Continue
}

#[dotnet_intrinsic("static bool System.Double::IsNaN(double)")]
pub fn intrinsic_double_is_nan<'gc, T: TypedStackOps<'gc>>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let val = ctx.pop_f64();
    ctx.push_i32(if val.is_nan() { 1 } else { 0 });
    StepResult::Continue
}

#[dotnet_intrinsic("static bool System.Double::IsNegativeInfinity(double)")]
pub fn intrinsic_double_is_negative_infinity<'gc, T: TypedStackOps<'gc>>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let val = ctx.pop_f64();
    ctx.push_i32(if val == f64::NEG_INFINITY { 1 } else { 0 });
    StepResult::Continue
}

#[dotnet_intrinsic("static bool System.Double::IsPositiveInfinity(double)")]
pub fn intrinsic_double_is_positive_infinity<'gc, T: TypedStackOps<'gc>>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let val = ctx.pop_f64();
    ctx.push_i32(if val == f64::INFINITY { 1 } else { 0 });
    StepResult::Continue
}

#[dotnet_intrinsic("static double System.Math::Min(double, double)")]
pub fn intrinsic_math_min_double<'gc, T: TypedStackOps<'gc>>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let b = ctx.pop_f64();
    let a = ctx.pop_f64();
    ctx.push_f64(f64::min(a, b));
    StepResult::Continue
}

#[dotnet_intrinsic("static double System.Math::Max(double, double)")]
pub fn intrinsic_math_max_double<'gc, T: TypedStackOps<'gc>>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let b = ctx.pop_f64();
    let a = ctx.pop_f64();
    ctx.push_f64(f64::max(a, b));
    StepResult::Continue
}

#[dotnet_intrinsic("static double System.Math::Abs(double)")]
pub fn intrinsic_math_abs_double<'gc, T: TypedStackOps<'gc>>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let val = ctx.pop_f64();
    ctx.push_f64(val.abs());
    StepResult::Continue
}

#[dotnet_intrinsic("static double System.Math::Pow(double, double)")]
pub fn intrinsic_math_pow_double<'gc, T: TypedStackOps<'gc>>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let y = ctx.pop_f64();
    let x = ctx.pop_f64();
    ctx.push_f64(x.powf(y));
    StepResult::Continue
}

#[dotnet_intrinsic("static int System.Math::Min(int, int)")]
pub fn intrinsic_math_min_int<'gc, T: TypedStackOps<'gc>>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let b = ctx.pop_i32();
    let a = ctx.pop_i32();
    ctx.push_i32(std::cmp::min(a, b));
    StepResult::Continue
}

#[dotnet_intrinsic("static uint System.Math::Min(uint, uint)")]
pub fn intrinsic_math_min_uint<'gc, T: TypedStackOps<'gc>>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let b = ctx.pop_i32() as u32;
    let a = ctx.pop_i32() as u32;
    ctx.push_i32(std::cmp::min(a, b) as i32);
    StepResult::Continue
}

// CLI integer stack representations are truncated to the byte operand width.
fn byte_operand(value: StackValue<'_>) -> Result<u8, StepResult> {
    match value {
        StackValue::Int32(value) => Ok(value as u8),
        StackValue::Int64(value) => Ok(value as u8),
        StackValue::NativeInt(value) => Ok(value as u8),
        other => Err(StepResult::type_error(
            "byte-compatible integer",
            format!("{other:?}"),
        )),
    }
}

#[dotnet_intrinsic("static byte System.Math::Min(byte, byte)")]
pub fn intrinsic_math_min_byte<'gc, T: TypedStackOps<'gc>>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let b = match byte_operand(ctx.pop()) {
        Ok(value) => value,
        Err(step) => return step,
    };
    let a = match byte_operand(ctx.pop()) {
        Ok(value) => value,
        Err(step) => return step,
    };
    ctx.push_i32(std::cmp::min(a, b) as i32);
    StepResult::Continue
}

#[dotnet_intrinsic("static int System.Math::Max(int, int)")]
pub fn intrinsic_math_max_int<'gc, T: TypedStackOps<'gc>>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let b = ctx.pop_i32();
    let a = ctx.pop_i32();
    ctx.push_i32(std::cmp::max(a, b));
    StepResult::Continue
}

#[dotnet_intrinsic("static uint System.Math::Max(uint, uint)")]
pub fn intrinsic_math_max_uint<'gc, T: TypedStackOps<'gc>>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let b = ctx.pop_i32() as u32;
    let a = ctx.pop_i32() as u32;
    ctx.push_i32(std::cmp::max(a, b) as i32);
    StepResult::Continue
}

#[dotnet_intrinsic("static byte System.Math::Max(byte, byte)")]
pub fn intrinsic_math_max_byte<'gc, T: TypedStackOps<'gc>>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let b = match byte_operand(ctx.pop()) {
        Ok(value) => value,
        Err(step) => return step,
    };
    let a = match byte_operand(ctx.pop()) {
        Ok(value) => value,
        Err(step) => return step,
    };
    ctx.push_i32(std::cmp::max(a, b) as i32);
    StepResult::Continue
}

#[dotnet_intrinsic("static int System.Math::Abs(int)")]
pub fn intrinsic_math_abs_int<'gc, T: TypedStackOps<'gc>>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let val = ctx.pop_i32();
    ctx.push_i32(val.abs());
    StepResult::Continue
}

#[dotnet_intrinsic("static double System.Math::Sqrt(double)")]
pub fn intrinsic_math_sqrt<'gc, T: TypedStackOps<'gc>>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let val = ctx.pop_f64();
    ctx.push_f64(val.sqrt());
    StepResult::Continue
}

#[dotnet_intrinsic("static double System.Math::Acos(double)")]
#[dotnet_intrinsic("static float System.MathF::Acos(float)")]
#[dotnet_intrinsic("static double System.Math::Acosh(double)")]
#[dotnet_intrinsic("static float System.MathF::Acosh(float)")]
#[dotnet_intrinsic("static double System.Math::Asin(double)")]
#[dotnet_intrinsic("static float System.MathF::Asin(float)")]
#[dotnet_intrinsic("static double System.Math::Asinh(double)")]
#[dotnet_intrinsic("static float System.MathF::Asinh(float)")]
#[dotnet_intrinsic("static double System.Math::Atan(double)")]
#[dotnet_intrinsic("static float System.MathF::Atan(float)")]
#[dotnet_intrinsic("static double System.Math::Atanh(double)")]
#[dotnet_intrinsic("static float System.MathF::Atanh(float)")]
#[dotnet_intrinsic("static double System.Math::Atan2(double, double)")]
#[dotnet_intrinsic("static float System.MathF::Atan2(float, float)")]
#[dotnet_intrinsic("static double System.Math::Cbrt(double)")]
#[dotnet_intrinsic("static float System.MathF::Cbrt(float)")]
#[dotnet_intrinsic("static double System.Math::Ceiling(double)")]
#[dotnet_intrinsic("static float System.MathF::Ceiling(float)")]
#[dotnet_intrinsic("static double System.Math::Cos(double)")]
#[dotnet_intrinsic("static float System.MathF::Cos(float)")]
#[dotnet_intrinsic("static double System.Math::Cosh(double)")]
#[dotnet_intrinsic("static float System.MathF::Cosh(float)")]
#[dotnet_intrinsic("static double System.Math::Exp(double)")]
#[dotnet_intrinsic("static float System.MathF::Exp(float)")]
#[dotnet_intrinsic("static double System.Math::Floor(double)")]
#[dotnet_intrinsic("static float System.MathF::Floor(float)")]
#[dotnet_intrinsic("static double System.Math::Log(double)")]
#[dotnet_intrinsic("static float System.MathF::Log(float)")]
#[dotnet_intrinsic("static double System.Math::Log2(double)")]
#[dotnet_intrinsic("static float System.MathF::Log2(float)")]
#[dotnet_intrinsic("static double System.Math::Log10(double)")]
#[dotnet_intrinsic("static float System.MathF::Log10(float)")]
#[dotnet_intrinsic("static double System.Math::Sin(double)")]
#[dotnet_intrinsic("static float System.MathF::Sin(float)")]
#[dotnet_intrinsic("static double System.Math::Sinh(double)")]
#[dotnet_intrinsic("static float System.MathF::Sinh(float)")]
#[dotnet_intrinsic("static double System.Math::Tan(double)")]
#[dotnet_intrinsic("static float System.MathF::Tan(float)")]
#[dotnet_intrinsic("static double System.Math::Tanh(double)")]
#[dotnet_intrinsic("static float System.MathF::Tanh(float)")]
pub fn intrinsic_math_transcendental<'gc, T: TypedStackOps<'gc>>(
    ctx: &mut T,
    method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let operation = method.method().name.as_ref();
    if method.parent.type_name() == "System.MathF" {
        if operation == "Atan2" {
            let x = ctx.pop_f64() as f32;
            let y = ctx.pop_f64() as f32;
            ctx.push_f64(y.atan2(x) as f64);
            return StepResult::Continue;
        }
        let value = ctx.pop_f64() as f32;
        let result = match operation {
            "Acos" => value.acos(),
            "Acosh" => value.acosh(),
            "Asin" => value.asin(),
            "Asinh" => value.asinh(),
            "Atan" => value.atan(),
            "Atanh" => value.atanh(),
            "Cbrt" => value.cbrt(),
            "Ceiling" => value.ceil(),
            "Cos" => value.cos(),
            "Cosh" => value.cosh(),
            "Exp" => value.exp(),
            "Floor" => value.floor(),
            "Log" => value.ln(),
            "Log2" => value.log2(),
            "Log10" => value.log10(),
            "Sin" => value.sin(),
            "Sinh" => value.sinh(),
            "Tan" => value.tan(),
            "Tanh" => value.tanh(),
            _ => return StepResult::not_implemented(format!("MathF::{operation}")),
        };
        ctx.push_f64(result as f64);
    } else {
        if operation == "Atan2" {
            let x = ctx.pop_f64();
            let y = ctx.pop_f64();
            ctx.push_f64(y.atan2(x));
            return StepResult::Continue;
        }
        let value = ctx.pop_f64();
        let result = match operation {
            "Acos" => value.acos(),
            "Acosh" => value.acosh(),
            "Asin" => value.asin(),
            "Asinh" => value.asinh(),
            "Atan" => value.atan(),
            "Atanh" => value.atanh(),
            "Cbrt" => value.cbrt(),
            "Ceiling" => value.ceil(),
            "Cos" => value.cos(),
            "Cosh" => value.cosh(),
            "Exp" => value.exp(),
            "Floor" => value.floor(),
            "Log" => value.ln(),
            "Log2" => value.log2(),
            "Log10" => value.log10(),
            "Sin" => value.sin(),
            "Sinh" => value.sinh(),
            "Tan" => value.tan(),
            "Tanh" => value.tanh(),
            _ => return StepResult::not_implemented(format!("Math::{operation}")),
        };
        ctx.push_f64(result);
    }
    StepResult::Continue
}

#[dotnet_intrinsic("static double System.Math::FusedMultiplyAdd(double, double, double)")]
pub fn intrinsic_math_fused_multiply_add<'gc, T: TypedStackOps<'gc>>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let addend = ctx.pop_f64();
    let right = ctx.pop_f64();
    let left = ctx.pop_f64();
    ctx.push_f64(left.mul_add(right, addend));
    StepResult::Continue
}

#[dotnet_intrinsic("static float System.MathF::FusedMultiplyAdd(float, float, float)")]
pub fn intrinsic_mathf_fused_multiply_add<'gc, T: TypedStackOps<'gc>>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let addend = ctx.pop_f64() as f32;
    let right = ctx.pop_f64() as f32;
    let left = ctx.pop_f64() as f32;
    ctx.push_f64(left.mul_add(right, addend) as f64);
    StepResult::Continue
}

#[dotnet_intrinsic("static float System.MathF::Pow(float, float)")]
pub fn intrinsic_mathf_pow<'gc, T: TypedStackOps<'gc>>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let exponent = ctx.pop_f64() as f32;
    let value = ctx.pop_f64() as f32;
    ctx.push_f64(value.powf(exponent) as f64);
    StepResult::Continue
}

#[dotnet_intrinsic("static float System.MathF::Sqrt(float)")]
pub fn intrinsic_mathf_sqrt<'gc, T: TypedStackOps<'gc>>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let value = ctx.pop_f64() as f32;
    ctx.push_f64(value.sqrt() as f64);
    StepResult::Continue
}

// The float-rounding handlers preserve CoreLib's f32 intermediate rounding: its managed fallback
// relies on each add/subtract rounding to f32, while evaluation-stack floats are represented as f64.
#[dotnet_intrinsic("static float System.MathF::Round(float)")]
pub fn intrinsic_mathf_round<'gc, T: TypedStackOps<'gc>>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let value = (ctx.pop_f64() as f32).round_ties_even();
    ctx.push_f64(value as f64);
    StepResult::Continue
}

#[dotnet_intrinsic("static float System.MathF::Round(float, System.MidpointRounding)")]
pub fn intrinsic_mathf_round_with_mode<'gc, T: TypedStackOps<'gc> + ExceptionOps<'gc>>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let mode = ctx.pop_i32();
    let value = ctx.pop_f64() as f32;
    let rounded = match mode {
        0 => value.round_ties_even(),
        1 => value.round(),
        2 => value.trunc(),
        3 => value.floor(),
        4 => value.ceil(),
        _ => {
            return ctx.throw_by_name_with_message(
                "System.ArgumentException",
                "Invalid MidpointRounding value.",
            );
        }
    };
    ctx.push_f64(rounded as f64);
    StepResult::Continue
}

fn pop_math_output_pointer<'gc, T: TypedStackOps<'gc> + ExceptionOps<'gc>>(
    ctx: &mut T,
) -> Result<(PointerOrigin<'gc>, ByteOffset), StepResult> {
    match ctx.pop() {
        StackValue::ManagedPtr(pointer) if !pointer.is_null() => Ok((
            pointer.origin().clone(),
            pointer
                .byte_offset()
                .map_err(|e| StepResult::Error(e.into()))?,
        )),
        StackValue::UnmanagedPtr(pointer) => Ok((
            PointerOrigin::Unmanaged,
            ByteOffset::new(pointer.0.as_ptr().expose_provenance()),
        )),
        StackValue::NativeInt(address) if address != 0 => {
            Ok((PointerOrigin::Unmanaged, ByteOffset::new(address as usize)))
        }
        StackValue::ManagedPtr(_) | StackValue::NativeInt(0) => Err(ctx
            .throw_by_name_with_message(
                "System.NullReferenceException",
                "Object reference not set to an instance of an object.",
            )),
        other => Err(StepResult::type_error(
            "floating-point output pointer",
            format!("{other:?}"),
        )),
    }
}

fn write_math_output<'gc, T: TypedStackOps<'gc> + RawMemoryOps<'gc> + TypeLayoutOps>(
    ctx: &mut T,
    method: &MethodDescription,
    pointer: (PointerOrigin<'gc>, ByteOffset),
    value: f64,
    scalar: BaseType<ConcreteType>,
) -> StepResult {
    let layout = match ctx.type_layout(ConcreteType::new(method.resolution(), scalar)) {
        Ok(layout) => layout,
        Err(error) => return StepResult::Error(error.into()),
    };

    // SAFETY: F10.RawMemoryAccessValid — CoreLib calls these helpers with a writable pointer for
    // the matching scalar; `pop_math_output_pointer` preserves its exposed address for unmanaged
    // origins, and `write_unaligned` checks the complete range for managed origins using this layout.
    match unsafe {
        ctx.write_unaligned(
            pointer.0,
            pointer.1,
            StackValue::NativeFloat(value),
            &layout,
        )
    } {
        Ok(()) => StepResult::Continue,
        Err(error) => StepResult::Error(error.into()),
    }
}

#[dotnet_intrinsic("static double System.Math::ModF(double, double*)")]
pub fn intrinsic_math_modf<'gc, T>(
    ctx: &mut T,
    method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult
where
    T: TypedStackOps<'gc> + ExceptionOps<'gc> + RawMemoryOps<'gc> + TypeLayoutOps,
{
    let output = match pop_math_output_pointer(ctx) {
        Ok(output) => output,
        Err(step) => return step,
    };
    let value = ctx.pop_f64();
    let integral = value.trunc();
    let fraction = if value.is_nan() {
        value
    } else if value.is_infinite() {
        0.0_f64.copysign(value)
    } else {
        (value - integral).copysign(value)
    };
    match write_math_output(ctx, &method, output, integral, BaseType::Float64) {
        StepResult::Continue => {
            ctx.push_f64(fraction);
            StepResult::Continue
        }
        step => step,
    }
}

#[dotnet_intrinsic("static float System.MathF::ModF(float, float*)")]
pub fn intrinsic_mathf_modf<'gc, T>(
    ctx: &mut T,
    method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult
where
    T: TypedStackOps<'gc> + ExceptionOps<'gc> + RawMemoryOps<'gc> + TypeLayoutOps,
{
    let output = match pop_math_output_pointer(ctx) {
        Ok(output) => output,
        Err(step) => return step,
    };
    let value = ctx.pop_f64() as f32;
    let integral = value.trunc();
    let fraction = if value.is_nan() {
        value
    } else if value.is_infinite() {
        0.0_f32.copysign(value)
    } else {
        (value - integral).copysign(value)
    };
    match write_math_output(ctx, &method, output, integral as f64, BaseType::Float32) {
        StepResult::Continue => {
            ctx.push_f64(fraction as f64);
            StepResult::Continue
        }
        step => step,
    }
}

#[dotnet_intrinsic("static void System.Math::SinCos(double, double*, double*)")]
pub fn intrinsic_math_sincos<'gc, T>(
    ctx: &mut T,
    method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult
where
    T: TypedStackOps<'gc> + ExceptionOps<'gc> + RawMemoryOps<'gc> + TypeLayoutOps,
{
    let cos_output = match pop_math_output_pointer(ctx) {
        Ok(output) => output,
        Err(step) => return step,
    };
    let sin_output = match pop_math_output_pointer(ctx) {
        Ok(output) => output,
        Err(step) => return step,
    };
    let value = ctx.pop_f64();
    if let step @ StepResult::Error(_) =
        write_math_output(ctx, &method, sin_output, value.sin(), BaseType::Float64)
    {
        return step;
    }
    write_math_output(ctx, &method, cos_output, value.cos(), BaseType::Float64)
}

#[dotnet_intrinsic("static void System.MathF::SinCos(float, float*, float*)")]
pub fn intrinsic_mathf_sincos<'gc, T>(
    ctx: &mut T,
    method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult
where
    T: TypedStackOps<'gc> + ExceptionOps<'gc> + RawMemoryOps<'gc> + TypeLayoutOps,
{
    let cos_output = match pop_math_output_pointer(ctx) {
        Ok(output) => output,
        Err(step) => return step,
    };
    let sin_output = match pop_math_output_pointer(ctx) {
        Ok(output) => output,
        Err(step) => return step,
    };
    let value = ctx.pop_f64() as f32;
    if let step @ StepResult::Error(_) = write_math_output(
        ctx,
        &method,
        sin_output,
        value.sin() as f64,
        BaseType::Float32,
    ) {
        return step;
    }
    write_math_output(
        ctx,
        &method,
        cos_output,
        value.cos() as f64,
        BaseType::Float32,
    )
}

#[dotnet_intrinsic_field("static bool System.BitConverter::IsLittleEndian")]
pub fn intrinsic_bitconverter_is_little_endian<'gc, T: TypedStackOps<'gc>>(
    ctx: &mut T,
    _field: FieldDescription,
    _type_generics: Arc<[ConcreteType]>,
    _is_address: bool,
) -> StepResult {
    ctx.push_i32(1);
    StepResult::Continue
}

#[dotnet_intrinsic("static int System.Numerics.BitOperations::Log2(ulong)")]
pub fn intrinsic_bitoperations_log2_ulong<'gc, T: TypedStackOps<'gc>>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let val = ctx.pop_i64() as u64;
    let log2 = if val == 0 {
        0
    } else {
        (u64::BITS - 1 - val.leading_zeros()) as i32
    };
    ctx.push_i32(log2);
    StepResult::Continue
}

#[dotnet_intrinsic("static ulong System.UInt64::Log2(ulong)")]
pub fn intrinsic_uint64_log2<'gc, T: TypedStackOps<'gc>>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let val = ctx.pop_i64() as u64;
    let log2 = if val == 0 {
        0
    } else {
        (u64::BITS - 1 - val.leading_zeros()) as u64
    };
    ctx.push_i64(log2 as i64);
    StepResult::Continue
}

#[dotnet_intrinsic("static uint System.UInt32::Log2(uint)")]
pub fn intrinsic_uint32_log2<'gc, T: TypedStackOps<'gc>>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let val = ctx.pop_i32() as u32;
    let log2 = if val == 0 {
        0
    } else {
        u32::BITS - 1 - val.leading_zeros()
    };
    ctx.push_i32(log2 as i32);
    StepResult::Continue
}

#[dotnet_intrinsic("static int System.Numerics.BitOperations::Log2(uint)")]
pub fn intrinsic_bitoperations_log2_uint<'gc, T: TypedStackOps<'gc>>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let val = ctx.pop_i32() as u32;
    let log2 = if val == 0 {
        0
    } else {
        (u32::BITS - 1 - val.leading_zeros()) as i32
    };
    ctx.push_i32(log2);
    StepResult::Continue
}

#[dotnet_intrinsic("static uint System.Numerics.BitOperations::RotateLeft(uint, int)")]
pub fn intrinsic_bitoperations_rotate_left_uint<'gc, T: TypedStackOps<'gc>>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let offset = ctx.pop_i32() as u32;
    let value = ctx.pop_i32() as u32;
    ctx.push_i32(value.rotate_left(offset) as i32);
    StepResult::Continue
}

#[dotnet_intrinsic("static uint System.Numerics.BitOperations::RotateRight(uint, int)")]
pub fn intrinsic_bitoperations_rotate_right_uint<'gc, T: TypedStackOps<'gc>>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let offset = ctx.pop_i32() as u32;
    let value = ctx.pop_i32() as u32;
    ctx.push_i32(value.rotate_right(offset) as i32);
    StepResult::Continue
}

#[dotnet_intrinsic("static ulong System.Numerics.BitOperations::RotateLeft(ulong, int)")]
pub fn intrinsic_bitoperations_rotate_left_ulong<'gc, T: TypedStackOps<'gc>>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let offset = ctx.pop_i32() as u32;
    let value = ctx.pop_i64() as u64;
    ctx.push_i64(value.rotate_left(offset) as i64);
    StepResult::Continue
}

#[dotnet_intrinsic("static ulong System.Numerics.BitOperations::RotateRight(ulong, int)")]
pub fn intrinsic_bitoperations_rotate_right_ulong<'gc, T: TypedStackOps<'gc>>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let offset = ctx.pop_i32() as u32;
    let value = ctx.pop_i64() as u64;
    ctx.push_i64(value.rotate_right(offset) as i64);
    StepResult::Continue
}

#[dotnet_intrinsic(
    "static char* System.Diagnostics.Tracing.XplatEventLogger::EventSource_GetClrConfig(string)"
)]
pub fn intrinsic_eventsource_get_clr_config<'gc, T: TypedStackOps<'gc>>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let _ = ctx.pop();
    ctx.push_isize(0);
    StepResult::Continue
}

#[dotnet_intrinsic("static void System.Diagnostics.Debugger::LogInternal(int, string, string)")]
pub fn intrinsic_debugger_log_internal<'gc, T: TypedStackOps<'gc>>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    ctx.drop_top(3);
    StepResult::Continue
}

#[dotnet_intrinsic(
    "static string System.Exception::GetMessageFromNativeResources(System.Exception/ExceptionMessageKind)"
)]
pub fn intrinsic_exception_get_message_from_native_resources<'gc, T: TypedStackOps<'gc>>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let _ = ctx.pop();
    ctx.push_obj(ObjectRef(None));
    StepResult::Continue
}

#[dotnet_intrinsic(
    "static void System.Environment::FailFast(System.Runtime.CompilerServices.StackCrawlMarkHandle, string, System.Runtime.CompilerServices.ObjectHandleOnStack, string)"
)]
pub fn intrinsic_environment_failfast<'gc, T: TypedStackOps<'gc>>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    ctx.drop_top(4);
    StepResult::Continue
}

#[dotnet_intrinsic("static bool System.Diagnostics.Tracing.EventSource::get_IsSupported()")]
#[dotnet_intrinsic("bool System.Diagnostics.Tracing.EventSource::IsEnabled()")]
pub fn intrinsic_eventsource_disabled<'gc, T: TypedStackOps<'gc>>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    if !_method.signature().parameters.is_empty() {
        // Instance overloads consume `this` plus explicit parameters.
        ctx.drop_top(1 + _method.signature().parameters.len());
    } else if _method.signature().instance {
        ctx.pop();
    }
    ctx.push_i32(0);
    StepResult::Continue
}

#[cfg(test)]
mod byte_operand_tests {
    use super::*;

    #[test]
    fn integer_representations_truncate_to_byte() {
        for (value, expected) in [
            (StackValue::Int32(0), 0),
            (StackValue::Int32(255), 255),
            (StackValue::Int32(-1), 255),
            (StackValue::Int64(256), 0),
            (StackValue::Int64(i64::MIN + 257), 1),
            (StackValue::NativeInt(-257), 255),
            (StackValue::NativeInt(256), 0),
        ] {
            assert_eq!(byte_operand(value).unwrap(), expected);
        }
    }

    struct TestStack(dotnet_vm_data::stack::EvaluationStack<'static>);

    impl dotnet_vm_ops::ops::EvalStackOps<'static> for TestStack {
        fn push(&mut self, value: StackValue<'static>) {
            self.0.push(value);
        }
        fn pop(&mut self) -> StackValue<'static> {
            self.0.pop()
        }
        fn pop_safe(&mut self) -> Result<StackValue<'static>, VmError> {
            self.0.pop_safe()
        }
        fn pop_multiple(&mut self, count: usize) -> Vec<StackValue<'static>> {
            let mut values: Vec<_> = (0..count).map(|_| self.0.pop()).collect();
            values.reverse();
            values
        }
        fn peek(&self) -> Option<StackValue<'static>> {
            (self.0.top_of_stack().as_usize() != 0).then(|| self.0.peek_stack())
        }
        fn peek_stack(&self) -> StackValue<'static> {
            self.0.peek_stack()
        }
        fn peek_stack_at(&self, offset: usize) -> StackValue<'static> {
            self.0.peek_stack_at(offset)
        }
        fn top_of_stack(&self) -> dotnet_utils::StackSlotIndex {
            self.0.top_of_stack()
        }
    }
    impl TypedStackOps<'static> for TestStack {
        fn push_string(&mut self, _: dotnet_value::CLRString) {
            unreachable!()
        }
        fn push_interned_string(&mut self, _: &[u16]) {
            unreachable!()
        }
        fn push_interned_string_object(
            &mut self,
            _: &[u16],
            _: dotnet_value::object::ObjectRef<'static>,
        ) {
            unreachable!()
        }
        fn find_interned_string(&self, _: &[u16]) -> dotnet_value::object::ObjectRef<'static> {
            unreachable!()
        }
    }

    #[test]
    fn min_max_decode_each_operand_in_stack_order() {
        use dotnet_types::{TypeDescription, resolution::ResolutionS};
        use dotnet_vm_ops::ops::EvalStackOps;
        use dotnetdll::prelude::MethodMemberIndex;
        let method = MethodDescription::new(
            TypeDescription::NULL,
            GenericLookup::default(),
            ResolutionS::NULL,
            MethodMemberIndex::Method(0),
        );
        let lookup = GenericLookup::default();
        for (op, expected) in [
            (
                intrinsic_math_min_byte::<TestStack>
                    as fn(&mut TestStack, MethodDescription, &GenericLookup) -> StepResult,
                1,
            ),
            (intrinsic_math_max_byte::<TestStack>, 255),
        ] {
            let mut stack = TestStack(dotnet_vm_data::stack::EvaluationStack::new());
            stack.push(StackValue::NativeInt(-1));
            stack.push(StackValue::Int64(257));
            assert!(matches!(
                op(&mut stack, method.clone(), &lookup),
                StepResult::Continue
            ));
            assert!(matches!(stack.pop(), StackValue::Int32(value) if value == expected));

            stack.push(StackValue::Int32(42));
            stack.push(StackValue::NativeFloat(1.5));
            assert!(matches!(
                op(&mut stack, method.clone(), &lookup),
                StepResult::Error(VmError::Execution(ExecutionError::TypeMismatch {
                    expected: "byte-compatible integer",
                    ..
                }))
            ));
            assert!(matches!(stack.pop(), StackValue::Int32(42)));

            stack.push(StackValue::NativeFloat(1.5));
            stack.push(StackValue::Int32(42));
            assert!(matches!(
                op(&mut stack, method.clone(), &lookup),
                StepResult::Error(VmError::Execution(ExecutionError::TypeMismatch {
                    expected: "byte-compatible integer",
                    ..
                }))
            ));
            assert!(stack.peek().is_none());
        }
    }

    #[test]
    fn non_integer_reports_type_mismatch() {
        match byte_operand(StackValue::NativeFloat(1.5)) {
            Err(StepResult::Error(VmError::Execution(ExecutionError::TypeMismatch {
                expected,
                actual,
            }))) => {
                assert_eq!(expected, "byte-compatible integer");
                assert_eq!(&*actual, "NativeFloat(1.5)");
            }
            other => panic!("unexpected result: {other:?}"),
        }
    }
}

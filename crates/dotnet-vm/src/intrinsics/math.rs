use crate::{StepResult, resolution::ValueResolution, stack::ops::VesOps};
use dotnet_macros::{dotnet_intrinsic, dotnet_intrinsic_field};
use dotnet_types::{generics::GenericLookup, members::MethodDescription};
use dotnet_value::{
    StackValue,
    object::{HeapStorage, ObjectRef},
};
use std::sync::Arc;

#[dotnet_intrinsic("static bool System.Runtime.Intrinsics.Vector128::get_IsHardwareAccelerated()")]
#[dotnet_intrinsic("static bool System.Runtime.Intrinsics.Vector256::get_IsHardwareAccelerated()")]
#[dotnet_intrinsic("static bool System.Numerics.Vector::get_IsHardwareAccelerated()")]
pub fn intrinsic_vector_is_hardware_accelerated<'gc, 'm: 'gc>(
    ctx: &mut dyn VesOps<'gc, 'm>,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    ctx.push_i32(0);
    StepResult::Continue
}

#[dotnet_intrinsic(
    "static System.Collections.Generic.EqualityComparer<T> System.Collections.Generic.EqualityComparer<T>::get_Default()"
)]
pub fn intrinsic_equality_comparer_get_default<'gc, 'm: 'gc>(
    ctx: &mut dyn VesOps<'gc, 'm>,
    _method: MethodDescription,
    generics: &GenericLookup,
) -> StepResult {
    let target_type = generics.type_generics[0].clone();
    let comparer_type_name = "DotnetRs.Comparers.Equality/GenericEqualityComparer`1";
    let comparer_td = vm_try!(ctx.loader().corlib_type(comparer_type_name));

    let new_lookup = GenericLookup::new(vec![target_type]);
    let res_ctx = ctx.with_generics(generics).with_generics(&new_lookup);
    let instance = ObjectRef::new(
        ctx.gc(),
        HeapStorage::Obj(vm_try!(res_ctx.new_object(comparer_td))),
    );

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
pub fn intrinsic_numeric_create_truncating<'gc, 'm: 'gc>(
    ctx: &mut dyn VesOps<'gc, 'm>,
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
                _ => panic!("unsupported CreateTruncating target type: {}", target_type),
            }
        };
    }

    match value {
        StackValue::Int32(v) => convert!(v),
        StackValue::Int64(v) => convert!(v),
        StackValue::NativeInt(v) => convert!(v),
        _ => panic!("unsupported CreateTruncating source type: {:?}", value),
    }

    StepResult::Continue
}

#[dotnet_intrinsic("static bool System.Double::IsInfinity(double)")]
pub fn intrinsic_double_is_infinity<'gc, 'm: 'gc>(
    ctx: &mut dyn VesOps<'gc, 'm>,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let val = ctx.pop_f64();
    ctx.push_i32(if val.is_infinite() { 1 } else { 0 });
    StepResult::Continue
}

#[dotnet_intrinsic("static bool System.Double::IsNaN(double)")]
pub fn intrinsic_double_is_nan<'gc, 'm: 'gc>(
    ctx: &mut dyn VesOps<'gc, 'm>,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let val = ctx.pop_f64();
    ctx.push_i32(if val.is_nan() { 1 } else { 0 });
    StepResult::Continue
}

#[dotnet_intrinsic("static bool System.Double::IsNegativeInfinity(double)")]
pub fn intrinsic_double_is_negative_infinity<'gc, 'm: 'gc>(
    ctx: &mut dyn VesOps<'gc, 'm>,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let val = ctx.pop_f64();
    ctx.push_i32(if val == f64::NEG_INFINITY { 1 } else { 0 });
    StepResult::Continue
}

#[dotnet_intrinsic("static bool System.Double::IsPositiveInfinity(double)")]
pub fn intrinsic_double_is_positive_infinity<'gc, 'm: 'gc>(
    ctx: &mut dyn VesOps<'gc, 'm>,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let val = ctx.pop_f64();
    ctx.push_i32(if val == f64::INFINITY { 1 } else { 0 });
    StepResult::Continue
}

#[dotnet_intrinsic("static double System.Math::Min(double, double)")]
pub fn intrinsic_math_min_double<'gc, 'm: 'gc>(
    ctx: &mut dyn VesOps<'gc, 'm>,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let b = ctx.pop_f64();
    let a = ctx.pop_f64();
    ctx.push_f64(f64::min(a, b));
    StepResult::Continue
}

#[dotnet_intrinsic("static double System.Math::Max(double, double)")]
pub fn intrinsic_math_max_double<'gc, 'm: 'gc>(
    ctx: &mut dyn VesOps<'gc, 'm>,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let b = ctx.pop_f64();
    let a = ctx.pop_f64();
    ctx.push_f64(f64::max(a, b));
    StepResult::Continue
}

#[dotnet_intrinsic("static double System.Math::Abs(double)")]
pub fn intrinsic_math_abs_double<'gc, 'm: 'gc>(
    ctx: &mut dyn VesOps<'gc, 'm>,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let val = ctx.pop_f64();
    ctx.push_f64(val.abs());
    StepResult::Continue
}

#[dotnet_intrinsic("static double System.Math::Pow(double, double)")]
pub fn intrinsic_math_pow_double<'gc, 'm: 'gc>(
    ctx: &mut dyn VesOps<'gc, 'm>,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let y = ctx.pop_f64();
    let x = ctx.pop_f64();
    ctx.push_f64(x.powf(y));
    StepResult::Continue
}

#[dotnet_intrinsic("static int System.Math::Min(int, int)")]
pub fn intrinsic_math_min_int<'gc, 'm: 'gc>(
    ctx: &mut dyn VesOps<'gc, 'm>,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let b = ctx.pop_i32();
    let a = ctx.pop_i32();
    ctx.push_i32(std::cmp::min(a, b));
    StepResult::Continue
}

#[dotnet_intrinsic("static int System.Math::Max(int, int)")]
pub fn intrinsic_math_max_int<'gc, 'm: 'gc>(
    ctx: &mut dyn VesOps<'gc, 'm>,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let b = ctx.pop_i32();
    let a = ctx.pop_i32();
    ctx.push_i32(std::cmp::max(a, b));
    StepResult::Continue
}

#[dotnet_intrinsic("static int System.Math::Abs(int)")]
pub fn intrinsic_math_abs_int<'gc, 'm: 'gc>(
    ctx: &mut dyn VesOps<'gc, 'm>,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let val = ctx.pop_i32();
    ctx.push_i32(val.abs());
    StepResult::Continue
}

#[dotnet_intrinsic("static double System.Math::Sqrt(double)")]
pub fn intrinsic_math_sqrt<'gc, 'm: 'gc>(
    ctx: &mut dyn VesOps<'gc, 'm>,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let val = ctx.pop_f64();
    ctx.push_f64(val.sqrt());
    StepResult::Continue
}

#[dotnet_intrinsic_field("static bool System.BitConverter::IsLittleEndian")]
pub fn intrinsic_bitconverter_is_little_endian<'gc, 'm: 'gc>(
    ctx: &mut dyn VesOps<'gc, 'm>,
    _field: dotnet_types::members::FieldDescription,
    _type_generics: Arc<[dotnet_types::generics::ConcreteType]>,
    _is_address: bool,
) -> StepResult {
    ctx.push_i32(1);
    StepResult::Continue
}

#[dotnet_intrinsic("static int System.Numerics.BitOperations::Log2(ulong)")]
pub fn intrinsic_bitoperations_log2_ulong<'gc, 'm: 'gc>(
    ctx: &mut dyn VesOps<'gc, 'm>,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let val = ctx.pop_i64() as u64;
    ctx.push_i32(val.leading_zeros() as i32);
    StepResult::Continue
}

use crate::{
    StepResult, resolution::ValueResolution,
    stack::ops::VesOps,
};
use dotnet_macros::{dotnet_intrinsic, dotnet_intrinsic_field};
use dotnet_types::{generics::GenericLookup, members::MethodDescription};
use dotnet_utils::gc::GCHandle;
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
    gc: GCHandle<'gc>,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    ctx.push_i32(gc, 0);
    StepResult::Continue
}

#[dotnet_intrinsic(
    "static System.Collections.Generic.EqualityComparer<T> System.Collections.Generic.EqualityComparer<T>::get_Default()"
)]
pub fn intrinsic_equality_comparer_get_default<'gc, 'm: 'gc>(
    ctx: &mut dyn VesOps<'gc, 'm>,
    gc: GCHandle<'gc>,
    _method: MethodDescription,
    generics: &GenericLookup,
) -> StepResult {
    let target_type = generics.type_generics[0].clone();
    let comparer_type_name = "System.Collections.Generic.GenericEqualityComparer`1";
    let comparer_td = ctx.loader().corlib_type(comparer_type_name);

    let new_lookup = GenericLookup::new(vec![target_type]);
    let res_ctx = ctx.with_generics(generics).with_generics(&new_lookup);
    let instance = ObjectRef::new(gc, HeapStorage::Obj(res_ctx.new_object(comparer_td)));

    ctx.push_obj(gc, instance);
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
    gc: GCHandle<'gc>,
    method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let value = ctx.pop(gc);
    let target_type = method.parent.definition().type_name();

    macro_rules! convert {
        ($val:expr) => {
            match target_type.as_str() {
                "Byte" | "System.Byte" => ctx.push_i32(gc, ($val as u8) as i32),
                "SByte" | "System.SByte" => ctx.push_i32(gc, ($val as i8) as i32),
                "UInt16" | "System.UInt16" => ctx.push_i32(gc, ($val as u16) as i32),
                "Int16" | "System.Int16" => ctx.push_i32(gc, ($val as i16) as i32),
                "UInt32" | "System.UInt32" => ctx.push_i32(gc, ($val as u32) as i32),
                "Int32" | "System.Int32" => ctx.push_i32(gc, $val as i32),
                "UInt64" | "System.UInt64" => ctx.push_i64(gc, ($val as u64) as i64),
                "Int64" | "System.Int64" => ctx.push_i64(gc, $val as i64),
                "UIntPtr" | "System.UIntPtr" => ctx.push_isize(gc, ($val as usize) as isize),
                "IntPtr" | "System.IntPtr" => ctx.push_isize(gc, $val as isize),
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

#[dotnet_intrinsic("static double System.Math::Min(double, double)")]
pub fn intrinsic_math_min_double<'gc, 'm: 'gc>(
    ctx: &mut dyn VesOps<'gc, 'm>,
    gc: GCHandle<'gc>,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let b = ctx.pop_f64(gc);
    let a = ctx.pop_f64(gc);
    ctx.push_f64(gc, f64::min(a, b));
    StepResult::Continue
}

#[dotnet_intrinsic("static double System.Math::Max(double, double)")]
pub fn intrinsic_math_max_double<'gc, 'm: 'gc>(
    ctx: &mut dyn VesOps<'gc, 'm>,
    gc: GCHandle<'gc>,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let b = ctx.pop_f64(gc);
    let a = ctx.pop_f64(gc);
    ctx.push_f64(gc, f64::max(a, b));
    StepResult::Continue
}

#[dotnet_intrinsic("static double System.Math::Abs(double)")]
pub fn intrinsic_math_abs_double<'gc, 'm: 'gc>(
    ctx: &mut dyn VesOps<'gc, 'm>,
    gc: GCHandle<'gc>,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let val = ctx.pop_f64(gc);
    ctx.push_f64(gc, val.abs());
    StepResult::Continue
}

#[dotnet_intrinsic("static double System.Math::Pow(double, double)")]
pub fn intrinsic_math_pow_double<'gc, 'm: 'gc>(
    ctx: &mut dyn VesOps<'gc, 'm>,
    gc: GCHandle<'gc>,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let y = ctx.pop_f64(gc);
    let x = ctx.pop_f64(gc);
    ctx.push_f64(gc, x.powf(y));
    StepResult::Continue
}

#[dotnet_intrinsic("static int System.Math::Min(int, int)")]
pub fn intrinsic_math_min_int<'gc, 'm: 'gc>(
    ctx: &mut dyn VesOps<'gc, 'm>,
    gc: GCHandle<'gc>,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let b = ctx.pop_i32(gc);
    let a = ctx.pop_i32(gc);
    ctx.push_i32(gc, std::cmp::min(a, b));
    StepResult::Continue
}

#[dotnet_intrinsic("static int System.Math::Max(int, int)")]
pub fn intrinsic_math_max_int<'gc, 'm: 'gc>(
    ctx: &mut dyn VesOps<'gc, 'm>,
    gc: GCHandle<'gc>,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let b = ctx.pop_i32(gc);
    let a = ctx.pop_i32(gc);
    ctx.push_i32(gc, std::cmp::max(a, b));
    StepResult::Continue
}

#[dotnet_intrinsic("static int System.Math::Abs(int)")]
pub fn intrinsic_math_abs_int<'gc, 'm: 'gc>(
    ctx: &mut dyn VesOps<'gc, 'm>,
    gc: GCHandle<'gc>,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let val = ctx.pop_i32(gc);
    ctx.push_i32(gc, val.abs());
    StepResult::Continue
}

#[dotnet_intrinsic("static double System.Math::Sqrt(double)")]
pub fn intrinsic_math_sqrt<'gc, 'm: 'gc>(
    ctx: &mut dyn VesOps<'gc, 'm>,
    gc: GCHandle<'gc>,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let val = ctx.pop_f64(gc);
    ctx.push_f64(gc, val.sqrt());
    StepResult::Continue
}

#[dotnet_intrinsic_field("static bool System.BitConverter::IsLittleEndian")]
pub fn intrinsic_bitconverter_is_little_endian<'gc, 'm: 'gc>(
    ctx: &mut dyn VesOps<'gc, 'm>,
    gc: GCHandle<'gc>,
    _field: dotnet_types::members::FieldDescription,
    _type_generics: Arc<[dotnet_types::generics::ConcreteType]>,
    _is_address: bool,
) -> StepResult {
    ctx.push_i32(gc, 1);
    StepResult::Continue
}

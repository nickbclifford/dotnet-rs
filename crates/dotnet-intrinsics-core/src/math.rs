use dotnet_macros::{dotnet_intrinsic, dotnet_intrinsic_field};
use dotnet_types::{
    generics::{ConcreteType, GenericLookup},
    members::{FieldDescription, MethodDescription},
};
use dotnet_value::{
    StackValue,
    object::{HeapStorage, ObjectRef},
};
use dotnet_vm_ops::{
    StepResult,
    ops::{LoaderOps, MemoryOps, TypedStackOps},
};
use dotnetdll::prelude::{BaseType, MethodType, TypeSource, UserType};
use std::sync::Arc;

fn concrete_to_method_type(concrete: &ConcreteType) -> MethodType {
    let mapped = concrete
        .get()
        .clone()
        .map(|inner| concrete_to_method_type(&inner));
    MethodType::Base(Box::new(mapped))
}

#[dotnet_intrinsic("static bool System.Runtime.Intrinsics.Vector128::get_IsHardwareAccelerated()")]
#[dotnet_intrinsic("static bool System.Runtime.Intrinsics.Vector256::get_IsHardwareAccelerated()")]
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
    let target_type = generics.type_generics[0].clone();
    let comparer_type_name = "DotnetRs.Comparers.Equality/GenericEqualityComparer`1";
    let comparer_td = crate::vm_try!(ctx.loader().corlib_type(comparer_type_name));

    let comparer_method_type = MethodType::Base(Box::new(BaseType::Type {
        source: TypeSource::Generic {
            base: UserType::Definition(comparer_td.index),
            parameters: vec![concrete_to_method_type(&target_type)],
        },
        value_kind: None,
    }));
    // Resolve the generic comparer in the comparer type's owning resolution,
    // not the caller frame's source resolution.
    let comparer_concrete = crate::vm_try!(generics.make_concrete(
        comparer_td.resolution.clone(),
        comparer_method_type,
        ctx.loader().as_ref(),
    ));
    let comparer_closed_td = crate::vm_try!(ctx.loader().find_concrete_type(comparer_concrete));
    let comparer_lookup = GenericLookup::new(vec![target_type]);

    let instance = ObjectRef::new(
        ctx.gc_with_token(&ctx.no_active_borrows_token()),
        HeapStorage::Obj(crate::vm_try!(
            ctx.new_object_with_lookup(comparer_closed_td, &comparer_lookup)
        )),
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

#[dotnet_intrinsic("static byte System.Math::Min(byte, byte)")]
pub fn intrinsic_math_min_byte<'gc, T: TypedStackOps<'gc>>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let b = ctx.pop_i32() as u8;
    let a = ctx.pop_i32() as u8;
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

#[dotnet_intrinsic("static byte System.Math::Max(byte, byte)")]
pub fn intrinsic_math_max_byte<'gc, T: TypedStackOps<'gc>>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let b = ctx.pop_i32() as u8;
    let a = ctx.pop_i32() as u8;
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
    ctx.pop_multiple(3);
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
    ctx.pop_multiple(4);
    StepResult::Continue
}

#[dotnet_intrinsic("static bool System.Diagnostics.Tracing.EventSource::get_IsSupported()")]
#[dotnet_intrinsic("bool System.Diagnostics.Tracing.EventSource::IsEnabled()")]
pub fn intrinsic_eventsource_disabled<'gc, T: TypedStackOps<'gc>>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    if !_method.method().signature.parameters.is_empty() {
        // Instance overloads consume `this` plus explicit parameters.
        ctx.pop_multiple(1 + _method.method().signature.parameters.len());
    } else if _method.method().signature.instance {
        ctx.pop();
    }
    ctx.push_i32(0);
    StepResult::Continue
}

#[dotnet_intrinsic("static string System.SR::GetResourceString(string)")]
#[dotnet_intrinsic("static string System.SR::GetResourceString(string, string)")]
#[dotnet_intrinsic("static string System.SR::InternalGetResourceString(string)")]
pub fn intrinsic_sr_get_resource_string<'gc, T: TypedStackOps<'gc>>(
    ctx: &mut T,
    method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let arity = method.method().signature.parameters.len();
    let key = if arity == 2 {
        let fallback = ctx.pop();
        let key = ctx.pop();
        if matches!(key, StackValue::ObjectRef(ObjectRef(Some(_)))) {
            key
        } else {
            fallback
        }
    } else {
        ctx.pop()
    };

    match key {
        StackValue::ObjectRef(o) => ctx.push_obj(o),
        _ => ctx.push_obj(ObjectRef(None)),
    }
    StepResult::Continue
}

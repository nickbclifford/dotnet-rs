use crate::{
    context::ResolutionContext, resolution::ValueResolution, CallStack, StepResult,
};
use dotnet_types::{generics::GenericLookup, members::MethodDescription};
use dotnet_utils::gc::GCHandle;
use dotnet_value::{
    object::{HeapStorage, ObjectRef},
    StackValue,
};
use dotnet_macros::{dotnet_intrinsic, dotnet_intrinsic_field};

#[dotnet_intrinsic("static bool System.Runtime.Intrinsics.Vector128::get_IsHardwareAccelerated()")]
#[dotnet_intrinsic("static bool System.Runtime.Intrinsics.Vector256::get_IsHardwareAccelerated()")]
#[dotnet_intrinsic("static bool System.Numerics.Vector::get_IsHardwareAccelerated()")]
pub fn intrinsic_vector_is_hardware_accelerated<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    vm_push!(stack, gc, Int32(0));
    StepResult::InstructionStepped
}

#[dotnet_intrinsic("static System.Collections.Generic.EqualityComparer<T> System.Collections.Generic.EqualityComparer<T>::get_Default()")]
pub fn intrinsic_equality_comparer_get_default<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    method: MethodDescription,
    generics: &GenericLookup,
) -> StepResult {
    let target_type = generics.type_generics[0].clone();
    let comparer_type_name = "System.Collections.Generic.GenericEqualityComparer`1";
    let comparer_td = stack.loader().corlib_type(comparer_type_name);

    let new_lookup = GenericLookup::new(vec![target_type]);
    let ctx = ResolutionContext::for_method(
        method,
        stack.loader(),
        generics,
        stack.shared.caches.clone(),
    )
    .with_generics(&new_lookup);
    let instance = ObjectRef::new(gc, HeapStorage::Obj(ctx.new_object(comparer_td)));

    vm_push!(stack, gc, ObjectRef(instance));
    StepResult::InstructionStepped
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
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let value = vm_pop!(stack, gc);
    let target_type = method.parent.definition().type_name();

    macro_rules! convert {
        ($val:expr) => {
            match target_type.as_str() {
                "Byte" | "System.Byte" => vm_push!(stack, gc, Int32(($val as u8) as i32)),
                "SByte" | "System.SByte" => vm_push!(stack, gc, Int32(($val as i8) as i32)),
                "UInt16" | "System.UInt16" => vm_push!(stack, gc, Int32(($val as u16) as i32)),
                "Int16" | "System.Int16" => vm_push!(stack, gc, Int32(($val as i16) as i32)),
                "UInt32" | "System.UInt32" => vm_push!(stack, gc, Int32(($val as u32) as i32)),
                "Int32" | "System.Int32" => vm_push!(stack, gc, Int32($val as i32)),
                "UInt64" | "System.UInt64" => vm_push!(stack, gc, Int64(($val as u64) as i64)),
                "Int64" | "System.Int64" => vm_push!(stack, gc, Int64($val as i64)),
                "UIntPtr" | "System.UIntPtr" => {
                    vm_push!(stack, gc, NativeInt(($val as usize) as isize))
                }
                "IntPtr" | "System.IntPtr" => vm_push!(stack, gc, NativeInt($val as isize)),
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

    StepResult::InstructionStepped
}

#[dotnet_intrinsic("static double System.Math::Min(double, double)")]
pub fn intrinsic_math_min_double<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let b = vm_pop!(stack, gc);
    let a = vm_pop!(stack, gc);
    match (&a, &b) {
        (StackValue::NativeFloat(av), StackValue::NativeFloat(bv)) => {
            vm_push!(stack, gc, NativeFloat(av.min(*bv)));
        }
        _ => panic!(
            "Math.Min(double, double) called with non-double arguments: {:?}, {:?}",
            a, b
        ),
    }
    StepResult::InstructionStepped
}

#[dotnet_intrinsic("static double System.Math::Sqrt(double)")]
pub fn intrinsic_math_sqrt<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let val = vm_pop!(stack, gc);
    match val {
        StackValue::NativeFloat(f) => vm_push!(stack, gc, NativeFloat(f.sqrt())),
        _ => panic!("Math.Sqrt called with non-double argument: {:?}", val),
    }
    StepResult::InstructionStepped
}

#[dotnet_intrinsic_field("static bool System.BitConverter::IsLittleEndian")]
pub fn intrinsic_bitconverter_is_little_endian<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    _field: dotnet_types::members::FieldDescription,
    _type_generics: Vec<dotnet_types::generics::ConcreteType>,
    _is_address: bool,
) -> StepResult {
    vm_push!(stack, gc, Int32(1));
    StepResult::InstructionStepped
}

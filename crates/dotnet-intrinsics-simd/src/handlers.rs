use crate::SimdIntrinsicHost;
use dotnet_macros::dotnet_intrinsic;
use dotnet_types::{
    generics::{ConcreteType, GenericLookup},
    members::MethodDescription,
};
use dotnet_vm_data::StepResult;
use dotnetdll::prelude::BaseType;

#[inline]
fn vector_element_type_is_supported(ty: &ConcreteType) -> bool {
    matches!(
        ty.get(),
        BaseType::Int8
            | BaseType::UInt8
            | BaseType::Int16
            | BaseType::UInt16
            | BaseType::Int32
            | BaseType::UInt32
            | BaseType::Int64
            | BaseType::UInt64
            | BaseType::Float32
            | BaseType::Float64
            | BaseType::IntPtr
            | BaseType::UIntPtr
    )
}

#[dotnet_intrinsic("static bool System.Runtime.Intrinsics.Vector128::get_IsHardwareAccelerated()")]
pub fn intrinsic_vector128_is_hardware_accelerated<'gc, T: SimdIntrinsicHost<'gc>>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    ctx.push_i32(i32::from(ctx.simd_vector128_is_hardware_accelerated()));
    StepResult::Continue
}

#[dotnet_intrinsic("static bool System.Runtime.Intrinsics.Vector256::get_IsHardwareAccelerated()")]
pub fn intrinsic_vector256_is_hardware_accelerated<'gc, T: SimdIntrinsicHost<'gc>>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    ctx.push_i32(i32::from(ctx.simd_vector256_is_hardware_accelerated()));
    StepResult::Continue
}

#[dotnet_intrinsic("static bool System.Runtime.Intrinsics.Vector64<T>::get_IsSupported()")]
#[dotnet_intrinsic("static bool System.Runtime.Intrinsics.Vector128<T>::get_IsSupported()")]
#[dotnet_intrinsic("static bool System.Runtime.Intrinsics.Vector256<T>::get_IsSupported()")]
pub fn intrinsic_vector_is_supported<'gc, T: SimdIntrinsicHost<'gc>>(
    ctx: &mut T,
    _method: MethodDescription,
    generics: &GenericLookup,
) -> StepResult {
    let supported = generics
        .type_generics
        .first()
        .is_some_and(vector_element_type_is_supported);
    ctx.push_i32(i32::from(supported));
    StepResult::Continue
}

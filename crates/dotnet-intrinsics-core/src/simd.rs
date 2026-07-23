//! Conservative `System.Runtime.Intrinsics` capability handlers.
//!
//! Hardware acceleration remains disabled until the VM implements the broader
//! `Vector128`/`Vector256` intrinsic surface.

use dotnet_macros::dotnet_intrinsic;
use dotnet_types::{
    generics::{ConcreteType, GenericLookup},
    members::MethodDescription,
};
use dotnet_vm_ops::{StepResult, ops::SimdIntrinsicHost};
use dotnetdll::prelude::BaseType;

#[inline]
pub fn vector128_is_hardware_accelerated() -> bool {
    false
}

#[inline]
pub fn vector256_is_hardware_accelerated() -> bool {
    false
}

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

#[cfg(test)]
mod tests {
    use super::{vector128_is_hardware_accelerated, vector256_is_hardware_accelerated};

    #[test]
    fn hardware_acceleration_is_disabled_until_full_intrinsics_support_lands() {
        assert!(!vector128_is_hardware_accelerated());
        assert!(!vector256_is_hardware_accelerated());
    }

    #[test]
    fn vector256_probe_implies_vector128_probe() {
        assert!(!vector256_is_hardware_accelerated() || vector128_is_hardware_accelerated());
    }
}

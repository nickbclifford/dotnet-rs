use crate::SimdIntrinsicHost;
use dotnet_macros::dotnet_intrinsic;
use dotnet_types::{generics::GenericLookup, members::MethodDescription};
use dotnet_vm_data::StepResult;

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

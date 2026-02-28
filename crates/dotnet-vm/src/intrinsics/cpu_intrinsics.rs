//! CPU-specific intrinsics (SIMD, SSE, AVX, etc.)
//!
//! These intrinsics expose hardware-specific features. Since dotnet-rs is a
//! software VM, we return `false` for all `IsSupported` queries to indicate
//! that hardware intrinsics are not available.
use crate::{StepResult, stack::ops::TypedStackOps};
use dotnet_macros::dotnet_intrinsic;
use dotnet_types::{generics::GenericLookup, members::MethodDescription};

// X86 Intrinsics - all IsSupported return false

#[dotnet_intrinsic("static bool System.Runtime.Intrinsics.X86.X86Base::get_IsSupported()")]
pub fn intrinsic_x86base_is_supported<'gc, 'm: 'gc, T: TypedStackOps<'gc>>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    ctx.push_i32(0);
    StepResult::Continue
}

#[dotnet_intrinsic("static bool System.Runtime.Intrinsics.X86.X86Base/X64::get_IsSupported()")]
pub fn intrinsic_x86base_x64_is_supported<'gc, 'm: 'gc, T: TypedStackOps<'gc>>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    ctx.push_i32(0);
    StepResult::Continue
}

#[dotnet_intrinsic("static bool System.Runtime.Intrinsics.X86.Lzcnt::get_IsSupported()")]
pub fn intrinsic_lzcnt_is_supported<'gc, 'm: 'gc, T: TypedStackOps<'gc>>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    ctx.push_i32(0);
    StepResult::Continue
}

#[dotnet_intrinsic("static bool System.Runtime.Intrinsics.X86.Lzcnt/X64::get_IsSupported()")]
pub fn intrinsic_lzcnt_x64_is_supported<'gc, 'm: 'gc, T: TypedStackOps<'gc>>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    ctx.push_i32(0);
    StepResult::Continue
}

#[dotnet_intrinsic("static bool System.Runtime.Intrinsics.X86.Sse::get_IsSupported()")]
pub fn intrinsic_sse_is_supported<'gc, 'm: 'gc, T: TypedStackOps<'gc>>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    ctx.push_i32(0);
    StepResult::Continue
}

#[dotnet_intrinsic("static bool System.Runtime.Intrinsics.X86.Sse2::get_IsSupported()")]
pub fn intrinsic_sse2_is_supported<'gc, 'm: 'gc, T: TypedStackOps<'gc>>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    ctx.push_i32(0);
    StepResult::Continue
}

#[dotnet_intrinsic("static bool System.Runtime.Intrinsics.X86.Sse2/X64::get_IsSupported()")]
pub fn intrinsic_sse2_x64_is_supported<'gc, 'm: 'gc, T: TypedStackOps<'gc>>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    ctx.push_i32(0);
    StepResult::Continue
}

#[dotnet_intrinsic("static bool System.Runtime.Intrinsics.X86.Sse3::get_IsSupported()")]
pub fn intrinsic_sse3_is_supported<'gc, 'm: 'gc, T: TypedStackOps<'gc>>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    ctx.push_i32(0);
    StepResult::Continue
}

#[dotnet_intrinsic("static bool System.Runtime.Intrinsics.X86.Ssse3::get_IsSupported()")]
pub fn intrinsic_ssse3_is_supported<'gc, 'm: 'gc, T: TypedStackOps<'gc>>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    ctx.push_i32(0);
    StepResult::Continue
}

#[dotnet_intrinsic("static bool System.Runtime.Intrinsics.X86.Sse41::get_IsSupported()")]
pub fn intrinsic_sse41_is_supported<'gc, 'm: 'gc, T: TypedStackOps<'gc>>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    ctx.push_i32(0);
    StepResult::Continue
}

#[dotnet_intrinsic("static bool System.Runtime.Intrinsics.X86.Sse42::get_IsSupported()")]
pub fn intrinsic_sse42_is_supported<'gc, 'm: 'gc, T: TypedStackOps<'gc>>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    ctx.push_i32(0);
    StepResult::Continue
}

#[dotnet_intrinsic("static bool System.Runtime.Intrinsics.X86.Avx::get_IsSupported()")]
pub fn intrinsic_avx_is_supported<'gc, 'm: 'gc, T: TypedStackOps<'gc>>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    ctx.push_i32(0);
    StepResult::Continue
}

#[dotnet_intrinsic("static bool System.Runtime.Intrinsics.X86.Avx2::get_IsSupported()")]
pub fn intrinsic_avx2_is_supported<'gc, 'm: 'gc, T: TypedStackOps<'gc>>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    ctx.push_i32(0);
    StepResult::Continue
}

#[dotnet_intrinsic("static bool System.Runtime.Intrinsics.X86.Bmi1::get_IsSupported()")]
pub fn intrinsic_bmi1_is_supported<'gc, 'm: 'gc, T: TypedStackOps<'gc>>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    ctx.push_i32(0);
    StepResult::Continue
}

#[dotnet_intrinsic("static bool System.Runtime.Intrinsics.X86.Bmi2::get_IsSupported()")]
pub fn intrinsic_bmi2_is_supported<'gc, 'm: 'gc, T: TypedStackOps<'gc>>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    ctx.push_i32(0);
    StepResult::Continue
}

#[dotnet_intrinsic("static bool System.Runtime.Intrinsics.X86.Pclmulqdq::get_IsSupported()")]
pub fn intrinsic_pclmulqdq_is_supported<'gc, 'm: 'gc, T: TypedStackOps<'gc>>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    ctx.push_i32(0);
    StepResult::Continue
}

#[dotnet_intrinsic("static bool System.Runtime.Intrinsics.X86.Popcnt::get_IsSupported()")]
pub fn intrinsic_popcnt_is_supported<'gc, 'm: 'gc, T: TypedStackOps<'gc>>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    ctx.push_i32(0);
    StepResult::Continue
}

#[dotnet_intrinsic("static bool System.Runtime.Intrinsics.X86.Aes::get_IsSupported()")]
pub fn intrinsic_aes_is_supported<'gc, 'm: 'gc, T: TypedStackOps<'gc>>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    ctx.push_i32(0);
    StepResult::Continue
}

// ARM intrinsics

#[dotnet_intrinsic("static bool System.Runtime.Intrinsics.Arm.AdvSimd::get_IsSupported()")]
pub fn intrinsic_advsimd_is_supported<'gc, 'm: 'gc, T: TypedStackOps<'gc>>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    ctx.push_i32(0);
    StepResult::Continue
}

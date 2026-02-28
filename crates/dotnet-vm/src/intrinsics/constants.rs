use crate::{StepResult, TypedStackOps};
use dotnet_macros::dotnet_intrinsic;
use dotnet_types::{generics::GenericLookup, members::MethodDescription};

#[dotnet_intrinsic(
    "static bool System.Runtime.CompilerServices.RuntimeHelpers::IsKnownConstant(string)"
)]
pub fn is_known_constant<'gc, 'm: 'gc, T: TypedStackOps<'gc>>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    ctx.push_i32(0);
    StepResult::Continue
}

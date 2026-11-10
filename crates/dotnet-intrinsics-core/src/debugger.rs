use dotnet_macros::dotnet_intrinsic;
use dotnet_types::{generics::GenericLookup, members::MethodDescription};
use dotnet_vm_data::StepResult;
use dotnet_vm_ops::ops::TypedStackOps;

/// System.Diagnostics.Debugger::get_IsAttached()
///
/// CoreLib's property forwards to a `QCall`. No managed debugger can attach to this VM, so the
/// answer is always `false`. Framework code asks it from hot paths (`TaskScheduler..ctor`,
/// `AsyncTaskMethodBuilder.GetStateMachineBox`, `Task.InternalWaitCore`), and
/// `NotifyOfCrossThreadDependency` only acts when it returns `true`.
#[dotnet_intrinsic("static bool System.Diagnostics.Debugger::get_IsAttached()")]
pub fn intrinsic_debugger_is_attached<'gc, T: TypedStackOps<'gc>>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    ctx.push_i32(0);
    StepResult::Continue
}

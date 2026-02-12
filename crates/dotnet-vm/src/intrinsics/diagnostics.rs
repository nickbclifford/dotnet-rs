use crate::{StepResult, stack::ops::VesOps};
use dotnet_macros::dotnet_intrinsic;
use dotnet_types::{generics::GenericLookup, members::MethodDescription};

#[dotnet_intrinsic(
    "static bool System.Diagnostics.Tracing.XplatEventLogger::IsEventSourceLoggingEnabled()"
)]
pub fn intrinsic_is_event_source_logging_enabled<'gc, 'm: 'gc>(
    ctx: &mut dyn VesOps<'gc, 'm>,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    // Return false (0)
    ctx.push_i32(0);
    StepResult::Continue
}

#[dotnet_intrinsic(
    "static nint System.Diagnostics.Tracing.EventPipeInternal::CreateProvider(string, void*, void*)"
)]
pub fn intrinsic_eventpipe_create_provider<'gc, 'm: 'gc>(
    ctx: &mut dyn VesOps<'gc, 'm>,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    // static extern nint CreateProvider(string providerName, delegate* callback, void* callbackContext);
    // Arguments are popped in reverse order.
    let _context = ctx.pop();
    let _callback = ctx.pop();
    let _provider_name = ctx.pop();

    // Return value: nint (handle).
    // Pushing 0 (invalid handle) to satisfy caller.
    ctx.push_isize(0);

    StepResult::Continue
}

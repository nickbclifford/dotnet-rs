use crate::{StepResult, stack::ops::VesOps};
use dotnet_macros::dotnet_intrinsic;
use dotnet_types::{generics::GenericLookup, members::MethodDescription};
use dotnet_utils::gc::GCHandle;

#[dotnet_intrinsic(
    "static bool System.Diagnostics.Tracing.XplatEventLogger::IsEventSourceLoggingEnabled()"
)]
pub fn intrinsic_is_event_source_logging_enabled<'gc, 'm: 'gc>(
    ctx: &mut dyn VesOps<'gc, 'm>,
    gc: GCHandle<'gc>,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    // Return false (0)
    ctx.push_i32(gc, 0);
    StepResult::Continue
}

#[dotnet_intrinsic(
    "static ulong System.Diagnostics.Tracing.EventPipeInternal::CreateProvider(string, IntPtr, IntPtr)"
)]
pub fn intrinsic_eventpipe_create_provider<'gc, 'm: 'gc>(
    ctx: &mut dyn VesOps<'gc, 'm>,
    gc: GCHandle<'gc>,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    // static extern ulong CreateProvider(string providerName, IntPtr callback, IntPtr callbackContext);
    // Arguments are popped in reverse order.
    let _context = ctx.pop(gc);
    let _callback = ctx.pop(gc);
    let _provider_name = ctx.pop(gc);

    // Return value: likely ulong (handle).
    // Pushing 0 (invalid handle) to satisfy caller.
    ctx.push_i64(gc, 0);

    StepResult::Continue
}

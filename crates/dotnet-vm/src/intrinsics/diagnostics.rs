use crate::{vm_pop, vm_push, CallStack, StepResult};
use dotnet_macros::dotnet_intrinsic;
use dotnet_types::{generics::GenericLookup, members::MethodDescription};
use dotnet_utils::gc::GCHandle;

// use dotnet_value::StackValue;

#[dotnet_intrinsic(
    "static bool System.Diagnostics.Tracing.XplatEventLogger::IsEventSourceLoggingEnabled()"
)]
pub fn intrinsic_is_event_source_logging_enabled<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    // Return false (0)
    vm_push!(stack, gc, Int32(0));
    StepResult::InstructionStepped
}

#[dotnet_intrinsic("static ulong System.Diagnostics.Tracing.EventPipeInternal::CreateProvider(string, IntPtr, IntPtr)")]
pub fn intrinsic_eventpipe_create_provider<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    // static extern ulong CreateProvider(string providerName, IntPtr callback, IntPtr callbackContext);
    // Arguments are popped in reverse order.
    let _context = vm_pop!(stack, gc);
    let _callback = vm_pop!(stack, gc);
    let _provider_name = vm_pop!(stack, gc);

    // Return value: likely ulong (handle).
    // Pushing 0 (invalid handle) to satisfy caller.
    vm_push!(stack, gc, NativeInt(0));

    StepResult::InstructionStepped
}

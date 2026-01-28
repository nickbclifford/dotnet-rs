use crate::{vm_push, CallStack, StepResult};
use dotnet_types::{generics::GenericLookup, members::MethodDescription};
use dotnet_utils::gc::GCHandle;

// use dotnet_value::StackValue;

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

//! Delegate invocation support for runtime-managed delegate methods.
//!
//! Delegates have methods (ctor, Invoke, BeginInvoke, EndInvoke) with no CIL body -
//! they are implemented by the runtime (ECMA-335 §II.14.6).
use crate::{DelegateInvokeHost, helpers::get_delegate_info};
use dotnet_types::{generics::GenericLookup, members::MethodDescription};
use dotnet_vm_data::StepResult;
use dotnet_vm_ops::{
    intrinsic_args::{ArgPolicy, expect_stack_object_with_policy},
    ops::DelegateIntrinsicHost,
};

pub(super) fn invoke_delegate<'gc, T: DelegateIntrinsicHost<'gc> + DelegateInvokeHost<'gc>>(
    ctx: &mut T,
    invoke_method: MethodDescription,
    _lookup: &GenericLookup,
    inline: bool,
) -> StepResult {
    let num_invoke_args = invoke_method.signature().parameters.len();

    // Stack order: [delegate_instance, arg0, arg1, ..., argN]. Keep only the invoke arguments
    // in the VM-owned reusable buffer, then pop the receiver separately. This avoids allocating
    // a temporary argument vector and copying it again for the target call.
    ctx.pop_call_args_into_buffer(num_invoke_args);
    let delegate_value = ctx.pop();

    let delegate_ref = match expect_stack_object_with_policy(
        ctx,
        &delegate_value,
        "delegate object reference",
        ArgPolicy::ManagedNullNre,
    ) {
        Ok(delegate_ref) => delegate_ref,
        Err(step) => return step,
    };

    let (target, method_index) = get_delegate_info(ctx, delegate_ref);

    // Look up the actual method from the registry
    let (target_method, target_lookup) = ctx.delegate_lookup_method_by_index(method_index);
    let mut args = std::mem::take(ctx.call_args_buffer_mut());

    // Match PreparedCall::for_delegate_target without transferring ownership of the reusable
    // buffer: a bound target (closed delegate) is followed by the invoke arguments; an open
    // delegate's first invoke argument is the callee's `this` or first parameter.
    if dotnet_vm_ops::prepared_call::delegate_target_is_bound(&target_method, args.len()) {
        ctx.push(dotnet_value::StackValue::ObjectRef(target));
    }
    for arg in args.drain(..) {
        ctx.push(arg);
    }
    *ctx.call_args_buffer_mut() = args;

    // Dispatch to the target method
    if inline {
        ctx.delegate_dispatch_method_inline(target_method, target_lookup)
    } else {
        ctx.delegate_dispatch_method(target_method, target_lookup)
    }
}

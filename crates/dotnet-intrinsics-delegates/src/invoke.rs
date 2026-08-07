//! Delegate invocation support for runtime-managed delegate methods.
//!
//! Delegates have methods (ctor, Invoke, BeginInvoke, EndInvoke) with no CIL body -
//! they are implemented by the runtime (ECMA-335 §II.14.6).
use crate::{
    DelegateInvokeHost,
    helpers::{get_delegate_info, get_multicast_targets_ref},
};
use dotnet_macros::dotnet_intrinsic;
use dotnet_types::{generics::GenericLookup, members::MethodDescription};
use dotnet_vm_data::{MulticastState, StepResult};
use dotnet_vm_ops::{
    intrinsic_args::{ArgPolicy, expect_stack_object_with_policy},
    ops::{DelegateIntrinsicHost, ExceptionOps},
};

pub(super) fn invoke_delegate<'gc, T: DelegateIntrinsicHost<'gc> + DelegateInvokeHost<'gc>>(
    ctx: &mut T,
    invoke_method: MethodDescription,
    _lookup: &GenericLookup,
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

    // Check for multicast targets
    let multicast_targets = if let Some(targets_ref) = get_multicast_targets_ref(ctx, delegate_ref)
    {
        let targets_len = targets_ref.as_vector(|v| v.layout.length);
        if targets_len > 1 {
            Some(
                targets_ref
                    .0
                    .expect("get_multicast_targets_ref returned a non-null object reference"),
            )
        } else {
            // If len == 1, check if it's not 'this'
            let first_target = targets_ref.as_vector(|v| {
                let gc = ctx.gc_with_token(&ctx.no_active_borrows_token());
                v.object_ref_elements(&gc)
                    .next()
                    .expect("multicast targets vector must contain first element")
            });
            if first_target != delegate_ref {
                Some(
                    targets_ref
                        .0
                        .expect("get_multicast_targets_ref returned a non-null object reference"),
                )
            } else {
                None
            }
        }
    } else {
        None
    };

    if let Some(targets_handle) = multicast_targets {
        // Push a dummy frame for the current Invoke method
        let method_info = dotnet_vm_ops::vm_try!(ctx.delegate_method_info(invoke_method, _lookup));
        let args = std::mem::take(ctx.call_args_buffer_mut());

        // Push a copy for the first target's frame while the multicast state retains the reusable
        // vector for subsequent targets. The vector itself is moved; no argument-vector copy is
        // made.
        ctx.push(dotnet_value::StackValue::ObjectRef(delegate_ref));
        for arg in &args {
            ctx.push(arg.clone());
        }

        dotnet_vm_ops::vm_try!(ctx.delegate_call_frame(method_info, _lookup.clone()));

        // Set multicast state
        ctx.frame_stack_mut().current_frame_mut().multicast_state = Some(MulticastState {
            targets: targets_handle,
            next_index: 0,
            args,
        });

        return StepResult::FramePushed;
    }

    let (target, method_index) = get_delegate_info(ctx, delegate_ref);

    // Look up the actual method from the registry
    let (target_method, target_lookup) = ctx.delegate_lookup_method_by_index(method_index);
    let mut args = std::mem::take(ctx.call_args_buffer_mut());

    // Match PreparedCall::for_delegate_target without transferring ownership of the reusable
    // buffer: an instance target (or a closed static target) is followed by the invoke arguments.
    if target_method.signature().instance || target.0.is_some() {
        ctx.push(dotnet_value::StackValue::ObjectRef(target));
    }
    for arg in args.drain(..) {
        ctx.push(arg);
    }
    *ctx.call_args_buffer_mut() = args;

    // Dispatch to the target method
    ctx.delegate_dispatch_method(target_method, target_lookup)
}

#[dotnet_intrinsic("object System.Delegate::DynamicInvoke(object[])")]
pub fn delegate_dynamic_invoke<'gc, T: ExceptionOps<'gc>>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    // ...
    ctx.throw_by_name_with_message(
        "System.NotSupportedException",
        "DynamicInvoke is not supported.",
    )
}

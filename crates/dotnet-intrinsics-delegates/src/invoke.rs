//! Delegate invocation support for runtime-managed delegate methods.
//!
//! Delegates have methods (ctor, Invoke, BeginInvoke, EndInvoke) with no CIL body -
//! they are implemented by the runtime (ECMA-335 §II.14.6).
use crate::{DelegateInvokeHost, NULL_REF_MSG, helpers::get_multicast_targets_ref};
use dotnet_macros::dotnet_intrinsic;
use dotnet_types::{generics::GenericLookup, members::MethodDescription};
use dotnet_value::{StackValue, object::ObjectRef};
use dotnet_vm_ops::{
    MulticastState, StepResult,
    ops::{DelegateIntrinsicHost, ExceptionOps},
};

pub(super) fn invoke_delegate<'gc, T: DelegateIntrinsicHost<'gc> + DelegateInvokeHost<'gc>>(
    ctx: &mut T,
    invoke_method: MethodDescription,
    _lookup: &GenericLookup,
) -> StepResult {
    let num_invoke_args = invoke_method.method().signature.parameters.len();

    // Stack order: [delegate_instance, arg0, arg1, ..., argN]
    // pop_multiple returns them in order they were on stack.
    let args = ctx.pop_multiple(num_invoke_args + 1);

    let delegate_ref = match &args[0] {
        StackValue::ObjectRef(r) => r,
        _ => panic!("Expected delegate object reference, got {:?}", &args[0]),
    };

    if delegate_ref.0.is_none() {
        return ctx.throw_by_name_with_message("System.NullReferenceException", NULL_REF_MSG);
    }

    // Check for multicast targets
    let multicast_targets = if let Some(targets_ref) = get_multicast_targets_ref(ctx, *delegate_ref)
    {
        let targets_len = targets_ref.as_vector(|v| v.layout.length);
        if targets_len > 1 {
            Some(targets_ref.0.unwrap())
        } else {
            // If len == 1, check if it's not 'this'
            let first_target = targets_ref.as_vector(|v| {
                let offset = 0;
                unsafe {
                    ObjectRef::read_branded(
                        &v.get()[offset..],
                        &ctx.gc_with_token(&ctx.no_active_borrows_token()),
                    )
                }
            });
            if first_target != *delegate_ref {
                Some(targets_ref.0.unwrap())
            } else {
                None
            }
        }
    } else {
        None
    };

    if let Some(targets_handle) = multicast_targets {
        // Push a dummy frame for the current Invoke method
        let method_info = crate::vm_try!(ctx.delegate_method_info(invoke_method, _lookup));

        // Push arguments back onto stack so call_frame can consume them
        for arg in &args {
            ctx.push(arg.clone());
        }

        crate::vm_try!(ctx.delegate_call_frame(method_info, _lookup.clone()));

        // Set multicast state
        ctx.frame_stack_mut().current_frame_mut().multicast_state = Some(MulticastState {
            targets: targets_handle,
            next_index: 0,
            args: args[1..].to_vec(),
        });

        return StepResult::FramePushed;
    }

    let (target, method_index) = delegate_ref.as_object(|instance| {
        let delegate_type = ctx
            .loader()
            .corlib_type("System.Delegate")
            .expect("System.Delegate must exist");

        // Read Target field
        let target = instance
            .instance_storage
            .field::<ObjectRef<'gc>>(delegate_type.clone(), "_target")
            .unwrap()
            .read();

        // Read _method field
        let method_index = instance
            .instance_storage
            .field::<usize>(delegate_type, "_method")
            .unwrap()
            .read();

        (target, method_index)
    });

    // Look up the actual method from the registry
    let (target_method, target_lookup) = ctx.delegate_lookup_method_by_index(method_index);

    // Push arguments back onto stack
    if target_method.method().signature.instance {
        ctx.push(StackValue::ObjectRef(target));
    }

    for arg in &args[1..] {
        ctx.push(arg.clone());
    }

    // Dispatch to the target method
    ctx.delegate_dispatch_method(target_method, target_lookup.clone())
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

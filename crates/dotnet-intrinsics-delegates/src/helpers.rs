//! Delegate invocation support for runtime-managed delegate methods.
//!
//! Delegates have methods (ctor, Invoke, BeginInvoke, EndInvoke) with no CIL body -
//! they are implemented by the runtime (ECMA-335 §II.14.6).
use crate::{DelegateInvokeHost, invoke::invoke_delegate};
use dotnet_types::{
    TypeDescription,
    generics::{ConcreteType, GenericLookup},
    members::MethodDescription,
};
use dotnet_value::object::ObjectRef;
use dotnet_vm_ops::{
    StepResult,
    ops::{DelegateIntrinsicHost, LoaderOps, MemoryOps, ResolutionOps},
};

#[allow(dead_code)]
const NULL_REF_MSG: &str = "Object reference not set to an instance of an object.";
const NOT_SUPPORTED_MSG: &str = "BeginInvoke and EndInvoke are not supported.";

/// Check if a type is a delegate type (inherits from System.Delegate or System.MulticastDelegate)
pub fn is_delegate_type<'gc, T: LoaderOps + ResolutionOps<'gc>>(
    ctx: &T,
    td: TypeDescription,
) -> bool {
    let delegate = match ctx.loader().corlib_type("System.Delegate") {
        Ok(v) => v,
        Err(_) => return false,
    };
    let multicast = match ctx.loader().corlib_type("System.MulticastDelegate") {
        Ok(v) => v,
        Err(_) => return false,
    };

    let td_concrete = ConcreteType::from(td);
    ctx.is_a(td_concrete.clone(), ConcreteType::from(delegate))
        .unwrap_or(false)
        || ctx
            .is_a(td_concrete, ConcreteType::from(multicast))
            .unwrap_or(false)
}

/// Try to dispatch a delegate runtime method. Returns Some(result) if handled.
pub fn try_delegate_dispatch<'gc, T: DelegateIntrinsicHost<'gc> + DelegateInvokeHost<'gc>>(
    ctx: &mut T,
    method: MethodDescription,
    lookup: &GenericLookup,
) -> Option<StepResult> {
    // Quick check: only handle methods without bodies
    if method.method().body.is_some() {
        return None;
    }

    // Check if this is a delegate type
    if !is_delegate_type(ctx, method.parent.clone()) {
        return None;
    }

    let method_name = &*method.method().name;
    match method_name {
        "Invoke" => Some(invoke_delegate(ctx, method, lookup)),
        ".ctor" => None, // Constructor is handled by support library stub
        "BeginInvoke" => {
            Some(ctx.throw_by_name_with_message("System.NotSupportedException", NOT_SUPPORTED_MSG))
        }
        "EndInvoke" => {
            Some(ctx.throw_by_name_with_message("System.NotSupportedException", NOT_SUPPORTED_MSG))
        }
        _ => None,
    }
}

pub(super) fn get_delegate_info<'gc, T: LoaderOps>(
    ctx: &T,
    obj: ObjectRef<'gc>,
) -> (ObjectRef<'gc>, usize) {
    obj.as_object(|instance| {
        let delegate_type = ctx
            .loader()
            .corlib_type("System.Delegate")
            .expect("System.Delegate must exist");
        let target = instance
            .instance_storage
            .field::<ObjectRef<'gc>>(delegate_type.clone(), "_target")
            .unwrap()
            .read();
        let index = instance
            .instance_storage
            .field::<usize>(delegate_type, "_method")
            .unwrap()
            .read();
        (target, index)
    })
}

pub(super) fn get_multicast_targets_ref<'gc, T: LoaderOps + ResolutionOps<'gc>>(
    ctx: &T,
    obj: ObjectRef<'gc>,
) -> Option<ObjectRef<'gc>> {
    obj.as_object(|instance| {
        let multicast_type = ctx
            .loader()
            .corlib_type("System.MulticastDelegate")
            .expect("System.MulticastDelegate must exist");

        if !ctx
            .is_a(
                ConcreteType::from(instance.description.clone()),
                ConcreteType::from(multicast_type.clone()),
            )
            .unwrap_or(false)
        {
            return None;
        }

        let targets_ref = instance
            .instance_storage
            .field::<ObjectRef<'gc>>(multicast_type, "targets")
            .unwrap()
            .read();
        if targets_ref.0.is_some() {
            Some(targets_ref)
        } else {
            None
        }
    })
}

pub(super) fn delegates_equal<'gc, T: LoaderOps + ResolutionOps<'gc>>(
    ctx: &T,
    a: ObjectRef<'gc>,
    b: ObjectRef<'gc>,
) -> bool {
    if a == b {
        return true;
    }
    if a.0.is_none() || b.0.is_none() {
        return false;
    }

    let (target_a, index_a) = get_delegate_info(ctx, a);
    let (target_b, index_b) = get_delegate_info(ctx, b);

    target_a == target_b && index_a == index_b
}

pub(super) fn get_invocation_list<'gc, T: LoaderOps + ResolutionOps<'gc> + MemoryOps<'gc>>(
    ctx: &T,
    obj: ObjectRef<'gc>,
) -> Vec<ObjectRef<'gc>> {
    if let Some(targets_ref) = get_multicast_targets_ref(ctx, obj) {
        targets_ref.as_vector(|v| {
            let len = v.layout.length;
            let mut result = Vec::with_capacity(len);
            for i in 0..len {
                let entry = unsafe {
                    ObjectRef::read_branded(
                        &v.get()[i * ObjectRef::SIZE..],
                        &ctx.gc_with_token(&ctx.no_active_borrows_token()),
                    )
                };
                result.push(entry);
            }
            result
        })
    } else {
        vec![obj]
    }
}

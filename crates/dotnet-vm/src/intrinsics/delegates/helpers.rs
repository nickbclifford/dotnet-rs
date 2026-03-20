//! Delegate invocation support for runtime-managed delegate methods.
//!
//! Delegates have methods (ctor, Invoke, BeginInvoke, EndInvoke) with no CIL body -
//! they are implemented by the runtime (ECMA-335 §II.14.6).
use crate::{
    StepResult,
    stack::ops::{
        CallOps, EvalStackOps, ExceptionOps, LoaderOps, MemoryOps, ReflectionOps, ResolutionOps,
        VesInternals,
    },
};
use dotnet_types::{
    TypeDescription, comparer::decompose_type_source, generics::GenericLookup,
    members::MethodDescription,
};
use dotnet_value::object::ObjectRef;
use dotnetdll::prelude::*;

#[allow(dead_code)]
const NULL_REF_MSG: &str = "Object reference not set to an instance of an object.";
const NOT_SUPPORTED_MSG: &str = "BeginInvoke and EndInvoke are not supported.";

use super::invoke::invoke_delegate;

/// Check if a type is a delegate type (inherits from System.Delegate or System.MulticastDelegate)
pub fn is_delegate_type<'gc, T: LoaderOps + ResolutionOps<'gc>>(
    ctx: &T,
    mut td: TypeDescription,
) -> bool {
    loop {
        let definition = td.definition();
        let raw_type_name = definition.type_name();
        let type_name = ctx.loader().canonical_type_name(&raw_type_name);
        if type_name == "System.Delegate" || type_name == "System.MulticastDelegate" {
            return true;
        }

        if let Some(extends) = &definition.extends {
            let (ut, _) = decompose_type_source::<MemberType>(extends);
            let base_td = ctx
                .resolver()
                .locate_type(td.resolution.clone(), ut)
                .expect("Failed to locate base type");
            if base_td == td {
                break;
            }
            td = base_td;
        } else {
            break;
        }
    }
    false
}

/// Try to dispatch a delegate runtime method. Returns Some(result) if handled.
pub fn try_delegate_dispatch<
    'gc,
    T: ExceptionOps<'gc>
        + CallOps<'gc>
        + EvalStackOps<'gc>
        + ReflectionOps<'gc>
        + VesInternals<'gc>
        + LoaderOps
        + ResolutionOps<'gc>
        + MemoryOps<'gc>,
>(
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

        let mut curr = instance.description.clone();
        let mut is_multicast = false;
        loop {
            let raw_type_name = curr.type_name();
            if ctx.loader().canonical_type_name(&raw_type_name) == "System.MulticastDelegate" {
                is_multicast = true;
                break;
            }
            if let Some(parent) = curr.definition().extends.as_ref() {
                let (ut, _) = decompose_type_source::<MemberType>(parent);
                let next = ctx
                    .resolver()
                    .locate_type(curr.resolution.clone(), ut)
                    .expect("Failed to locate base type");
                if next == curr {
                    break;
                }
                curr = next;
            } else {
                break;
            }
        }
        if !is_multicast {
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
                        &ctx.gc_with_token(&dotnet_utils::NoActiveBorrows::new()),
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

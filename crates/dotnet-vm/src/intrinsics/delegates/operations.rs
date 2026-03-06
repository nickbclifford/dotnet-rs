//! Delegate invocation support for runtime-managed delegate methods.
//!
//! Delegates have methods (ctor, Invoke, BeginInvoke, EndInvoke) with no CIL body -
//! they are implemented by the runtime (ECMA-335 §II.14.6).
use crate::{
    StepResult,
    stack::ops::{
        EvalStackOps, ExceptionOps, LoaderOps, MemoryOps, ReflectionOps, ResolutionOps,
        TypedStackOps,
    },
};
use dotnet_macros::dotnet_intrinsic;
use dotnet_types::{
    generics::{ConcreteType, GenericLookup},
    members::MethodDescription,
};
use dotnet_value::{
    StackValue,
    object::{HeapStorage, ObjectRef},
};

const NULL_REF_MSG: &str = "Object reference not set to an instance of an object.";
#[allow(dead_code)]
const NOT_SUPPORTED_MSG: &str = "BeginInvoke and EndInvoke are not supported.";

use super::helpers::*;

#[dotnet_intrinsic("object System.Delegate::get_Target()")]
pub fn delegate_get_target<'gc, T: TypedStackOps<'gc> + ExceptionOps<'gc> + LoaderOps>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let this = ctx.pop_obj();
    if this.0.is_none() {
        return ctx.throw_by_name_with_message("System.NullReferenceException", NULL_REF_MSG);
    }

    let target = this.as_object(|instance| {
        let delegate_type = ctx
            .loader()
            .corlib_type("System.Delegate")
            .expect("System.Delegate must exist");
        instance
            .instance_storage
            .field::<ObjectRef<'gc>>(delegate_type, "_target")
            .unwrap()
            .read()
    });

    ctx.push_obj(target);
    StepResult::Continue
}

#[dotnet_intrinsic("System.Reflection.MethodInfo System.Delegate::get_Method()")]
pub fn delegate_get_method<
    'gc,
    T: TypedStackOps<'gc> + ExceptionOps<'gc> + LoaderOps + ReflectionOps<'gc>,
>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let this = ctx.pop_obj();
    if this.0.is_none() {
        return ctx.throw_by_name_with_message("System.NullReferenceException", NULL_REF_MSG);
    }

    let method_index = this.as_object(|instance| {
        let delegate_type = ctx
            .loader()
            .corlib_type("System.Delegate")
            .expect("System.Delegate must exist");
        instance
            .instance_storage
            .field::<usize>(delegate_type, "_method")
            .unwrap()
            .read()
    });

    let (target_method, target_lookup) = ctx.lookup_method_by_index(method_index);
    let method_obj = ctx.get_runtime_method_obj(target_method, target_lookup);
    ctx.push_obj(method_obj);
    StepResult::Continue
}

#[dotnet_intrinsic("bool System.Delegate::Equals(object)")]
#[dotnet_intrinsic("bool System.MulticastDelegate::Equals(object)")]
pub fn delegate_equals<
    'gc,
    T: EvalStackOps<'gc>
        + TypedStackOps<'gc>
        + ExceptionOps<'gc>
        + LoaderOps
        + ResolutionOps<'gc>
        + ReflectionOps<'gc>
        + MemoryOps<'gc>,
>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let other_val = ctx.pop();
    let this_obj = ctx.pop_obj();

    if this_obj.0.is_none() {
        return ctx.throw_by_name_with_message("System.NullReferenceException", NULL_REF_MSG);
    }

    let other_obj = match other_val {
        StackValue::ObjectRef(obj) => obj,
        _ => {
            ctx.push_i32(0);
            return StepResult::Continue;
        }
    };

    if other_obj.0.is_none() {
        ctx.push_i32(0);
        return StepResult::Continue;
    }

    if this_obj == other_obj {
        ctx.push_i32(1);
        return StepResult::Continue;
    }

    // Check if other is a delegate
    if !is_delegate_type(ctx, vm_try!(ctx.get_heap_description(other_obj))) {
        ctx.push_i32(0);
        return StepResult::Continue;
    }

    // Compare targets and methods
    let (this_target, this_index) = get_delegate_info(ctx, this_obj);
    let (other_target, other_index) = get_delegate_info(ctx, other_obj);

    if this_index != other_index || this_target != other_target {
        ctx.push_i32(0);
        return StepResult::Continue;
    }

    // If they are multicast, we should ideally compare invocation lists.
    // However, ECMA-335 says for MulticastDelegate they are equal if they have the same invocation list.
    // Our MulticastDelegate stub uses 'targets' array.
    let this_invocation_list_ref = get_multicast_targets_ref(ctx, this_obj);
    let other_invocation_list_ref = get_multicast_targets_ref(ctx, other_obj);

    match (this_invocation_list_ref, other_invocation_list_ref) {
        (Some(this_ref), Some(other_ref)) => {
            let this_len = this_ref.as_vector(|v| v.layout.length);
            let other_len = other_ref.as_vector(|v| v.layout.length);
            if this_len != other_len {
                ctx.push_i32(0);
            } else {
                let mut equal = true;
                for i in 0..this_len {
                    let t1 = this_ref.as_vector(|v| unsafe {
                        ObjectRef::read_branded(
                            &v.get()[i * ObjectRef::SIZE..],
                            &ctx.gc_with_token(&dotnet_utils::NoActiveBorrows::new()),
                        )
                    });
                    let t2 = other_ref.as_vector(|v| unsafe {
                        ObjectRef::read_branded(
                            &v.get()[i * ObjectRef::SIZE..],
                            &ctx.gc_with_token(&dotnet_utils::NoActiveBorrows::new()),
                        )
                    });
                    // We should probably call Equals recursively or compare info
                    // For now, let's just compare info of elements
                    let (i1, idx1) = get_delegate_info(ctx, t1);
                    let (i2, idx2) = get_delegate_info(ctx, t2);
                    if i1 != i2 || idx1 != idx2 {
                        equal = false;
                        break;
                    }
                }
                ctx.push_i32(if equal { 1 } else { 0 });
            }
        }
        (None, None) => ctx.push_i32(1), // Already compared basic info
        _ => ctx.push_i32(0),
    }

    StepResult::Continue
}

#[dotnet_intrinsic("int System.Delegate::GetHashCode()")]
#[dotnet_intrinsic("int System.MulticastDelegate::GetHashCode()")]
pub fn delegate_get_hash_code<
    'gc,
    T: TypedStackOps<'gc> + ExceptionOps<'gc> + LoaderOps + ResolutionOps<'gc>,
>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let this = ctx.pop_obj();
    if this.0.is_none() {
        return ctx.throw_by_name_with_message("System.NullReferenceException", NULL_REF_MSG);
    }

    let (target, index) = get_delegate_info(ctx, this);
    let mut hash = index as i32;
    if let Some(t) = target.0 {
        hash ^= unsafe { t.as_ptr() as i32 };
    }

    // For multicast, include targets?
    if let Some(list_ref) = get_multicast_targets_ref(ctx, this) {
        hash ^= list_ref.as_vector(|v| v.layout.length as i32);
    }

    ctx.push_i32(hash);
    StepResult::Continue
}

#[dotnet_intrinsic(
    "static System.Delegate System.Delegate::Combine(System.Delegate, System.Delegate)"
)]
pub fn delegate_combine<
    'gc,
    T: TypedStackOps<'gc> + LoaderOps + ResolutionOps<'gc> + MemoryOps<'gc>,
>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let b = ctx.pop_obj();
    let a = ctx.pop_obj();

    if a.0.is_none() {
        ctx.push_obj(b);
        return StepResult::Continue;
    }
    if b.0.is_none() {
        ctx.push_obj(a);
        return StepResult::Continue;
    }

    let list_a = get_invocation_list(ctx, a);
    let list_b = get_invocation_list(ctx, b);

    let mut combined = Vec::with_capacity(list_a.len() + list_b.len());
    combined.extend(list_a);
    combined.extend(list_b);

    // Create new delegate of same type as a
    let new_delegate = ctx.clone_object(a);

    // Create new array for targets
    let delegate_type = vm_try!(ctx.loader().corlib_type("System.Delegate"));
    let delegate_concrete = ConcreteType::from(delegate_type);
    let array_v = vm_try!(ctx.new_vector(delegate_concrete, combined.len()));
    let array_obj = ObjectRef::new(
        ctx.gc_with_token(&dotnet_utils::NoActiveBorrows::new()),
        HeapStorage::Vec(array_v),
    );
    ctx.register_new_object(&array_obj);

    array_obj.as_vector_mut(
        ctx.gc_with_token(&dotnet_utils::NoActiveBorrows::new()),
        |v| {
            for (i, &el) in combined.iter().enumerate() {
                el.write(&mut v.get_mut()[i * ObjectRef::SIZE..]);
            }
        },
    );

    // Set 'targets' field on new_delegate
    let multicast_type = vm_try!(ctx.loader().corlib_type("System.MulticastDelegate"));
    new_delegate.as_object_mut(
        ctx.gc_with_token(&dotnet_utils::NoActiveBorrows::new()),
        |instance| {
            array_obj.write(
                &mut instance
                    .instance_storage
                    .get_field_mut_local(multicast_type, "targets"),
            );
        },
    );

    ctx.push_obj(new_delegate);
    StepResult::Continue
}

#[dotnet_intrinsic(
    "static System.Delegate System.Delegate::Remove(System.Delegate, System.Delegate)"
)]
pub fn delegate_remove<
    'gc,
    T: TypedStackOps<'gc> + LoaderOps + ResolutionOps<'gc> + MemoryOps<'gc> + ReflectionOps<'gc>,
>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let value = ctx.pop_obj();
    let source = ctx.pop_obj();

    if source.0.is_none() {
        ctx.push_obj(source);
        return StepResult::Continue;
    }
    if value.0.is_none() {
        ctx.push_obj(source);
        return StepResult::Continue;
    }

    let list_source = get_invocation_list(ctx, source);
    let list_value = get_invocation_list(ctx, value);

    if list_value.is_empty() {
        ctx.push_obj(source);
        return StepResult::Continue;
    }

    // Find last occurrence of list_value in list_source
    let mut found_index = None;
    if list_source.len() >= list_value.len() {
        for i in (0..=list_source.len() - list_value.len()).rev() {
            let mut match_found = true;
            for j in 0..list_value.len() {
                if !delegates_equal(ctx, list_source[i + j], list_value[j]) {
                    match_found = false;
                    break;
                }
            }
            if match_found {
                found_index = Some(i);
                break;
            }
        }
    }

    if let Some(idx) = found_index {
        let mut new_list = Vec::with_capacity(list_source.len() - list_value.len());
        new_list.extend_from_slice(&list_source[..idx]);
        new_list.extend_from_slice(&list_source[idx + list_value.len()..]);

        if new_list.is_empty() {
            ctx.push_obj(ObjectRef(None));
        } else if new_list.len() == 1 {
            ctx.push_obj(new_list[0]);
        } else {
            // Create new MulticastDelegate
            let new_delegate = ctx.clone_object(source);
            let delegate_type = vm_try!(ctx.loader().corlib_type("System.Delegate"));
            let delegate_concrete = ConcreteType::from(delegate_type);
            let array_v = vm_try!(ctx.new_vector(delegate_concrete, new_list.len()));
            let array_obj = ObjectRef::new(
                ctx.gc_with_token(&dotnet_utils::NoActiveBorrows::new()),
                dotnet_value::object::HeapStorage::Vec(array_v),
            );
            ctx.register_new_object(&array_obj);

            array_obj.as_vector_mut(
                ctx.gc_with_token(&dotnet_utils::NoActiveBorrows::new()),
                |v| {
                    for (i, &el) in new_list.iter().enumerate() {
                        el.write(&mut v.get_mut()[i * ObjectRef::SIZE..]);
                    }
                },
            );

            let multicast_type = vm_try!(ctx.loader().corlib_type("System.MulticastDelegate"));
            new_delegate.as_object_mut(
                ctx.gc_with_token(&dotnet_utils::NoActiveBorrows::new()),
                |instance| {
                    array_obj.write(
                        &mut instance
                            .instance_storage
                            .get_field_mut_local(multicast_type, "targets"),
                    );
                },
            );

            ctx.push_obj(new_delegate);
        }
    } else {
        ctx.push_obj(source);
    }

    StepResult::Continue
}

/*
fn get_runtime_return_type<'gc, T: VesOps<'gc>>(ctx: &T, res_ctx: &ResolutionContext<'_>, method: &MethodDescription) -> RuntimeType {
    match &method.method().signature.return_type.1 {
        Some(dotnetdll::prelude::ParameterType::Value(t))
        | Some(dotnetdll::prelude::ParameterType::Ref(t)) => {
            ctx.make_runtime_type(res_ctx, t)
        }
        Some(dotnetdll::prelude::ParameterType::TypedReference) => {
            todo!("TypedReference in DynamicInvoke return")
        }
        None => RuntimeType::Void,
    }
}
*/

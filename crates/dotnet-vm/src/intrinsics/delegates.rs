//! Delegate invocation support for runtime-managed delegate methods.
//!
//! Delegates have methods (ctor, Invoke, BeginInvoke, EndInvoke) with no CIL body -
//! they are implemented by the runtime (ECMA-335 §II.14.6).
use crate::{
    MethodInfo, StepResult,
    stack::{context::MulticastState, ops::VesOps},
};
use dotnet_macros::dotnet_intrinsic;
use dotnet_types::{
    TypeDescription,
    comparer::decompose_type_source,
    generics::{ConcreteType, GenericLookup},
    members::MethodDescription,
};
use dotnet_value::{StackValue, object::ObjectRef};
use dotnetdll::prelude::*;

/// Check if a type is a delegate type (inherits from System.Delegate or System.MulticastDelegate)
pub fn is_delegate_type<'gc, 'm, T: VesOps<'gc, 'm> + ?Sized>(
    ctx: &T,
    mut td: TypeDescription,
) -> bool {
    loop {
        let definition = td.definition();
        let type_name = definition.type_name();
        if type_name == "System.Delegate" || type_name == "System.MulticastDelegate" {
            return true;
        }

        // Also check for DotnetRs stubs
        if type_name == "DotnetRs.Delegate" || type_name == "DotnetRs.MulticastDelegate" {
            return true;
        }

        if let Some(extends) = &definition.extends {
            let (ut, _) = decompose_type_source::<MemberType>(extends);
            let base_td = ctx
                .resolver()
                .locate_type(td.resolution, ut)
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
pub fn try_delegate_dispatch<'gc, 'm, T: VesOps<'gc, 'm> + ?Sized>(
    ctx: &mut T,
    method: MethodDescription,
    lookup: &GenericLookup,
) -> Option<StepResult> {
    // Quick check: only handle methods without bodies
    if method.method.body.is_some() {
        return None;
    }

    // Check if this is a delegate type
    if !is_delegate_type(ctx, method.parent) {
        return None;
    }

    let method_name = &*method.method.name;
    match method_name {
        "Invoke" => Some(invoke_delegate(ctx, method, lookup)),
        ".ctor" => None, // Constructor is handled by support library stub
        "BeginInvoke" => Some(ctx.throw_by_name("System.NotSupportedException")),
        "EndInvoke" => Some(ctx.throw_by_name("System.NotSupportedException")),
        _ => None,
    }
}

fn invoke_delegate<'gc, 'm, T: VesOps<'gc, 'm> + ?Sized>(
    ctx: &mut T,
    invoke_method: MethodDescription,
    _lookup: &GenericLookup,
) -> StepResult {
    let num_invoke_args = invoke_method.method.signature.parameters.len();

    // Stack order: [delegate_instance, arg0, arg1, ..., argN]
    // pop_multiple returns them in order they were on stack.
    let args = ctx.pop_multiple(num_invoke_args + 1);

    let delegate_ref = match &args[0] {
        StackValue::ObjectRef(r) => r,
        _ => panic!("Expected delegate object reference, got {:?}", &args[0]),
    };

    if delegate_ref.0.is_none() {
        return ctx.throw_by_name("System.NullReferenceException");
    }

    // Check for multicast targets
    let multicast_targets = delegate_ref.as_object(|instance| {
        let multicast_type = ctx
            .loader()
            .corlib_type("DotnetRs.MulticastDelegate")
            .expect("DotnetRs.MulticastDelegate must exist");

        // We can't use is_assignable_to easily here because it needs a TypeComparer or similar
        // Let's use is_delegate_type's logic or just check if it's a MulticastDelegate
        let mut is_multicast = false;
        let mut curr = instance.description;
        loop {
            if curr.type_name() == "DotnetRs.MulticastDelegate"
                || curr.type_name() == "System.MulticastDelegate"
            {
                is_multicast = true;
                break;
            }
            if let Some(parent) = curr.definition().extends.as_ref() {
                let (ut, _) = decompose_type_source::<MemberType>(parent);
                let next = ctx
                    .resolver()
                    .locate_type(curr.resolution, ut)
                    .expect("Failed to locate base type");
                if next == curr {
                    break;
                }
                curr = next;
            } else {
                break;
            }
        }

        if is_multicast {
            let targets_bytes = instance
                .instance_storage
                .get_field_local(multicast_type, "targets");
            let targets_ref = unsafe { ObjectRef::read_unchecked(&targets_bytes) };
            if let Some(targets_handle) = targets_ref.0 {
                let targets_len = targets_ref.as_vector(|v| v.layout.length);
                if targets_len > 1 {
                    return Some(targets_handle);
                }
                // If len == 1, check if it's not 'this'
                let first_target = targets_ref.as_vector(|v| {
                    let offset = 0;
                    unsafe { ObjectRef::read_unchecked(&v.get()[offset..]) }
                });
                if first_target != *delegate_ref {
                    return Some(targets_handle);
                }
            }
        }
        None
    });

    if let Some(targets_handle) = multicast_targets {
        // Push a dummy frame for the current Invoke method
        let method_info = vm_try!(MethodInfo::new(
            invoke_method,
            _lookup,
            ctx.shared().clone()
        ));

        // Push arguments back onto stack so call_frame can consume them
        for arg in &args {
            ctx.push(arg.clone());
        }

        vm_try!(ctx.call_frame(method_info, _lookup.clone()));

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
            .corlib_type("DotnetRs.Delegate")
            .expect("DotnetRs.Delegate must exist");

        // Read Target field
        let target_bytes = instance
            .instance_storage
            .get_field_local(delegate_type, "_target");
        let target = unsafe { ObjectRef::read_unchecked(&target_bytes) };

        // Read _method field
        let method_handle_bytes = instance
            .instance_storage
            .get_field_local(delegate_type, "_method");
        let method_index = isize::from_ne_bytes(
            method_handle_bytes[..8]
                .try_into()
                .expect("RuntimeMethodHandle size mismatch"),
        ) as usize;

        (target, method_index)
    });

    // Look up the actual method from the registry
    let (target_method, target_lookup) = ctx.lookup_method_by_index(method_index);

    // Push arguments back onto stack
    if target_method.method.signature.instance {
        ctx.push(StackValue::ObjectRef(target));
    }

    for arg in &args[1..] {
        ctx.push(arg.clone());
    }

    // Dispatch to the target method
    ctx.dispatch_method(target_method, target_lookup.clone())
}

#[dotnet_intrinsic("object System.Delegate::get_Target()")]
#[dotnet_intrinsic("object DotnetRs.Delegate::get_Target()")]
pub fn delegate_get_target<'gc, 'm: 'gc>(
    ctx: &mut dyn VesOps<'gc, 'm>,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let this = ctx.pop_obj();
    if this.0.is_none() {
        return ctx.throw_by_name("System.NullReferenceException");
    }

    let target = this.as_object(|instance| {
        let delegate_type = ctx
            .loader()
            .corlib_type("DotnetRs.Delegate")
            .expect("DotnetRs.Delegate must exist");
        let target_bytes = instance
            .instance_storage
            .get_field_local(delegate_type, "_target");
        unsafe { ObjectRef::read_unchecked(&target_bytes) }
    });

    ctx.push_obj(target);
    StepResult::Continue
}

#[dotnet_intrinsic("System.Reflection.MethodInfo System.Delegate::get_Method()")]
#[dotnet_intrinsic("System.Reflection.MethodInfo DotnetRs.Delegate::get_Method()")]
pub fn delegate_get_method<'gc, 'm: 'gc>(
    ctx: &mut dyn VesOps<'gc, 'm>,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let this = ctx.pop_obj();
    if this.0.is_none() {
        return ctx.throw_by_name("System.NullReferenceException");
    }

    let method_index = this.as_object(|instance| {
        let delegate_type = ctx
            .loader()
            .corlib_type("DotnetRs.Delegate")
            .expect("DotnetRs.Delegate must exist");
        let method_handle_bytes = instance
            .instance_storage
            .get_field_local(delegate_type, "_method");
        isize::from_ne_bytes(
            method_handle_bytes[..8]
                .try_into()
                .expect("RuntimeMethodHandle size mismatch"),
        ) as usize
    });

    let (target_method, target_lookup) = ctx.lookup_method_by_index(method_index);
    let method_obj = ctx.get_runtime_method_obj(target_method, target_lookup);
    ctx.push_obj(method_obj);
    StepResult::Continue
}

#[dotnet_intrinsic("bool System.Delegate::Equals(object)")]
#[dotnet_intrinsic("bool DotnetRs.Delegate::Equals(object)")]
#[dotnet_intrinsic("bool System.MulticastDelegate::Equals(object)")]
#[dotnet_intrinsic("bool DotnetRs.MulticastDelegate::Equals(object)")]
pub fn delegate_equals<'gc, 'm: 'gc>(
    ctx: &mut dyn VesOps<'gc, 'm>,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let other_val = ctx.pop();
    let this_obj = ctx.pop_obj();

    if this_obj.0.is_none() {
        return ctx.throw_by_name("System.NullReferenceException");
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
    if !is_delegate_type(ctx, vm_try!(ctx.get_heap_description(other_obj.0.unwrap()))) {
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
                        ObjectRef::read_unchecked(&v.get()[i * ObjectRef::SIZE..])
                    });
                    let t2 = other_ref.as_vector(|v| unsafe {
                        ObjectRef::read_unchecked(&v.get()[i * ObjectRef::SIZE..])
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
#[dotnet_intrinsic("int DotnetRs.Delegate::GetHashCode()")]
#[dotnet_intrinsic("int System.MulticastDelegate::GetHashCode()")]
#[dotnet_intrinsic("int DotnetRs.MulticastDelegate::GetHashCode()")]
pub fn delegate_get_hash_code<'gc, 'm: 'gc>(
    ctx: &mut dyn VesOps<'gc, 'm>,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let this = ctx.pop_obj();
    if this.0.is_none() {
        return ctx.throw_by_name("System.NullReferenceException");
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

fn get_delegate_info<'gc, 'm, T: VesOps<'gc, 'm> + ?Sized>(
    ctx: &T,
    obj: ObjectRef<'gc>,
) -> (ObjectRef<'gc>, usize) {
    obj.as_object(|instance| {
        let delegate_type = ctx
            .loader()
            .corlib_type("DotnetRs.Delegate")
            .expect("DotnetRs.Delegate must exist");
        let target_bytes = instance
            .instance_storage
            .get_field_local(delegate_type, "_target");
        let target = unsafe { ObjectRef::read_unchecked(&target_bytes) };
        let method_handle_bytes = instance
            .instance_storage
            .get_field_local(delegate_type, "_method");
        let index = isize::from_ne_bytes(method_handle_bytes[..8].try_into().unwrap()) as usize;
        (target, index)
    })
}

fn get_multicast_targets_ref<'gc, 'm, T: VesOps<'gc, 'm> + ?Sized>(
    ctx: &T,
    obj: ObjectRef<'gc>,
) -> Option<ObjectRef<'gc>> {
    obj.as_object(|instance| {
        let multicast_type = ctx
            .loader()
            .corlib_type("DotnetRs.MulticastDelegate")
            .expect("DotnetRs.MulticastDelegate must exist");

        let mut curr = instance.description;
        let mut is_multicast = false;
        loop {
            if curr.type_name() == "DotnetRs.MulticastDelegate"
                || curr.type_name() == "System.MulticastDelegate"
            {
                is_multicast = true;
                break;
            }
            if let Some(parent) = curr.definition().extends.as_ref() {
                let (ut, _) = decompose_type_source::<MemberType>(parent);
                let next = ctx
                    .resolver()
                    .locate_type(curr.resolution, ut)
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

        let targets_bytes = instance
            .instance_storage
            .get_field_local(multicast_type, "targets");
        let targets_ref = unsafe { ObjectRef::read_unchecked(&targets_bytes) };
        if targets_ref.0.is_some() {
            Some(targets_ref)
        } else {
            None
        }
    })
}

#[dotnet_intrinsic("object System.Delegate::DynamicInvoke(object[])")]
#[dotnet_intrinsic("object DotnetRs.Delegate::DynamicInvoke(object[])")]
pub fn delegate_dynamic_invoke<'gc, 'm: 'gc>(
    ctx: &mut dyn VesOps<'gc, 'm>,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    // ...
    ctx.throw_by_name("System.NotSupportedException")
}

fn delegates_equal<'gc, 'm, T: VesOps<'gc, 'm> + ?Sized>(
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

fn get_invocation_list<'gc, 'm, T: VesOps<'gc, 'm> + ?Sized>(
    ctx: &T,
    obj: ObjectRef<'gc>,
) -> Vec<ObjectRef<'gc>> {
    if let Some(targets_ref) = get_multicast_targets_ref(ctx, obj) {
        targets_ref.as_vector(|v| {
            let len = v.layout.length;
            let mut result = Vec::with_capacity(len);
            for i in 0..len {
                let entry = unsafe { ObjectRef::read_unchecked(&v.get()[i * ObjectRef::SIZE..]) };
                result.push(entry);
            }
            result
        })
    } else {
        vec![obj]
    }
}

#[dotnet_intrinsic(
    "static System.Delegate System.Delegate::Combine(System.Delegate, System.Delegate)"
)]
#[dotnet_intrinsic(
    "static DotnetRs.Delegate DotnetRs.Delegate::Combine(DotnetRs.Delegate, DotnetRs.Delegate)"
)]
pub fn delegate_combine<'gc, 'm: 'gc>(
    ctx: &mut dyn VesOps<'gc, 'm>,
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
    let array_obj = ObjectRef::new(ctx.gc(), dotnet_value::object::HeapStorage::Vec(array_v));
    ctx.register_new_object(&array_obj);

    array_obj.as_vector_mut(ctx.gc(), |v| {
        for (i, &el) in combined.iter().enumerate() {
            el.write(&mut v.get_mut()[i * ObjectRef::SIZE..]);
        }
    });

    // Set 'targets' field on new_delegate
    let multicast_type = vm_try!(ctx.loader().corlib_type("DotnetRs.MulticastDelegate"));
    new_delegate.as_object_mut(ctx.gc(), |instance| {
        array_obj.write(
            &mut instance
                .instance_storage
                .get_field_mut_local(multicast_type, "targets"),
        );
    });

    ctx.push_obj(new_delegate);
    StepResult::Continue
}

#[dotnet_intrinsic(
    "static System.Delegate System.Delegate::Remove(System.Delegate, System.Delegate)"
)]
#[dotnet_intrinsic(
    "static DotnetRs.Delegate DotnetRs.Delegate::Remove(DotnetRs.Delegate, DotnetRs.Delegate)"
)]
pub fn delegate_remove<'gc, 'm: 'gc>(
    ctx: &mut dyn VesOps<'gc, 'm>,
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
            let array_obj =
                ObjectRef::new(ctx.gc(), dotnet_value::object::HeapStorage::Vec(array_v));
            ctx.register_new_object(&array_obj);

            array_obj.as_vector_mut(ctx.gc(), |v| {
                for (i, &el) in new_list.iter().enumerate() {
                    el.write(&mut v.get_mut()[i * ObjectRef::SIZE..]);
                }
            });

            let multicast_type = vm_try!(ctx.loader().corlib_type("DotnetRs.MulticastDelegate"));
            new_delegate.as_object_mut(ctx.gc(), |instance| {
                array_obj.write(
                    &mut instance
                        .instance_storage
                        .get_field_mut_local(multicast_type, "targets"),
                );
            });

            ctx.push_obj(new_delegate);
        }
    } else {
        ctx.push_obj(source);
    }

    StepResult::Continue
}

/*
fn get_runtime_return_type<'gc, 'm, T: VesOps<'gc, 'm> + ?Sized>(ctx: &T, res_ctx: &crate::ResolutionContext<'_, 'm>, method: &MethodDescription) -> RuntimeType {
    match &method.method.signature.return_type.1 {
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

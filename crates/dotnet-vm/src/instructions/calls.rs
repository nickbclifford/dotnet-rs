use crate::{
    instructions::StepResult,
    layout::type_layout,
    resolution::{TypeResolutionExt, ValueResolution},
    vm_expect_stack, vm_pop, vm_push, vm_trace, CallStack,
};
use dotnet_macros::dotnet_instruction;
use dotnet_utils::gc::GCHandle;
use dotnet_value::{
    layout::HasLayout,
    object::{HeapStorage, ObjectRef},
    StackValue,
};
use dotnetdll::prelude::*;
use std::{mem::align_of, ptr};

#[dotnet_instruction(Call)]
pub fn call<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    param0: &MethodSource,
) -> StepResult {
    // Use unified dispatch pipeline for static calls
    stack.unified_dispatch(gc, param0, None, None)
}

#[dotnet_instruction(CallVirtual)]
pub fn callvirt<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    param0: &MethodSource,
) -> StepResult {
    // Use unified dispatch pipeline for virtual calls
    // Note: We still need to pop args to extract this_type before dispatch

    // Determine number of arguments to extract this_type
    let (base_method, _) = stack.find_generic_method(param0);
    let num_args = 1 + base_method.method.signature.parameters.len();
    let mut args = Vec::new();
    for _ in 0..num_args {
        args.push(vm_pop!(stack, gc));
    }
    args.reverse();

    // Extract runtime type from this argument (value types are passed as managed pointers - I.8.9.7)
    let this_value = args[0].clone();
    let this_type = match this_value {
        StackValue::ObjectRef(ObjectRef(None)) => {
            return stack.throw_by_name(gc, "System.NullReferenceException")
        }
        StackValue::ObjectRef(ObjectRef(Some(o))) => {
            stack.current_context().get_heap_description(o)
        },
        StackValue::ManagedPtr(m) => m.inner_type,
        rest => panic!("invalid this argument for virtual call (expected ObjectRef or ManagedPtr, received {:?})", rest)
    };

    // TODO: check explicit overrides

    // Push arguments back and dispatch
    for a in args {
        vm_push!(stack, gc, a);
    }
    stack.unified_dispatch(gc, param0, Some(this_type), None)
}

#[dotnet_instruction(CallConstrained)]
pub fn call_constrained<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    constraint: &MethodType,
    source: &MethodSource,
) -> StepResult {
    // according to the standard, this doesn't really make sense
    // because the constrained prefix should only be on callvirt
    // however, this appears to be used for static interface dispatch?

    let constraint_type = stack.current_context().make_concrete(constraint);
    let (method, lookup) = stack.find_generic_method(source);

    let td = stack.loader().find_concrete_type(constraint_type.clone());

    for o in td.definition().overrides.iter() {
        let target = stack
            .current_context()
            .locate_method(o.implementation, &lookup);
        let declaration = stack
            .current_context()
            .locate_method(o.declaration, &lookup);
        if method == declaration {
            vm_trace!(stack, "-- dispatching to {:?} --", target);
            // Note: Uses dispatch_method directly since method is already resolved
            // via explicit override lookup (static interface dispatch pattern)
            return stack.dispatch_method(gc, target, lookup);
        }
    }

    panic!(
        "could not find method to dispatch to for constrained call({:?}, {:?})",
        constraint_type, method
    );
}

#[dotnet_instruction(CallVirtualConstrained)]
pub fn callvirt_constrained<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    constraint: &MethodType,
    source: &MethodSource,
) -> StepResult {
    let (base_method, lookup) = stack.find_generic_method(source);

    // Pop all arguments (this + parameters)
    let num_args = 1 + base_method.method.signature.parameters.len();
    let mut args: Vec<_> = (0..num_args).map(|_| vm_pop!(stack, gc)).collect();
    args.reverse();

    let ctx = stack.current_context();

    let constraint_type_source = ctx.make_concrete(constraint);
    let constraint_type = stack
        .loader()
        .find_concrete_type(constraint_type_source.clone());

    // Determine dispatch strategy based on constraint type
    let method = if constraint_type.is_value_type(&ctx) {
        // Value type: check for direct override first
        if let Some(overriding_method) = stack.loader().find_method_in_type_with_substitution(
            constraint_type,
            &base_method.method.name,
            &base_method.method.signature,
            base_method.resolution(),
            &lookup,
        ) {
            // Value type has its own implementation
            overriding_method
        } else {
            // No override: box the value and use base implementation
            let ptr = args[0].as_ptr();
            if ptr.is_null() {
                return stack.throw_by_name(gc, "System.NullReferenceException");
            }

            let value_size = type_layout(constraint_type_source.clone(), &ctx).size();

            let mut value_vec = vec![0u8; value_size];
            unsafe { ptr::copy_nonoverlapping(ptr, value_vec.as_mut_ptr(), value_size) };
            let value_data = &value_vec;
            let value = ctx.read_cts_value(&constraint_type_source, value_data, gc);

            let boxed = ObjectRef::new(
                gc,
                HeapStorage::Boxed(
                    ctx.new_value_type(&constraint_type_source, value.into_stack(gc)),
                ),
            );
            stack.register_new_object(&boxed);

            args[0] = StackValue::ObjectRef(boxed);
            let this_type = ctx.get_heap_description(boxed.0.unwrap());
            stack.resolve_virtual_method(base_method, this_type, Some(&ctx))
        }
    } else {
        // Reference type: dereference the managed pointer
        vm_expect_stack!(let ManagedPtr(m) = args[0].clone());
        let ptr = match m.pointer() {
            Some(p) => p.as_ptr(),
            None => return stack.throw_by_name(gc, "System.NullReferenceException"),
        };
        debug_assert!(
            (ptr as usize).is_multiple_of(align_of::<ObjectRef>()),
            "ManagedPtr value is not aligned for ObjectRef"
        );
        // Create a slice from the pointer. We know ObjectRef is pointer-sized.

        let mut value_vec = vec![0u8; ObjectRef::SIZE];
        unsafe { ptr::copy_nonoverlapping(ptr, value_vec.as_mut_ptr(), ObjectRef::SIZE) };
        let value_bytes = &value_vec;
        let obj_ref = unsafe { ObjectRef::read_branded(value_bytes, gc) };

        if obj_ref.0.is_none() {
            return stack.throw_by_name(gc, "System.NullReferenceException");
        }

        args[0] = StackValue::ObjectRef(obj_ref);

        // For reference types with constrained callvirt, try to find the method
        // implementation directly in the constraint type first
        if let Some(impl_method) = stack.loader().find_method_in_type_with_substitution(
            constraint_type,
            &base_method.method.name,
            &base_method.method.signature,
            base_method.resolution(),
            &lookup,
        ) {
            impl_method
        } else {
            // Fall back to normal virtual dispatch
            let this_type = ctx.get_heap_description(obj_ref.0.unwrap());
            stack.resolve_virtual_method(base_method, this_type, Some(&ctx))
        }
    };

    for arg in args {
        vm_push!(stack, gc, arg);
    }

    // Note: CallVirtualConstrained uses dispatch_method directly
    // instead of unified_dispatch because it performs custom method resolution
    // (boxing value types, constraint-specific lookup) that doesn't fit the
    // standard virtual dispatch pattern. The method is already fully resolved here.
    stack.dispatch_method(gc, method, lookup)
}

use crate::{StepResult, layout::type_layout, resolution::TypeResolutionExt, stack::ops::VesOps};
use dotnet_macros::dotnet_instruction;
use dotnet_value::{
    StackValue,
    layout::HasLayout,
    object::{HeapStorage, ObjectRef},
};
use dotnetdll::prelude::*;
use std::{mem::align_of, ptr};

#[dotnet_instruction(Call { param0 })]
pub fn call<'gc, 'm: 'gc, T: VesOps<'gc, 'm> + ?Sized>(
    ctx: &mut T,
    param0: &MethodSource,
) -> StepResult {
    // Use unified dispatch pipeline for static calls
    ctx.unified_dispatch(param0, None, None)
}

#[dotnet_instruction(CallVirtual { param0 })]
pub fn callvirt<'gc, 'm: 'gc, T: VesOps<'gc, 'm> + ?Sized>(
    ctx: &mut T,
    param0: &MethodSource,
) -> StepResult {
    // Use unified dispatch pipeline for virtual calls
    // Note: We still need to pop args to extract this_type before dispatch

    // Determine number of arguments to extract this_type
    let (base_method, _) = vm_try!(
        ctx.resolver()
            .find_generic_method(param0, &ctx.current_context())
    );
    let num_args = 1 + base_method.method.signature.parameters.len();
    let args = ctx.pop_multiple(num_args);

    // Extract runtime type from this argument (value types are passed as managed pointers - I.8.9.7)
    let this_value = args[0].clone();
    let this_type = match this_value {
        StackValue::ObjectRef(ObjectRef(None)) => {
            return ctx.throw_by_name("System.NullReferenceException");
        }
        StackValue::ObjectRef(ObjectRef(Some(o))) => {
            vm_try!(ctx.current_context().get_heap_description(o))
        }
        StackValue::ManagedPtr(m) => m.inner_type,
        rest => {
            return StepResult::Error(crate::error::VmError::Execution(
                crate::error::ExecutionError::TypeMismatch {
                    expected: "ObjectRef or ManagedPtr".to_string(),
                    actual: format!("{:?}", rest),
                },
            ));
        }
    };

    // TODO: check explicit overrides

    // Push arguments back and dispatch
    for a in args {
        ctx.push(a);
    }
    ctx.unified_dispatch(param0, Some(this_type), None)
}

#[dotnet_instruction(CallConstrained(constraint, source))]
pub fn call_constrained<'gc, 'm: 'gc, T: VesOps<'gc, 'm> + ?Sized>(
    ctx: &mut T,
    constraint: &MethodType,
    source: &MethodSource,
) -> StepResult {
    // according to the standard, this doesn't really make sense
    // because the constrained prefix should only be on callvirt
    // however, this appears to be used for static interface dispatch?

    let constraint_type = vm_try!(ctx.current_context().make_concrete(constraint));
    let (method, lookup) = vm_try!(
        ctx.resolver()
            .find_generic_method(source, &ctx.current_context())
    );

    let td = vm_try!(ctx.loader().find_concrete_type(constraint_type.clone()));

    for o in td.definition().overrides.iter() {
        let target = vm_try!(
            ctx.current_context()
                .locate_method(o.implementation, &lookup, None)
        );
        let declaration = vm_try!(ctx.current_context().locate_method(
            o.declaration,
            &lookup,
            None
        ));
        if method == declaration {
            vm_trace!(ctx, "-- dispatching to {:?} --", target);
            // Note: Uses dispatch_method directly since method is already resolved
            return ctx.dispatch_method(target, lookup);
        }
    }

    StepResult::Error(crate::error::VmError::Execution(
        crate::error::ExecutionError::NotImplemented(format!(
            "could not find method to dispatch to for constrained call({:?}, {:?})",
            constraint_type, method
        )),
    ))
}

#[dotnet_instruction(CallVirtualConstrained(constraint, source))]
pub fn callvirt_constrained<'gc, 'm: 'gc, T: VesOps<'gc, 'm> + ?Sized>(
    ctx: &mut T,
    constraint: &MethodType,
    source: &MethodSource,
) -> StepResult {
    let (base_method, lookup) = vm_try!(
        ctx.resolver()
            .find_generic_method(source, &ctx.current_context())
    );

    // Pop all arguments (this + parameters)
    let num_args = 1 + base_method.method.signature.parameters.len();
    let mut args = ctx.pop_multiple(num_args);

    let constraint_type_source = vm_try!(ctx.make_concrete(constraint));
    let constraint_type = vm_try!(
        ctx.loader()
            .find_concrete_type(constraint_type_source.clone())
    );

    // Determine dispatch strategy based on constraint type
    let method = if vm_try!(constraint_type.is_value_type(&ctx.current_context())) {
        // Value type: check for direct override first
        if let Some(overriding_method) = ctx.loader().find_method_in_type_with_substitution(
            constraint_type,
            &base_method.method.name,
            &base_method.method.signature,
            base_method.resolution(),
            &lookup,
        ) {
            // Value type has its own implementation
            Ok(overriding_method)
        } else {
            // No override: box the value and use base implementation
            let m = args[0].as_managed_ptr();
            let ptr = m
                .pointer()
                .map(|p: ptr::NonNull<u8>| p.as_ptr())
                .unwrap_or(ptr::null_mut());
            if ptr.is_null() {
                return ctx.throw_by_name("System.NullReferenceException");
            }

            let value_size = vm_try!(type_layout(
                vm_try!(ctx.make_concrete(constraint)),
                &ctx.current_context()
            ))
            .size();

            let mut value_vec = vec![0u8; value_size];
            // SAFETY: Memory is allocated with sufficient size (value_size) and ptr is valid
            // for the current thread's evaluation stack.
            unsafe { ptr::copy_nonoverlapping(ptr, value_vec.as_mut_ptr(), value_size) };
            let value_data = &value_vec;
            let value = vm_try!(ctx.read_cts_value(&constraint_type_source, value_data));

            let boxed = ObjectRef::new(
                ctx.gc(),
                HeapStorage::Boxed(vm_try!(
                    ctx.new_value_type(&constraint_type_source, value.into_stack(ctx.gc()))
                )),
            );
            ctx.register_new_object(&boxed);

            args[0] = StackValue::ObjectRef(boxed);
            let this_type = vm_try!(ctx.get_heap_description(boxed.0.unwrap()));
            ctx.resolver().resolve_virtual_method(
                base_method,
                this_type,
                &lookup,
                &ctx.current_context(),
            )
        }
    } else {
        // Reference type: dereference the managed pointer
        let m = args[0].as_managed_ptr();
        let Some(p) = m.pointer() else {
            return ctx.throw_by_name("System.NullReferenceException");
        };
        let ptr = p.as_ptr();
        debug_assert!(
            (ptr as usize).is_multiple_of(align_of::<ObjectRef>()),
            "ManagedPtr value is not aligned for ObjectRef"
        );
        // Create a slice from the pointer. We know ObjectRef is pointer-sized.

        let mut value_vec = vec![0u8; ObjectRef::SIZE];
        // SAFETY: ObjectRef size is fixed and ptr is valid for the current thread's evaluation stack.
        unsafe { ptr::copy_nonoverlapping(ptr, value_vec.as_mut_ptr(), ObjectRef::SIZE) };
        let value_bytes = &value_vec;

        // SAFETY: value_bytes contains a valid ObjectRef from the stack and gc is the current arena.
        let obj_ref = unsafe { ObjectRef::read_branded(value_bytes, &ctx.gc()) };

        if obj_ref.0.is_none() {
            return ctx.throw_by_name("System.NullReferenceException");
        }

        args[0] = StackValue::ObjectRef(obj_ref);

        // For reference types with constrained callvirt, try to find the method
        // implementation directly in the constraint type first
        if let Some(impl_method) = ctx.loader().find_method_in_type_with_substitution(
            constraint_type,
            &base_method.method.name,
            &base_method.method.signature,
            base_method.resolution(),
            &lookup,
        ) {
            Ok(impl_method)
        } else {
            // Fall back to normal virtual dispatch
            let this_type = vm_try!(ctx.get_heap_description(obj_ref.0.unwrap()));
            ctx.resolver().resolve_virtual_method(
                base_method,
                this_type,
                &lookup,
                &ctx.current_context(),
            )
        }
    };

    let method = vm_try!(method);

    for arg in args {
        ctx.push(arg);
    }

    // Note: CallVirtualConstrained uses dispatch_method directly
    // instead of unified_dispatch because it performs custom method resolution
    // (boxing value types, constraint-specific lookup) that doesn't fit the
    // standard virtual dispatch pattern. The method is already fully resolved here.
    ctx.dispatch_method(method, lookup)
}

use crate::{
    StepResult,
    instructions::NULL_REF_MSG,
    layout::type_layout,
    resolution::TypeResolutionExt,
    stack::ops::{
        CallOps, EvalStackOps, ExceptionOps, LoaderOps, MemoryOps, RawMemoryOps, ReflectionOps,
        ResolutionOps, StackOps,
    },
};
use dotnet_types::TypeDescription;
use dotnet_value::pointer::PointerOrigin;

const ACCESS_VIOLATION_MSG: &str = "Attempted to read or write protected memory.";
use dotnet_macros::dotnet_instruction;
use dotnet_value::{StackValue, object::ObjectRef};
use dotnetdll::prelude::*;

#[dotnet_instruction(Jump(param0))]
pub fn jmp<'gc, T: CallOps<'gc>>(ctx: &mut T, param0: &MethodSource) -> StepResult {
    ctx.unified_dispatch_jmp(param0, None)
}

#[dotnet_instruction(Call { param0, tail_call })]
pub fn call<'gc, T: CallOps<'gc>>(
    ctx: &mut T,
    param0: &MethodSource,
    tail_call: bool,
) -> StepResult {
    // Use unified dispatch pipeline for calls. If `tail.` is present, attempt to honor it.
    if tail_call {
        ctx.unified_dispatch_tail(param0, None, None)
    } else {
        ctx.unified_dispatch(param0, None, None)
    }
}

#[dotnet_instruction(CallVirtual { param0 })]
pub fn callvirt<
    'gc,
    T: CallOps<'gc>
        + ResolutionOps<'gc>
        + ExceptionOps<'gc>
        + EvalStackOps<'gc>
        + LoaderOps
        + ReflectionOps<'gc>,
>(
    ctx: &mut T,
    param0: &MethodSource,
) -> StepResult {
    // Use unified dispatch pipeline for virtual calls
    // Only inspect `this` from the current stack state; avoid pop/push argument churn.

    // Determine number of arguments to extract this_type
    let (base_method, _) = dotnet_vm_ops::vm_try!(
        ctx.resolver()
            .find_generic_method(param0, &ctx.current_context())
    );
    let num_args = 1 + base_method.method().signature.parameters.len();
    let this_value = ctx.peek_stack_at(num_args - 1);

    // Extract runtime type from this argument (value types are passed as managed pointers - I.8.9.7)
    let this_type = match this_value {
        StackValue::ObjectRef(ObjectRef(None)) => {
            // Preserve previous callvirt semantics: arguments are consumed before an early throw.
            let _ = ctx.pop_multiple(num_args);
            return ctx.throw_by_name_with_message("System.NullReferenceException", NULL_REF_MSG);
        }
        StackValue::ObjectRef(ObjectRef(Some(o))) => {
            dotnet_vm_ops::vm_try!(ctx.current_context().get_heap_description(o))
        }
        StackValue::ManagedPtr(m) => m.inner_type(),
        rest => {
            let _ = ctx.pop_multiple(num_args);
            return StepResult::type_error("ObjectRef or ManagedPtr", format!("{:?}", rest));
        }
    };

    ctx.unified_dispatch(param0, Some(this_type), None)
}

#[dotnet_instruction(CallVirtualTail(param0))]
pub fn callvirt_tail<
    'gc,
    T: CallOps<'gc>
        + ResolutionOps<'gc>
        + ExceptionOps<'gc>
        + EvalStackOps<'gc>
        + LoaderOps
        + ReflectionOps<'gc>,
>(
    ctx: &mut T,
    param0: &MethodSource,
) -> StepResult {
    // Tail-prefixed callvirt: extract runtime this type (and perform null check), then request
    // tail dispatch.
    let (base_method, _) = dotnet_vm_ops::vm_try!(
        ctx.resolver()
            .find_generic_method(param0, &ctx.current_context())
    );
    let num_args = 1 + base_method.method().signature.parameters.len();
    let this_value = ctx.peek_stack_at(num_args - 1);

    let this_type = match this_value {
        StackValue::ObjectRef(ObjectRef(None)) => {
            // Preserve previous callvirt semantics: arguments are consumed before an early throw.
            let _ = ctx.pop_multiple(num_args);
            return ctx.throw_by_name_with_message("System.NullReferenceException", NULL_REF_MSG);
        }
        StackValue::ObjectRef(ObjectRef(Some(o))) => {
            dotnet_vm_ops::vm_try!(ctx.current_context().get_heap_description(o))
        }
        StackValue::ManagedPtr(m) => m.inner_type(),
        rest => {
            let _ = ctx.pop_multiple(num_args);
            return StepResult::type_error("ObjectRef or ManagedPtr", format!("{:?}", rest));
        }
    };

    ctx.unified_dispatch_tail(param0, Some(this_type), None)
}

#[dotnet_instruction(CallConstrained(constraint, source))]
pub fn call_constrained<'gc, T: CallOps<'gc> + ResolutionOps<'gc> + LoaderOps>(
    ctx: &mut T,
    constraint: &MethodType,
    source: &MethodSource,
) -> StepResult {
    // according to the standard, this doesn't really make sense
    // because the constrained prefix should only be on callvirt
    // however, this appears to be used for static interface dispatch?

    let constraint_type = dotnet_vm_ops::vm_try!(ctx.current_context().make_concrete(constraint));
    let (method, lookup) = dotnet_vm_ops::vm_try!(
        ctx.resolver()
            .find_generic_method(source, &ctx.current_context())
    );

    let td: TypeDescription =
        dotnet_vm_ops::vm_try!(ctx.loader().find_concrete_type(constraint_type.clone()));

    for o in td.definition().overrides.iter() {
        let target = dotnet_vm_ops::vm_try!(ctx.current_context().locate_method(
            o.implementation,
            &lookup,
            None
        ));
        let declaration = dotnet_vm_ops::vm_try!(ctx.current_context().locate_method(
            o.declaration,
            &lookup,
            None
        ));
        if method == declaration {
            // vm_trace!(ctx, "-- dispatching to {:?} --", target);
            // Note: Uses dispatch_method directly since method is already resolved
            return ctx.dispatch_method(target, lookup);
        }
    }

    // Static abstract interface members (e.g., INumber<T>.Min) may be implemented
    // directly on the constrained concrete type without an overrides entry.
    if let Some(impl_method) = ctx.loader().find_method_in_type_with_substitution(
        td,
        &method.method().name,
        &method.method().signature,
        method.resolution(),
        &lookup,
        false,
    ) {
        return ctx.dispatch_method(impl_method, lookup);
    }

    // Generic-math style static interface members can have default bodies on the
    // interface itself (no concrete override entry on the constrained type).
    // In that case dispatch to the resolved interface method directly.
    ctx.dispatch_method(method, lookup)
}

#[dotnet_instruction(CallVirtualConstrained(constraint, source))]
pub fn callvirt_constrained<
    'gc,
    T: CallOps<'gc>
        + ResolutionOps<'gc>
        + ExceptionOps<'gc>
        + StackOps<'gc>
        + MemoryOps<'gc>
        + RawMemoryOps<'gc>
        + LoaderOps
        + ReflectionOps<'gc>,
>(
    ctx: &mut T,
    constraint: &MethodType,
    source: &MethodSource,
) -> StepResult {
    let (base_method, lookup) = dotnet_vm_ops::vm_try!(
        ctx.resolver()
            .find_generic_method(source, &ctx.current_context())
    );

    // Pop all arguments (this + parameters)
    let num_args = 1 + base_method.method().signature.parameters.len();
    let mut args = ctx.pop_multiple(num_args);

    let constraint_type_source = dotnet_vm_ops::vm_try!(ctx.make_concrete(constraint));
    let constraint_type: TypeDescription = dotnet_vm_ops::vm_try!(
        ctx.loader()
            .find_concrete_type(constraint_type_source.clone())
    );

    // Determine dispatch strategy based on constraint type
    let method = if dotnet_vm_ops::vm_try!(
        constraint_type
            .clone()
            .is_value_type(&ctx.current_context())
    ) {
        // Value type: check for direct override first
        if let Some(overriding_method) = ctx.loader().find_method_in_type_with_substitution(
            constraint_type.clone(),
            &base_method.method().name,
            &base_method.method().signature,
            base_method.resolution(),
            &lookup,
            false,
        ) {
            // Value type has its own implementation
            Ok(overriding_method)
        } else {
            // No override: box the value and use base implementation
            let m = args[0].as_managed_ptr();
            if m.is_null() {
                return ctx
                    .throw_by_name_with_message("System.NullReferenceException", NULL_REF_MSG);
            }

            let constraint_concrete = dotnet_vm_ops::vm_try!(ctx.make_concrete(constraint));
            let layout = dotnet_vm_ops::vm_try!(type_layout(
                constraint_concrete.clone(),
                &ctx.current_context()
            ));

            let value = match unsafe {
                ctx.read_unaligned(
                    m.origin().clone(),
                    m.byte_offset(),
                    &layout,
                    Some(constraint_type.clone()),
                )
            } {
                Ok(v) => v,
                Err(_) => {
                    return ctx.throw_by_name_with_message(
                        "System.AccessViolationException",
                        ACCESS_VIOLATION_MSG,
                    );
                }
            };

            let boxed = dotnet_vm_ops::vm_try!(ctx.box_value(&constraint_type_source, value));
            args[0] = StackValue::ObjectRef(boxed);
            let this_type = dotnet_vm_ops::vm_try!(ctx.get_heap_description(boxed));
            ctx.resolver().resolve_virtual_method(
                base_method,
                this_type,
                &lookup,
                &ctx.current_context(),
            )
        }
    } else {
        // Reference type: the 'this' may already be an ObjectRef (common case),
        // or a ManagedPtr to an ObjectRef in some generic/constrained contexts.
        let obj_ref = match &args[0] {
            StackValue::ObjectRef(o) => *o,
            StackValue::ManagedPtr(m) => {
                if m.is_null() {
                    return ctx
                        .throw_by_name_with_message("System.NullReferenceException", NULL_REF_MSG);
                }
                // If the managed pointer originates from the evaluation stack (ldarga/ldloca),
                // and targets a reference type with zero offset, the pointed memory holds a
                // StackValue::ObjectRef, not a serialized ObjectRef. Read it via the slot.
                match (m.origin(), m.byte_offset()) {
                    (PointerOrigin::Stack(idx), off) if off.as_usize() == 0 => {
                        match ctx.get_slot_ref(*idx).clone() {
                            StackValue::ObjectRef(o) => o,
                            rest => {
                                return StepResult::type_error(
                                    "ObjectRef at argument/local slot",
                                    format!("{:?}", rest),
                                );
                            }
                        }
                    }
                    _ => unsafe {
                        // Heap/static/transient: the target stores a serialized ObjectRef
                        m.with_data(ObjectRef::SIZE, |data| {
                            ObjectRef::read_branded(
                                data,
                                &ctx.gc_with_token(&ctx.no_active_borrows_token()),
                            )
                        })
                    },
                }
            }
            rest => {
                return StepResult::type_error("ObjectRef or ManagedPtr", format!("{:?}", rest));
            }
        };

        if obj_ref.0.is_none() {
            return ctx.throw_by_name_with_message("System.NullReferenceException", NULL_REF_MSG);
        }

        args[0] = StackValue::ObjectRef(obj_ref);

        // For reference types with constrained callvirt, try to find the method
        // implementation directly in the constraint type first
        if let Some(impl_method) = ctx.loader().find_method_in_type_with_substitution(
            constraint_type,
            &base_method.method().name,
            &base_method.method().signature,
            base_method.resolution(),
            &lookup,
            false,
        ) {
            Ok(impl_method)
        } else {
            // Fall back to normal virtual dispatch
            let this_type = dotnet_vm_ops::vm_try!(ctx.get_heap_description(obj_ref));
            ctx.resolver().resolve_virtual_method(
                base_method,
                this_type,
                &lookup,
                &ctx.current_context(),
            )
        }
    };

    let method = dotnet_vm_ops::vm_try!(method);

    for arg in args {
        ctx.push(arg);
    }

    // Note: CallVirtualConstrained uses dispatch_method directly
    // instead of unified_dispatch because it performs custom method resolution
    // (boxing value types, constraint-specific lookup) that doesn't fit the
    // standard virtual dispatch pattern. The method is already fully resolved here.
    ctx.dispatch_method(method, lookup)
}

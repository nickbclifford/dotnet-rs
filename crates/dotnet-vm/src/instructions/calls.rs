use crate::{
    StepResult,
    instructions::NULL_REF_MSG,
    layout::type_layout,
    resolution::TypeResolutionExt,
    stack::{
        ops::{UnifiedDispatchTarget, VesOps, VmCallOps, VmLoaderOps, VmResolutionOps},
        receiver_object_generics,
    },
};
use dotnet_types::{
    TypeDescription,
    generics::{ConcreteType, GenericLookup},
    members::MethodDescription,
};
use dotnet_value::{StackValue, object::ObjectRef, pointer::PointerOrigin};

const ACCESS_VIOLATION_MSG: &str = "Attempted to read or write protected memory.";
use dotnet_macros::dotnet_instruction;
use dotnetdll::prelude::*;

fn merge_receiver_type_generics_into_lookup(
    lookup: &GenericLookup,
    receiver: ObjectRef<'_>,
) -> GenericLookup {
    let mut merged = lookup.clone();

    if receiver.0.is_some() {
        let receiver_lookup = receiver_object_generics(receiver);

        if !receiver_lookup.type_generics.is_empty() {
            merged.set_type_generics(receiver_lookup.type_generics);
        }
    }

    merged
}

#[dotnet_instruction(Jump(param0))]
pub fn jmp<'gc, T: VmCallOps<'gc>>(ctx: &mut T, param0: &MethodSource) -> StepResult {
    ctx.unified_dispatch_jmp(param0, None)
}

#[dotnet_instruction(ArgumentList)]
pub fn arglist<'gc, T: dotnet_vm_ops::ops::ExceptionOps<'gc>>(ctx: &mut T) -> StepResult {
    ctx.throw_by_name_with_message(
        "System.InvalidProgramException",
        "Vararg calling convention not supported.",
    )
}

#[dotnet_instruction(Call { param0, tail_call })]
pub fn call<'gc, T: VmCallOps<'gc>>(
    ctx: &mut T,
    param0: &MethodSource,
    tail_call: bool,
) -> StepResult {
    // Use unified dispatch pipeline for calls. If `tail.` is present, attempt to honor it.
    if tail_call {
        ctx.unified_dispatch_tail(UnifiedDispatchTarget::Source(param0), None, None)
    } else {
        ctx.unified_dispatch(UnifiedDispatchTarget::Source(param0), None, None)
    }
}

#[dotnet_instruction(CallIndirect { param0, tail_call })]
pub fn calli<'gc, T: VesOps<'gc>>(
    ctx: &mut T,
    param0: &dotnetdll::resolved::signature::MaybeUnmanagedMethod<MethodType>,
    tail_call: bool,
) -> StepResult {
    use dotnetdll::resolved::signature::StandAloneCallingConvention;
    let unsupported = || {
        StepResult::Error(
            crate::error::ExecutionError::NotImplemented(
                "unsupported indirect call target or signature".into(),
            )
            .into(),
        )
    };
    if !matches!(
        param0.calling_convention,
        StandAloneCallingConvention::DefaultManaged | StandAloneCallingConvention::Vararg
    ) {
        let target = match ctx.peek() {
            Some(StackValue::UnmanagedPtr(p)) => Some(p.0),
            Some(StackValue::NativeInt(p)) => {
                // SAFETY: F11.PInvokeAbiAgreement — `calli`'s unmanaged pointer is supplied by
                // the IL caller, which must ensure it denotes a live function with this ABI.
                std::ptr::NonNull::new(unsafe {
                    dotnet_value::pointer::unmanaged_ptr_from_addr(p as usize)
                })
            }
            _ => return unsupported(),
        };
        let Some(target) = target else {
            return ctx.throw_by_name_with_message(
                "System.AccessViolationException",
                ACCESS_VIOLATION_MSG,
            );
        };
        let shared = ctx.shared().clone();
        if !shared.pinvoke.allows_indirect_calls() {
            return unsupported();
        }
        ctx.pop();
        // SAFETY: F11.PInvokeAbiAgreement — unmanaged `calli` requires the IL caller to supply
        // a live native function pointer with the ABI and signature in `param0`. The P/Invoke
        // sandbox explicitly permits indirect calls before the pointer can be invoked.
        return unsafe { dotnet_pinvoke::indirect_call(ctx, param0, target, &shared.pinvoke) };
    }
    if param0.calling_convention == StandAloneCallingConvention::Vararg {
        return ctx.throw_by_name_with_message(
            "System.InvalidProgramException",
            "Vararg calling convention not supported.",
        );
    }
    if param0.varargs.is_some() {
        return unsupported();
    }
    if param0.explicit_this && (!param0.instance || param0.parameters.is_empty()) {
        return ctx.throw_by_name_with_message(
            "System.InvalidProgramException",
            "An explicit receiver is required for this call signature.",
        );
    }
    let Some(StackValue::NativeInt(index)) = ctx.peek() else {
        return unsupported();
    };
    let Ok(index) = usize::try_from(index) else {
        return unsupported();
    };
    #[cfg(feature = "multithreading")]
    let target = ctx
        .shared()
        .reflection_registry
        .runtime_methods_rev
        .get(&index)
        .map(|entry| entry.value().clone());
    #[cfg(not(feature = "multithreading"))]
    let target = ctx.reflection().methods_read().get(index).cloned();
    let Some((method, lookup)) = target else {
        return unsupported();
    };
    if method.signature().calling_convention != CallingConvention::Default
        || method.signature().explicit_this
        || method.signature().varargs.is_some()
    {
        return unsupported();
    }
    let signature = MethodSignature {
        instance: param0.instance,
        explicit_this: false,
        calling_convention: CallingConvention::Default,
        // An explicit-this call site encodes its receiver as its first parameter, while an
        // instance method's definition encodes the receiver separately from its parameters.
        parameters: if param0.explicit_this {
            param0.parameters[1..].to_vec()
        } else {
            param0.parameters.clone()
        },
        return_type: param0.return_type.clone(),
        varargs: None,
    };
    let current = ctx.current_context();
    if !ctx.loader().comparer().signatures_equal(
        &current.resolution,
        &signature,
        Some(current.generics),
        &method.resolution(),
        method.signature(),
        Some(&lookup),
    ) {
        return unsupported();
    }
    ctx.pop();
    let target = UnifiedDispatchTarget::Resolved(method, lookup);
    if tail_call {
        ctx.unified_dispatch_tail(target, None, None)
    } else {
        ctx.unified_dispatch(target, None, None)
    }
}

fn dispatch_callvirt<'gc, T: VesOps<'gc>>(
    ctx: &mut T,
    param0: &MethodSource,
    tail_call: bool,
) -> StepResult {
    // Determine number of arguments to extract this_type
    let (base_method, lookup) = dotnet_vm_ops::vm_try!(
        ctx.resolver()
            .find_generic_method(param0, &ctx.current_context())
    );
    if base_method.signature().calling_convention == CallingConvention::Vararg {
        return ctx.throw_by_name_with_message(
            "System.InvalidProgramException",
            "Vararg calling convention not supported.",
        );
    }
    let num_args = 1 + base_method.signature().parameters.len();
    let this_value = ctx.peek_stack_at(num_args - 1);

    // Extract runtime type from this argument (value types are passed as managed pointers - I.8.9.7)
    let this_type = match this_value {
        StackValue::ObjectRef(ObjectRef(None)) => {
            // Preserve previous callvirt semantics: arguments are consumed before an early throw.
            ctx.drop_top(num_args);
            return ctx.throw_by_name_with_message("System.NullReferenceException", NULL_REF_MSG);
        }
        StackValue::ObjectRef(ObjectRef(Some(o))) => {
            dotnet_vm_ops::vm_try!(ctx.current_context().get_heap_description(o))
        }
        StackValue::ManagedPtr(m) => m.inner_type(),
        rest => {
            ctx.drop_top(num_args);
            return StepResult::type_error("ObjectRef or ManagedPtr", format!("{:?}", rest));
        }
    };

    if tail_call {
        ctx.unified_dispatch_tail(
            UnifiedDispatchTarget::Resolved(base_method, lookup),
            Some(this_type),
            None,
        )
    } else {
        ctx.unified_dispatch(
            UnifiedDispatchTarget::Resolved(base_method, lookup),
            Some(this_type),
            None,
        )
    }
}

#[dotnet_instruction(CallVirtual { param0 })]
pub fn callvirt<'gc, T: VesOps<'gc>>(ctx: &mut T, param0: &MethodSource) -> StepResult {
    // Use unified dispatch pipeline for virtual calls
    // Only inspect `this` from the current stack state; avoid pop/push argument churn.
    dispatch_callvirt(ctx, param0, false)
}

#[dotnet_instruction(CallVirtualTail(param0))]
pub fn callvirt_tail<'gc, T: VesOps<'gc>>(ctx: &mut T, param0: &MethodSource) -> StepResult {
    // Tail-prefixed callvirt: extract runtime this type (and perform null check), then request
    // tail dispatch.
    dispatch_callvirt(ctx, param0, true)
}

#[dotnet_instruction(CallConstrained(constraint, source))]
pub fn call_constrained<'gc, T: VmCallOps<'gc> + VmResolutionOps<'gc> + VmLoaderOps>(
    ctx: &mut T,
    constraint: &MethodType,
    source: &MethodSource,
) -> StepResult {
    dispatch_constrained_static(ctx, constraint, source, false)
}

#[dotnet_instruction(CallConstrainedTail(constraint, source))]
pub fn call_constrained_tail<'gc, T: VmCallOps<'gc> + VmResolutionOps<'gc> + VmLoaderOps>(
    ctx: &mut T,
    constraint: &MethodType,
    source: &MethodSource,
) -> StepResult {
    dispatch_constrained_static(ctx, constraint, source, true)
}

fn dispatch_constrained_static<'gc, T: VmCallOps<'gc> + VmResolutionOps<'gc> + VmLoaderOps>(
    ctx: &mut T,
    constraint: &MethodType,
    source: &MethodSource,
    tail_call: bool,
) -> StepResult {
    let constraint_type = dotnet_vm_ops::vm_try!(ctx.current_context().make_concrete(constraint));
    let (method, lookup) = dotnet_vm_ops::vm_try!(
        ctx.resolver()
            .find_generic_method(source, &ctx.current_context())
    );
    let target = dotnet_vm_ops::vm_try!(ctx.resolver().resolve_static_constrained_method(
        constraint_type.clone(),
        method,
        &lookup,
    ));

    // The implementation's declaring type, not the interface declaration, owns its type
    // slots. A default interface body instead retains the declaration's instantiation.
    // Construct this on every dispatch; the resolver cache stores metadata only.
    let constraint_description =
        dotnet_vm_ops::vm_try!(ctx.loader().find_concrete_type(constraint_type.clone()));
    let dispatch_lookup = static_constraint_dispatch_lookup(
        &target,
        &constraint_description,
        &constraint_type,
        &lookup,
    );
    if tail_call {
        ctx.unified_dispatch_tail(
            UnifiedDispatchTarget::Resolved(target, dispatch_lookup),
            None,
            None,
        )
    } else {
        ctx.dispatch_method_inline(target, dispatch_lookup)
    }
}

fn static_constraint_dispatch_lookup(
    target: &MethodDescription,
    constraint_description: &TypeDescription,
    constraint_type: &ConcreteType,
    source: &GenericLookup,
) -> GenericLookup {
    let mut result = if &target.parent == constraint_description {
        constraint_type.make_lookup()
    } else {
        target.parent_generics.clone()
    };
    result.set_method_generics(source.method_generics.clone());
    result
}

#[dotnet_instruction(CallVirtualConstrained(constraint, source))]
pub fn callvirt_constrained<'gc, T: VesOps<'gc>>(
    ctx: &mut T,
    constraint: &MethodType,
    source: &MethodSource,
) -> StepResult {
    dispatch_constrained_virtual(ctx, constraint, source, false)
}

#[dotnet_instruction(CallVirtualConstrainedTail(constraint, source))]
pub fn callvirt_constrained_tail<'gc, T: VesOps<'gc>>(
    ctx: &mut T,
    constraint: &MethodType,
    source: &MethodSource,
) -> StepResult {
    dispatch_constrained_virtual(ctx, constraint, source, true)
}

fn dispatch_constrained_virtual<'gc, T: VesOps<'gc>>(
    ctx: &mut T,
    constraint: &MethodType,
    source: &MethodSource,
    tail_call: bool,
) -> StepResult {
    let (base_method, lookup) = dotnet_vm_ops::vm_try!(
        ctx.resolver()
            .find_generic_method(source, &ctx.current_context())
    );

    // `Object.GetType` is not virtual, so a value-type constraint always boxes the receiver and
    // the callee reads the constraint's Type object. Select a missing shared Type object here,
    // before the receiver is boxed, so a retry re-executes this instruction on its original stack.
    #[cfg(feature = "multithreading")]
    if base_method.method().name == "GetType"
        && base_method.signature().parameters.is_empty()
        && ctx
            .loader()
            .canonical_type_name(&base_method.parent.type_name())
            == "System.Object"
    {
        let concrete = dotnet_vm_ops::vm_try!(ctx.make_concrete(constraint));
        let constraint_type: TypeDescription =
            dotnet_vm_ops::vm_try!(ctx.loader().find_concrete_type(concrete.clone()));
        if dotnet_vm_ops::vm_try!(constraint_type.is_value_type(&ctx.current_context()))
            && let Some(target) =
                dotnet_types::runtime::runtime_type_from_concrete(ctx.loader().as_ref(), &concrete)
            && let Some(step) =
                ctx.await_reflection_object(crate::stack::ops::ReflectionObjectKey::Type(&target))
        {
            return step;
        }
    }

    // Pop all arguments (this + parameters)
    let num_args = 1 + base_method.signature().parameters.len();
    ctx.pop_call_args_into_buffer(num_args);
    let mut args = std::mem::take(ctx.call_args_buffer_mut());

    let constraint_type_source = dotnet_vm_ops::vm_try!(ctx.make_concrete(constraint));
    let constraint_type: TypeDescription = dotnet_vm_ops::vm_try!(
        ctx.loader()
            .find_concrete_type(constraint_type_source.clone())
    );

    // When the constrained method is resolved against the constraint type itself
    // (a direct override on a value type, or an implementation found on a reference
    // type), the callee must execute under the constraint type's own generic
    // instantiation — NOT the caller frame's `lookup`. Reusing the caller's lookup
    // leaks the caller's type arguments into the callee; for a self-referential
    // instantiation such as `StructMultiKey<StructMultiKey<..>, ..>` this makes the
    // inner `GetHashCode` re-resolve `!T1` to the outer type and recurse forever.
    // `make_lookup()` derives the type arguments from the concrete constraint type;
    // method generics (e.g. a generic interface method) are preserved from the call.
    let constraint_lookup = {
        let mut l = constraint_type_source.make_lookup();
        l.set_method_generics(lookup.method_generics.clone());
        l
    };

    // Determine dispatch strategy based on constraint type. Each branch yields the
    // resolved method plus the generic lookup the callee frame must run under.
    let (method, dispatch_lookup) = if dotnet_vm_ops::vm_try!(
        constraint_type
            .clone()
            .is_value_type(&ctx.current_context())
    ) {
        // Value type: check for direct override first
        if let Some(overriding_method) =
            dotnet_vm_ops::vm_try!(ctx.loader().find_method_in_type_internal(
                constraint_type.clone(),
                &base_method.method().name,
                base_method.signature(),
                base_method.resolution(),
                Some(&lookup),
                Some(&constraint_lookup),
                false,
            ))
        {
            // Value type has its own implementation: run it under the constraint
            // type's instantiation.
            (Ok(overriding_method), constraint_lookup.clone())
        } else {
            // No override: box the value and use base implementation
            let m = dotnet_vm_ops::vm_try!(args[0].try_as_managed_ptr());
            if m.is_null() {
                return ctx
                    .throw_by_name_with_message("System.NullReferenceException", NULL_REF_MSG);
            }

            let constraint_concrete = dotnet_vm_ops::vm_try!(ctx.make_concrete(constraint));
            let layout = dotnet_vm_ops::vm_try!(type_layout(
                constraint_concrete.clone(),
                &ctx.current_context()
            ));

            // SAFETY: F10.RawMemoryAccessValid — `m` is a non-null managed pointer, and its origin/offset identify storage
            // for the resolved constrained type layout; `read_unaligned` verifies that range.
            let value = match unsafe {
                ctx.read_unaligned(
                    m.origin().clone(),
                    dotnet_vm_ops::vm_try!(m.byte_offset()),
                    &layout,
                    Some(constraint_type.clone()),
                )
            } {
                Ok(v) => v,
                Err(error) => {
                    if let Some(step) = super::stale_frame_home(m.origin(), &error) {
                        return step;
                    }
                    return ctx.throw_by_name_with_message(
                        "System.AccessViolationException",
                        ACCESS_VIOLATION_MSG,
                    );
                }
            };

            let boxed = dotnet_vm_ops::vm_try!(ctx.box_value(&constraint_type_source, value));
            args[0] = StackValue::ObjectRef(boxed);
            let this_type = dotnet_vm_ops::vm_try!(ctx.get_heap_description(boxed));
            let dispatch_lookup = merge_receiver_type_generics_into_lookup(&lookup, boxed);
            let resolved = ctx.resolver().resolve_virtual_method(
                base_method,
                this_type,
                &dispatch_lookup,
                &ctx.current_context(),
            );
            (resolved, dispatch_lookup)
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
                match (m.origin(), dotnet_vm_ops::vm_try!(m.byte_offset())) {
                    (PointerOrigin::FrameHome(_), offset) => {
                        let layout = dotnet_value::layout::LayoutManager::Scalar(
                            dotnet_value::layout::Scalar::ObjectRef,
                        );
                        // SAFETY: F10.FrameHomeChecked — `read_unaligned` resolves the frame-home
                        // origin through its live frame and checks the object-reference range.
                        match unsafe {
                            ctx.read_unaligned(m.origin().clone(), offset, &layout, None)
                        } {
                            Ok(StackValue::ObjectRef(o)) => o,
                            Ok(rest) => {
                                return StepResult::type_error("ObjectRef", format!("{:?}", rest));
                            }
                            Err(error) => {
                                if let Some(step) = super::stale_frame_home(m.origin(), &error) {
                                    return step;
                                }
                                return ctx.throw_by_name_with_message(
                                    "System.AccessViolationException",
                                    ACCESS_VIOLATION_MSG,
                                );
                            }
                        }
                    }

                    _ => {
                        #[expect(
                            clippy::multiple_unsafe_ops_per_block,
                            reason = "reading a serialized ObjectRef and branding it share one managed-pointer validity proof"
                        )]
                        // SAFETY: F1.GcHandleRooted — Heap/static/transient targets store a serialized ObjectRef, and
                        // `m` supplies the validated managed range used to brand that reference.
                        unsafe {
                            // Heap/static/transient: the target stores a serialized ObjectRef
                            m.with_data(ObjectRef::SIZE, |data| {
                                ObjectRef::read_branded(
                                    data,
                                    &ctx.gc_with_token(&ctx.no_active_borrows_token()),
                                )
                            })
                        }
                    }
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

        // ECMA-335 III.2.1: for a reference-type constraint the pointer is dereferenced and the
        // call is an ordinary `callvirt` on the object, so the target is chosen by the object's
        // runtime type. The constraint type only names the static type: `T = object` still
        // dispatches `ToString` to the receiver's override.
        let this_type = dotnet_vm_ops::vm_try!(ctx.get_heap_description(obj_ref));
        let dispatch_lookup = merge_receiver_type_generics_into_lookup(&lookup, obj_ref);
        let resolved = ctx.resolver().resolve_virtual_method(
            base_method,
            this_type,
            &dispatch_lookup,
            &ctx.current_context(),
        );
        (resolved, dispatch_lookup)
    };

    let method = dotnet_vm_ops::vm_try!(method);
    // Virtual resolution may select an inherited implementation whose declaring type has a
    // different instantiation from the receiver. Use the resolver's ancestor lookup, as the
    // ordinary virtual dispatch pipeline does, while retaining the call's method arguments.
    let mut dispatch_lookup = dispatch_lookup;
    let arity = method.parent.definition().generic_parameters.len();
    if arity == 0 {
        dispatch_lookup.set_type_generics(Vec::new().into());
    } else if method.parent_generics.type_generics.len() == arity {
        dispatch_lookup.set_type_generics(method.parent_generics.type_generics.clone());
    } else {
        dispatch_lookup.set_type_generics(
            dispatch_lookup
                .type_generics
                .iter()
                .take(arity)
                .cloned()
                .collect::<Vec<_>>()
                .into(),
        );
    }

    for arg in args.drain(..) {
        ctx.push(arg);
    }
    *ctx.call_args_buffer_mut() = args;

    if tail_call {
        ctx.unified_dispatch_tail(
            UnifiedDispatchTarget::Resolved(method, dispatch_lookup),
            None,
            None,
        )
    } else {
        ctx.dispatch_method_inline(method, dispatch_lookup)
    }
}

#[cfg(test)]
mod constrained_lookup_tests {
    use super::*;
    use crate::test_utils::parameterless_i32_signature;
    use dotnet_assemblies::AssemblyLoader;
    use dotnet_types::resolution::ResolutionS;
    use dotnetdll::resolved::{generic::Generic, members::Method};

    #[test]
    fn static_dispatch_uses_target_type_and_preserves_method_arguments() {
        let loader = AssemblyLoader::new_bare("constrained-lookup".into()).unwrap();
        let mut res = Resolution::new(Module::new("constrained.dll"));
        let mut implementation = TypeDefinition::new(None, "Implementation");
        implementation.generic_parameters.push(Generic::new("T"));
        let implementation = res.push_type_definition(implementation);
        let mut interface = TypeDefinition::new(None, "Interface");
        interface.generic_parameters.push(Generic::new("T"));
        let interface = res.push_type_definition(interface);
        for ty in [implementation, interface] {
            res.push_method(
                ty,
                Method::new(
                    Accessibility::Public,
                    parameterless_i32_signature(),
                    "Run",
                    None,
                ),
            );
        }
        let res = loader.register_owned_assembly(res);
        let implementation = TypeDescription::new(res.clone(), implementation);
        let interface = TypeDescription::new(res.clone(), interface);
        let int = ConcreteType::new(ResolutionS::NULL, BaseType::Int32);
        let long = ConcreteType::new(ResolutionS::NULL, BaseType::Int64);
        let string = ConcreteType::new(ResolutionS::NULL, BaseType::String);
        let concrete = ConcreteType::new(
            res.clone(),
            BaseType::Type {
                source: TypeSource::Generic {
                    base: UserType::Definition(implementation.index),
                    parameters: vec![int.clone()],
                },
                value_kind: None,
            },
        );
        let leaked = GenericLookup::from_arcs(
            vec![long.clone(), string.clone()].into(),
            vec![string.clone(), long.clone()].into(),
        );
        let target = MethodDescription::new(
            implementation.clone(),
            leaked.clone(),
            res.clone(),
            MethodMemberIndex::Method(0),
        );
        let actual =
            static_constraint_dispatch_lookup(&target, &implementation, &concrete, &leaked);
        assert_eq!(&*actual.type_generics, std::slice::from_ref(&int));
        assert_eq!(actual.method_generics, leaked.method_generics);
        let mut default_target = MethodDescription::new(
            interface,
            GenericLookup::default(),
            res,
            MethodMemberIndex::Method(0),
        );
        default_target
            .parent_generics
            .set_type_generics(vec![long.clone()].into());
        let actual =
            static_constraint_dispatch_lookup(&default_target, &implementation, &concrete, &leaked);
        assert_eq!(&*actual.type_generics, &[long]);
        assert_eq!(actual.method_generics, leaked.method_generics);
    }
}

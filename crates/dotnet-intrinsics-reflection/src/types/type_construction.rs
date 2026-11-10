use crate::ReflectionIntrinsicHost;
use dotnet_macros::dotnet_intrinsic;
use dotnet_types::{
    error::{ExecutionError, VmError},
    generics::{ConcreteType, GenericLookup},
    members::MethodDescription,
    runtime::RuntimeType,
};
use dotnet_value::{
    StackValue,
    object::{HeapStorage, ObjectRef},
};
use dotnet_vm_data::{FrameReturnAction, StepResult};
use dotnet_vm_ops::SupportSlotOps;
use dotnet_vm_ops::effects::{
    NativeInvocation, NativeOperandView, NativeResult, NativeSelection, VmEffects,
};
use dotnet_vm_ops::ops::TypedStackOps;
use dotnetdll::prelude::{MethodMemberIndex, TypeSource};

#[dotnet_intrinsic(
    "static void System.Runtime.CompilerServices.RuntimeHelpers::RunClassConstructor(System.RuntimeTypeHandle)"
)]
pub fn intrinsic_runtime_helpers_run_class_constructor<'gc, T: ReflectionIntrinsicHost<'gc>>(
    ctx: &mut T,
    _method: MethodDescription,
    generics: &GenericLookup,
) -> StepResult {
    let _gc = ctx.gc_with_token(&ctx.no_active_borrows_token());
    let arg = ctx.peek_stack();
    let StackValue::ValueType(handle) = arg else {
        return StepResult::Error(VmError::Execution(ExecutionError::TypeMismatch {
            expected: "RuntimeTypeHandle",
            actual: format!("{arg:?}").into(),
        }));
    };

    let target_obj = ctx
        .loader()
        .rth_value_field(&handle.instance_storage, handle.description)
        .expect("validated RuntimeTypeHandle support slot")
        .read();
    let target_type = dotnet_vm_ops::vm_try!(crate::common::resolve_runtime_type(ctx, target_obj));
    let target_ct = target_type.to_concrete(ctx.loader().as_ref());
    let target_desc = ctx
        .loader()
        .find_concrete_type(target_ct)
        .expect("Type must exist for RunClassConstructor");

    let res = ctx.initialize_static_storage(target_desc, generics.clone());
    if res != StepResult::Continue {
        return res;
    }

    let _ = ctx.pop();
    StepResult::Continue
}

#[dotnet_intrinsic("static System.Array System.Array::CreateInstance(System.Type, int)")]
pub fn intrinsic_array_create_instance<'gc, T: ReflectionIntrinsicHost<'gc>>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let length = ctx.pop_i32();
    let element_type = ctx.pop_obj();

    if element_type.0.is_none() {
        return ctx.throw_by_name_with_message(
            "System.ArgumentNullException",
            "Value cannot be null. (Parameter 'elementType')",
        );
    }

    if length < 0 {
        return ctx.throw_by_name_with_message(
            "System.ArgumentOutOfRangeException",
            "length must be non-negative.",
        );
    }

    let element_runtime_type =
        dotnet_vm_ops::vm_try!(crate::common::resolve_runtime_type(ctx, element_type));
    let vector = dotnet_vm_ops::vm_try!(ctx.new_vector(
        element_runtime_type.to_concrete(ctx.loader().as_ref()),
        length as usize,
    ));

    let gc = ctx.gc_with_token(&ctx.no_active_borrows_token());
    let array_ref = ObjectRef::new(gc, HeapStorage::Vec(Box::new(vector)));
    ctx.register_new_object(&array_ref);
    ctx.push_obj(array_ref);
    StepResult::Continue
}

#[dotnet_intrinsic("static object System.Activator::CreateInstance()")]
pub fn intrinsic_activator_create_instance<'gc, T: ReflectionIntrinsicHost<'gc>>(
    ctx: &mut T,
    _method: MethodDescription,
    generics: &GenericLookup,
) -> StepResult {
    let target_ct = dotnet_vm_ops::vm_try!(generics.cloned_method_arg(0));
    let target_td = ctx
        .loader()
        .find_concrete_type(target_ct.clone())
        .expect("Type must exist for Activator.CreateInstance");

    let empty_lookup = ctx.reflection_empty_generics();
    if dotnet_vm_ops::vm_try!(
        ctx.reflection_is_value_type_with_lookup(target_td.clone(), &empty_lookup)
    ) {
        let instance = dotnet_vm_ops::vm_try!(ctx.new_object(target_td));
        ctx.push_value_type(instance);
        StepResult::Continue
    } else {
        let instance = dotnet_vm_ops::vm_try!(ctx.new_object(target_td.clone()));
        let mut new_lookup = GenericLookup::default();
        if let dotnetdll::prelude::BaseType::Type {
            source: TypeSource::Generic { parameters, .. },
            ..
        } = target_ct.get()
        {
            new_lookup.set_type_generics(parameters.clone().into());
        }

        if let Some(desc) = crate::common::find_parameterless_ctor(&target_td, &new_lookup) {
            let info = dotnet_vm_ops::vm_try!(ctx.reflection_method_info(desc, &new_lookup));
            dotnet_vm_ops::vm_try!(ctx.reflection_constructor_frame(instance, info, new_lookup));
            return StepResult::FramePushed;
        }

        // Keep this as a recoverable host error: metadata/program input can legitimately request
        // activation of a type without an accessible parameterless constructor.
        StepResult::Error(VmError::Execution(ExecutionError::InternalError(
            format!(
                "could not find a parameterless constructor in {:?}",
                target_td
            )
            .into(),
        )))
    }
}

/// Resolve the same positional constructor as the synchronous Activator path, without side effects.
pub fn resolve_activator_constructor<'gc, T: ReflectionIntrinsicHost<'gc>>(
    ctx: &T,
    target: ObjectRef<'gc>,
    parameters: ObjectRef<'gc>,
) -> Option<(MethodDescription, GenericLookup)> {
    target.0?;
    let (td, lookup) = match crate::common::resolve_runtime_type(ctx, target).ok()? {
        RuntimeType::Type(td) => (td, GenericLookup::default()),
        RuntimeType::Generic(td, args) => {
            let lookup = GenericLookup::new(
                args.iter()
                    .map(|arg| arg.to_concrete(ctx.loader().as_ref()))
                    .collect(),
            );
            (td, lookup)
        }
        _ => return None,
    };
    let gc = ctx.gc_with_token(&ctx.no_active_borrows_token());
    let args = if parameters.0.is_none() {
        Vec::new()
    } else {
        parameters.as_heap_storage(|storage| match storage {
            HeapStorage::Vec(vector) => Some(vector.object_ref_elements(&gc).collect::<Vec<_>>()),
            _ => None,
        })?
    };
    let method = crate::invoke_args::select_constructor(ctx, &td, &lookup, &args)?;
    Some((method, lookup))
}

pub fn activator_select<'gc, T: ReflectionIntrinsicHost<'gc>>(
    ctx: &T,
    operands: NativeOperandView<'_, 'gc>,
    _method: &MethodDescription,
    _generics: &GenericLookup,
) -> NativeSelection {
    let (StackValue::ObjectRef(target), StackValue::ObjectRef(parameters)) =
        (operands.peek_at(1), operands.peek_at(0))
    else {
        return NativeSelection::Fallback;
    };
    let Some((method, lookup)) = resolve_activator_constructor(ctx, target, parameters) else {
        return NativeSelection::Fallback;
    };
    if !crate::methods::invoke_arguments_supported(ctx, &method, &lookup, parameters) {
        return NativeSelection::Fallback;
    }
    tracing::debug!(reflection_invoke_path = "async");
    NativeSelection::Async(NativeInvocation {
        start: 0,
        length: 0,
        parameter_count: 2,
    })
}

#[dotnet_intrinsic("static object System.Activator::CreateInstance(System.Type, object[])", select = activator_select, fallback = intrinsic_activator_create_instance_type_args)]
pub async fn activator_async(fx: VmEffects) -> NativeResult {
    crate::methods::run_reflection_call(fx).await
}

pub fn intrinsic_activator_create_instance_type_args<'gc, T: ReflectionIntrinsicHost<'gc>>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let gc = ctx.gc_with_token(&ctx.no_active_borrows_token());
    let ctor_args_obj = ctx.pop_obj();
    let target_type_obj = ctx.pop_obj();

    if target_type_obj.0.is_none() {
        return ctx.throw_by_name_with_message(
            "System.ArgumentNullException",
            "Value cannot be null. (Parameter 'type')",
        );
    }

    let target_runtime_type = match crate::common::resolve_runtime_type(ctx, target_type_obj) {
        Ok(t) => t,
        Err(_) => {
            return ctx.throw_by_name_with_message("System.ArgumentException", "Arg_MustBeType");
        }
    };

    let (target_td, lookup, invoke_return_type) = match target_runtime_type {
        RuntimeType::Type(td) => (td.clone(), GenericLookup::default(), RuntimeType::Type(td)),
        RuntimeType::Generic(td, type_args) => {
            let lookup = GenericLookup::new(
                type_args
                    .iter()
                    .map(|arg| arg.to_concrete(ctx.loader().as_ref()))
                    .collect(),
            );
            (
                td.clone(),
                lookup,
                RuntimeType::Generic(td, type_args.clone()),
            )
        }
        _ => {
            return ctx.throw_by_name_with_message("System.ArgumentException", "Arg_MustBeType");
        }
    };

    let ctor_arg_objs = if ctor_args_obj.0.is_none() {
        Vec::new()
    } else {
        dotnet_vm_ops::vm_try!(ctor_args_obj.try_as_vector(
            |v: &dotnet_value::object::Vector<'gc>| v.object_ref_elements(&gc).collect::<Vec<_>>()
        ))
    };

    let Some(ctor_desc) =
        crate::invoke_args::select_constructor(ctx, &target_td, &lookup, &ctor_arg_objs)
    else {
        // A struct always has a default value, even without a declared constructor.
        if ctor_arg_objs.is_empty()
            && dotnet_vm_ops::vm_try!(
                ctx.reflection_is_value_type_with_lookup(target_td.clone(), &lookup)
            )
        {
            let instance = dotnet_vm_ops::vm_try!(
                ctx.reflection_new_object_with_lookup(target_td.clone(), &lookup)
            );
            let boxed = ObjectRef::new(gc, HeapStorage::Boxed(Box::new(instance)));
            ctx.register_new_object(&boxed);
            ctx.push_obj(boxed);
            return StepResult::Continue;
        }
        return ctx.throw_by_name_with_message(
            "System.MissingMethodException",
            "No matching constructor found.",
        );
    };

    let ctor_args =
        match crate::methods::unmarshal_invoke_params(ctx, &gc, &ctor_desc, &lookup, ctor_args_obj)
        {
            Ok(args) => args,
            Err(thrown) => return thrown,
        };

    let instance =
        dotnet_vm_ops::vm_try!(ctx.reflection_new_object_with_lookup(target_td.clone(), &lookup));
    let this_obj = ObjectRef::new(gc, HeapStorage::Obj(Box::new(instance)));
    ctx.register_new_object(&this_obj);

    // Preserve the created instance so return-frame reflection boxing can return it
    // after the constructor (which itself returns void) completes.
    ctx.push_obj(this_obj);

    // Constructor call arguments: `this` first, then positional constructor args.
    ctx.push_obj(this_obj);
    for arg in ctor_args {
        ctx.push(arg);
    }

    ctx.set_frame_return_action(FrameReturnAction::InvokeReturn(invoke_return_type));
    ctx.reflection_dispatch_method(ctor_desc, lookup)
}

pub fn handle_create_instance_check_this<'gc, T: TypedStackOps<'gc>>(
    ctx: &mut T,
    _generics: &GenericLookup,
) -> StepResult {
    let _obj = ctx.pop_obj();
    StepResult::Continue
}

pub fn handle_make_generic_type<'gc, T: ReflectionIntrinsicHost<'gc>>(
    ctx: &mut T,
    _generics: &GenericLookup,
) -> StepResult {
    let gc = ctx.gc_with_token(&ctx.no_active_borrows_token());
    let parameters = ctx.pop_obj();
    let target = ctx.pop_obj();

    if ctx.check_gc_safe_point() {
        return StepResult::Yield;
    }

    let target_rt = dotnet_vm_ops::vm_try!(crate::common::resolve_runtime_type(ctx, target));

    if let RuntimeType::Type(td) | RuntimeType::Generic(td, _) = target_rt {
        let param_objs = dotnet_vm_ops::vm_try!(parameters.try_as_vector(
            |v: &dotnet_value::object::Vector<'gc>| {
                v.object_ref_elements(&gc).collect::<Vec<_>>()
            }
        ));
        let mut new_generics = Vec::with_capacity(param_objs.len());
        for p_obj in param_objs {
            new_generics.push(dotnet_vm_ops::vm_try!(crate::common::resolve_runtime_type(
                ctx, p_obj
            )));
        }

        #[cfg(feature = "generic-constraint-validation")]
        {
            let loader = ctx.loader().clone();
            let new_generics_concrete: Vec<ConcreteType> = new_generics
                .iter()
                .map(|rt| rt.to_concrete(loader.as_ref()))
                .collect();
            let lookup = GenericLookup::new(new_generics_concrete);
            if let Err(_e) = lookup.validate_constraints(
                td.resolution.clone(),
                loader.as_ref(),
                &td.definition().generic_parameters,
                false,
            ) {
                return ctx.throw_by_name_with_message(
                    "System.TypeLoadException",
                    "Generic constraint violation.",
                );
            }
        }

        let new_rt = RuntimeType::Generic(td, new_generics);

        let rt_obj = crate::common::get_runtime_type(ctx, new_rt);
        ctx.push_obj(rt_obj);
        StepResult::Continue
    } else {
        ctx.throw_by_name_with_message(
            "System.InvalidOperationException",
            "MakeGenericType may only be called on a type for which IsGenericTypeDefinition is true.",
        )
    }
}

pub fn handle_make_by_ref_type<'gc, T: ReflectionIntrinsicHost<'gc>>(
    ctx: &mut T,
    _generics: &GenericLookup,
) -> StepResult {
    let target = ctx.pop_obj();

    if target.0.is_none() {
        return ctx.throw_by_name_with_message(
            "System.NullReferenceException",
            "Object reference not set to an instance of an object.",
        );
    }

    let target_rt = dotnet_vm_ops::vm_try!(crate::common::resolve_runtime_type(ctx, target));
    let by_ref_rt = RuntimeType::ByRef(Box::new(target_rt));
    let rt_obj = crate::common::get_runtime_type(ctx, by_ref_rt);
    ctx.push_obj(rt_obj);
    StepResult::Continue
}

pub fn handle_make_array_type<'gc, T: ReflectionIntrinsicHost<'gc>>(
    ctx: &mut T,
    _generics: &GenericLookup,
) -> StepResult {
    let target = ctx.pop_obj();

    if target.0.is_none() {
        return ctx.throw_by_name_with_message(
            "System.NullReferenceException",
            "Object reference not set to an instance of an object.",
        );
    }

    let target_rt = dotnet_vm_ops::vm_try!(crate::common::resolve_runtime_type(ctx, target));
    let array_rt = RuntimeType::Vector(Box::new(target_rt));
    let rt_obj = crate::common::get_runtime_type(ctx, array_rt);
    ctx.push_obj(rt_obj);
    StepResult::Continue
}

pub fn handle_create_instance_default_ctor<'gc, T: ReflectionIntrinsicHost<'gc>>(
    ctx: &mut T,
    _generics: &GenericLookup,
) -> StepResult {
    let _ = ctx.pop();
    let _ = ctx.pop();
    let target_obj = ctx.pop_obj();

    if ctx.check_gc_safe_point() {
        return StepResult::Yield;
    }

    let target_rt = dotnet_vm_ops::vm_try!(crate::common::resolve_runtime_type(ctx, target_obj));

    let (td, type_generics) = match target_rt {
        RuntimeType::Type(td) => (td, vec![]),
        RuntimeType::Generic(td, args) => (td, args.clone()),
        _ => {
            // Keep this recoverable: the reflected runtime type can come from user code and may
            // denote an uninstantiable shape (pointer/byref/etc.), which is not a VM invariant.
            return StepResult::Error(VmError::Execution(ExecutionError::InternalError(
                format!("cannot create instance of {:?}", target_rt).into(),
            )));
        }
    };

    let type_generics_concrete: Vec<ConcreteType> = type_generics
        .iter()
        .map(|a| a.to_concrete(ctx.loader().as_ref()))
        .collect();
    let new_lookup = GenericLookup::new(type_generics_concrete);

    let instance =
        dotnet_vm_ops::vm_try!(ctx.reflection_new_object_with_lookup(td.clone(), &new_lookup));

    if let Some(desc) = td
        .definition()
        .methods
        .iter()
        .enumerate()
        .find_map(|(i, m)| {
            if !m.runtime_special_name || m.name != ".ctor" {
                return None;
            }
            let d = MethodDescription::new(
                td.clone(),
                new_lookup.clone(),
                td.resolution.clone(),
                MethodMemberIndex::Method(i),
            );
            let sig = d.signature();
            (sig.instance && sig.parameters.is_empty()).then_some(d)
        })
    {
        let info = dotnet_vm_ops::vm_try!(ctx.reflection_method_info(desc, &new_lookup));
        dotnet_vm_ops::vm_try!(ctx.reflection_constructor_frame(instance, info, new_lookup));
        return StepResult::FramePushed;
    }

    // Keep this as a recoverable host error: `Activator.CreateInstance(Type)` may target a type
    // that does not expose a parameterless constructor.
    StepResult::Error(VmError::Execution(ExecutionError::InternalError(
        format!("could not find a parameterless constructor in {:?}", td).into(),
    )))
}

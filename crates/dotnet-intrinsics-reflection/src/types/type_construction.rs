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
use dotnet_vm_data::StepResult;
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

    let target_obj = handle
        .instance_storage
        .field::<ObjectRef<'gc>>(handle.description, "_value")
        .unwrap()
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
    let target_ct = generics.method_generics[0].clone();
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
            new_lookup.type_generics = parameters.clone().into();
        }

        if let Some(desc) = target_td
            .definition()
            .methods
            .iter()
            .enumerate()
            .find_map(|(i, m)| {
                if m.name != ".ctor" {
                    return None;
                }
                let d = MethodDescription::new(
                    target_td.clone(),
                    new_lookup.clone(),
                    target_td.resolution.clone(),
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

#[dotnet_intrinsic("static object System.Activator::CreateInstance(System.Type, object[])")]
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

    let ctor_args = if ctor_args_obj.0.is_none() {
        Vec::new()
    } else {
        dotnet_vm_ops::vm_try!(ctor_args_obj.try_as_vector(
            |v: &dotnet_value::object::Vector<'gc>| {
                v.get()
                    .chunks_exact(ObjectRef::SIZE)
                    .map(|chunk| unsafe { ObjectRef::read_branded(chunk, &gc) })
                    .map(StackValue::ObjectRef)
                    .collect::<Vec<_>>()
            }
        ))
    };

    let Some(ctor_desc) =
        target_td
            .definition()
            .methods
            .iter()
            .enumerate()
            .find_map(|(index, method)| {
                if method.name != ".ctor" {
                    return None;
                }

                let candidate = MethodDescription::new(
                    target_td.clone(),
                    lookup.clone(),
                    target_td.resolution.clone(),
                    MethodMemberIndex::Method(index),
                );
                let signature = candidate.signature();
                (signature.instance && signature.parameters.len() == ctor_args.len())
                    .then_some(candidate)
            })
    else {
        return ctx.throw_by_name_with_message(
            "System.MissingMethodException",
            "No matching constructor found.",
        );
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

    ctx.frame_stack_mut()
        .current_frame_mut()
        .awaiting_invoke_return = Some(invoke_return_type);
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
                v.get()
                    .chunks_exact(ObjectRef::SIZE)
                    .map(|chunk| unsafe { ObjectRef::read_branded(chunk, &gc) })
                    .collect::<Vec<_>>()
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

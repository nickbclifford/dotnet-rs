use crate::ReflectionIntrinsicHost;
use dotnet_macros::dotnet_intrinsic;
#[cfg(feature = "generic-constraint-validation")]
use dotnet_types::generics::ConcreteType;
use dotnet_types::{
    error::ExecutionError, generics::GenericLookup, members::MethodDescription,
    runtime::RuntimeType,
};
use dotnet_value::{
    StackValue,
    object::{HeapStorage, ObjectRef},
};
use dotnet_vm_data::StepResult;
use dotnet_vm_ops::effects::{
    NativeInvocation, NativeOperandView, NativeResult, NativeSelection, VmEffects,
};
use dotnet_vm_ops::ops::TypedStackOps;

#[dotnet_intrinsic(
    "static void System.Runtime.CompilerServices.RuntimeHelpers::RunClassConstructor(System.RuntimeTypeHandle)"
)]
pub async fn intrinsic_runtime_helpers_run_class_constructor(mut fx: VmEffects) -> NativeResult {
    let (description, generics) = {
        let mut vm = fx.vm().await?;
        vm.with(|region| region.class_constructor_target())??
    };
    fx.ensure_initialized(description, generics).await
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

pub fn activator_default_select<'gc, T: ReflectionIntrinsicHost<'gc>>(
    _ctx: &T,
    _operands: NativeOperandView<'_, 'gc>,
    method: &MethodDescription,
    _generics: &GenericLookup,
) -> NativeSelection {
    tracing::debug!(reflection_invoke_path = "async");
    NativeSelection::Async(NativeInvocation {
        start: 0,
        length: 0,
        parameter_count: method.signature().parameters.len(),
    })
}

#[dotnet_intrinsic("static object System.Activator::CreateInstance()", select = activator_default_select)]
#[dotnet_intrinsic("object System.RuntimeType::CreateInstanceDefaultCtor(bool, bool)", select = activator_default_select)]
#[dotnet_intrinsic("object DotnetRs.RuntimeType::CreateInstanceDefaultCtor(bool, bool)", select = activator_default_select)]
pub async fn activator_default_async(fx: VmEffects) -> NativeResult {
    crate::methods::run_reflection_call(fx).await
}

/// What `Activator.CreateInstance(Type, object[])` creates once its operands are validated.
pub enum ActivatorTarget<'gc> {
    /// Allocate an instance of the declaring type and run this constructor on it.
    Constructor(MethodDescription, GenericLookup),
    /// A value type with no matching constructor and no arguments: its boxed default value.
    Default(ObjectRef<'gc>),
}

/// Resolve `Activator.CreateInstance(Type, object[])` the way the CLR does, raising its managed
/// exceptions: `ArgumentNullException` for a null type, `ArgumentException` for an open generic,
/// `NotSupportedException` for a byref-like type, and `MissingMethodException` when no public
/// constructor binds or the type is abstract. With no arguments, a value type without a matching
/// constructor produces its default value. Among several binding constructors the one needing the
/// fewest conversions wins rather than raising `AmbiguousMatchException` (a recorded deviation).
pub fn activator_target<'gc, T: ReflectionIntrinsicHost<'gc>>(
    ctx: &mut T,
    target: ObjectRef<'gc>,
    parameters: ObjectRef<'gc>,
) -> Result<ActivatorTarget<'gc>, StepResult> {
    if target.0.is_none() {
        return Err(ctx.throw_by_name_with_message(
            "System.ArgumentNullException",
            "Value cannot be null. (Parameter 'type')",
        ));
    }
    let runtime = crate::common::resolve_runtime_type(ctx, target).map_err(|_| {
        ctx.throw_by_name_with_message("System.ArgumentException", "Arg_MustBeType")
    })?;
    if contains_generic_parameters(&runtime) {
        let message = format!(
            "Cannot create an instance of {} because Type.ContainsGenericParameters is true.",
            runtime.get_name()
        );
        return Err(ctx.throw_by_name_with_message("System.ArgumentException", &message));
    }
    let (td, lookup) = match &runtime {
        RuntimeType::Type(td) => (td.clone(), GenericLookup::default()),
        RuntimeType::Generic(td, args) => (
            td.clone(),
            GenericLookup::new(
                args.iter()
                    .map(|arg| arg.to_concrete(ctx.loader().as_ref()))
                    .collect(),
            ),
        ),
        // Arrays declare no constructors that `Activator` binds.
        RuntimeType::Vector(_) | RuntimeType::Array(..) => {
            return Err(missing_constructor(ctx, &runtime.get_name()));
        }
        // Constructing a string needs the intrinsic constructor protocol, which reflection does
        // not use; pointers, byrefs and `void` are not creatable.
        RuntimeType::String
        | RuntimeType::Void
        | RuntimeType::Pointer(_)
        | RuntimeType::ByRef(_)
        | RuntimeType::ValuePointer(..)
        | RuntimeType::FunctionPointer(_) => {
            return Err(
                ctx.throw_by_name_with_message("System.ArgumentException", "Arg_MustBeType")
            );
        }
        // Primitives, `object` and `TypedReference` are ordinary CoreLib type definitions.
        other => {
            let td = ctx
                .loader()
                .find_concrete_type(other.to_concrete(ctx.loader().as_ref()))
                .map_err(|error| StepResult::Error(error.into()))?;
            (td, GenericLookup::default())
        }
    };
    if crate::invoke_args::is_byref_like(ctx, &td) {
        return Err(crate::invoke_args::throw_byref_like_value(ctx));
    }
    let gc = ctx.gc_with_token(&ctx.no_active_borrows_token());
    let args = if parameters.0.is_none() {
        Vec::new()
    } else {
        parameters
            .try_as_vector(|vector: &dotnet_value::object::Vector<'gc>| {
                vector.object_ref_elements(&gc).collect::<Vec<_>>()
            })
            .map_err(|error| StepResult::Error(error.into()))?
    };
    if let Some(method) = crate::invoke_args::select_constructor(ctx, &td, &lookup, &args) {
        if td.definition().flags.abstract_type {
            let message = format!(
                "Cannot dynamically create an instance of type '{}'. Reason: Cannot create an abstract class.",
                td.type_name()
            );
            return Err(ctx.throw_by_name_with_message("System.MissingMethodException", &message));
        }
        return Ok(ActivatorTarget::Constructor(method, lookup));
    }
    let value_type = ctx
        .reflection_is_value_type_with_lookup(td.clone(), &lookup)
        .map_err(|error| StepResult::Error(error.into()))?;
    if args.is_empty() && value_type {
        let instance = ctx
            .reflection_new_object_with_lookup(td, &lookup)
            .map_err(|error| StepResult::Error(error.into()))?;
        let boxed = ObjectRef::new(gc, HeapStorage::Boxed(Box::new(instance)));
        ctx.register_new_object(&boxed);
        return Ok(ActivatorTarget::Default(boxed));
    }
    Err(missing_constructor(ctx, &td.type_name()))
}

fn missing_constructor<'gc, T: ReflectionIntrinsicHost<'gc>>(
    ctx: &mut T,
    name: &str,
) -> StepResult {
    ctx.throw_by_name_with_message(
        "System.MissingMethodException",
        &format!("Constructor on type '{name}' not found."),
    )
}

/// Whether `runtime` mentions an unbound type or method generic parameter (`List<>`, or `T[]`
/// inside a generic method), which `Type.ContainsGenericParameters` reports.
fn contains_generic_parameters(runtime: &RuntimeType) -> bool {
    match runtime {
        RuntimeType::TypeParameter { .. } | RuntimeType::MethodParameter { .. } => true,
        RuntimeType::Generic(_, args) => args.iter().any(contains_generic_parameters),
        RuntimeType::Vector(inner)
        | RuntimeType::Array(inner, _)
        | RuntimeType::Pointer(inner)
        | RuntimeType::ByRef(inner)
        | RuntimeType::ValuePointer(inner, _) => contains_generic_parameters(inner),
        _ => false,
    }
}

/// Every `Activator.CreateInstance(Type, object[])` runs in the native body; validation there
/// raises the CLR's managed exceptions.
pub fn activator_select<'gc, T: ReflectionIntrinsicHost<'gc>>(
    _ctx: &T,
    operands: NativeOperandView<'_, 'gc>,
    _method: &MethodDescription,
    _generics: &GenericLookup,
) -> NativeSelection {
    if !matches!(
        (operands.peek_at(1), operands.peek_at(0)),
        (
            StackValue::ObjectRef(_),
            StackValue::ObjectRef(_) | StackValue::Int32(_)
        )
    ) {
        return NativeSelection::Reject(
            ExecutionError::InternalError(
                "Activator.CreateInstance operands are not objects".into(),
            )
            .into(),
        );
    }
    tracing::debug!(reflection_invoke_path = "async");
    NativeSelection::Async(NativeInvocation {
        start: 0,
        length: 0,
        parameter_count: 2,
    })
}

#[dotnet_intrinsic("static object System.Activator::CreateInstance(System.Type, object[])", select = activator_select)]
#[dotnet_intrinsic("static object System.Activator::CreateInstance(System.Type, bool)", select = activator_select)]
pub async fn activator_async(fx: VmEffects) -> NativeResult {
    crate::methods::run_reflection_call(fx).await
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

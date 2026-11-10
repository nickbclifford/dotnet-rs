use crate::ReflectionIntrinsicHost;
use crate::invoke_args::{
    ArgPlan, ParamShape, nullable_inner, nullable_with_value, param_shape, plan_argument,
    receiver_fits, widen_boxed,
};
use dotnet_intrinsics_delegates::helpers::{
    set_delegate_multicast_targets, set_delegate_target_method,
};
use dotnet_macros::dotnet_intrinsic;
use dotnet_types::{
    TypeDescription, WellKnown,
    error::TypeResolutionError,
    generics::{ConcreteType, GenericLookup},
    members::MethodDescription,
    runtime::{RuntimeType, runtime_type_from_concrete},
};
use dotnet_utils::{ByteOffset, gc::GCHandle};
use dotnet_value::{
    StackValue,
    layout::HasLayout,
    object::{CTSValue, HeapStorage, ObjectRef},
};
use dotnet_vm_data::{FrameReturnAction, StepResult};
use dotnet_vm_ops::SupportSlotOps;
use dotnet_vm_ops::effects::{
    NativeInvocation, NativeOperandView, NativeResult, NativeSelection, NativeValue, VmEffects,
};
use dotnet_vm_ops::intrinsic_args::type_mismatch;
use dotnetdll::prelude::{BaseType, ParameterType};

fn invoke_operand_window_supported(values: &[StackValue<'_>; 6]) -> bool {
    matches!(values[0], StackValue::ObjectRef(_))
        && matches!(values[1], StackValue::ObjectRef(_))
        && matches!(values[2], StackValue::Int32(_))
        && matches!(values[3], StackValue::ObjectRef(_))
        && matches!(values[4], StackValue::ObjectRef(_))
        && matches!(values[5], StackValue::ObjectRef(_))
}

/// Enums stay on the synchronous path: their arguments keep the registered fallback exercised.
fn scalar_like<'gc, T: ReflectionIntrinsicHost<'gc>>(_ctx: &T, concrete: &ConcreteType) -> bool {
    scalar_type(concrete)
}

fn scalar_type(concrete: &ConcreteType) -> bool {
    matches!(
        concrete.get(),
        BaseType::Boolean
            | BaseType::Char
            | BaseType::Int8
            | BaseType::UInt8
            | BaseType::Int16
            | BaseType::UInt16
            | BaseType::Int32
            | BaseType::UInt32
            | BaseType::Int64
            | BaseType::UInt64
            | BaseType::IntPtr
            | BaseType::UIntPtr
            | BaseType::Float32
            | BaseType::Float64
    )
}

/// Check that argument marshalling can be rooted before selecting an effects call.
pub fn invoke_arguments_supported<'gc, T: ReflectionIntrinsicHost<'gc>>(
    ctx: &T,
    method: &MethodDescription,
    lookup: &GenericLookup,
    parameters: ObjectRef<'gc>,
) -> bool {
    let gc = ctx.gc_with_token(&ctx.no_active_borrows_token());
    let args = if parameters.0.is_none() {
        method.signature().parameters.is_empty().then(Vec::new)
    } else {
        parameters.as_heap_storage(|storage| {
            let HeapStorage::Vec(vector) = storage else {
                return None;
            };
            if vector.layout.length != method.signature().parameters.len()
                || !matches!(vector.element.get(), BaseType::Object)
            {
                return None;
            }
            Some(vector.object_ref_elements(&gc).collect::<Vec<_>>())
        })
    };
    let Some(args) = args else {
        return false;
    };
    args.into_iter()
        .zip(&method.signature().parameters)
        .all(|(arg, param)| {
            let (ty, byref) = match &param.1 {
                ParameterType::Ref(ty) => (ty, true),
                ParameterType::Value(ty) => (ty, false),
                _ => return false,
            };
            let Ok(shape) = param_shape(ctx, method, lookup, ty) else {
                return false;
            };
            if !shape.value_type {
                return true;
            }
            let plan = plan_argument(ctx, arg, &shape, byref);
            if byref {
                return scalar_like(ctx, &shape.concrete)
                    && (arg.0.is_none() || plan == ArgPlan::Exact);
            }
            let transportable = scalar_like(ctx, &shape.concrete)
                || (shape.td.definition().name == "Nullable`1"
                    && nullable_inner(&shape.concrete)
                        .is_some_and(|inner| scalar_like(ctx, &inner)));
            transportable
                && matches!(
                    plan,
                    ArgPlan::Exact
                        | ArgPlan::Widen { .. }
                        | ArgPlan::Default
                        | ArgPlan::NullableSome
                )
        })
}

/// Choose reflection calls whose arguments and result are transportable without side effects.
pub fn invoke_select<'gc, T: ReflectionIntrinsicHost<'gc>>(
    ctx: &T,
    operands: NativeOperandView<'_, 'gc>,
    entry_method: &MethodDescription,
    _generics: &GenericLookup,
) -> NativeSelection {
    let count = entry_method.signature().parameters.len();
    let values = if count == 4 {
        [
            operands.peek_at(4),
            StackValue::null(),
            operands.peek_at(3),
            operands.peek_at(2),
            operands.peek_at(1),
            operands.peek_at(0),
        ]
    } else {
        std::array::from_fn(|index| operands.peek_at(5 - index))
    };
    if !invoke_operand_window_supported(&values) {
        return NativeSelection::Fallback;
    }
    let StackValue::ObjectRef(method_obj @ ObjectRef(Some(_))) = values[0] else {
        return NativeSelection::Fallback;
    };
    let Ok((method, lookup)) = crate::common::resolve_runtime_method(ctx, method_obj) else {
        return NativeSelection::Fallback;
    };
    if matches!(
        method.signature().return_type.1,
        Some(ParameterType::Ref(_))
    ) {
        return NativeSelection::Reject(
            dotnet_types::error::ExecutionError::NotImplemented(
                "reflection invocation with a byref return".into(),
            )
            .into(),
        );
    }
    if method.signature().instance
        && count != 4
        && !matches!(values[1], StackValue::ObjectRef(ObjectRef(Some(_))))
    {
        return NativeSelection::Fallback;
    }
    // A receiver of the wrong type is rejected by the synchronous path, which can throw.
    if method.signature().instance
        && count != 4
        && let StackValue::ObjectRef(receiver) = values[1]
        && !crate::invoke_args::receiver_fits(ctx, &method, &lookup, receiver)
    {
        return NativeSelection::Fallback;
    }
    if let Some(return_type) = &method.signature().return_type.1 {
        let ParameterType::Value(ty) = return_type else {
            return NativeSelection::Fallback;
        };
        let Ok(concrete) =
            lookup.make_concrete(method.resolution(), ty.clone(), ctx.loader().as_ref())
        else {
            return NativeSelection::Fallback;
        };
        let Ok(td) = ctx.loader().find_concrete_type(concrete.clone()) else {
            return NativeSelection::Fallback;
        };
        let Ok(value_type) = ctx.reflection_is_value_type_with_lookup(td, &lookup) else {
            return NativeSelection::Fallback;
        };
        if value_type && !scalar_type(&concrete) {
            return NativeSelection::Fallback;
        }
    }
    let StackValue::ObjectRef(parameters) = values[4] else {
        return NativeSelection::Fallback;
    };
    if !invoke_arguments_supported(ctx, &method, &lookup, parameters) {
        return NativeSelection::Fallback;
    }
    tracing::debug!(reflection_invoke_path = "async");
    NativeSelection::Async(NativeInvocation {
        start: 0,
        length: 0,
        parameter_count: count,
    })
}

#[dotnet_intrinsic("object DotnetRs.ConstructorInfo::Invoke(object, System.Reflection.BindingFlags, System.Reflection.Binder, object[], System.Globalization.CultureInfo)", select = invoke_select, fallback = runtime_method_info_intrinsic_call)]
#[dotnet_intrinsic("object DotnetRs.ConstructorInfo::Invoke(System.Reflection.BindingFlags, System.Reflection.Binder, object[], System.Globalization.CultureInfo)", select = invoke_select, fallback = runtime_method_info_intrinsic_call)]
#[dotnet_intrinsic("object DotnetRs.MethodInfo::Invoke(object, System.Reflection.BindingFlags, System.Reflection.Binder, object[], System.Globalization.CultureInfo)", select = invoke_select, fallback = runtime_method_info_intrinsic_call)]
pub async fn invoke_async(fx: VmEffects) -> NativeResult {
    run_reflection_call(fx).await
}

pub async fn run_reflection_call(mut fx: VmEffects) -> NativeResult {
    let (call, constructor_result) = {
        let mut vm = fx.vm().await?;
        vm.prepare_reflection()?
    };
    let method = call.method.clone();
    let generics = call.generics.clone();
    let result = fx.call(call).await?;
    let mut vm = fx.vm().await?;
    let value = match constructor_result {
        Some(instance) => {
            vm.with(|region| region.reflection_constructor_result(&method, &generics, &instance))??
        }
        None => vm.with(|region| region.reflection_result(&method, &generics, result))??,
    };
    vm.push_value(NativeValue::Value(value))
}

#[dotnet_intrinsic(
    "static System.Reflection.MethodBase System.Reflection.MethodBase::GetMethodFromHandle(System.RuntimeMethodHandle)"
)]
#[dotnet_intrinsic(
    "static System.Reflection.MethodBase System.Reflection.MethodBase::GetMethodFromHandle(System.RuntimeMethodHandle, System.RuntimeTypeHandle)"
)]
pub fn intrinsic_method_base_get_method_from_handle<'gc, T: ReflectionIntrinsicHost<'gc>>(
    ctx: &mut T,
    method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    if method.signature().parameters.len() == 2 {
        let _declaring_type = ctx.pop_value_type();
    }
    let handle = ctx.pop_value_type();

    let handle_type =
        dotnet_vm_ops::vm_try!(ctx.loader().corlib_wkt(WellKnown::RuntimeMethodHandle));
    let method_obj = ctx
        .loader()
        .rmh_value_field(&handle.instance_storage, handle_type)
        .expect("validated RuntimeMethodHandle support slot")
        .read();
    if method_obj.0.is_none() {
        return ctx.throw_by_name_with_message(
            "System.ArgumentException",
            "The method handle does not reference a managed method.",
        );
    }

    ctx.push_obj(method_obj);
    StepResult::Continue
}

#[dotnet_intrinsic("string System.Reflection.MethodInfo::get_Name()")]
#[dotnet_intrinsic("System.Type System.Reflection.MethodInfo::get_DeclaringType()")]
#[dotnet_intrinsic("System.Type System.Reflection.MethodInfo::get_ReturnType()")]
#[dotnet_intrinsic(
    "System.Reflection.MethodAttributes System.Reflection.MethodBase::get_Attributes()"
)]
#[dotnet_intrinsic(
    "System.Reflection.CallingConventions System.Reflection.MethodBase::get_CallingConvention()"
)]
#[dotnet_intrinsic("bool System.Reflection.MethodBase::get_IsGenericMethod()")]
#[dotnet_intrinsic("bool System.Reflection.MethodBase::get_IsGenericMethodDefinition()")]
#[dotnet_intrinsic("bool System.Reflection.MethodBase::get_ContainsGenericParameters()")]
#[dotnet_intrinsic("System.Type[] System.Reflection.MethodBase::GetGenericArguments()")]
#[dotnet_intrinsic("System.RuntimeMethodHandle System.Reflection.MethodBase::get_MethodHandle()")]
#[dotnet_intrinsic(
    "System.Reflection.MethodInfo System.Reflection.MethodInfo::MakeGenericMethod(System.Type[])"
)]
#[dotnet_intrinsic("System.Delegate System.Reflection.MethodInfo::CreateDelegate(System.Type)")]
#[dotnet_intrinsic(
    "System.Delegate System.Reflection.MethodInfo::CreateDelegate(System.Type, object)"
)]
#[dotnet_intrinsic("string System.Reflection.MethodInfo::ToString()")]
#[dotnet_intrinsic("string DotnetRs.MethodInfo::get_Name()")]
#[dotnet_intrinsic("string DotnetRs.MethodInfo::GetName()")]
#[dotnet_intrinsic("System.Type DotnetRs.MethodInfo::get_DeclaringType()")]
#[dotnet_intrinsic("System.Type DotnetRs.MethodInfo::GetDeclaringType()")]
#[dotnet_intrinsic("System.Type DotnetRs.MethodInfo::get_ReturnType()")]
#[dotnet_intrinsic("System.Type DotnetRs.MethodInfo::GetReturnType()")]
#[dotnet_intrinsic("System.Reflection.MethodAttributes DotnetRs.MethodInfo::get_Attributes()")]
#[dotnet_intrinsic("System.Reflection.MethodAttributes DotnetRs.MethodInfo::GetAttributes()")]
#[dotnet_intrinsic(
    "System.Reflection.CallingConventions DotnetRs.MethodInfo::get_CallingConvention()"
)]
#[dotnet_intrinsic(
    "System.Reflection.CallingConventions DotnetRs.MethodInfo::GetCallingConvention()"
)]
#[dotnet_intrinsic("bool DotnetRs.MethodInfo::get_IsGenericMethod()")]
#[dotnet_intrinsic("bool DotnetRs.MethodInfo::GetIsGenericMethod()")]
#[dotnet_intrinsic("bool DotnetRs.MethodInfo::get_IsGenericMethodDefinition()")]
#[dotnet_intrinsic("bool DotnetRs.MethodInfo::GetIsGenericMethodDefinition()")]
#[dotnet_intrinsic("bool DotnetRs.MethodInfo::get_ContainsGenericParameters()")]
#[dotnet_intrinsic("bool DotnetRs.MethodInfo::GetContainsGenericParameters()")]
#[dotnet_intrinsic("System.Type[] DotnetRs.MethodInfo::GetGenericArguments()")]
#[dotnet_intrinsic("System.RuntimeMethodHandle DotnetRs.MethodInfo::get_MethodHandle()")]
#[dotnet_intrinsic("System.RuntimeMethodHandle DotnetRs.MethodInfo::GetMethodHandle()")]
#[dotnet_intrinsic("System.Delegate DotnetRs.MethodInfo::CreateDelegate(System.Type)")]
#[dotnet_intrinsic("System.Delegate DotnetRs.MethodInfo::CreateDelegate(System.Type, object)")]
#[dotnet_intrinsic("string DotnetRs.MethodInfo::ToString()")]
#[dotnet_intrinsic("System.Reflection.MethodInfo DotnetRs.MethodInfo::GetBaseDefinition()")]
#[dotnet_intrinsic(
    "System.Reflection.MethodInfo DotnetRs.MethodInfo::GetGenericMethodDefinition()"
)]
#[dotnet_intrinsic("string DotnetRs.ConstructorInfo::get_Name()")]
#[dotnet_intrinsic("string DotnetRs.ConstructorInfo::GetName()")]
#[dotnet_intrinsic("System.Type DotnetRs.ConstructorInfo::get_DeclaringType()")]
#[dotnet_intrinsic("System.Type DotnetRs.ConstructorInfo::GetDeclaringType()")]
#[dotnet_intrinsic("System.Reflection.MethodAttributes DotnetRs.ConstructorInfo::get_Attributes()")]
#[dotnet_intrinsic("System.Reflection.MethodAttributes DotnetRs.ConstructorInfo::GetAttributes()")]
#[dotnet_intrinsic("System.RuntimeMethodHandle DotnetRs.ConstructorInfo::get_MethodHandle()")]
#[dotnet_intrinsic("System.RuntimeMethodHandle DotnetRs.ConstructorInfo::GetMethodHandle()")]
#[dotnet_intrinsic(
    "System.Reflection.ParameterInfo[] System.Reflection.MethodBase::GetParameters()"
)]
#[dotnet_intrinsic("System.Reflection.ParameterInfo[] DotnetRs.MethodInfo::GetParameters()")]
#[dotnet_intrinsic("System.Reflection.ParameterInfo[] DotnetRs.ConstructorInfo::GetParameters()")]
#[dotnet_intrinsic("object[] DotnetRs.MethodInfo::GetCustomAttributes(bool)")]
#[dotnet_intrinsic("object[] DotnetRs.MethodInfo::GetCustomAttributes(System.Type, bool)")]
#[dotnet_intrinsic("object[] DotnetRs.ConstructorInfo::GetCustomAttributes(bool)")]
#[dotnet_intrinsic("object[] DotnetRs.ConstructorInfo::GetCustomAttributes(System.Type, bool)")]
#[dotnet_intrinsic("bool DotnetRs.MethodInfo::IsDefined(System.Type, bool)")]
#[dotnet_intrinsic("bool DotnetRs.ConstructorInfo::IsDefined(System.Type, bool)")]
pub fn runtime_method_info_intrinsic_call<'gc, T: ReflectionIntrinsicHost<'gc>>(
    ctx: &mut T,
    method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let gc = ctx.gc_with_token(&ctx.no_active_borrows_token());
    let method_name = &*method.method().name;
    let param_count = method.signature().parameters.len();

    let result = match (method_name, param_count) {
        ("get_ContainsGenericParameters", 0) => {
            // Every dispatched method is fully resolved, so report no unresolved type params.
            let _obj = ctx.pop_obj();
            ctx.push_i32(0);
            Some(StepResult::Continue)
        }
        ("GetIsGenericMethod" | "get_IsGenericMethod", 0) => {
            let method_obj = ctx.pop_obj();
            let (method, _) =
                dotnet_vm_ops::vm_try!(crate::common::resolve_runtime_method(ctx, method_obj));
            ctx.push_i32(if method.method().generic_parameters.is_empty() {
                0
            } else {
                1
            });
            Some(StepResult::Continue)
        }
        ("GetIsGenericMethodDefinition" | "get_IsGenericMethodDefinition", 0) => {
            let method_obj = ctx.pop_obj();
            let (method, lookup) =
                dotnet_vm_ops::vm_try!(crate::common::resolve_runtime_method(ctx, method_obj));
            let is_definition =
                !method.method().generic_parameters.is_empty() && lookup.method_generics.is_empty();
            ctx.push_i32(if is_definition { 1 } else { 0 });
            Some(StepResult::Continue)
        }
        ("IsDefined", 2) => {
            let _inherit = ctx.pop_i32();
            let attribute_type_obj = ctx.pop_obj();
            let method_obj = ctx.pop_obj();
            let attribute_filter = dotnet_vm_ops::vm_try!(crate::common::resolve_attribute_filter(
                ctx,
                attribute_type_obj
            ));
            let (method, _) =
                dotnet_vm_ops::vm_try!(crate::common::resolve_runtime_method(ctx, method_obj));
            let attrs = dotnet_vm_ops::vm_try!(crate::types::collect_method_custom_attributes(
                ctx,
                method,
                attribute_filter
            ));
            ctx.push_i32(if attrs.is_empty() { 0 } else { 1 });
            Some(StepResult::Continue)
        }
        ("GetName" | "get_Name", 0) => {
            let obj = ctx.pop_obj();
            let (method, _) =
                dotnet_vm_ops::vm_try!(crate::common::resolve_runtime_method(ctx, obj));
            ctx.push_string(method.method().name.clone().into());
            Some(StepResult::Continue)
        }
        ("GetDeclaringType" | "get_DeclaringType", 0) => {
            let obj = ctx.pop_obj();
            let (method, lookup) =
                dotnet_vm_ops::vm_try!(crate::common::resolve_runtime_method(ctx, obj));
            let declaring_type = declaring_runtime_type(ctx, &method, &lookup);
            let rt_obj = crate::common::get_runtime_type(ctx, declaring_type);
            ctx.push_obj(rt_obj);
            Some(StepResult::Continue)
        }
        ("GetMethodHandle" | "get_MethodHandle", 0) => {
            let obj = ctx.pop_obj();

            let rmh = ctx
                .loader()
                .corlib_wkt(WellKnown::RuntimeMethodHandle)
                .expect("System.RuntimeMethodHandle must exist");
            let instance = dotnet_vm_ops::vm_try!(ctx.new_object(rmh.clone()));
            ctx.loader()
                .rmh_value_field(&instance.instance_storage, rmh)
                .expect("validated RuntimeMethodHandle support slot")
                .write(obj);

            ctx.push(StackValue::ValueType(instance));
            Some(StepResult::Continue)
        }
        ("get_ReturnType" | "GetReturnType", 0) => {
            let obj = ctx.pop_obj();
            let (method, lookup) =
                dotnet_vm_ops::vm_try!(crate::common::resolve_runtime_method(ctx, obj));
            let rt = resolve_return_type(ctx, &method, &lookup);
            let rt_obj = crate::common::get_runtime_type(ctx, rt);
            ctx.push_obj(rt_obj);
            Some(StepResult::Continue)
        }
        ("GetAttributes" | "get_Attributes" | "GetMethodFlags", 0) => {
            let obj = ctx.pop_obj();
            let (method, _) =
                dotnet_vm_ops::vm_try!(crate::common::resolve_runtime_method(ctx, obj));
            ctx.push_i32(method_attributes_flags(&method));
            Some(StepResult::Continue)
        }
        ("GetParameters", 0) => {
            let obj = ctx.pop_obj();
            let (method, lookup) =
                dotnet_vm_ops::vm_try!(crate::common::resolve_runtime_method(ctx, obj));
            let method_index =
                crate::common::get_runtime_method_index(ctx, method.clone(), lookup.clone())
                    as usize;

            let param_count = method.signature().parameters.len();

            let pi_type = ctx
                .loader()
                .corlib_wkt(WellKnown::SupportParameterInfo)
                .expect("DotnetRs.ParameterInfo not found");

            let mut pi_objs = Vec::with_capacity(param_count);
            for i in 0..param_count {
                let pi_obj = dotnet_vm_ops::vm_try!(ctx.new_object(pi_type.clone()));
                let pi_ref = ObjectRef::new(gc, HeapStorage::Obj(Box::new(pi_obj)));
                ctx.register_new_object(&pi_ref);
                let pi_type_inner = pi_type.clone();
                pi_ref.as_object_mut(gc, |instance| {
                    ctx.loader()
                        .parameter_info_method_index_field(
                            &instance.instance_storage,
                            pi_type_inner.clone(),
                        )
                        .expect("validated ParameterInfo method-index support slot")
                        .write(method_index);
                    ctx.loader()
                        .parameter_info_position_field(&instance.instance_storage, pi_type_inner)
                        .expect("validated ParameterInfo position support slot")
                        .write(i as i32);
                });
                pi_objs.push(pi_ref);
            }

            let array_element_type = ctx
                .loader()
                .corlib_wkt(WellKnown::ReflectionParameterInfo)
                .expect("System.Reflection.ParameterInfo not found");
            let array_obj =
                dotnet_vm_ops::vm_try!(ctx.new_vector(array_element_type.into(), param_count));
            let array_ref = ObjectRef::new(gc, HeapStorage::Vec(Box::new(array_obj)));
            ctx.register_new_object(&array_ref);

            for (i, pi_ref) in pi_objs.into_iter().enumerate() {
                let elem_type = array_ref.as_vector(|v| v.element.clone());
                let cts_val: CTSValue<'gc> = dotnet_vm_ops::vm_try!(
                    ctx.new_cts_value(&elem_type, StackValue::ObjectRef(pi_ref))
                );
                array_ref.as_vector_mut(gc, |v| {
                    let elem_size = v.layout.element_layout.size();
                    let start = (elem_size * i).as_usize();
                    let end = (elem_size * (i + 1)).as_usize();
                    cts_val.write(&mut v.get_mut()[start..end]);
                });
            }

            ctx.push_obj(array_ref);
            Some(StepResult::Continue)
        }
        ("GetBaseDefinition", 0) => {
            let method_obj = ctx.pop_obj();
            let (method, lookup) =
                dotnet_vm_ops::vm_try!(crate::common::resolve_runtime_method(ctx, method_obj));
            let (base_method, base_lookup) =
                dotnet_vm_ops::vm_try!(resolve_base_method_definition(ctx, &method, &lookup));
            let base_obj = crate::common::get_runtime_method_obj(ctx, base_method, base_lookup);
            ctx.push_obj(base_obj);
            Some(StepResult::Continue)
        }
        ("GetGenericArguments", 0) => {
            let method_obj = ctx.pop_obj();
            let (method, lookup) =
                dotnet_vm_ops::vm_try!(crate::common::resolve_runtime_method(ctx, method_obj));

            let generic_arity = method.method().generic_parameters.len();
            let type_type = dotnet_vm_ops::vm_try!(ctx.loader().corlib_wkt(WellKnown::Type));
            if generic_arity == 0 {
                return crate::types::populate_reflection_array(
                    ctx,
                    Vec::new(),
                    ConcreteType::from(type_type),
                );
            }

            let concrete_method_generics = lookup.method_generics.get(..generic_arity);
            let generic_args = (0..generic_arity)
                .map(|index| {
                    let runtime_type = concrete_method_generics
                        .and_then(|args| {
                            runtime_type_from_concrete(ctx.loader().as_ref(), &args[index])
                        })
                        .unwrap_or_else(|| RuntimeType::MethodParameter {
                            owner: method.clone(),
                            index: index as u16,
                        });
                    crate::common::get_runtime_type(ctx, runtime_type)
                })
                .collect();

            return crate::types::populate_reflection_array(
                ctx,
                generic_args,
                ConcreteType::from(type_type),
            );
        }
        ("GetGenericMethodDefinition", 0) => {
            let method_obj = ctx.pop_obj();
            let (method, lookup) =
                dotnet_vm_ops::vm_try!(crate::common::resolve_runtime_method(ctx, method_obj));

            if method.method().generic_parameters.is_empty() {
                return ctx.throw_by_name_with_message(
                    "System.InvalidOperationException",
                    "Method is not a generic method.",
                );
            }

            let definition_lookup =
                GenericLookup::from_arcs(lookup.type_generics, Vec::new().into());
            let method_obj = crate::common::get_runtime_method_obj(ctx, method, definition_lookup);
            ctx.push_obj(method_obj);
            Some(StepResult::Continue)
        }
        ("Invoke", 5) => {
            let _culture = ctx.pop();
            let parameters_obj = ctx.pop_obj();
            let _binder = ctx.pop();
            let _flags = ctx.pop_i32();
            let this_obj = ctx.pop();
            let method_obj = ctx.pop_obj();

            let (method, lookup) =
                dotnet_vm_ops::vm_try!(crate::common::resolve_runtime_method(ctx, method_obj));
            if matches!(
                method.signature().return_type.1,
                Some(ParameterType::Ref(_))
            ) {
                return StepResult::Error(
                    dotnet_types::error::ExecutionError::NotImplemented(
                        "reflection invocation with a byref return".into(),
                    )
                    .into(),
                );
            }
            let is_constructor = method.method().name == ".ctor";

            if let Err(thrown) = validate_invoke_target(ctx, &method, &lookup, &this_obj) {
                return thrown;
            }

            if is_constructor {
                ctx.push(this_obj.clone());
            }

            let mut args = Vec::new();
            if method.signature().instance {
                args.push(this_obj);
            }

            let mut invoke_args =
                match unmarshal_invoke_params(ctx, &gc, &method, &lookup, parameters_obj) {
                    Ok(a) => a,
                    Err(res) => return res,
                };
            args.append(&mut invoke_args);

            let return_type = if is_constructor {
                RuntimeType::Type(method.parent.clone())
            } else {
                resolve_return_type(ctx, &method, &lookup)
            };

            ctx.set_frame_return_action(FrameReturnAction::InvokeReturn(return_type));

            for arg in args {
                ctx.push(arg);
            }

            return ctx.reflection_dispatch_method(method, lookup);
        }
        ("Invoke", 4) => {
            let _culture = ctx.pop();
            let parameters_obj = ctx.pop_obj();
            let _binder = ctx.pop();
            let _flags = ctx.pop_i32();
            let method_obj = ctx.pop_obj();

            let (method, lookup) =
                dotnet_vm_ops::vm_try!(crate::common::resolve_runtime_method(ctx, method_obj));

            let instance = dotnet_vm_ops::vm_try!(
                ctx.reflection_new_object_with_lookup(method.parent.clone(), &lookup)
            );
            let this_obj = ObjectRef::new(gc, HeapStorage::Obj(Box::new(instance)));
            ctx.register_new_object(&this_obj);

            ctx.push_obj(this_obj);

            let mut args = Vec::new();
            args.push(StackValue::ObjectRef(this_obj));

            let mut invoke_args =
                match unmarshal_invoke_params(ctx, &gc, &method, &lookup, parameters_obj) {
                    Ok(a) => a,
                    Err(res) => return res,
                };
            args.append(&mut invoke_args);

            ctx.set_frame_return_action(FrameReturnAction::InvokeReturn(RuntimeType::Type(
                method.parent.clone(),
            )));

            for arg in args {
                ctx.push(arg);
            }

            return ctx.reflection_dispatch_method(method, lookup);
        }
        ("MakeGenericMethod", 1) => {
            let type_args_obj = ctx.pop_obj();
            let method_obj = ctx.pop_obj();
            let (method, mut lookup) =
                dotnet_vm_ops::vm_try!(crate::common::resolve_runtime_method(ctx, method_obj));

            let expected_arity = method.method().generic_parameters.len();
            if expected_arity == 0 {
                return ctx.throw_by_name_with_message(
                    "System.InvalidOperationException",
                    "Method is not a generic method definition.",
                );
            }

            let type_arg_refs = dotnet_vm_ops::vm_try!(type_args_obj.try_as_vector(
                |v: &dotnet_value::object::Vector<'gc>| {
                    v.object_ref_elements(&gc).collect::<Vec<_>>()
                }
            ));

            if type_arg_refs.len() != expected_arity {
                return ctx.throw_by_name_with_message(
                    "System.ArgumentException",
                    "Incorrect number of generic type arguments.",
                );
            }

            let mut method_generics = Vec::with_capacity(type_arg_refs.len());
            for type_arg_ref in type_arg_refs {
                let runtime_type =
                    dotnet_vm_ops::vm_try!(crate::common::resolve_runtime_type(ctx, type_arg_ref));
                method_generics.push(runtime_type.to_concrete(ctx.loader().as_ref()));
            }

            lookup.set_method_generics(method_generics.into());

            let method_obj = crate::common::get_runtime_method_obj(ctx, method, lookup);
            ctx.push_obj(method_obj);
            Some(StepResult::Continue)
        }
        ("CreateDelegate", 1) => {
            let delegate_type_obj = ctx.pop_obj();
            let method_obj = ctx.pop_obj();
            return create_method_info_delegate(
                ctx,
                method_obj,
                delegate_type_obj,
                ObjectRef(None),
            );
        }
        ("CreateDelegate", 2) => {
            let target_obj = ctx.pop_obj();
            let delegate_type_obj = ctx.pop_obj();
            let method_obj = ctx.pop_obj();
            return create_method_info_delegate(ctx, method_obj, delegate_type_obj, target_obj);
        }
        ("GetCustomAttributes", 1) => {
            let _inherit = ctx.pop_i32();
            let method_obj = ctx.pop_obj();
            let (method, _) =
                dotnet_vm_ops::vm_try!(crate::common::resolve_runtime_method(ctx, method_obj));
            let attrs = dotnet_vm_ops::vm_try!(crate::types::collect_method_custom_attributes(
                ctx, method, None
            ));
            let object_type = dotnet_vm_ops::vm_try!(ctx.loader().corlib_wkt(WellKnown::Object));
            return crate::types::populate_reflection_array(
                ctx,
                attrs,
                ConcreteType::from(object_type),
            );
        }
        ("GetCustomAttributes", 2) => {
            let _inherit = ctx.pop_i32();
            let attribute_type_obj = ctx.pop_obj();
            let method_obj = ctx.pop_obj();

            let attribute_filter = dotnet_vm_ops::vm_try!(crate::common::resolve_attribute_filter(
                ctx,
                attribute_type_obj
            ));

            let (method, _) =
                dotnet_vm_ops::vm_try!(crate::common::resolve_runtime_method(ctx, method_obj));
            let attrs = dotnet_vm_ops::vm_try!(crate::types::collect_method_custom_attributes(
                ctx,
                method,
                attribute_filter
            ));
            let object_type = dotnet_vm_ops::vm_try!(ctx.loader().corlib_wkt(WellKnown::Object));
            return crate::types::populate_reflection_array(
                ctx,
                attrs,
                ConcreteType::from(object_type),
            );
        }
        _ => None,
    };

    let _ = result.unwrap_or_else(|| {
        panic!(
            "unimplemented method info intrinsic: {}.{}({})",
            method.parent.type_name(),
            method_name,
            param_count
        )
    });
    StepResult::Continue
}

fn is_type_or_ancestor_named<'gc, T: ReflectionIntrinsicHost<'gc>>(
    ctx: &T,
    td: &TypeDescription,
    canonical_name: &str,
) -> bool {
    let is_named = |candidate: &TypeDescription| {
        let raw_type_name = candidate.type_name();
        ctx.loader().canonical_type_name(&raw_type_name) == canonical_name
    };

    is_named(td)
        || ctx
            .loader()
            .ancestors(td.clone())
            .any(|(ancestor, _)| is_named(&ancestor))
}

fn create_method_info_delegate<'gc, T: ReflectionIntrinsicHost<'gc>>(
    ctx: &mut T,
    method_obj: ObjectRef<'gc>,
    delegate_type_obj: ObjectRef<'gc>,
    target_obj: ObjectRef<'gc>,
) -> StepResult {
    let (method, lookup) =
        dotnet_vm_ops::vm_try!(crate::common::resolve_runtime_method(ctx, method_obj));

    let delegate_runtime_type =
        dotnet_vm_ops::vm_try!(crate::common::resolve_runtime_type(ctx, delegate_type_obj));
    let delegate_td = match &delegate_runtime_type {
        RuntimeType::Type(td) | RuntimeType::Generic(td, _) => td.clone(),
        _ => {
            return ctx.throw_by_name_with_message(
                "System.ArgumentException",
                "delegateType must be a RuntimeType representing a class type.",
            );
        }
    };

    let delegate_base = dotnet_vm_ops::vm_try!(ctx.loader().corlib_wkt(WellKnown::Delegate));
    if !is_type_or_ancestor_named(ctx, &delegate_td, "System.Delegate") {
        return ctx.throw_by_name_with_message(
            "System.ArgumentException",
            "Type must derive from System.Delegate.",
        );
    }

    let delegate_lookup =
        crate::types::build_generic_lookup_from_runtime_type(ctx, &delegate_runtime_type);
    let delegate_instance =
        dotnet_vm_ops::vm_try!(ctx.new_object_with_lookup(delegate_td.clone(), &delegate_lookup));
    let gc = ctx.gc_with_token(&ctx.no_active_borrows_token());
    let delegate_ref = ObjectRef::new(gc, HeapStorage::Obj(Box::new(delegate_instance)));
    ctx.register_new_object(&delegate_ref);

    let method_index = ctx.reflection_runtime_method_index_get_or_insert(method, lookup);
    set_delegate_target_method(ctx, delegate_ref, target_obj, method_index);

    if is_type_or_ancestor_named(ctx, &delegate_td, "System.MulticastDelegate") {
        let mut targets =
            dotnet_vm_ops::vm_try!(ctx.new_vector(ConcreteType::from(delegate_base), 1));
        targets.write_object_ref_elements_mut(&[delegate_ref]);
        let targets_ref = ObjectRef::new(gc, HeapStorage::Vec(Box::new(targets)));
        ctx.register_new_object(&targets_ref);

        set_delegate_multicast_targets(ctx, delegate_ref, targets_ref);
    }

    ctx.push_obj(delegate_ref);
    StepResult::Continue
}

#[dotnet_intrinsic(
    "static System.IntPtr System.RuntimeMethodHandle::GetFunctionPointer(System.RuntimeMethodHandle)"
)]
pub fn intrinsic_method_handle_get_function_pointer<'gc, T: ReflectionIntrinsicHost<'gc>>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let _gc = ctx.gc_with_token(&ctx.no_active_borrows_token());
    let handle = ctx.pop_value_type();
    let method_obj = ctx
        .loader()
        .rmh_value_field(&handle.instance_storage, handle.description)
        .expect("validated RuntimeMethodHandle support slot")
        .read();
    let (method, lookup) =
        dotnet_vm_ops::vm_try!(crate::common::resolve_runtime_method(ctx, method_obj));
    let index = crate::common::get_runtime_method_index(ctx, method, lookup);
    ctx.push_isize(index as isize);
    StepResult::Continue
}

fn trim_method_lookup(lookup: &GenericLookup, method: &MethodDescription) -> GenericLookup {
    let method_generic_arity = method.method().generic_parameters.len();
    if lookup.method_generics.len() <= method_generic_arity {
        return lookup.clone();
    }

    GenericLookup::from_arcs(
        lookup.type_generics.clone(),
        lookup.method_generics[..method_generic_arity].into(),
    )
}

fn resolve_base_method_definition<'gc, T: ReflectionIntrinsicHost<'gc>>(
    ctx: &T,
    method: &MethodDescription,
    lookup: &GenericLookup,
) -> Result<(MethodDescription, GenericLookup), TypeResolutionError> {
    let trimmed_lookup = trim_method_lookup(lookup, method);
    if !method.method().virtual_member {
        return Ok((method.clone(), trimmed_lookup));
    }

    if matches!(
        method.method().vtable_layout,
        dotnetdll::resolved::members::VtableLayout::NewSlot
    ) {
        return Ok((method.clone(), trimmed_lookup));
    }

    let mut signature_lookup = method.parent_generics.clone();
    signature_lookup.set_method_generics(trimmed_lookup.method_generics.clone());

    let ancestors: Vec<_> = ctx.loader().ancestors(method.parent.clone()).collect();
    if ancestors.len() <= 1 {
        return Ok((method.clone(), trimmed_lookup));
    }

    let mut base_method = method.clone();
    let mut base_lookup = trimmed_lookup.clone();
    let mut current_lookup = trimmed_lookup.clone();

    for index in 0..(ancestors.len() - 1) {
        let (current_type, extends_generics) = &ancestors[index];
        let (parent_type, _) = &ancestors[index + 1];

        let next_type_generics = extends_generics
            .iter()
            .map(|t| {
                current_lookup.make_concrete(
                    current_type.resolution.clone(),
                    (*t).clone(),
                    ctx.loader().as_ref(),
                )
            })
            .collect::<Result<Vec<_>, _>>()?;

        let parent_lookup = GenericLookup::from_arcs(
            next_type_generics.into(),
            trimmed_lookup.method_generics.clone(),
        );

        if let Some(parent_method) = ctx.loader().find_method_in_type_internal(
            parent_type.clone(),
            &method.method().name,
            method.signature(),
            method.resolution(),
            Some(&signature_lookup),
            Some(&parent_lookup),
            false,
        ) && parent_method.method().virtual_member
        {
            base_method = parent_method;
            base_lookup = parent_lookup.clone();
        }

        current_lookup = parent_lookup;
    }

    Ok((base_method, base_lookup))
}

fn method_attributes_flags(method: &MethodDescription) -> i32 {
    let method_def = method.method();
    let mut flags = method_def.accessibility.to_mask() as i32;

    if !method.signature().instance {
        flags |= 0x0010; // MethodAttributes.Static
    }
    if method_def.sealed {
        flags |= 0x0020; // MethodAttributes.Final
    }
    if method_def.virtual_member {
        flags |= 0x0040; // MethodAttributes.Virtual
    }
    if method_def.hide_by_sig {
        flags |= 0x0080; // MethodAttributes.HideBySig
    }
    if matches!(
        method_def.vtable_layout,
        dotnetdll::resolved::members::VtableLayout::NewSlot
    ) {
        flags |= 0x0100; // MethodAttributes.NewSlot
    }
    if method_def.strict {
        flags |= 0x0200; // MethodAttributes.CheckAccessOnOverride
    }
    if method_def.abstract_member {
        flags |= 0x0400; // MethodAttributes.Abstract
    }
    if method_def.special_name {
        flags |= 0x0800; // MethodAttributes.SpecialName
    }
    if method_def.runtime_special_name {
        flags |= 0x1000; // MethodAttributes.RTSpecialName
    }
    if method_def.pinvoke.is_some() {
        flags |= 0x2000; // MethodAttributes.PinvokeImpl
    }
    if method_def.security.is_some() {
        flags |= 0x4000; // MethodAttributes.HasSecurity
    }
    if method_def.require_sec_object {
        flags |= 0x8000; // MethodAttributes.RequireSecObject
    }

    flags
}

fn resolve_return_type<'gc, T: ReflectionIntrinsicHost<'gc>>(
    ctx: &T,
    method: &MethodDescription,
    lookup: &GenericLookup,
) -> RuntimeType {
    match &method.signature().return_type.1 {
        Some(ParameterType::Value(t)) | Some(ParameterType::Ref(t)) => {
            ctx.reflection_make_runtime_type_for_method(method.clone(), lookup, t)
        }
        Some(ParameterType::TypedReference) => RuntimeType::TypedReference,
        None => RuntimeType::Void,
    }
}

fn declaring_runtime_type<'gc, T: ReflectionIntrinsicHost<'gc>>(
    ctx: &T,
    method: &MethodDescription,
    lookup: &GenericLookup,
) -> RuntimeType {
    let parent_runtime_type = || {
        let concrete = ConcreteType::from(method.parent.clone());
        runtime_type_from_concrete(ctx.loader().as_ref(), &concrete)
            .unwrap_or_else(|| RuntimeType::Type(method.parent.clone()))
    };

    let parent_arity = method.parent.definition().generic_parameters.len();
    if parent_arity == 0 {
        return parent_runtime_type();
    }

    let type_generics = if lookup.type_generics.len() >= parent_arity {
        &lookup.type_generics[..parent_arity]
    } else if method.parent_generics.type_generics.len() >= parent_arity {
        &method.parent_generics.type_generics[..parent_arity]
    } else {
        return parent_runtime_type();
    };

    let Some(args) = type_generics
        .iter()
        .map(|t| runtime_type_from_concrete(ctx.loader().as_ref(), t))
        .collect::<Option<Vec<_>>>()
    else {
        return parent_runtime_type();
    };

    RuntimeType::Generic(method.parent.clone(), args)
}

#[derive(Clone, Copy)]
struct InvokeByRefSlot<'gc> {
    owner: ObjectRef<'gc>,
    offset: ByteOffset,
}

struct UnboxedInvokeArg<'gc> {
    stack_value: StackValue<'gc>,
    replacement_box: Option<ObjectRef<'gc>>,
}

fn argument_mismatch<'gc, T: ReflectionIntrinsicHost<'gc>>(
    ctx: &mut T,
    arg: ObjectRef<'gc>,
    shape: &ParamShape,
) -> StepResult {
    let actual = if arg.0.is_none() {
        "null".to_string()
    } else {
        ctx.get_heap_description(arg)
            .map(|td| td.type_name())
            .unwrap_or_else(|_| "object".to_string())
    };
    ctx.throw_by_name_with_message(
        "System.ArgumentException",
        &format!(
            "Object of type '{actual}' cannot be converted to type '{}'.",
            shape.td.type_name()
        ),
    )
}

fn unbox_param_to_stack_value<'gc, T: ReflectionIntrinsicHost<'gc>>(
    ctx: &mut T,
    arg: ObjectRef<'gc>,
    method: &MethodDescription,
    param_type: &ParameterType<dotnetdll::prelude::MethodType>,
    lookup: &GenericLookup,
    byref_slot: Option<InvokeByRefSlot<'gc>>,
) -> Result<UnboxedInvokeArg<'gc>, StepResult> {
    match param_type {
        ParameterType::TypedReference => {
            if arg.0.is_none() {
                return Err(type_mismatch("boxed System.TypedReference", "null"));
            }

            let val = match arg.as_heap_storage(|s| {
                if let HeapStorage::Boxed(o) = s {
                    let tr_type = ctx
                        .loader()
                        .corlib_wkt(WellKnown::TypedReference)
                        .expect("System.TypedReference must exist");
                    Ok(o.instance_storage
                        .with_data(|data| ctx.read_cts_value(&tr_type.into(), data)))
                } else {
                    Err(format!("{s:?}"))
                }
            }) {
                Ok(v) => v,
                Err(actual) => {
                    return Err(type_mismatch("boxed System.TypedReference", actual));
                }
            };

            match val {
                Ok(v) => Ok(UnboxedInvokeArg {
                    stack_value: v.into_stack(),
                    replacement_box: None,
                }),
                Err(e) => Err(StepResult::Error(e.into())),
            }
        }
        ParameterType::Value(t) => {
            let shape =
                param_shape(ctx, method, lookup, t).map_err(|e| StepResult::Error(e.into()))?;
            let stack_value = match plan_argument(ctx, arg, &shape, false) {
                ArgPlan::Reject => return Err(argument_mismatch(ctx, arg, &shape)),
                ArgPlan::Reference => StackValue::ObjectRef(arg),
                ArgPlan::Exact => {
                    let value = arg.as_heap_storage(|s| match s {
                        HeapStorage::Boxed(o) => Some(
                            o.instance_storage
                                .with_data(|data| ctx.read_cts_value(&shape.concrete, data)),
                        ),
                        _ => None,
                    });
                    match value {
                        Some(Ok(v)) => v.into_stack(),
                        Some(Err(e)) => return Err(StepResult::Error(e.into())),
                        None => return Err(argument_mismatch(ctx, arg, &shape)),
                    }
                }
                ArgPlan::Widen { from, to } => match widen_boxed(arg, from, to) {
                    Some(value) => value,
                    None => return Err(argument_mismatch(ctx, arg, &shape)),
                },
                ArgPlan::NullableSome => nullable_with_value(ctx, &shape, arg)?,
                ArgPlan::Default => {
                    let instance = ctx
                        .reflection_new_object_with_lookup(
                            shape.td.clone(),
                            &shape.concrete.make_lookup(),
                        )
                        .map_err(|e| StepResult::Error(e.into()))?;
                    match instance
                        .instance_storage
                        .with_data(|data| ctx.read_cts_value(&shape.concrete, data))
                    {
                        Ok(v) => v.into_stack(),
                        Err(e) => return Err(StepResult::Error(e.into())),
                    }
                }
            };
            Ok(UnboxedInvokeArg {
                stack_value,
                replacement_box: None,
            })
        }
        ParameterType::Ref(t) => {
            let shape =
                param_shape(ctx, method, lookup, t).map_err(|e| StepResult::Error(e.into()))?;
            if plan_argument(ctx, arg, &shape, true) == ArgPlan::Reject {
                return Err(argument_mismatch(ctx, arg, &shape));
            }
            let ParamShape {
                concrete: concrete_param_type,
                td,
                value_type: is_value_type,
            } = shape.clone();

            if is_value_type {
                let (arg_obj, replacement_box) = if arg.0.is_some() {
                    (arg, None)
                } else {
                    let boxed_lookup = concrete_param_type.make_lookup();
                    let boxed =
                        match ctx.reflection_new_object_with_lookup(td.clone(), &boxed_lookup) {
                            Ok(v) => v,
                            Err(e) => return Err(StepResult::Error(e.into())),
                        };
                    let token = ctx.no_active_borrows_token();
                    let gc = ctx.gc_with_token(&token);
                    let obj = ObjectRef::new(gc, HeapStorage::Boxed(Box::new(boxed)));
                    ctx.register_new_object(&obj);
                    (obj, Some(obj))
                };

                // The plan already proved a non-null argument is a box of exactly this type.
                let data_ptr = match arg_obj.as_heap_storage(|s| {
                    if let HeapStorage::Boxed(o) = s {
                        Ok(o.instance_storage
                            .with_data(|data| data.as_ptr() as *mut u8))
                    } else {
                        Err(format!("{s:?}"))
                    }
                }) {
                    Ok(v) => v,
                    Err(_actual) => {
                        return Err(argument_mismatch(ctx, arg, &shape));
                    }
                };

                Ok(UnboxedInvokeArg {
                    stack_value: StackValue::managed_ptr_with_owner(
                        data_ptr,
                        td,
                        Some(arg_obj),
                        false,
                        None,
                    ),
                    replacement_box,
                })
            } else {
                let slot = byref_slot.expect(
                    "MethodInfo.Invoke by-ref reference args require object[] slot backing",
                );
                let element_ptr = slot.owner.as_heap_storage(|s| match s {
                    HeapStorage::Vec(v) => {
                        Ok(v.get().as_ptr().wrapping_add(slot.offset.as_usize()) as *mut u8)
                    }
                    other => Err(format!("{other:?}")),
                });

                let data_ptr = match element_ptr {
                    Ok(ptr) => ptr,
                    Err(actual) => {
                        return Err(type_mismatch("object[]", actual));
                    }
                };

                // Reference-type by-ref parameters need writable slot backing so assignments like
                // `ref SomeClass x; x = newObj;` update the original object[] argument array.
                Ok(UnboxedInvokeArg {
                    stack_value: StackValue::managed_ptr_with_owner(
                        data_ptr,
                        td,
                        Some(slot.owner),
                        false,
                        Some(slot.offset),
                    ),
                    replacement_box: None,
                })
            }
        }
    }
}

fn parameter_count_mismatch<'gc, T: ReflectionIntrinsicHost<'gc>>(ctx: &mut T) -> StepResult {
    ctx.throw_by_name_with_message(
        "System.Reflection.TargetParameterCountException",
        "Parameter count mismatch.",
    )
}

/// Throw `TargetException` unless `receiver` is a valid `this` for `method`.
///
/// Static methods ignore the receiver, and a constructor without a receiver allocates its own.
pub fn validate_invoke_target<'gc, T: ReflectionIntrinsicHost<'gc>>(
    ctx: &mut T,
    method: &MethodDescription,
    lookup: &GenericLookup,
    receiver: &StackValue<'gc>,
) -> Result<(), StepResult> {
    if !method.signature().instance {
        return Ok(());
    }
    let receiver = match receiver {
        StackValue::ObjectRef(receiver) => *receiver,
        _ => ObjectRef(None),
    };
    if receiver.0.is_none() {
        if method.method().name == ".ctor" {
            return Ok(());
        }
        return Err(ctx.throw_by_name_with_message(
            "System.Reflection.TargetException",
            "Non-static method requires a target.",
        ));
    }
    if receiver_fits(ctx, method, lookup, receiver) {
        Ok(())
    } else {
        Err(ctx.throw_by_name_with_message(
            "System.Reflection.TargetException",
            "Object does not match target type.",
        ))
    }
}

/// Marshal argument slots within the current mutation. The argument array may have changed since
/// selection, so every slot is classified again.
pub fn unmarshal_invoke_params<'gc, T: ReflectionIntrinsicHost<'gc>>(
    ctx: &mut T,
    gc: &GCHandle<'gc>,
    method: &MethodDescription,
    lookup: &GenericLookup,
    parameters_obj: ObjectRef<'gc>,
) -> Result<Vec<StackValue<'gc>>, StepResult> {
    let mut args = Vec::new();
    let expected = method.signature().parameters.len();

    if parameters_obj.0.is_none() {
        return if expected == 0 {
            Ok(args)
        } else {
            Err(parameter_count_mismatch(ctx))
        };
    }

    let vector = match parameters_obj.as_heap_storage(|s| match s {
        HeapStorage::Vec(v) => Ok(v.clone()),
        other => Err(format!("{other:?}")),
    }) {
        Ok(v) => v,
        Err(actual) => {
            return Err(type_mismatch("object[]", actual));
        }
    };
    if vector.layout.length != expected {
        return Err(parameter_count_mismatch(ctx));
    }

    let mut replacement_boxes = Vec::new();
    let mut elements = vector.object_ref_elements(gc);
    let element_size = vector.layout.element_layout.size();
    for i in 0..vector.layout.length {
        let arg_obj = elements
            .next()
            .expect("invoke parameters storage must match vector length");
        let param_type = &method.signature().parameters[i].1;
        let byref_slot = InvokeByRefSlot {
            owner: parameters_obj,
            offset: element_size * i,
        };
        let arg =
            unbox_param_to_stack_value(ctx, arg_obj, method, param_type, lookup, Some(byref_slot))?;

        if let Some(owner) = arg.replacement_box {
            replacement_boxes.push((i, owner));
        }

        args.push(arg.stack_value);
    }

    if !replacement_boxes.is_empty() {
        parameters_obj.as_vector_mut(*gc, |vector| {
            let mut elements = vector.object_ref_elements(gc).collect::<Vec<_>>();
            for (index, owner) in replacement_boxes {
                elements[index] = owner;
            }
            vector.write_object_ref_elements_mut(&elements);
        });
    }

    Ok(args)
}

#[cfg(test)]
mod invoke_selection_tests {
    use super::*;
    use dotnet_value::{ManagedPtr, pointer::FrameHomeSerial};

    #[test]
    fn operand_window_rejects_managed_pointer_origins_without_consuming_values() {
        let mut storage = [0_u8; 8];
        let address = std::ptr::NonNull::from(&mut storage[0]);
        let frame = ManagedPtr::new_frame_home(
            address,
            TypeDescription::NULL,
            FrameHomeSerial::new(1),
            ByteOffset::new(0),
            false,
        );
        let static_pointer = ManagedPtr::new_static(
            Some(address),
            TypeDescription::NULL,
            TypeDescription::NULL,
            Default::default(),
            false,
            ByteOffset::new(0),
        );
        let unmanaged = ManagedPtr::new(Some(address), TypeDescription::NULL, None, false, None);
        for pointer in [frame, static_pointer, unmanaged] {
            for index in [0, 1, 3, 4, 5] {
                let mut values = [
                    StackValue::null(),
                    StackValue::null(),
                    StackValue::Int32(0),
                    StackValue::null(),
                    StackValue::null(),
                    StackValue::null(),
                ];
                values[index] = StackValue::ManagedPtr(pointer.clone().into());
                assert!(!invoke_operand_window_supported(&values));
                let StackValue::ManagedPtr(retained) = &values[index] else {
                    unreachable!()
                };
                assert_eq!(retained.origin(), pointer.origin());
            }
        }
    }
}

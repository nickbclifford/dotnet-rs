use crate::ReflectionIntrinsicHost;
use dotnet_macros::dotnet_intrinsic;
use dotnet_types::{
    error::{ExecutionError, VmError},
    generics::{ConcreteType, GenericLookup},
    members::MethodDescription,
    runtime::RuntimeType,
};
use dotnet_utils::gc::GCHandle;
use dotnet_value::{
    StackValue,
    layout::HasLayout,
    object::{CTSValue, HeapStorage, ObjectRef},
};
use dotnet_vm_data::StepResult;
use dotnetdll::prelude::ParameterType;

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
#[dotnet_intrinsic(
    "object DotnetRs.MethodInfo::Invoke(object, System.Reflection.BindingFlags, System.Reflection.Binder, object[], System.Globalization.CultureInfo)"
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
    "object DotnetRs.ConstructorInfo::Invoke(object, System.Reflection.BindingFlags, System.Reflection.Binder, object[], System.Globalization.CultureInfo)"
)]
#[dotnet_intrinsic(
    "object DotnetRs.ConstructorInfo::Invoke(System.Reflection.BindingFlags, System.Reflection.Binder, object[], System.Globalization.CultureInfo)"
)]
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
        ("IsDefined", 2) => {
            let _inherit = ctx.pop_i32();
            let attribute_type_obj = ctx.pop_obj();
            let method_obj = ctx.pop_obj();
            let attribute_filter = if attribute_type_obj.0.is_some() {
                let filter_rt = dotnet_vm_ops::vm_try!(
                    crate::common::resolve_runtime_type(ctx, attribute_type_obj)
                );
                match filter_rt {
                    RuntimeType::Type(td) | RuntimeType::Generic(td, _) => Some(td),
                    _ => None,
                }
            } else {
                None
            };
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
            let (method, _) =
                dotnet_vm_ops::vm_try!(crate::common::resolve_runtime_method(ctx, obj));
            let rt_obj = crate::common::get_runtime_type(ctx, RuntimeType::Type(method.parent));
            ctx.push_obj(rt_obj);
            Some(StepResult::Continue)
        }
        ("GetMethodHandle" | "get_MethodHandle", 0) => {
            let obj = ctx.pop_obj();

            let rmh = ctx
                .loader()
                .corlib_type("System.RuntimeMethodHandle")
                .expect("System.RuntimeMethodHandle must exist");
            let instance = dotnet_vm_ops::vm_try!(ctx.new_object(rmh.clone()));
            instance
                .instance_storage
                .field::<ObjectRef<'gc>>(rmh, "_value")
                .unwrap()
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
                .corlib_type("DotnetRs.ParameterInfo")
                .expect("DotnetRs.ParameterInfo not found");

            let mut pi_objs = Vec::with_capacity(param_count);
            for i in 0..param_count {
                let pi_obj = dotnet_vm_ops::vm_try!(ctx.new_object(pi_type.clone()));
                let pi_ref = ObjectRef::new(gc, HeapStorage::Obj(Box::new(pi_obj)));
                ctx.register_new_object(&pi_ref);
                let pi_type_inner = pi_type.clone();
                pi_ref.as_object_mut(gc, |instance| {
                    instance
                        .instance_storage
                        .field::<usize>(pi_type_inner.clone(), "method_index")
                        .unwrap()
                        .write(method_index);
                    instance
                        .instance_storage
                        .field::<i32>(pi_type_inner, "position")
                        .unwrap()
                        .write(i as i32);
                });
                pi_objs.push(pi_ref);
            }

            let array_element_type = ctx
                .loader()
                .corlib_type("System.Reflection.ParameterInfo")
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
        ("Invoke", 5) => {
            let _culture = ctx.pop();
            let parameters_obj = ctx.pop_obj();
            let _binder = ctx.pop();
            let _flags = ctx.pop_i32();
            let this_obj = ctx.pop();
            let method_obj = ctx.pop_obj();

            let (method, lookup) =
                dotnet_vm_ops::vm_try!(crate::common::resolve_runtime_method(ctx, method_obj));
            let is_constructor = method.method().name == ".ctor";

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

            ctx.frame_stack_mut()
                .current_frame_mut()
                .awaiting_invoke_return = Some(return_type);

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

            let instance = dotnet_vm_ops::vm_try!(ctx.new_object(method.parent.clone()));
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

            ctx.frame_stack_mut()
                .current_frame_mut()
                .awaiting_invoke_return = Some(RuntimeType::Type(method.parent.clone()));

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
                    v.get()
                        .chunks_exact(ObjectRef::SIZE)
                        .map(|chunk| unsafe { ObjectRef::read_branded(chunk, &gc) })
                        .collect::<Vec<_>>()
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

            lookup.method_generics = method_generics.into();

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
            let object_type = dotnet_vm_ops::vm_try!(ctx.loader().corlib_type("System.Object"));
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

            let attribute_filter = if attribute_type_obj.0.is_some() {
                let filter_runtime_type = dotnet_vm_ops::vm_try!(
                    crate::common::resolve_runtime_type(ctx, attribute_type_obj)
                );
                match filter_runtime_type {
                    RuntimeType::Type(td) | RuntimeType::Generic(td, _) => Some(td),
                    _ => None,
                }
            } else {
                None
            };

            let (method, _) =
                dotnet_vm_ops::vm_try!(crate::common::resolve_runtime_method(ctx, method_obj));
            let attrs = dotnet_vm_ops::vm_try!(crate::types::collect_method_custom_attributes(
                ctx,
                method,
                attribute_filter
            ));
            let object_type = dotnet_vm_ops::vm_try!(ctx.loader().corlib_type("System.Object"));
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

    let delegate_base = dotnet_vm_ops::vm_try!(ctx.loader().corlib_type("System.Delegate"));
    let delegate_concrete = ConcreteType::from(delegate_td.clone());
    let delegate_base_concrete = ConcreteType::from(delegate_base.clone());
    if !dotnet_vm_ops::vm_try!(ctx.is_a(delegate_concrete, delegate_base_concrete)) {
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

    delegate_ref.as_object_mut(gc, |instance| {
        instance
            .instance_storage
            .field::<ObjectRef<'gc>>(delegate_base.clone(), "_target")
            .unwrap()
            .write(target_obj);
        instance
            .instance_storage
            .field::<usize>(delegate_base.clone(), "_method")
            .unwrap()
            .write(method_index);
    });

    let multicast_type =
        dotnet_vm_ops::vm_try!(ctx.loader().corlib_type("System.MulticastDelegate"));
    if dotnet_vm_ops::vm_try!(ctx.is_a(
        ConcreteType::from(delegate_td),
        ConcreteType::from(multicast_type.clone()),
    )) {
        let mut targets =
            dotnet_vm_ops::vm_try!(ctx.new_vector(ConcreteType::from(delegate_base), 1));
        delegate_ref.write(&mut targets.get_mut()[..ObjectRef::SIZE]);
        let targets_ref = ObjectRef::new(gc, HeapStorage::Vec(Box::new(targets)));
        ctx.register_new_object(&targets_ref);

        delegate_ref.as_object_mut(gc, |instance| {
            instance
                .instance_storage
                .field::<ObjectRef<'gc>>(multicast_type.clone(), "targets")
                .unwrap()
                .write(targets_ref);
        });
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
    let method_obj = handle
        .instance_storage
        .field::<ObjectRef<'gc>>(handle.description, "_value")
        .unwrap()
        .read();
    let (method, lookup) =
        dotnet_vm_ops::vm_try!(crate::common::resolve_runtime_method(ctx, method_obj));
    let index = crate::common::get_runtime_method_index(ctx, method, lookup);
    ctx.push_isize(index as isize);
    StepResult::Continue
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

fn unbox_param_to_stack_value<'gc, T: ReflectionIntrinsicHost<'gc>>(
    ctx: &mut T,
    arg: ObjectRef<'gc>,
    param_type: &ParameterType<dotnetdll::prelude::MethodType>,
    lookup: &GenericLookup,
    _param_index: usize,
) -> Result<StackValue<'gc>, StepResult> {
    match param_type {
        ParameterType::TypedReference => {
            if arg.0.is_none() {
                return Err(StepResult::Error(VmError::Execution(
                    ExecutionError::TypeMismatch {
                        expected: "boxed System.TypedReference",
                        actual: "null".into(),
                    },
                )));
            }

            let val = match arg.as_heap_storage(|s| {
                if let HeapStorage::Boxed(o) = s {
                    let tr_type = ctx
                        .loader()
                        .corlib_type("System.TypedReference")
                        .expect("System.TypedReference must exist");
                    Ok(o.instance_storage
                        .with_data(|data| ctx.read_cts_value(&tr_type.into(), data)))
                } else {
                    Err(format!("{s:?}"))
                }
            }) {
                Ok(v) => v,
                Err(actual) => {
                    return Err(StepResult::Error(VmError::Execution(
                        ExecutionError::TypeMismatch {
                            expected: "boxed System.TypedReference",
                            actual: actual.into(),
                        },
                    )));
                }
            };

            match val {
                Ok(v) => Ok(v.into_stack()),
                Err(e) => Err(StepResult::Error(e.into())),
            }
        }
        ParameterType::Value(t) | ParameterType::Ref(t) => {
            let concrete_param_type = match ctx.make_concrete(t) {
                Ok(v) => v,
                Err(e) => return Err(StepResult::Error(e.into())),
            };
            let td = ctx
                .loader()
                .find_concrete_type(concrete_param_type.clone())
                .expect("Parameter type must exist for MethodInfo.Invoke");

            let is_value_type = match ctx.reflection_is_value_type_with_lookup(td, lookup) {
                Ok(v) => v,
                Err(e) => return Err(StepResult::Error(e.into())),
            };

            if is_value_type {
                if arg.0.is_none() {
                    Ok(StackValue::null())
                } else {
                    let val = match arg.as_heap_storage(|s| {
                        if let HeapStorage::Boxed(o) = s {
                            Ok(o.instance_storage
                                .with_data(|data| ctx.read_cts_value(&concrete_param_type, data)))
                        } else {
                            Err(format!("{s:?}"))
                        }
                    }) {
                        Ok(v) => v,
                        Err(actual) => {
                            return Err(StepResult::Error(VmError::Execution(
                                ExecutionError::TypeMismatch {
                                    expected: "boxed value",
                                    actual: actual.into(),
                                },
                            )));
                        }
                    };

                    match val {
                        Ok(v) => Ok(v.into_stack()),
                        Err(e) => Err(StepResult::Error(e.into())),
                    }
                }
            } else {
                Ok(StackValue::ObjectRef(arg))
            }
        }
    }
}

fn unmarshal_invoke_params<'gc, T: ReflectionIntrinsicHost<'gc>>(
    ctx: &mut T,
    gc: &GCHandle<'gc>,
    method: &MethodDescription,
    lookup: &GenericLookup,
    parameters_obj: ObjectRef<'gc>,
) -> Result<Vec<StackValue<'gc>>, StepResult> {
    let mut args = Vec::new();

    if parameters_obj.0.is_some() {
        let vector = match parameters_obj.as_heap_storage(|s| match s {
            HeapStorage::Vec(v) => Ok(v.clone()),
            other => Err(format!("{other:?}")),
        }) {
            Ok(v) => v,
            Err(actual) => {
                return Err(StepResult::Error(VmError::Execution(
                    ExecutionError::TypeMismatch {
                        expected: "object[]",
                        actual: actual.into(),
                    },
                )));
            }
        };

        for i in 0..vector.layout.length {
            let mut element_bytes = [0u8; ObjectRef::SIZE];
            element_bytes
                .copy_from_slice(&vector.get()[i * ObjectRef::SIZE..(i + 1) * ObjectRef::SIZE]);
            let arg_obj = unsafe { ObjectRef::read_branded(&element_bytes, gc) };
            let param_type = &method.signature().parameters[i].1;
            let arg = unbox_param_to_stack_value(ctx, arg_obj, param_type, lookup, i)?;
            args.push(arg);
        }
    }

    Ok(args)
}

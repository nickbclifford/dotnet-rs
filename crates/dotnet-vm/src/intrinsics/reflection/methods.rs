use crate::{StepResult, resolution::TypeResolutionExt, stack::ops::VesOps};
use dotnet_macros::dotnet_intrinsic;
use dotnet_types::{generics::GenericLookup, members::MethodDescription, runtime::RuntimeType};
use dotnet_value::{StackValue, object::ObjectRef};

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
pub fn runtime_method_info_intrinsic_call<'gc, 'm: 'gc>(
    ctx: &mut dyn VesOps<'gc, 'm>,
    method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let gc = ctx.gc();
    let method_name = &*method.method.name;
    let param_count = method.method.signature.parameters.len();

    let result = match (method_name, param_count) {
        ("GetName" | "get_Name", 0) => {
            let obj = ctx.pop_obj();
            let (method, _) = ctx.resolve_runtime_method(obj);
            ctx.push_string(method.method.name.clone().into());
            Some(StepResult::Continue)
        }
        ("GetDeclaringType" | "get_DeclaringType", 0) => {
            let obj = ctx.pop_obj();
            let (method, _) = ctx.resolve_runtime_method(obj);
            let rt_obj = ctx.get_runtime_type(RuntimeType::Type(method.parent));
            ctx.push_obj(rt_obj);
            Some(StepResult::Continue)
        }
        ("GetMethodHandle" | "get_MethodHandle", 0) => {
            let obj = ctx.pop_obj();

            let rmh = ctx
                .loader()
                .corlib_type("System.RuntimeMethodHandle")
                .expect("System.RuntimeMethodHandle must exist");
            let instance = vm_try!(ctx.new_object(rmh));
            obj.write(&mut instance.instance_storage.get_field_mut_local(rmh, "_value"));

            ctx.push(StackValue::ValueType(instance));
            Some(StepResult::Continue)
        }
        ("get_ReturnType" | "GetReturnType", 0) => {
            let obj = ctx.pop_obj();
            let (method, lookup) = ctx.resolve_runtime_method(obj);
            let res_ctx = ctx.with_generics(&lookup);
            let rt = match &method.method.signature.return_type.1 {
                Some(dotnetdll::prelude::ParameterType::Value(t))
                | Some(dotnetdll::prelude::ParameterType::Ref(t)) => {
                    ctx.make_runtime_type(&res_ctx, t)
                }
                Some(dotnetdll::prelude::ParameterType::TypedReference) => {
                    RuntimeType::TypedReference
                }
                None => RuntimeType::Void,
            };
            let rt_obj = ctx.get_runtime_type(rt);
            ctx.push_obj(rt_obj);
            Some(StepResult::Continue)
        }
        ("Invoke", 5) => {
            let _culture = ctx.pop(); // CultureInfo
            let parameters_obj = ctx.pop_obj(); // object[]
            let _binder = ctx.pop(); // Binder
            let _flags = ctx.pop_i32(); // BindingFlags
            let this_obj = ctx.pop(); // object (can be null for static)
            let method_obj = ctx.pop_obj(); // MethodInfo/ConstructorInfo

            let (method, lookup) = ctx.resolve_runtime_method(method_obj);

            // Handle parameters
            let mut args = Vec::new();
            if method.method.signature.instance {
                args.push(this_obj);
            }

            if parameters_obj.0.is_some() {
                let vector = parameters_obj.as_heap_storage(|s| {
                    if let dotnet_value::object::HeapStorage::Vec(v) = s {
                        v.clone()
                    } else {
                        panic!("parameters is not an array")
                    }
                });

                for i in 0..vector.layout.length {
                    let mut element_bytes = [0u8; ObjectRef::SIZE];
                    element_bytes.copy_from_slice(
                        &vector.get()[i * ObjectRef::SIZE..(i + 1) * ObjectRef::SIZE],
                    );
                    let arg_obj = unsafe { ObjectRef::read_branded(&element_bytes, &gc) };

                    let param_type = match &method.method.signature.parameters[i].1 {
                        dotnetdll::prelude::ParameterType::Value(t)
                        | dotnetdll::prelude::ParameterType::Ref(t) => t,
                        dotnetdll::prelude::ParameterType::TypedReference => {
                            if arg_obj.0.is_none() {
                                panic!("TypedReference parameter cannot be null");
                            }
                            let val = arg_obj.as_heap_storage(|s| {
                                if let dotnet_value::object::HeapStorage::Boxed(o) = s {
                                    let tr_type = ctx
                                        .loader()
                                        .corlib_type("System.TypedReference")
                                        .expect("System.TypedReference must exist");
                                    ctx.read_cts_value(&tr_type.into(), &o.instance_storage.get())
                                } else {
                                    panic!("Expected boxed TypedReference for parameter {}", i)
                                }
                            });
                            args.push(vm_try!(val).into_stack());
                            continue;
                        }
                    };
                    let res_ctx = ctx.with_generics(&lookup);
                    let concrete_param_type = vm_try!(ctx.make_concrete(param_type));
                    let td = ctx
                        .loader()
                        .find_concrete_type(concrete_param_type.clone())
                        .expect("Parameter type must exist for MethodInfo.Invoke");

                    if vm_try!(td.is_value_type(&res_ctx)) {
                        if arg_obj.0.is_none() {
                            args.push(StackValue::null());
                        } else {
                            let val = arg_obj.as_heap_storage(|s| {
                                if let dotnet_value::object::HeapStorage::Boxed(o) = s {
                                    ctx.read_cts_value(
                                        &concrete_param_type,
                                        &o.instance_storage.get(),
                                    )
                                } else {
                                    panic!("Expected boxed value for parameter {}", i)
                                }
                            });
                            args.push(vm_try!(val).into_stack());
                        }
                    } else {
                        args.push(StackValue::ObjectRef(arg_obj));
                    }
                }
            }

            let res_ctx = ctx.with_generics(&lookup);
            let return_type = match &method.method.signature.return_type.1 {
                Some(dotnetdll::prelude::ParameterType::Value(t))
                | Some(dotnetdll::prelude::ParameterType::Ref(t)) => {
                    ctx.make_runtime_type(&res_ctx, t)
                }
                Some(dotnetdll::prelude::ParameterType::TypedReference) => {
                    RuntimeType::TypedReference
                }
                None => RuntimeType::Void,
            };
            ctx.frame_stack_mut()
                .current_frame_mut()
                .awaiting_invoke_return = Some(return_type);

            for arg in args {
                ctx.push(arg);
            }

            return ctx.dispatch_method(method, lookup);
        }
        ("Invoke", 4) => {
            let _culture = ctx.pop(); // CultureInfo
            let parameters_obj = ctx.pop_obj(); // object[]
            let _binder = ctx.pop(); // Binder
            let _flags = ctx.pop_i32(); // BindingFlags
            let method_obj = ctx.pop_obj(); // ConstructorInfo

            let (method, lookup) = ctx.resolve_runtime_method(method_obj);

            // For ConstructorInfo.Invoke(parameters), we need to create the instance
            let instance = vm_try!(ctx.new_object(method.parent));
            let this_obj = ObjectRef::new(gc, dotnet_value::object::HeapStorage::Obj(instance));
            ctx.register_new_object(&this_obj);

            let mut args = Vec::new();
            args.push(StackValue::ObjectRef(this_obj));

            if parameters_obj.0.is_some() {
                let vector = parameters_obj.as_heap_storage(|s| {
                    if let dotnet_value::object::HeapStorage::Vec(v) = s {
                        v.clone()
                    } else {
                        panic!("parameters is not an array")
                    }
                });

                for i in 0..vector.layout.length {
                    let mut element_bytes = [0u8; ObjectRef::SIZE];
                    element_bytes.copy_from_slice(
                        &vector.get()[i * ObjectRef::SIZE..(i + 1) * ObjectRef::SIZE],
                    );
                    let arg_obj = unsafe { ObjectRef::read_branded(&element_bytes, &gc) };

                    let param_type = match &method.method.signature.parameters[i].1 {
                        dotnetdll::prelude::ParameterType::Value(t)
                        | dotnetdll::prelude::ParameterType::Ref(t) => t,
                        dotnetdll::prelude::ParameterType::TypedReference => {
                            if arg_obj.0.is_none() {
                                panic!("TypedReference parameter cannot be null");
                            }
                            let val = arg_obj.as_heap_storage(|s| {
                                if let dotnet_value::object::HeapStorage::Boxed(o) = s {
                                    let tr_type = ctx
                                        .loader()
                                        .corlib_type("System.TypedReference")
                                        .expect("System.TypedReference must exist");
                                    ctx.read_cts_value(&tr_type.into(), &o.instance_storage.get())
                                } else {
                                    panic!("Expected boxed TypedReference for parameter {}", i)
                                }
                            });
                            args.push(vm_try!(val).into_stack());
                            continue;
                        }
                    };
                    let res_ctx = ctx.with_generics(&lookup);
                    let concrete_param_type = vm_try!(ctx.make_concrete(param_type));
                    let td = ctx
                        .loader()
                        .find_concrete_type(concrete_param_type.clone())
                        .expect("Parameter type must exist for ConstructorInfo.Invoke");

                    if vm_try!(td.is_value_type(&res_ctx)) {
                        if arg_obj.0.is_none() {
                            args.push(StackValue::null());
                        } else {
                            let val = arg_obj.as_heap_storage(|s| {
                                if let dotnet_value::object::HeapStorage::Boxed(o) = s {
                                    ctx.read_cts_value(
                                        &concrete_param_type,
                                        &o.instance_storage.get(),
                                    )
                                } else {
                                    panic!("Expected boxed value for parameter {}", i)
                                }
                            });
                            args.push(vm_try!(val).into_stack());
                        }
                    } else {
                        args.push(StackValue::ObjectRef(arg_obj));
                    }
                }
            }

            ctx.frame_stack_mut()
                .current_frame_mut()
                .awaiting_invoke_return = Some(RuntimeType::Type(method.parent));

            for arg in args {
                ctx.push(arg);
            }

            return ctx.dispatch_method(method, lookup);
        }
        _ => None,
    };

    let _ = result.expect("unimplemented method info intrinsic");
    StepResult::Continue
}

#[dotnet_intrinsic(
    "static System.IntPtr System.RuntimeMethodHandle::GetFunctionPointer(System.RuntimeMethodHandle)"
)]
#[dotnet_intrinsic(
    "static System.IntPtr DotnetRs.RuntimeMethodHandle::GetFunctionPointer(DotnetRs.RuntimeMethodHandle)"
)]
pub fn intrinsic_method_handle_get_function_pointer<'gc, 'm: 'gc>(
    ctx: &mut dyn VesOps<'gc, 'm>,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let gc = ctx.gc();
    let handle = ctx.pop_value_type();
    let method_obj = unsafe {
        ObjectRef::read_branded(
            &handle
                .instance_storage
                .get_field_local(handle.description, "_value"),
            &gc,
        )
    };
    let (method, lookup) = ctx.resolve_runtime_method(method_obj);
    let index = ctx.get_runtime_method_index(method, lookup);
    ctx.push_isize(index as isize);
    StepResult::Continue
}

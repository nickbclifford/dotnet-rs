use crate::{
    MethodInfo, StepResult,
    context::ResolutionContext,
    layout::type_layout,
    resolution::{TypeResolutionExt, ValueResolution},
    stack::ops::VesOps,
};
use dotnet_assemblies::SUPPORT_ASSEMBLY;
use dotnet_macros::dotnet_intrinsic;
use dotnet_types::{
    TypeDescription,
    comparer::decompose_type_source,
    generics::{ConcreteType, GenericLookup},
    members::MethodDescription,
    runtime::RuntimeType,
};
use dotnet_utils::gc::GCHandle;
use dotnet_value::{
    StackValue,
    layout::{LayoutManager, Scalar},
    object::{HeapStorage, ObjectRef},
};
use dotnetdll::prelude::{
    Accessibility, BaseType, Kind, MemberAccessibility, MemberType, TypeSource,
};

#[dotnet_intrinsic("object[] System.Reflection.Assembly::GetCustomAttributes(System.Type, bool)")]
pub fn intrinsic_assembly_get_custom_attributes<'gc, 'm: 'gc>(
    ctx: &mut dyn VesOps<'gc, 'm>,
    gc: GCHandle<'gc>,
    method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let num_args = method.method.signature.parameters.len()
        + if method.method.signature.instance {
            1
        } else {
            0
        };
    for _ in 0..num_args {
        let _ = ctx.pop(gc);
    }

    // Return an empty array of Attribute
    let attribute_type = ctx.loader().corlib_type("System.Attribute");
    let array = ctx.current_context().new_vector(attribute_type.into(), 0);
    let obj = ObjectRef::new(gc, HeapStorage::Vec(array));
    ctx.register_new_object(&obj);
    ctx.push_obj(gc, obj);

    StepResult::Continue
}

#[dotnet_intrinsic(
    "static System.Attribute[] System.Attribute::GetCustomAttributes(System.Reflection.MemberInfo, System.Type, bool)"
)]
pub fn intrinsic_attribute_get_custom_attributes<'gc, 'm: 'gc>(
    ctx: &mut dyn VesOps<'gc, 'm>,
    gc: GCHandle<'gc>,
    method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let num_args = method.method.signature.parameters.len()
        + if method.method.signature.instance {
            1
        } else {
            0
        };
    for _ in 0..num_args {
        let _ = ctx.pop(gc);
    }

    // Return an empty array of Attribute
    let attribute_type = ctx.loader().corlib_type("System.Attribute");
    let array = ctx.current_context().new_vector(attribute_type.into(), 0);
    let obj = ObjectRef::new(gc, HeapStorage::Vec(array));
    ctx.register_new_object(&obj);
    ctx.push_obj(gc, obj);

    StepResult::Continue
}

#[dotnet_intrinsic("static System.Type System.Type::GetType(string)")]
pub fn intrinsic_type_get_type<'gc, 'm: 'gc>(
    ctx: &mut dyn VesOps<'gc, 'm>,
    gc: GCHandle<'gc>,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let name_obj = ctx.pop_obj(gc);
    if name_obj.0.is_none() {
        ctx.push(gc, StackValue::null());
        return StepResult::Continue;
    }

    let name = name_obj.as_heap_storage(|s| {
        if let HeapStorage::Str(s) = s {
            s.as_string()
        } else {
            panic!("GetType argument is not a string")
        }
    });

    let td = ctx.loader().corlib_type(&name);
    if !td.is_null() {
        let rt_obj = ctx.get_runtime_type(gc, RuntimeType::Type(td));
        ctx.push_obj(gc, rt_obj);
    } else {
        ctx.push(gc, StackValue::null());
    }

    StepResult::Continue
}

#[dotnet_intrinsic("string System.Type::get_Name()")]
#[dotnet_intrinsic("string System.Type::get_Namespace()")]
#[dotnet_intrinsic("System.Reflection.Assembly System.Type::get_Assembly()")]
#[dotnet_intrinsic("System.Type System.Type::get_BaseType()")]
#[dotnet_intrinsic("bool System.Type::get_IsGenericType()")]
#[dotnet_intrinsic("System.Type System.Type::GetGenericTypeDefinition()")]
#[dotnet_intrinsic("System.Type[] System.Type::GetGenericArguments()")]
#[dotnet_intrinsic("System.RuntimeTypeHandle System.Type::get_TypeHandle()")]
#[dotnet_intrinsic("System.Type System.Type::MakeGenericType(System.Type[])")]
#[dotnet_intrinsic("string DotnetRs.RuntimeType::get_Name()")]
#[dotnet_intrinsic("string DotnetRs.RuntimeType::GetName()")]
#[dotnet_intrinsic("string DotnetRs.RuntimeType::get_Namespace()")]
#[dotnet_intrinsic("string DotnetRs.RuntimeType::GetNamespace()")]
#[dotnet_intrinsic("System.Reflection.Assembly DotnetRs.RuntimeType::get_Assembly()")]
#[dotnet_intrinsic("System.Reflection.Assembly DotnetRs.RuntimeType::GetAssembly()")]
#[dotnet_intrinsic("System.Type DotnetRs.RuntimeType::get_BaseType()")]
#[dotnet_intrinsic("System.Type DotnetRs.RuntimeType::GetBaseType()")]
#[dotnet_intrinsic("bool DotnetRs.RuntimeType::get_IsGenericType()")]
#[dotnet_intrinsic("bool DotnetRs.RuntimeType::GetIsGenericType()")]
#[dotnet_intrinsic("System.Type DotnetRs.RuntimeType::get_GenericTypeDefinition()")]
#[dotnet_intrinsic("System.Type DotnetRs.RuntimeType::GetGenericTypeDefinition()")]
#[dotnet_intrinsic("System.Type[] DotnetRs.RuntimeType::GetGenericArguments()")]
#[dotnet_intrinsic("System.RuntimeTypeHandle DotnetRs.RuntimeType::get_TypeHandle()")]
#[dotnet_intrinsic("System.Type DotnetRs.RuntimeType::MakeGenericType(System.Type[])")]
#[dotnet_intrinsic(
    "System.Reflection.MethodInfo[] DotnetRs.RuntimeType::GetMethods(System.Reflection.BindingFlags)"
)]
#[dotnet_intrinsic(
    "System.Reflection.MethodInfo DotnetRs.RuntimeType::GetMethodImpl(string, System.Reflection.BindingFlags, System.Reflection.Binder, System.Reflection.CallingConventions, System.Type[], System.Reflection.ParameterModifier[])"
)]
#[dotnet_intrinsic(
    "System.Reflection.ConstructorInfo[] DotnetRs.RuntimeType::GetConstructors(System.Reflection.BindingFlags)"
)]
#[dotnet_intrinsic(
    "System.Reflection.ConstructorInfo DotnetRs.RuntimeType::GetConstructorImpl(System.Reflection.BindingFlags, System.Reflection.Binder, System.Reflection.CallingConventions, System.Type[], System.Reflection.ParameterModifier[])"
)]
#[dotnet_intrinsic(
    "System.Reflection.FieldInfo[] DotnetRs.RuntimeType::GetFields(System.Reflection.BindingFlags)"
)]
#[dotnet_intrinsic(
    "System.Reflection.FieldInfo DotnetRs.RuntimeType::GetField(string, System.Reflection.BindingFlags)"
)]
#[dotnet_intrinsic(
    "System.Reflection.PropertyInfo[] DotnetRs.RuntimeType::GetProperties(System.Reflection.BindingFlags)"
)]
#[dotnet_intrinsic(
    "System.Reflection.PropertyInfo DotnetRs.RuntimeType::GetPropertyImpl(string, System.Reflection.BindingFlags, System.Reflection.Binder, System.Type, System.Type[], System.Reflection.ParameterModifier[])"
)]
pub fn runtime_type_intrinsic_call<'gc, 'm: 'gc>(
    ctx: &mut dyn VesOps<'gc, 'm>,
    gc: GCHandle<'gc>,
    method: MethodDescription,
    generics: &GenericLookup,
) -> StepResult {
    let method_name = &*method.method.name;
    let param_count = method.method.signature.parameters.len();

    let result = match (method_name, param_count) {
        ("CreateInstanceCheckThis", 0) => {
            let _obj = ctx.pop_obj(gc);
            // For now, we don't perform any actual checks.
            // In a real VM, this would check if the type is abstract, has a ctor, etc.
            Some(StepResult::Continue)
        }
        ("GetAssembly" | "get_Assembly", 0) => {
            let obj = ctx.pop_obj(gc);

            let target_type = ctx.resolve_runtime_type(obj);
            let resolution = target_type.resolution(ctx.loader());

            let cached_asm = ctx.reflection().asms_read().get(&resolution).copied();
            if let Some(o) = cached_asm {
                ctx.push_obj(gc, o);
                return StepResult::Continue;
            }

            let support_res = ctx.loader().get_assembly(SUPPORT_ASSEMBLY);
            let (index, definition) = support_res
                .definition()
                .type_definitions
                .iter()
                .enumerate()
                .find(|(_, a)| a.type_name() == "DotnetRs.Assembly")
                .expect("could find DotnetRs.Assembly in support library");
            let type_index = support_res.type_definition_index(index).unwrap();
            let res_ctx = ResolutionContext::new(
                generics,
                ctx.loader(),
                support_res,
                ctx.shared().caches.clone(),
                Some(ctx.shared().clone()),
            );
            let asm_handle =
                res_ctx.new_object(TypeDescription::new(support_res, definition, type_index));
            let data = (resolution.as_raw() as usize).to_ne_bytes();
            asm_handle
                .instance_storage
                .get_field_mut_local(asm_handle.description, "resolution")
                .copy_from_slice(&data);
            let v = ObjectRef::new(gc, HeapStorage::Obj(asm_handle));
            ctx.register_new_object(&v);

            ctx.reflection().asms_write().insert(resolution, v);
            ctx.push_obj(gc, v);
            Some(StepResult::Continue)
        }
        ("GetNamespace" | "get_Namespace", 0) => {
            let obj = ctx.pop_obj(gc);
            let target_type = ctx.resolve_runtime_type(obj);
            match target_type {
                RuntimeType::Type(td) | RuntimeType::Generic(td, _) => {
                    match td.definition().namespace.as_ref() {
                        None => ctx.push(gc, StackValue::null()),
                        Some(n) => ctx.push_string(gc, n.clone().into()),
                    }
                }
                _ => ctx.push_string(gc, "System".into()),
            }
            Some(StepResult::Continue)
        }
        ("GetMethods", 1) => {
            let flags = ctx.pop_i32(gc);
            let obj = ctx.pop_obj(gc);
            let target_type = ctx.resolve_runtime_type(obj);

            const BINDING_FLAGS_INSTANCE: i32 = 4;
            const BINDING_FLAGS_STATIC: i32 = 8;
            const BINDING_FLAGS_PUBLIC: i32 = 16;
            const BINDING_FLAGS_NON_PUBLIC: i32 = 32;

            let mut methods_objs = Vec::new();
            if let RuntimeType::Type(td) | RuntimeType::Generic(td, _) = target_type {
                for m in td.definition().methods.iter() {
                    let is_public =
                        m.accessibility == MemberAccessibility::Access(Accessibility::Public);
                    let is_static = !m.signature.instance;

                    let match_public = if is_public {
                        (flags & BINDING_FLAGS_PUBLIC) != 0
                    } else {
                        (flags & BINDING_FLAGS_NON_PUBLIC) != 0
                    };
                    let match_static = if is_static {
                        (flags & BINDING_FLAGS_STATIC) != 0
                    } else {
                        (flags & BINDING_FLAGS_INSTANCE) != 0
                    };

                    if match_public && match_static && m.name != ".ctor" && m.name != ".cctor" {
                        let desc = MethodDescription {
                            parent: td,
                            method: m,
                            method_resolution: td.resolution,
                        };
                        let mut lookup = GenericLookup::default();
                        if let RuntimeType::Generic(_, args) = &target_type {
                            lookup.type_generics = args
                                .iter()
                                .map(|a| a.to_concrete(ctx.loader()))
                                .collect::<Vec<_>>()
                                .into();
                        }
                        methods_objs.push(ctx.get_runtime_method_obj(gc, desc, lookup));
                    }
                }
            }

            let method_info_type = ctx.loader().corlib_type("System.Reflection.MethodInfo");
            let mut vector =
                ctx.new_vector(ConcreteType::from(method_info_type), methods_objs.len());
            for (i, m) in methods_objs.into_iter().enumerate() {
                m.write(&mut vector.get_mut()[i * ObjectRef::SIZE..(i + 1) * ObjectRef::SIZE]);
            }
            ctx.push_obj(gc, ObjectRef::new(gc, HeapStorage::Vec(vector)));
            Some(StepResult::Continue)
        }
        ("GetMethodImpl", 6) => {
            let _modifiers = ctx.pop(gc);
            let _types = ctx.pop(gc); // TODO: match by parameter types
            let _call_convention = ctx.pop(gc);
            let _binder = ctx.pop(gc);
            let flags = ctx.pop_i32(gc);
            let name_obj = ctx.pop_obj(gc);
            let obj = ctx.pop_obj(gc);

            let name = name_obj.as_heap_storage(|s| {
                if let HeapStorage::Str(s) = s {
                    s.as_string()
                } else {
                    panic!("name is not a string")
                }
            });
            let target_type = ctx.resolve_runtime_type(obj);

            const BINDING_FLAGS_INSTANCE: i32 = 4;
            const BINDING_FLAGS_STATIC: i32 = 8;
            const BINDING_FLAGS_PUBLIC: i32 = 16;
            const BINDING_FLAGS_NON_PUBLIC: i32 = 32;

            let mut found_method = None;
            if let RuntimeType::Type(td) | RuntimeType::Generic(td, _) = target_type {
                for m in td.definition().methods.iter() {
                    if m.name != name {
                        continue;
                    }
                    let is_public =
                        m.accessibility == MemberAccessibility::Access(Accessibility::Public);
                    let is_static = !m.signature.instance;

                    let match_public = if is_public {
                        (flags & BINDING_FLAGS_PUBLIC) != 0
                    } else {
                        (flags & BINDING_FLAGS_NON_PUBLIC) != 0
                    };
                    let match_static = if is_static {
                        (flags & BINDING_FLAGS_STATIC) != 0
                    } else {
                        (flags & BINDING_FLAGS_INSTANCE) != 0
                    };

                    if match_public && match_static {
                        let desc = MethodDescription {
                            parent: td,
                            method: m,
                            method_resolution: td.resolution,
                        };
                        let mut lookup = GenericLookup::default();
                        if let RuntimeType::Generic(_, args) = &target_type {
                            lookup.type_generics = args
                                .iter()
                                .map(|a| a.to_concrete(ctx.loader()))
                                .collect::<Vec<_>>()
                                .into();
                        }
                        found_method = Some(ctx.get_runtime_method_obj(gc, desc, lookup));
                        break;
                    }
                }
            }

            if let Some(m) = found_method {
                ctx.push_obj(gc, m);
            } else {
                ctx.push(gc, StackValue::null());
            }
            Some(StepResult::Continue)
        }
        ("GetConstructors", 1) => {
            let flags = ctx.pop_i32(gc);
            let obj = ctx.pop_obj(gc);
            let target_type = ctx.resolve_runtime_type(obj);

            const BINDING_FLAGS_INSTANCE: i32 = 4;
            const BINDING_FLAGS_STATIC: i32 = 8;
            const BINDING_FLAGS_PUBLIC: i32 = 16;
            const BINDING_FLAGS_NON_PUBLIC: i32 = 32;

            let mut methods_objs = Vec::new();
            if let RuntimeType::Type(td) | RuntimeType::Generic(td, _) = target_type {
                for m in td.definition().methods.iter() {
                    let is_public =
                        m.accessibility == MemberAccessibility::Access(Accessibility::Public);
                    let is_static = !m.signature.instance;

                    let match_public = if is_public {
                        (flags & BINDING_FLAGS_PUBLIC) != 0
                    } else {
                        (flags & BINDING_FLAGS_NON_PUBLIC) != 0
                    };
                    let match_static = if is_static {
                        (flags & BINDING_FLAGS_STATIC) != 0
                    } else {
                        (flags & BINDING_FLAGS_INSTANCE) != 0
                    };

                    if match_public && match_static && m.name == ".ctor" {
                        let desc = MethodDescription {
                            parent: td,
                            method: m,
                            method_resolution: td.resolution,
                        };
                        let mut lookup = GenericLookup::default();
                        if let RuntimeType::Generic(_, args) = &target_type {
                            lookup.type_generics = args
                                .iter()
                                .map(|a| a.to_concrete(ctx.loader()))
                                .collect::<Vec<_>>()
                                .into();
                        }
                        methods_objs.push(ctx.get_runtime_method_obj(gc, desc, lookup));
                    }
                }
            }

            let constructor_info_type = ctx
                .loader()
                .corlib_type("System.Reflection.ConstructorInfo");
            let mut vector = ctx.new_vector(
                ConcreteType::from(constructor_info_type),
                methods_objs.len(),
            );
            for (i, m) in methods_objs.into_iter().enumerate() {
                m.write(&mut vector.get_mut()[i * ObjectRef::SIZE..(i + 1) * ObjectRef::SIZE]);
            }
            ctx.push_obj(gc, ObjectRef::new(gc, HeapStorage::Vec(vector)));
            Some(StepResult::Continue)
        }
        ("GetName" | "get_Name", 0) => {
            let obj = ctx.pop_obj(gc);
            let target_type = ctx.resolve_runtime_type(obj);
            ctx.push_string(gc, target_type.get_name().into());
            Some(StepResult::Continue)
        }
        ("GetBaseType" | "get_BaseType", 0) => {
            let obj = ctx.pop_obj(gc);
            let target_type = ctx.resolve_runtime_type(obj);
            match target_type {
                RuntimeType::Type(td) | RuntimeType::Generic(td, _)
                    if td.definition().extends.is_some() =>
                {
                    // Get the first ancestor (the direct parent)
                    let mut ancestors = ctx.loader().ancestors(td);
                    ancestors.next(); // skip self
                    if let Some((base_td, base_generics)) = ancestors.next() {
                        let base_rt: RuntimeType = if base_td.definition().extends.is_none()
                            && base_td.type_name() == "System.Object"
                        {
                            RuntimeType::Object
                        } else if base_generics.is_empty() {
                            RuntimeType::Type(base_td)
                        } else {
                            // Convert member type generic parameters to runtime types
                            let runtime_generics: Vec<RuntimeType> = base_generics
                                .iter()
                                .map(|mt| match mt {
                                    MemberType::Base(b) => {
                                        match b.as_ref() {
                                            BaseType::Type { source, .. } => {
                                                let (ut, sub_generics) =
                                                    decompose_type_source::<MemberType>(source);
                                                let sub_td =
                                                    ctx.loader().locate_type(td.resolution, ut);
                                                if sub_generics.is_empty() {
                                                    RuntimeType::Type(sub_td)
                                                } else {
                                                    // TODO: properly handle generic types
                                                    RuntimeType::Type(sub_td)
                                                }
                                            }
                                            BaseType::Object => RuntimeType::Object,
                                            BaseType::String => RuntimeType::String,
                                            BaseType::Boolean => RuntimeType::Boolean,
                                            BaseType::Char => RuntimeType::Char,
                                            BaseType::Int8 => RuntimeType::Int8,
                                            BaseType::UInt8 => RuntimeType::UInt8,
                                            BaseType::Int16 => RuntimeType::Int16,
                                            BaseType::UInt16 => RuntimeType::UInt16,
                                            BaseType::Int32 => RuntimeType::Int32,
                                            BaseType::UInt32 => RuntimeType::UInt32,
                                            BaseType::Int64 => RuntimeType::Int64,
                                            BaseType::UInt64 => RuntimeType::UInt64,
                                            BaseType::Float32 => RuntimeType::Float32,
                                            BaseType::Float64 => RuntimeType::Float64,
                                            BaseType::IntPtr => RuntimeType::IntPtr,
                                            BaseType::UIntPtr => RuntimeType::UIntPtr,
                                            _ => RuntimeType::Object, // Fallback
                                        }
                                    }
                                    MemberType::TypeGeneric(i) => RuntimeType::TypeParameter {
                                        owner: td,
                                        index: *i as u16,
                                    },
                                })
                                .collect();
                            RuntimeType::Generic(base_td, runtime_generics)
                        };
                        let rt_obj = ctx.get_runtime_type(gc, base_rt);
                        ctx.push_obj(gc, rt_obj);
                    } else {
                        ctx.push(gc, StackValue::null());
                    }
                }
                _ => ctx.push(gc, StackValue::null()),
            }
            Some(StepResult::Continue)
        }
        ("GetIsGenericType" | "get_IsGenericType", 0) => {
            let obj = ctx.pop_obj(gc);
            let target_type = ctx.resolve_runtime_type(obj);
            let is_generic = matches!(target_type, RuntimeType::Generic(_, _));
            ctx.push_i32(gc, if is_generic { 1 } else { 0 });
            Some(StepResult::Continue)
        }
        ("GetGenericTypeDefinition" | "get_GenericTypeDefinition", 0) => {
            let obj = ctx.pop_obj(gc);
            let target_type = ctx.resolve_runtime_type(obj);
            match target_type {
                RuntimeType::Generic(td, _) => {
                    let n_params = td.definition().generic_parameters.len();
                    let params = (0..n_params as u16)
                        .map(|index| RuntimeType::TypeParameter { owner: td, index })
                        .collect();
                    let def_rt = RuntimeType::Generic(td, params);
                    let rt_obj = ctx.get_runtime_type(gc, def_rt);
                    ctx.push_obj(gc, rt_obj);
                }
                _ => return ctx.throw_by_name(gc, "System.InvalidOperationException"),
            }
            Some(StepResult::Continue)
        }
        ("GetGenericArguments" | "get_GenericArguments", 0) => {
            let obj = ctx.pop_obj(gc);
            let target_type = ctx.resolve_runtime_type(obj);
            let args = match target_type {
                RuntimeType::Generic(_, args) => args.clone(),
                _ => vec![],
            };

            // Check GC safe point before allocating type array
            ctx.check_gc_safe_point();

            let type_type_td = ctx.loader().corlib_type("System.Type");
            let type_type = ConcreteType::from(type_type_td);
            let mut vector = ctx.current_context().new_vector(type_type, args.len());
            for (i, (arg, chunk)) in args
                .into_iter()
                .zip(vector.get_mut().chunks_exact_mut(ObjectRef::SIZE))
                .enumerate()
            {
                // Check GC safe point periodically during loops with allocations
                // Check every 16 iterations
                if i % 16 == 0 {
                    ctx.check_gc_safe_point();
                }
                let arg_obj = ctx.get_runtime_type(gc, arg);
                arg_obj.write(chunk);
            }
            let obj = ObjectRef::new(gc, HeapStorage::Vec(vector));
            ctx.register_new_object(&obj);
            ctx.push_obj(gc, obj);
            Some(StepResult::Continue)
        }
        ("GetTypeHandle" | "get_TypeHandle", 0) => {
            let obj = ctx.pop_obj(gc);

            let rth = ctx.loader().corlib_type("System.RuntimeTypeHandle");
            let instance = ctx.current_context().new_object(rth);
            obj.write(&mut instance.instance_storage.get_field_mut_local(rth, "_value"));

            ctx.push(gc, StackValue::ValueType(instance));
            Some(StepResult::Continue)
        }
        ("MakeGenericType", 1) => {
            let parameters = ctx.pop_obj(gc);
            let target = ctx.pop_obj(gc);

            // Check GC safe point before potentially allocating generic type objects
            ctx.check_gc_safe_point();

            let target_rt = ctx.resolve_runtime_type(target);

            if let RuntimeType::Type(td) | RuntimeType::Generic(td, _) = target_rt {
                let param_objs = parameters.as_vector(|v: &dotnet_value::object::Vector<'gc>| {
                    v.get()
                        .chunks_exact(ObjectRef::SIZE)
                        .map(|chunk| unsafe { ObjectRef::read_branded(chunk, gc) })
                        .collect::<Vec<_>>()
                });
                let new_generics: Vec<_> = param_objs
                    .into_iter()
                    .map(|p_obj| ctx.resolve_runtime_type(p_obj).clone())
                    .collect();
                let new_rt = RuntimeType::Generic(td, new_generics);

                let rt_obj = ctx.get_runtime_type(gc, new_rt);
                ctx.push_obj(gc, rt_obj);
            } else {
                return ctx.throw_by_name(gc, "System.InvalidOperationException");
            }
            Some(StepResult::Continue)
        }
        ("CreateInstanceDefaultCtor", 2) => {
            let _ = ctx.pop(gc); // skipCheck
            let _ = ctx.pop(gc); // publicOnly
            let target_obj = ctx.pop_obj(gc);

            // Check GC safe point before object instantiation
            ctx.check_gc_safe_point();

            let target_rt = ctx.resolve_runtime_type(target_obj);

            let (td, type_generics) = match target_rt {
                RuntimeType::Type(td) => (td, vec![]),
                RuntimeType::Generic(td, args) => (td, args.clone()),
                _ => panic!("cannot create instance of {:?}", target_rt),
            };

            let type_generics_concrete: Vec<ConcreteType> = type_generics
                .iter()
                .map(|a| a.to_concrete(ctx.loader()))
                .collect();
            let new_lookup = GenericLookup::new(type_generics_concrete);
            let new_ctx = ctx.current_context().with_generics(&new_lookup);

            let instance = new_ctx.new_object(td);

            for m in &td.definition().methods {
                if m.runtime_special_name
                    && m.name == ".ctor"
                    && m.signature.instance
                    && m.signature.parameters.is_empty()
                {
                    let desc = MethodDescription {
                        parent: td,
                        method_resolution: td.resolution,
                        method: m,
                    };

                    ctx.constructor_frame(
                        gc,
                        instance,
                        MethodInfo::new(desc, &new_lookup, ctx.shared().clone()),
                        new_lookup,
                    );
                    return StepResult::FramePushed;
                }
            }

            panic!("could not find a parameterless constructor in {:?}", td)
        }
        _ => None,
    };

    result.unwrap_or_else(|| panic!("unimplemented runtime type intrinsic: {:?}", method))
}

#[dotnet_intrinsic(
    "static void System.RuntimeTypeHandle::GetActivationInfo(System.RuntimeTypeHandle, System.IntPtr&, System.IntPtr&, System.IntPtr&, bool&)"
)]
#[dotnet_intrinsic(
    "static void DotnetRs.RuntimeTypeHandle::GetActivationInfo(DotnetRs.RuntimeTypeHandle, System.IntPtr&, System.IntPtr&, System.IntPtr&, bool&)"
)]
pub fn runtime_type_handle_intrinsic_call<'gc, 'm: 'gc>(
    ctx: &mut dyn VesOps<'gc, 'm>,
    gc: GCHandle<'gc>,
    method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let method_name = &*method.method.name;
    let param_count = method.method.signature.parameters.len();

    match (method_name, param_count) {
        ("GetActivationInfo", 5) => {
            // static extern void GetActivationInfo(RuntimeTypeHandle type, out IntPtr pfnAllocator, out IntPtr allocatorFirstArg, out IntPtr pfnCtor, out bool ctorIsPublic);
            let ctor_is_public = ctx.pop_managed_ptr(gc);
            let pfn_ctor = ctx.pop_managed_ptr(gc);
            let allocator_first_arg = ctx.pop_managed_ptr(gc);
            let pfn_allocator = ctx.pop_managed_ptr(gc);

            let rt_obj = match ctx.pop(gc) {
                StackValue::ValueType(rth_handle) => unsafe {
                    ObjectRef::read_branded(
                        &rth_handle
                            .instance_storage
                            .get_field_local(rth_handle.description, "_value"),
                        gc,
                    )
                },
                StackValue::ObjectRef(rt_obj) => rt_obj,
                v => panic!(
                    "invalid type on stack ({:?}), expected ValueType(RuntimeTypeHandle) or ObjectRef(RuntimeType)",
                    v
                ),
            };

            let rt = ctx.resolve_runtime_type(rt_obj);
            let td = match rt {
                RuntimeType::Type(td) => td,
                RuntimeType::Generic(td, _) => td,
                _ => panic!("GetActivationInfo called on non-type: {:?}", rt),
            };

            // pfnAllocator = IntPtr.Zero
            ctx.push(gc, StackValue::NativeInt(0));
            unsafe {
                ctx.pop(gc).store(
                    pfn_allocator
                        .pointer()
                        .expect("pfn_allocator null")
                        .as_ptr(),
                    dotnetdll::prelude::StoreType::IntPtr,
                )
            };

            // allocatorFirstArg = IntPtr.Zero
            ctx.push(gc, StackValue::NativeInt(0));
            unsafe {
                ctx.pop(gc).store(
                    allocator_first_arg
                        .pointer()
                        .expect("allocator_first_arg null")
                        .as_ptr(),
                    dotnetdll::prelude::StoreType::IntPtr,
                )
            };

            // Find default ctor
            let mut found_ctor = false;
            for m in td.definition().methods.iter() {
                if m.name == ".ctor" && m.signature.instance && m.signature.parameters.is_empty() {
                    let method_idx = ctx.get_runtime_method_index(
                        MethodDescription {
                            parent: td,
                            method_resolution: td.resolution,
                            method: m,
                        },
                        if let RuntimeType::Generic(_, type_generics) = rt {
                            GenericLookup::new(
                                type_generics
                                    .iter()
                                    .map(|t| t.to_concrete(ctx.loader()))
                                    .collect(),
                            )
                        } else {
                            ctx.shared().empty_generics.clone()
                        },
                    );

                    ctx.push(gc, StackValue::NativeInt(method_idx as isize));
                    unsafe {
                        ctx.pop(gc).store(
                            pfn_ctor.pointer().expect("pfn_ctor null").as_ptr(),
                            dotnetdll::prelude::StoreType::IntPtr,
                        )
                    };

                    ctx.push(gc, StackValue::Int32(1));
                    unsafe {
                        ctx.pop(gc).store(
                            ctor_is_public
                                .pointer()
                                .expect("ctor_is_public null")
                                .as_ptr(),
                            dotnetdll::prelude::StoreType::Int8,
                        )
                    };
                    found_ctor = true;
                    break;
                }
            }

            if !found_ctor {
                panic!("Could not find default constructor for {}", td.type_name());
            }

            StepResult::Continue
        }
        _ => panic!(
            "Unknown RuntimeTypeHandle intrinsic: {}.{}",
            method.parent.type_name(),
            method_name
        ),
    }
}

#[dotnet_intrinsic(
    "static System.RuntimeTypeHandle System.Runtime.CompilerServices.RuntimeHelpers::GetMethodTable(System.RuntimeTypeHandle)"
)]
pub fn intrinsic_runtime_helpers_get_method_table<'gc, 'm: 'gc>(
    ctx: &mut dyn VesOps<'gc, 'm>,
    gc: GCHandle<'gc>,
    method: MethodDescription,
    generics: &GenericLookup,
) -> StepResult {
    let res_ctx = ResolutionContext::for_method(
        method,
        ctx.loader(),
        generics,
        ctx.shared().caches.clone(),
        Some(ctx.shared().clone()),
    );
    let obj = ctx.pop(gc);
    let object_type = match obj {
        StackValue::ObjectRef(ObjectRef(Some(h))) => res_ctx.get_heap_description(h),
        StackValue::ObjectRef(ObjectRef(None)) => {
            return ctx.throw_by_name(gc, "System.NullReferenceException");
        }
        _ => panic!("invalid type on stack"),
    };

    let mt_ptr = object_type.definition_ptr().unwrap().as_ptr();
    ctx.push_isize(gc, mt_ptr as isize);
    StepResult::Continue
}

#[dotnet_intrinsic(
    "static bool System.Runtime.CompilerServices.RuntimeHelpers::IsBitwiseEquatable<T>()"
)]
pub fn intrinsic_runtime_helpers_is_bitwise_equatable<'gc, 'm: 'gc>(
    ctx: &mut dyn VesOps<'gc, 'm>,
    gc: GCHandle<'gc>,
    _method: MethodDescription,
    generics: &GenericLookup,
) -> StepResult {
    let target = &generics.method_generics[0];
    let layout = type_layout(target.clone(), &ctx.current_context());
    let value = match &*layout {
        LayoutManager::Scalar(Scalar::ObjectRef) => false,
        LayoutManager::Scalar(_) => true,
        _ => false,
    };
    ctx.push_i32(gc, value as i32);
    StepResult::Continue
}

#[dotnet_intrinsic(
    "static bool System.Runtime.CompilerServices.RuntimeHelpers::IsReferenceOrContainsReferences()"
)]
pub fn intrinsic_runtime_helpers_is_reference_or_contains_references<'gc, 'm: 'gc>(
    ctx: &mut dyn VesOps<'gc, 'm>,
    gc: GCHandle<'gc>,
    _method: MethodDescription,
    generics: &GenericLookup,
) -> StepResult {
    let target = &generics.method_generics[0];
    let layout = type_layout(target.clone(), &ctx.current_context());
    ctx.push_i32(gc, layout.is_or_contains_refs() as i32);
    StepResult::Continue
}

#[dotnet_intrinsic(
    "static void System.Runtime.CompilerServices.RuntimeHelpers::RunClassConstructor(System.RuntimeTypeHandle)"
)]
pub fn intrinsic_runtime_helpers_run_class_constructor<'gc, 'm: 'gc>(
    ctx: &mut dyn VesOps<'gc, 'm>,
    gc: GCHandle<'gc>,
    _method: MethodDescription,
    generics: &GenericLookup,
) -> StepResult {
    let arg = ctx.peek_stack();
    let StackValue::ValueType(handle) = arg else {
        panic!(
            "RunClassConstructor expects a RuntimeTypeHandle, received {:?}",
            arg
        )
    };

    let target_obj = unsafe {
        ObjectRef::read_branded(
            &handle
                .instance_storage
                .get_field_local(handle.description, "_value"),
            gc,
        )
    };
    let target_type = ctx.resolve_runtime_type(target_obj);
    let target_ct = target_type.to_concrete(ctx.loader());
    let target_desc = ctx.loader().find_concrete_type(target_ct);

    let res = ctx.initialize_static_storage(gc, target_desc, generics.clone());
    if res != StepResult::Continue {
        return res;
    }

    // Initialization complete, pop the argument
    let _ = ctx.pop(gc);
    StepResult::Continue
}

#[dotnet_intrinsic("static object System.Activator::CreateInstance()")]
pub fn intrinsic_activator_create_instance<'gc, 'm: 'gc>(
    ctx: &mut dyn VesOps<'gc, 'm>,
    gc: GCHandle<'gc>,
    method: MethodDescription,
    generics: &GenericLookup,
) -> StepResult {
    let target_ct = generics.method_generics[0].clone();
    let target_td = ctx.loader().find_concrete_type(target_ct.clone());
    let res_ctx = ResolutionContext::for_method(
        method,
        ctx.loader(),
        generics,
        ctx.shared().caches.clone(),
        Some(ctx.shared().clone()),
    );

    if target_td.is_value_type(&ctx.current_context()) {
        let instance = res_ctx.new_object(target_td);
        ctx.push_value_type(gc, instance);
        StepResult::Continue
    } else {
        let instance = res_ctx.new_object(target_td);
        let mut new_lookup = GenericLookup::default();
        if let BaseType::Type {
            source: TypeSource::Generic { parameters, .. },
            ..
        } = target_ct.get()
        {
            new_lookup.type_generics = parameters.clone().into();
        }

        for m in target_td.definition().methods.iter() {
            if m.name == ".ctor" && m.signature.instance && m.signature.parameters.is_empty() {
                let desc = MethodDescription {
                    parent: target_td,
                    method_resolution: target_td.resolution,
                    method: m,
                };

                ctx.constructor_frame(
                    gc,
                    instance,
                    MethodInfo::new(desc, &new_lookup, ctx.shared().clone()),
                    new_lookup,
                );
                return StepResult::FramePushed;
            }
        }

        panic!(
            "could not find a parameterless constructor in {:?}",
            target_td
        )
    }
}

#[dotnet_intrinsic("static System.Type System.Type::GetTypeFromHandle(System.RuntimeTypeHandle)")]
pub fn intrinsic_get_from_handle<'gc, 'm: 'gc>(
    ctx: &mut dyn VesOps<'gc, 'm>,
    gc: GCHandle<'gc>,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let handle = ctx.pop_value_type(gc);
    let target = unsafe {
        ObjectRef::read_branded(
            &handle
                .instance_storage
                .get_field_local(handle.description, "_value"),
            gc,
        )
    };
    ctx.push_obj(gc, target);
    StepResult::Continue
}

#[dotnet_intrinsic(
    "static System.IntPtr System.RuntimeTypeHandle::ToIntPtr(System.RuntimeTypeHandle)"
)]
#[dotnet_intrinsic(
    "static System.IntPtr DotnetRs.RuntimeTypeHandle::ToIntPtr(DotnetRs.RuntimeTypeHandle)"
)]
pub fn intrinsic_type_handle_to_int_ptr<'gc, 'm: 'gc>(
    ctx: &mut dyn VesOps<'gc, 'm>,
    gc: GCHandle<'gc>,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let handle = ctx.pop_value_type(gc);
    let target = handle
        .instance_storage
        .get_field_local(handle.description, "_value");
    let val = usize::from_ne_bytes((&*target).try_into().unwrap());
    ctx.push_isize(gc, val as isize);
    StepResult::Continue
}

#[dotnet_intrinsic("bool System.Type::get_IsValueType()")]
#[dotnet_intrinsic("bool DotnetRs.RuntimeType::get_IsValueType()")]
#[dotnet_intrinsic("bool DotnetRs.RuntimeType::GetIsValueType()")]
pub fn intrinsic_type_get_is_value_type<'gc, 'm: 'gc>(
    ctx: &mut dyn VesOps<'gc, 'm>,
    gc: GCHandle<'gc>,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let o = ctx.pop_obj(gc);
    let target = ctx.resolve_runtime_type(o);
    let target_ct = target.to_concrete(ctx.loader());
    let target_desc = ctx.loader().find_concrete_type(target_ct);
    let value = target_desc.is_value_type(&ctx.current_context());
    ctx.push_i32(gc, value as i32);
    StepResult::Continue
}

#[dotnet_intrinsic("bool System.Type::get_IsEnum()")]
#[dotnet_intrinsic("bool DotnetRs.RuntimeType::get_IsEnum()")]
#[dotnet_intrinsic("bool DotnetRs.RuntimeType::GetIsEnum()")]
pub fn intrinsic_type_get_is_enum<'gc, 'm: 'gc>(
    ctx: &mut dyn VesOps<'gc, 'm>,
    gc: GCHandle<'gc>,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let o = ctx.pop_obj(gc);
    let target = ctx.resolve_runtime_type(o);
    let value = match target {
        RuntimeType::Type(td) | RuntimeType::Generic(td, _) => td.is_enum().is_some(),
        _ => false,
    };
    ctx.push_i32(gc, value as i32);
    StepResult::Continue
}

#[dotnet_intrinsic("bool System.Type::get_IsInterface()")]
#[dotnet_intrinsic("bool DotnetRs.RuntimeType::get_IsInterface()")]
#[dotnet_intrinsic("bool DotnetRs.RuntimeType::GetIsInterface()")]
pub fn intrinsic_type_get_is_interface<'gc, 'm: 'gc>(
    ctx: &mut dyn VesOps<'gc, 'm>,
    gc: GCHandle<'gc>,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let o = ctx.pop_obj(gc);
    let target = ctx.resolve_runtime_type(o);
    let value = match target {
        RuntimeType::Type(td) | RuntimeType::Generic(td, _) => {
            matches!(td.definition().flags.kind, Kind::Interface)
        }
        _ => false,
    };
    ctx.push_i32(gc, value as i32);
    StepResult::Continue
}

#[dotnet_intrinsic("static bool System.Type::op_Equality(System.Type, System.Type)")]
pub fn intrinsic_type_op_equality<'gc, 'm: 'gc>(
    ctx: &mut dyn VesOps<'gc, 'm>,
    gc: GCHandle<'gc>,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let o2 = ctx.pop_obj(gc);
    let o1 = ctx.pop_obj(gc);
    ctx.push_i32(gc, (o1 == o2) as i32);
    StepResult::Continue
}

#[dotnet_intrinsic("static bool System.Type::op_Inequality(System.Type, System.Type)")]
pub fn intrinsic_type_op_inequality<'gc, 'm: 'gc>(
    ctx: &mut dyn VesOps<'gc, 'm>,
    gc: GCHandle<'gc>,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let o2 = ctx.pop_obj(gc);
    let o1 = ctx.pop_obj(gc);
    ctx.push_i32(gc, (o1 != o2) as i32);
    StepResult::Continue
}

#[dotnet_intrinsic("static System.RuntimeTypeHandle System.Type::GetTypeHandle(object)")]
pub fn intrinsic_type_get_type_handle<'gc, 'm: 'gc>(
    ctx: &mut dyn VesOps<'gc, 'm>,
    gc: GCHandle<'gc>,
    _method: MethodDescription,
    generics: &GenericLookup,
) -> StepResult {
    let obj = ctx.pop_obj(gc);

    let rth = ctx.loader().corlib_type("System.RuntimeTypeHandle");
    let res_ctx = ResolutionContext::for_method(
        _method,
        ctx.loader(),
        generics,
        ctx.shared().caches.clone(),
        Some(ctx.shared().clone()),
    );
    let instance = res_ctx.new_object(rth);
    obj.write(&mut instance.instance_storage.get_field_mut_local(rth, "_value"));

    ctx.push_value_type(gc, instance);
    StepResult::Continue
}

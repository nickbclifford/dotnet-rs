use crate::{
    StepResult,
    intrinsics::reflection::types::{
        build_generic_lookup_from_runtime_type, populate_reflection_array,
    },
    resolution::{TypeResolutionExt, ValueResolution},
    stack::ops::{
        ExceptionOps, LoaderOps, MemoryOps, RawMemoryOps, ReflectionOps, ResolutionOps,
        TypedStackOps,
    },
};
use dotnet_macros::dotnet_intrinsic;
use dotnet_types::{
    generics::{ConcreteType, GenericLookup},
    members::MethodDescription,
    runtime::RuntimeType,
};
use dotnet_value::{
    StackValue,
    object::{HeapStorage, ObjectRef},
};
use dotnetdll::{prelude::Kind, resolved::types::Accessibility as TypeAccessibility};
use std::collections::{HashSet, VecDeque};

#[dotnet_intrinsic("static System.Type System.Type::GetTypeFromHandle(System.RuntimeTypeHandle)")]
pub fn intrinsic_get_from_handle<'gc, T: TypedStackOps<'gc> + MemoryOps<'gc>>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let _gc = ctx.gc_with_token(&dotnet_utils::NoActiveBorrows::new());
    let handle = ctx.pop_value_type();
    let target = handle
        .instance_storage
        .field::<ObjectRef<'gc>>(handle.description, "_value")
        .unwrap()
        .read();
    ctx.push_obj(target);
    StepResult::Continue
}

#[dotnet_intrinsic(
    "static System.IntPtr System.RuntimeTypeHandle::ToIntPtr(System.RuntimeTypeHandle)"
)]
pub fn intrinsic_type_handle_to_int_ptr<'gc, T: TypedStackOps<'gc>>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let handle = ctx.pop_value_type();
    let target = handle
        .instance_storage
        .field::<usize>(handle.description, "_value")
        .unwrap()
        .read();
    let val = target;
    ctx.push_isize(val as isize);
    StepResult::Continue
}

#[dotnet_intrinsic("bool System.Type::get_IsValueType()")]
#[dotnet_intrinsic("bool System.RuntimeType::get_IsValueType()")]
#[dotnet_intrinsic("bool System.RuntimeType::GetIsValueType()")]
pub fn intrinsic_type_get_is_value_type<
    'gc,
    T: TypedStackOps<'gc> + ReflectionOps<'gc> + LoaderOps + ResolutionOps<'gc>,
>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let o = ctx.pop_obj();
    let target = ctx.resolve_runtime_type(o);
    let target_ct = target.to_concrete(ctx.loader().as_ref());
    let target_desc = ctx
        .loader()
        .find_concrete_type(target_ct)
        .expect("Type must exist for get_IsValueType");
    let value = vm_try!(target_desc.is_value_type(&ctx.current_context()));
    ctx.push_i32(value as i32);
    StepResult::Continue
}

#[dotnet_intrinsic("bool System.Type::get_IsEnum()")]
#[dotnet_intrinsic("bool System.RuntimeType::get_IsEnum()")]
#[dotnet_intrinsic("bool System.RuntimeType::GetIsEnum()")]
pub fn intrinsic_type_get_is_enum<'gc, T: TypedStackOps<'gc> + ReflectionOps<'gc>>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let o = ctx.pop_obj();
    let target = ctx.resolve_runtime_type(o);
    let value = match target {
        RuntimeType::Type(td) | RuntimeType::Generic(td, _) => td.is_enum().is_some(),
        _ => false,
    };
    ctx.push_i32(value as i32);
    StepResult::Continue
}

#[dotnet_intrinsic("bool System.Type::get_IsInterface()")]
#[dotnet_intrinsic("bool System.RuntimeType::get_IsInterface()")]
#[dotnet_intrinsic("bool System.RuntimeType::GetIsInterface()")]
pub fn intrinsic_type_get_is_interface<'gc, T: TypedStackOps<'gc> + ReflectionOps<'gc>>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let o = ctx.pop_obj();
    let target = ctx.resolve_runtime_type(o);
    let value = match target {
        RuntimeType::Type(td) | RuntimeType::Generic(td, _) => {
            matches!(td.definition().flags.kind, Kind::Interface)
        }
        _ => false,
    };
    ctx.push_i32(value as i32);
    StepResult::Continue
}

#[dotnet_intrinsic("static bool System.Type::op_Equality(System.Type, System.Type)")]
pub fn intrinsic_type_op_equality<'gc, T: TypedStackOps<'gc>>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let o2 = ctx.pop_obj();
    let o1 = ctx.pop_obj();
    ctx.push_i32((o1 == o2) as i32);
    StepResult::Continue
}

#[dotnet_intrinsic("static bool System.Type::op_Inequality(System.Type, System.Type)")]
pub fn intrinsic_type_op_inequality<'gc, T: TypedStackOps<'gc>>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let o2 = ctx.pop_obj();
    let o1 = ctx.pop_obj();
    ctx.push_i32((o1 != o2) as i32);
    StepResult::Continue
}

#[dotnet_intrinsic("static System.RuntimeTypeHandle System.Type::GetTypeHandle(object)")]
pub fn intrinsic_type_get_type_handle<'gc, T: TypedStackOps<'gc> + LoaderOps + MemoryOps<'gc>>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let obj = ctx.pop_obj();

    let rth = vm_try!(ctx.loader().corlib_type("System.RuntimeTypeHandle"));
    let instance = vm_try!(ctx.new_object(rth.clone()));
    instance
        .instance_storage
        .field::<ObjectRef<'gc>>(rth, "_value")
        .unwrap()
        .write(obj);

    ctx.push_value_type(instance);
    StepResult::Continue
}

pub fn handle_get_namespace<'gc, T: TypedStackOps<'gc> + ReflectionOps<'gc>>(
    ctx: &mut T,
    _generics: &GenericLookup,
) -> StepResult {
    let obj = ctx.pop_obj();
    let target_type = ctx.resolve_runtime_type(obj);
    match target_type {
        RuntimeType::Type(td) | RuntimeType::Generic(td, _) => {
            match td.definition().namespace.as_ref() {
                None => ctx.push(StackValue::null()),
                Some(n) => ctx.push_string(n.clone().into()),
            }
        }
        _ => ctx.push_string("System".into()),
    }
    StepResult::Continue
}

pub fn handle_is_primitive_impl<'gc, T: TypedStackOps<'gc> + ReflectionOps<'gc>>(
    ctx: &mut T,
    _generics: &GenericLookup,
) -> StepResult {
    let obj = ctx.pop_obj();
    let rt = ctx.resolve_runtime_type(obj);
    let is_primitive = matches!(
        rt,
        RuntimeType::Boolean
            | RuntimeType::Char
            | RuntimeType::Int8
            | RuntimeType::UInt8
            | RuntimeType::Int16
            | RuntimeType::UInt16
            | RuntimeType::Int32
            | RuntimeType::UInt32
            | RuntimeType::Int64
            | RuntimeType::UInt64
            | RuntimeType::Float32
            | RuntimeType::Float64
            | RuntimeType::IntPtr
            | RuntimeType::UIntPtr
    );
    ctx.push_i32(if is_primitive { 1 } else { 0 });
    StepResult::Continue
}

pub fn handle_is_array_impl<'gc, T: TypedStackOps<'gc> + ReflectionOps<'gc>>(
    ctx: &mut T,
    _generics: &GenericLookup,
) -> StepResult {
    let obj = ctx.pop_obj();
    let rt = ctx.resolve_runtime_type(obj);
    let is_array = matches!(rt, RuntimeType::Vector(_) | RuntimeType::Array(_, _));
    ctx.push_i32(if is_array { 1 } else { 0 });
    StepResult::Continue
}

pub fn handle_is_by_ref_impl<'gc, T: TypedStackOps<'gc> + ReflectionOps<'gc>>(
    ctx: &mut T,
    _generics: &GenericLookup,
) -> StepResult {
    let obj = ctx.pop_obj();
    let rt = ctx.resolve_runtime_type(obj);
    let is_by_ref = matches!(rt, RuntimeType::ByRef(_));
    ctx.push_i32(if is_by_ref { 1 } else { 0 });
    StepResult::Continue
}

pub fn handle_is_pointer_impl<'gc, T: TypedStackOps<'gc> + ReflectionOps<'gc>>(
    ctx: &mut T,
    _generics: &GenericLookup,
) -> StepResult {
    let obj = ctx.pop_obj();
    let rt = ctx.resolve_runtime_type(obj);
    let is_pointer = matches!(
        rt,
        RuntimeType::Pointer(_) | RuntimeType::ValuePointer(_, _)
    );
    ctx.push_i32(if is_pointer { 1 } else { 0 });
    StepResult::Continue
}

pub fn handle_get_interfaces<
    'gc,
    T: TypedStackOps<'gc> + ReflectionOps<'gc> + LoaderOps + ResolutionOps<'gc> + MemoryOps<'gc>,
>(
    ctx: &mut T,
    _generics: &GenericLookup,
) -> StepResult {
    let obj = ctx.pop_obj();
    let rt = ctx.resolve_runtime_type(obj);

    let mut interfaces = vec![];
    let mut seen = HashSet::new();

    let target_type = match rt {
        RuntimeType::Type(_) | RuntimeType::Generic(_, _) => rt,
        _ => {
            let ct = rt.to_concrete(ctx.loader().as_ref());
            match ctx.loader().find_concrete_type(ct) {
                Ok(td) => RuntimeType::Type(td),
                Err(_) => {
                    let type_type = vm_try!(ctx.loader().corlib_type("System.Type"));
                    return populate_reflection_array(ctx, vec![], type_type.into());
                }
            }
        }
    };

    if let RuntimeType::Type(td) | RuntimeType::Generic(td, _) = &target_type {
        let mut queue = VecDeque::new();
        let lookup = build_generic_lookup_from_runtime_type(ctx, &target_type);
        queue.push_back((td.clone(), lookup));

        while let Some((curr_td, curr_lookup)) = queue.pop_front() {
            // Check interfaces implemented by this type
            for (_, interface_source) in &curr_td.definition().implements {
                let method_type = super::member_to_method_type(interface_source);
                let resolved_interface =
                    ctx.make_runtime_type(&ctx.with_generics(&curr_lookup), &method_type);

                if seen.insert(resolved_interface.clone()) {
                    interfaces.push(ctx.get_runtime_type(resolved_interface));
                }
            }

            // Check base type
            if let Some(base_source) = &curr_td.definition().extends {
                let method_type = super::member_to_method_type(base_source);
                let base_rt = ctx.make_runtime_type(&ctx.with_generics(&curr_lookup), &method_type);
                if let RuntimeType::Type(base_td) | RuntimeType::Generic(base_td, _) = &base_rt {
                    let base_lookup = build_generic_lookup_from_runtime_type(ctx, &base_rt);
                    queue.push_back((base_td.clone(), base_lookup));
                }
            }
        }
    }

    let type_type = vm_try!(ctx.loader().corlib_type("System.Type"));
    populate_reflection_array(ctx, interfaces, type_type.into())
}

pub fn handle_get_interface<
    'gc,
    T: TypedStackOps<'gc> + ReflectionOps<'gc> + LoaderOps + ResolutionOps<'gc>,
>(
    ctx: &mut T,
    _generics: &GenericLookup,
) -> StepResult {
    let ignore_case = ctx.pop_i32() != 0;
    let name_obj = ctx.pop_obj();
    let obj = ctx.pop_obj();

    let name = name_obj.as_heap_storage(|s| {
        if let HeapStorage::Str(s) = s {
            s.as_string()
        } else {
            panic!("name is not a string")
        }
    });

    let rt = ctx.resolve_runtime_type(obj);
    let mut seen = HashSet::new();
    let mut queue = VecDeque::new();

    let target_type = match rt {
        RuntimeType::Type(_) | RuntimeType::Generic(_, _) => rt,
        _ => {
            let ct = rt.to_concrete(ctx.loader().as_ref());
            match ctx.loader().find_concrete_type(ct) {
                Ok(td) => RuntimeType::Type(td),
                Err(_) => {
                    ctx.push(StackValue::null());
                    return StepResult::Continue;
                }
            }
        }
    };

    let mut found = None;
    if let RuntimeType::Type(td) | RuntimeType::Generic(td, _) = &target_type {
        let lookup = build_generic_lookup_from_runtime_type(ctx, &target_type);
        queue.push_back((td.clone(), lookup));

        'outer: while let Some((curr_td, curr_lookup)) = queue.pop_front() {
            for (_, interface_source) in &curr_td.definition().implements {
                let method_type = super::member_to_method_type(interface_source);
                let resolved_interface =
                    ctx.make_runtime_type(&ctx.with_generics(&curr_lookup), &method_type);

                if seen.insert(resolved_interface.clone()) {
                    let itf_name = resolved_interface.get_name();
                    let matches = if ignore_case {
                        itf_name.eq_ignore_ascii_case(&name)
                    } else {
                        itf_name == name
                    };

                    if matches {
                        found = Some(resolved_interface);
                        break 'outer;
                    }

                    if let RuntimeType::Type(itf_td) | RuntimeType::Generic(itf_td, _) =
                        &resolved_interface
                    {
                        let itf_lookup =
                            build_generic_lookup_from_runtime_type(ctx, &resolved_interface);
                        queue.push_back((itf_td.clone(), itf_lookup));
                    }
                }
            }

            if let Some(base_source) = &curr_td.definition().extends {
                let method_type = super::member_to_method_type(base_source);
                let base_rt = ctx.make_runtime_type(&ctx.with_generics(&curr_lookup), &method_type);
                if let RuntimeType::Type(base_td) | RuntimeType::Generic(base_td, _) = &base_rt {
                    let base_lookup = build_generic_lookup_from_runtime_type(ctx, &base_rt);
                    queue.push_back((base_td.clone(), base_lookup));
                }
            }
        }
    }

    if let Some(itf) = found {
        let rt_obj = ctx.get_runtime_type(itf);
        ctx.push_obj(rt_obj);
    } else {
        ctx.push(StackValue::null());
    }
    StepResult::Continue
}

pub fn handle_get_element_type<'gc, T: TypedStackOps<'gc> + ReflectionOps<'gc>>(
    ctx: &mut T,
    _generics: &GenericLookup,
) -> StepResult {
    let obj = ctx.pop_obj();
    let rt = ctx.resolve_runtime_type(obj);
    let element_type = match rt {
        RuntimeType::Vector(t)
        | RuntimeType::Array(t, _)
        | RuntimeType::Pointer(t)
        | RuntimeType::ByRef(t)
        | RuntimeType::ValuePointer(t, _) => Some(*t),
        _ => None,
    };
    if let Some(et) = element_type {
        let rt_obj = ctx.get_runtime_type(et);
        ctx.push_obj(rt_obj);
    } else {
        ctx.push(StackValue::null());
    }
    StepResult::Continue
}

pub fn handle_get_attribute_flags_impl<
    'gc,
    T: TypedStackOps<'gc> + ReflectionOps<'gc> + LoaderOps,
>(
    ctx: &mut T,
    _generics: &GenericLookup,
) -> StepResult {
    let obj = ctx.pop_obj();
    let rt = ctx.resolve_runtime_type(obj);

    let target_type = match rt {
        RuntimeType::Type(_) | RuntimeType::Generic(_, _) => rt,
        _ => {
            let ct = rt.to_concrete(ctx.loader().as_ref());
            match ctx.loader().find_concrete_type(ct) {
                Ok(td) => RuntimeType::Type(td),
                Err(_) => {
                    ctx.push_i32(0);
                    return StepResult::Continue;
                }
            }
        }
    };

    let mut attrs = 0u32;
    if let RuntimeType::Type(td) | RuntimeType::Generic(td, _) = target_type {
        let flags = td.definition().flags;

        attrs |= match flags.accessibility {
            TypeAccessibility::NotPublic => 0,
            TypeAccessibility::Public => 1,
            TypeAccessibility::Nested(acc) => match acc {
                dotnetdll::prelude::Accessibility::Public => 2,
                dotnetdll::prelude::Accessibility::Private => 3,
                dotnetdll::prelude::Accessibility::Family => 4,
                dotnetdll::prelude::Accessibility::Assembly => 5,
                dotnetdll::prelude::Accessibility::FamilyANDAssembly => 6,
                dotnetdll::prelude::Accessibility::FamilyORAssembly => 7,
            },
        };

        attrs |= match flags.layout {
            dotnetdll::prelude::Layout::Automatic => 0x00,
            dotnetdll::prelude::Layout::Sequential(_) => 0x08,
            dotnetdll::prelude::Layout::Explicit(_) => 0x10,
        };

        attrs |= match flags.kind {
            dotnetdll::prelude::Kind::Class => 0x00,
            dotnetdll::prelude::Kind::Interface => 0x20,
        };

        if flags.abstract_type {
            attrs |= 0x80;
        }
        if flags.sealed {
            attrs |= 0x100;
        }
        if flags.special_name {
            attrs |= 0x400;
        }
        if flags.imported {
            attrs |= 0x1000;
        }
        if flags.serializable {
            attrs |= 0x2000;
        }

        attrs |= match flags.string_formatting {
            dotnetdll::prelude::StringFormatting::ANSI => 0x00000,
            dotnetdll::prelude::StringFormatting::Unicode => 0x10000,
            dotnetdll::prelude::StringFormatting::Automatic => 0x20000,
            dotnetdll::prelude::StringFormatting::Custom(_) => 0x30000,
        };

        if td.definition().security.is_some() {
            attrs |= 0x40000;
        }
        if flags.before_field_init {
            attrs |= 0x100000;
        }
        if flags.runtime_special_name {
            attrs |= 0x800;
        }
    }

    ctx.push_i32(attrs as i32);
    StepResult::Continue
}

pub fn handle_get_name<'gc, T: TypedStackOps<'gc> + ReflectionOps<'gc>>(
    ctx: &mut T,
    _generics: &GenericLookup,
) -> StepResult {
    let obj = ctx.pop_obj();
    let target_type = ctx.resolve_runtime_type(obj);
    ctx.push_string(target_type.get_name().into());
    StepResult::Continue
}

pub fn handle_get_base_type<
    'gc,
    T: TypedStackOps<'gc> + ReflectionOps<'gc> + LoaderOps + ResolutionOps<'gc>,
>(
    ctx: &mut T,
    _generics: &GenericLookup,
) -> StepResult {
    let obj = ctx.pop_obj();
    let target_type = ctx.resolve_runtime_type(obj);
    match target_type {
        RuntimeType::Type(ref td) | RuntimeType::Generic(ref td, _) => {
            if let Some(base) = &td.definition().extends {
                let lookup = build_generic_lookup_from_runtime_type(ctx, &target_type);
                let method_type = super::member_to_method_type(base);
                let base_rt = ctx.make_runtime_type(&ctx.with_generics(&lookup), &method_type);
                let rt_obj = ctx.get_runtime_type(base_rt);
                ctx.push_obj(rt_obj);
            } else {
                ctx.push(StackValue::null());
            }
        }
        _ => ctx.push(StackValue::null()),
    }
    StepResult::Continue
}

pub fn handle_get_is_generic_type<'gc, T: TypedStackOps<'gc> + ReflectionOps<'gc>>(
    ctx: &mut T,
    _generics: &GenericLookup,
) -> StepResult {
    let obj = ctx.pop_obj();
    let target_type = ctx.resolve_runtime_type(obj);
    let is_generic = matches!(target_type, RuntimeType::Generic(_, _));
    ctx.push_i32(if is_generic { 1 } else { 0 });
    StepResult::Continue
}

pub fn handle_get_generic_type_definition<
    'gc,
    T: TypedStackOps<'gc> + ReflectionOps<'gc> + ExceptionOps<'gc>,
>(
    ctx: &mut T,
    _generics: &GenericLookup,
) -> StepResult {
    let obj = ctx.pop_obj();
    let target_type = ctx.resolve_runtime_type(obj);
    match target_type {
        RuntimeType::Generic(td, _) => {
            let n_params = td.definition().generic_parameters.len();
            let params = (0..n_params as u16)
                .map(|index| RuntimeType::TypeParameter {
                    owner: td.clone(),
                    index,
                })
                .collect();
            let def_rt = RuntimeType::Generic(td, params);
            let rt_obj = ctx.get_runtime_type(def_rt);
            ctx.push_obj(rt_obj);
            StepResult::Continue
        }
        _ => ctx.throw_by_name_with_message(
            "System.InvalidOperationException",
            "This operation is only valid on generic types.",
        ),
    }
}

pub fn handle_get_generic_arguments<
    'gc,
    T: TypedStackOps<'gc> + ReflectionOps<'gc> + LoaderOps + MemoryOps<'gc> + RawMemoryOps<'gc>,
>(
    ctx: &mut T,
    _generics: &GenericLookup,
) -> StepResult {
    let gc = ctx.gc_with_token(&dotnet_utils::NoActiveBorrows::new());
    let obj = ctx.pop_obj();
    let target_type = ctx.resolve_runtime_type(obj);
    let args = match target_type {
        RuntimeType::Generic(_, args) => args.clone(),
        _ => vec![],
    };

    // Check GC safe point before allocating type array
    if ctx.check_gc_safe_point() {
        return StepResult::Yield;
    }

    let type_type_td = vm_try!(ctx.loader().corlib_type("System.Type"));
    let type_type = ConcreteType::from(type_type_td);
    let mut vector = vm_try!(ctx.new_vector(type_type, args.len()));
    for (i, (arg, chunk)) in args
        .into_iter()
        .zip(vector.get_mut().chunks_exact_mut(ObjectRef::SIZE))
        .enumerate()
    {
        // Check GC safe point periodically during loops with allocations
        // Check every 16 iterations
        if i % 16 == 0 && ctx.check_gc_safe_point() {
            return StepResult::Yield;
        }
        let arg_obj = ctx.get_runtime_type(arg);
        arg_obj.write(chunk);
    }
    let obj = ObjectRef::new(gc, HeapStorage::Vec(vector));
    ctx.register_new_object(&obj);
    ctx.push_obj(obj);
    StepResult::Continue
}

pub fn handle_get_type_handle<
    'gc,
    T: TypedStackOps<'gc> + LoaderOps + ResolutionOps<'gc> + MemoryOps<'gc>,
>(
    ctx: &mut T,
    _generics: &GenericLookup,
) -> StepResult {
    let obj = ctx.pop_obj();

    let rth = vm_try!(ctx.loader().corlib_type("System.RuntimeTypeHandle"));
    let instance = vm_try!(ctx.current_context().new_object(rth.clone()));
    obj.write(&mut instance.instance_storage.get_field_mut_local(rth, "_value"));

    ctx.push(StackValue::ValueType(instance));
    StepResult::Continue
}

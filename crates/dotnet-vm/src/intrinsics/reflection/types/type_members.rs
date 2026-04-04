use crate::{
    StepResult,
    intrinsics::reflection::types::{
        build_generic_lookup_from_runtime_type, populate_reflection_array,
    },
    stack::ops::{
        EvalStackOps, ExceptionOps, LoaderOps, MemoryOps, ReflectionOps, ResolutionOps,
        TypedStackOps,
    },
};
use dotnet_assemblies::SUPPORT_ASSEMBLY;
use dotnet_macros::dotnet_intrinsic;
use dotnet_types::{
    TypeDescription,
    generics::{ConcreteType, GenericLookup},
    members::MethodDescription,
    runtime::RuntimeType,
};
use dotnet_value::{
    StackValue,
    object::{HeapStorage, ObjectRef},
};
use dotnetdll::prelude::{Accessibility, MemberAccessibility, MethodMemberIndex, TypeDefinition};

#[dotnet_intrinsic("object[] System.Reflection.Assembly::GetCustomAttributes(System.Type, bool)")]
pub fn intrinsic_assembly_get_custom_attributes<
    'gc,
    T: MemoryOps<'gc> + LoaderOps + EvalStackOps<'gc> + TypedStackOps<'gc> + ExceptionOps<'gc>,
>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let gc = ctx.gc_with_token(&ctx.no_active_borrows_token());
    let num_args = _method.method().signature.parameters.len()
        + if _method.method().signature.instance {
            1
        } else {
            0
        };
    for _ in 0..num_args {
        let _ = ctx.pop();
    }

    // Return an empty array of Attribute
    let attribute_type = vm_try!(ctx.loader().corlib_type("System.Attribute"));
    let array = vm_try!(ctx.new_vector(attribute_type.into(), 0));
    let obj = ObjectRef::new(gc, HeapStorage::Vec(array));
    ctx.register_new_object(&obj);
    ctx.push_obj(obj);

    StepResult::Continue
}

#[dotnet_intrinsic(
    "static System.Attribute[] System.Attribute::GetCustomAttributes(System.Reflection.MemberInfo, System.Type, bool)"
)]
pub fn intrinsic_attribute_get_custom_attributes<
    'gc,
    T: MemoryOps<'gc> + LoaderOps + EvalStackOps<'gc> + TypedStackOps<'gc> + ExceptionOps<'gc>,
>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let gc = ctx.gc_with_token(&ctx.no_active_borrows_token());
    let num_args = _method.method().signature.parameters.len()
        + if _method.method().signature.instance {
            1
        } else {
            0
        };
    for _ in 0..num_args {
        let _ = ctx.pop();
    }

    // Return an empty array of Attribute
    let attribute_type = vm_try!(ctx.loader().corlib_type("System.Attribute"));
    let array = vm_try!(ctx.new_vector(attribute_type.into(), 0));
    let obj = ObjectRef::new(gc, HeapStorage::Vec(array));
    ctx.register_new_object(&obj);
    ctx.push_obj(obj);

    StepResult::Continue
}

pub fn handle_get_assembly<
    'gc,
    T: MemoryOps<'gc> + TypedStackOps<'gc> + ReflectionOps<'gc> + LoaderOps + ExceptionOps<'gc>,
>(
    ctx: &mut T,
    _generics: &GenericLookup,
) -> StepResult {
    let gc = ctx.gc_with_token(&ctx.no_active_borrows_token());
    let obj = ctx.pop_obj();

    let target_type = ctx.resolve_runtime_type(obj);
    let resolution = target_type.resolution(ctx.loader().as_ref());

    let cached_asm = ctx.reflection().asms_read().get(&resolution).copied();
    if let Some(o) = cached_asm {
        ctx.push_obj(o);
        return StepResult::Continue;
    }

    let support_res = ctx
        .loader()
        .get_assembly(SUPPORT_ASSEMBLY)
        .expect("support library must be loadable");
    let (index, _definition) = support_res
        .definition()
        .type_definitions
        .iter()
        .enumerate()
        .find(|(_, a): &(usize, &TypeDefinition)| a.type_name() == "DotnetRs.Assembly")
        .expect("could find DotnetRs.Assembly in support library");
    let type_index = support_res.type_definition_index(index).unwrap();
    let asm_handle = vm_try!(ctx.new_object(TypeDescription::new(support_res, type_index)));
    asm_handle
        .instance_storage
        .field::<usize>(asm_handle.description.clone(), "resolution")
        .unwrap()
        .write(resolution.as_raw() as usize);
    let v = ObjectRef::new(gc, HeapStorage::Obj(asm_handle));
    ctx.register_new_object(&v);

    ctx.reflection().asms_write().insert(resolution, v);
    ctx.push_obj(v);
    StepResult::Continue
}

pub fn handle_get_methods<
    'gc,
    T: TypedStackOps<'gc>
        + ReflectionOps<'gc>
        + LoaderOps
        + ResolutionOps<'gc>
        + MemoryOps<'gc>
        + ExceptionOps<'gc>,
>(
    ctx: &mut T,
    _generics: &GenericLookup,
) -> StepResult {
    let flags = ctx.pop_i32();
    let obj = ctx.pop_obj();
    let target_type = ctx.resolve_runtime_type(obj);

    const BINDING_FLAGS_INSTANCE: i32 = 4;
    const BINDING_FLAGS_STATIC: i32 = 8;
    const BINDING_FLAGS_PUBLIC: i32 = 16;
    const BINDING_FLAGS_NON_PUBLIC: i32 = 32;

    let mut methods_objs = Vec::new();
    if let RuntimeType::Type(ref td) | RuntimeType::Generic(ref td, _) = target_type {
        for (idx, m) in td.definition().methods.iter().enumerate() {
            let is_public = m.accessibility == MemberAccessibility::Access(Accessibility::Public);
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
                let lookup = build_generic_lookup_from_runtime_type(ctx, &target_type);
                let desc = MethodDescription::new(
                    td.clone(),
                    lookup.clone(),
                    td.resolution.clone(),
                    MethodMemberIndex::Method(idx),
                );
                methods_objs.push(ctx.get_runtime_method_obj(desc, lookup));
            }
        }
    }

    let method_info_type = vm_try!(ctx.loader().corlib_type("System.Reflection.MethodInfo"));
    populate_reflection_array(ctx, methods_objs, ConcreteType::from(method_info_type))
}

pub fn handle_get_method_impl<
    'gc,
    T: EvalStackOps<'gc>
        + TypedStackOps<'gc>
        + ReflectionOps<'gc>
        + LoaderOps
        + ResolutionOps<'gc>
        + MemoryOps<'gc>
        + ExceptionOps<'gc>,
>(
    ctx: &mut T,
    _generics: &GenericLookup,
) -> StepResult {
    let _modifiers = ctx.pop();
    let _types = ctx.pop(); // TODO: match by parameter types
    let _call_convention = ctx.pop();
    let _binder = ctx.pop();
    let flags = ctx.pop_i32();
    let name_obj = ctx.pop_obj();
    let obj = ctx.pop_obj();

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
    if let RuntimeType::Type(ref td) | RuntimeType::Generic(ref td, _) = target_type {
        for (idx, m) in td.definition().methods.iter().enumerate() {
            if m.name != name {
                continue;
            }
            let is_public = m.accessibility == MemberAccessibility::Access(Accessibility::Public);
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
                let lookup = build_generic_lookup_from_runtime_type(ctx, &target_type);
                let desc = MethodDescription::new(
                    td.clone(),
                    lookup.clone(),
                    td.resolution.clone(),
                    MethodMemberIndex::Method(idx),
                );
                found_method = Some(ctx.get_runtime_method_obj(desc, lookup));
                break;
            }
        }
    }

    if let Some(m) = found_method {
        ctx.push_obj(m);
    } else {
        ctx.push(StackValue::null());
    }
    StepResult::Continue
}

pub fn handle_get_constructor_impl<
    'gc,
    T: EvalStackOps<'gc>
        + TypedStackOps<'gc>
        + ReflectionOps<'gc>
        + LoaderOps
        + ResolutionOps<'gc>
        + MemoryOps<'gc>
        + ExceptionOps<'gc>,
>(
    ctx: &mut T,
    _generics: &GenericLookup,
) -> StepResult {
    let _modifiers = ctx.pop();
    let _types = ctx.pop(); // TODO: match by parameter types
    let _call_convention = ctx.pop();
    let _binder = ctx.pop();
    let flags = ctx.pop_i32();
    let obj = ctx.pop_obj();

    let target_type = ctx.resolve_runtime_type(obj);

    const BINDING_FLAGS_INSTANCE: i32 = 4;
    const BINDING_FLAGS_STATIC: i32 = 8;
    const BINDING_FLAGS_PUBLIC: i32 = 16;
    const BINDING_FLAGS_NON_PUBLIC: i32 = 32;

    let mut found_constructor = None;
    if let RuntimeType::Type(ref td) | RuntimeType::Generic(ref td, _) = target_type {
        for (idx, m) in td.definition().methods.iter().enumerate() {
            if m.name != ".ctor" {
                continue;
            }
            let is_public = m.accessibility == MemberAccessibility::Access(Accessibility::Public);
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
                let lookup = build_generic_lookup_from_runtime_type(ctx, &target_type);
                let desc = MethodDescription::new(
                    td.clone(),
                    lookup.clone(),
                    td.resolution.clone(),
                    MethodMemberIndex::Method(idx),
                );
                found_constructor = Some(ctx.get_runtime_method_obj(desc, lookup));
                break;
            }
        }
    }

    if let Some(m) = found_constructor {
        ctx.push_obj(m);
    } else {
        ctx.push(StackValue::null());
    }
    StepResult::Continue
}

pub fn handle_get_members<
    'gc,
    T: TypedStackOps<'gc> + LoaderOps + MemoryOps<'gc> + ExceptionOps<'gc>,
>(
    ctx: &mut T,
    _generics: &GenericLookup,
) -> StepResult {
    let _flags = ctx.pop_i32();
    let _obj = ctx.pop_obj();
    let member_info_type = vm_try!(ctx.loader().corlib_type("System.Reflection.MemberInfo"));
    populate_reflection_array(ctx, vec![], ConcreteType::from(member_info_type))
}

pub fn handle_get_nested_types<
    'gc,
    T: TypedStackOps<'gc> + LoaderOps + MemoryOps<'gc> + ExceptionOps<'gc>,
>(
    ctx: &mut T,
    _generics: &GenericLookup,
) -> StepResult {
    let _flags = ctx.pop_i32();
    let _obj = ctx.pop_obj();
    let type_type = vm_try!(ctx.loader().corlib_type("System.Type"));
    populate_reflection_array(ctx, vec![], type_type.into())
}

pub fn handle_invoke_member<'gc, T: ExceptionOps<'gc> + EvalStackOps<'gc> + TypedStackOps<'gc>>(
    ctx: &mut T,
    _generics: &GenericLookup,
) -> StepResult {
    let _named_params = ctx.pop();
    let _culture = ctx.pop();
    let _modifiers = ctx.pop();
    let _args = ctx.pop();
    let _target = ctx.pop();
    let _binder = ctx.pop();
    let _invoke_attr = ctx.pop();
    let _name = ctx.pop();
    let _obj = ctx.pop_obj();

    ctx.throw_by_name_with_message(
        "System.NotSupportedException",
        "InvokeMember is not supported.",
    )
}

pub fn handle_get_fields<
    'gc,
    T: TypedStackOps<'gc>
        + ReflectionOps<'gc>
        + LoaderOps
        + ResolutionOps<'gc>
        + MemoryOps<'gc>
        + ExceptionOps<'gc>,
>(
    ctx: &mut T,
    _generics: &GenericLookup,
) -> StepResult {
    let flags = ctx.pop_i32();
    let obj = ctx.pop_obj();
    let target_type = ctx.resolve_runtime_type(obj);

    const BINDING_FLAGS_INSTANCE: i32 = 4;
    const BINDING_FLAGS_STATIC: i32 = 8;
    const BINDING_FLAGS_PUBLIC: i32 = 16;
    const BINDING_FLAGS_NON_PUBLIC: i32 = 32;

    let mut fields_objs = Vec::new();
    if let RuntimeType::Type(ref td) | RuntimeType::Generic(ref td, _) = target_type {
        for (index, f) in td.definition().fields.iter().enumerate() {
            let is_public = f.accessibility == MemberAccessibility::Access(Accessibility::Public);
            let is_static = f.static_member;

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
                let desc = dotnet_types::members::FieldDescription::new(
                    td.clone(),
                    td.resolution.clone(),
                    index,
                );
                let lookup = build_generic_lookup_from_runtime_type(ctx, &target_type);
                fields_objs.push(ctx.get_runtime_field_obj(desc, lookup));
            }
        }
    }

    let field_info_type = vm_try!(ctx.loader().corlib_type("System.Reflection.FieldInfo"));
    populate_reflection_array(ctx, fields_objs, ConcreteType::from(field_info_type))
}

pub fn handle_get_field<
    'gc,
    T: TypedStackOps<'gc>
        + ReflectionOps<'gc>
        + LoaderOps
        + ResolutionOps<'gc>
        + MemoryOps<'gc>
        + ExceptionOps<'gc>,
>(
    ctx: &mut T,
    _generics: &GenericLookup,
) -> StepResult {
    let flags = ctx.pop_i32();
    let name_obj = ctx.pop_obj();
    let obj = ctx.pop_obj();

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

    let mut found_field = None;
    if let RuntimeType::Type(ref td) | RuntimeType::Generic(ref td, _) = target_type {
        for (index, f) in td.definition().fields.iter().enumerate() {
            if f.name != name {
                continue;
            }
            let is_public = f.accessibility == MemberAccessibility::Access(Accessibility::Public);
            let is_static = f.static_member;

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
                let desc = dotnet_types::members::FieldDescription::new(
                    td.clone(),
                    td.resolution.clone(),
                    index,
                );
                let lookup = build_generic_lookup_from_runtime_type(ctx, &target_type);
                found_field = Some(ctx.get_runtime_field_obj(desc, lookup));
                break;
            }
        }
    }

    if let Some(f) = found_field {
        ctx.push_obj(f);
    } else {
        ctx.push(StackValue::null());
    }
    StepResult::Continue
}

pub fn handle_get_properties<
    'gc,
    T: TypedStackOps<'gc> + LoaderOps + MemoryOps<'gc> + ExceptionOps<'gc>,
>(
    ctx: &mut T,
    _generics: &GenericLookup,
) -> StepResult {
    let _flags = ctx.pop_i32();
    let _obj = ctx.pop_obj();
    let property_info_type = vm_try!(ctx.loader().corlib_type("System.Reflection.PropertyInfo"));
    populate_reflection_array(ctx, vec![], ConcreteType::from(property_info_type))
}

pub fn handle_get_property_impl<'gc, T: TypedStackOps<'gc>>(
    ctx: &mut T,
    _generics: &GenericLookup,
) -> StepResult {
    let _modifiers = ctx.pop();
    let _types = ctx.pop();
    let _return_type = ctx.pop();
    let _binder = ctx.pop();
    let _flags = ctx.pop();
    let _name = ctx.pop();
    let _obj = ctx.pop_obj();
    ctx.push(StackValue::null());
    StepResult::Continue
}

pub fn handle_get_constructors<
    'gc,
    T: TypedStackOps<'gc>
        + ReflectionOps<'gc>
        + LoaderOps
        + ResolutionOps<'gc>
        + MemoryOps<'gc>
        + ExceptionOps<'gc>,
>(
    ctx: &mut T,
    _generics: &GenericLookup,
) -> StepResult {
    let flags = ctx.pop_i32();
    let obj = ctx.pop_obj();
    let target_type = ctx.resolve_runtime_type(obj);

    const BINDING_FLAGS_INSTANCE: i32 = 4;
    const BINDING_FLAGS_STATIC: i32 = 8;
    const BINDING_FLAGS_PUBLIC: i32 = 16;
    const BINDING_FLAGS_NON_PUBLIC: i32 = 32;

    let mut constructors_objs = Vec::new();
    if let RuntimeType::Type(ref td) | RuntimeType::Generic(ref td, _) = target_type {
        for (idx, m) in td.definition().methods.iter().enumerate() {
            if m.name != ".ctor" {
                continue;
            }
            let is_public = m.accessibility == MemberAccessibility::Access(Accessibility::Public);
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
                let lookup = build_generic_lookup_from_runtime_type(ctx, &target_type);
                let desc = MethodDescription::new(
                    td.clone(),
                    lookup.clone(),
                    td.resolution.clone(),
                    MethodMemberIndex::Method(idx),
                );
                constructors_objs.push(ctx.get_runtime_method_obj(desc, lookup));
            }
        }
    }

    let constructor_info_type = vm_try!(
        ctx.loader()
            .corlib_type("System.Reflection.ConstructorInfo")
    );
    populate_reflection_array(
        ctx,
        constructors_objs,
        ConcreteType::from(constructor_info_type),
    )
}

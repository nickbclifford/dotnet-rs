use crate::{
    ReflectionIntrinsicHost,
    types::{build_generic_lookup_from_runtime_type, populate_reflection_array},
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
use dotnet_vm_ops::{
    StepResult,
    ops::{EvalStackOps, ExceptionOps, LoaderOps, MemoryOps, TypedStackOps},
};
use dotnetdll::prelude::{Accessibility, MemberAccessibility, MethodMemberIndex, TypeDefinition};

#[dotnet_intrinsic("object[] System.Reflection.Assembly::GetCustomAttributes(System.Type, bool)")]
pub fn intrinsic_assembly_get_custom_attributes<
    'gc,
    T: TypedStackOps<'gc> + MemoryOps<'gc> + LoaderOps,
>(
    ctx: &mut T,
    method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let gc = ctx.gc_with_token(&ctx.no_active_borrows_token());
    let num_args = method.method().signature.parameters.len()
        + if method.method().signature.instance {
            1
        } else {
            0
        };
    for _ in 0..num_args {
        let _ = ctx.pop();
    }

    let attribute_type = dotnet_vm_ops::vm_try!(ctx.loader().corlib_type("System.Attribute"));
    let array = dotnet_vm_ops::vm_try!(ctx.new_vector(attribute_type.into(), 0));
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
    T: TypedStackOps<'gc> + MemoryOps<'gc> + LoaderOps,
>(
    ctx: &mut T,
    method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let gc = ctx.gc_with_token(&ctx.no_active_borrows_token());
    let num_args = method.method().signature.parameters.len()
        + if method.method().signature.instance {
            1
        } else {
            0
        };
    for _ in 0..num_args {
        let _ = ctx.pop();
    }

    let attribute_type = dotnet_vm_ops::vm_try!(ctx.loader().corlib_type("System.Attribute"));
    let array = dotnet_vm_ops::vm_try!(ctx.new_vector(attribute_type.into(), 0));
    let obj = ObjectRef::new(gc, HeapStorage::Vec(array));
    ctx.register_new_object(&obj);
    ctx.push_obj(obj);

    StepResult::Continue
}

pub fn handle_get_assembly<'gc, T: ReflectionIntrinsicHost<'gc>>(
    ctx: &mut T,
    _generics: &GenericLookup,
) -> StepResult {
    let gc = ctx.gc_with_token(&ctx.no_active_borrows_token());
    let obj = ctx.pop_obj();

    let target_type = crate::common::resolve_runtime_type(ctx, obj);
    let resolution = target_type.resolution(ctx.loader().as_ref());

    if let Some(o) = ctx.reflection_cached_runtime_assembly(resolution.clone()) {
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
    let asm_handle =
        dotnet_vm_ops::vm_try!(ctx.new_object(TypeDescription::new(support_res, type_index)));
    asm_handle
        .instance_storage
        .field::<usize>(asm_handle.description.clone(), "resolution")
        .unwrap()
        .write(resolution.as_raw() as usize);
    let v = ObjectRef::new(gc, HeapStorage::Obj(asm_handle));
    ctx.register_new_object(&v);

    ctx.reflection_cache_runtime_assembly(resolution, v);
    ctx.push_obj(v);
    StepResult::Continue
}

pub fn handle_get_custom_attributes_bool<'gc, T: ReflectionIntrinsicHost<'gc>>(
    ctx: &mut T,
    _generics: &GenericLookup,
) -> StepResult {
    let _inherit = ctx.pop();
    let _obj = ctx.pop_obj();
    let object_type = dotnet_vm_ops::vm_try!(ctx.loader().corlib_type("System.Object"));
    populate_reflection_array(ctx, Vec::new(), ConcreteType::from(object_type))
}

pub fn handle_get_custom_attributes_typed<'gc, T: ReflectionIntrinsicHost<'gc>>(
    ctx: &mut T,
    _generics: &GenericLookup,
) -> StepResult {
    let _inherit = ctx.pop();
    let _attribute_type = ctx.pop_obj();
    let _obj = ctx.pop_obj();
    let object_type = dotnet_vm_ops::vm_try!(ctx.loader().corlib_type("System.Object"));
    populate_reflection_array(ctx, Vec::new(), ConcreteType::from(object_type))
}

pub fn handle_get_methods<'gc, T: ReflectionIntrinsicHost<'gc>>(
    ctx: &mut T,
    _generics: &GenericLookup,
) -> StepResult {
    let flags = ctx.pop_i32();
    let obj = ctx.pop_obj();
    let target_type = crate::common::resolve_runtime_type(ctx, obj);

    const BINDING_FLAGS_INSTANCE: i32 = 4;
    const BINDING_FLAGS_STATIC: i32 = 8;
    const BINDING_FLAGS_PUBLIC: i32 = 16;
    const BINDING_FLAGS_NON_PUBLIC: i32 = 32;

    let mut methods_objs = Vec::new();
    if let RuntimeType::Type(ref td) | RuntimeType::Generic(ref td, _) = target_type {
        let lookup = build_generic_lookup_from_runtime_type(ctx, &target_type);
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
                let desc = MethodDescription::new(
                    td.clone(),
                    lookup.clone(),
                    td.resolution.clone(),
                    MethodMemberIndex::Method(idx),
                );
                methods_objs.push(crate::common::get_runtime_method_obj(
                    ctx,
                    desc,
                    lookup.clone(),
                ));
            }
        }
    }

    let method_info_type =
        dotnet_vm_ops::vm_try!(ctx.loader().corlib_type("System.Reflection.MethodInfo"));
    populate_reflection_array(ctx, methods_objs, ConcreteType::from(method_info_type))
}

pub fn handle_get_method_impl<'gc, T: ReflectionIntrinsicHost<'gc>>(
    ctx: &mut T,
    _generics: &GenericLookup,
) -> StepResult {
    let _modifiers = ctx.pop();
    let _types = ctx.pop();
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
    let target_type = crate::common::resolve_runtime_type(ctx, obj);

    const BINDING_FLAGS_INSTANCE: i32 = 4;
    const BINDING_FLAGS_STATIC: i32 = 8;
    const BINDING_FLAGS_PUBLIC: i32 = 16;
    const BINDING_FLAGS_NON_PUBLIC: i32 = 32;

    let mut found_method = None;
    if let RuntimeType::Type(ref td) | RuntimeType::Generic(ref td, _) = target_type {
        let lookup = build_generic_lookup_from_runtime_type(ctx, &target_type);
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
                let desc = MethodDescription::new(
                    td.clone(),
                    lookup.clone(),
                    td.resolution.clone(),
                    MethodMemberIndex::Method(idx),
                );
                found_method = Some(crate::common::get_runtime_method_obj(
                    ctx,
                    desc,
                    lookup.clone(),
                ));
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

pub fn handle_get_constructor_impl<'gc, T: ReflectionIntrinsicHost<'gc>>(
    ctx: &mut T,
    _generics: &GenericLookup,
) -> StepResult {
    let _modifiers = ctx.pop();
    let _types = ctx.pop();
    let _call_convention = ctx.pop();
    let _binder = ctx.pop();
    let flags = ctx.pop_i32();
    let obj = ctx.pop_obj();

    let target_type = crate::common::resolve_runtime_type(ctx, obj);

    const BINDING_FLAGS_INSTANCE: i32 = 4;
    const BINDING_FLAGS_STATIC: i32 = 8;
    const BINDING_FLAGS_PUBLIC: i32 = 16;
    const BINDING_FLAGS_NON_PUBLIC: i32 = 32;

    let mut found_constructor = None;
    if let RuntimeType::Type(ref td) | RuntimeType::Generic(ref td, _) = target_type {
        let lookup = build_generic_lookup_from_runtime_type(ctx, &target_type);
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
                let desc = MethodDescription::new(
                    td.clone(),
                    lookup.clone(),
                    td.resolution.clone(),
                    MethodMemberIndex::Method(idx),
                );
                found_constructor = Some(crate::common::get_runtime_method_obj(
                    ctx,
                    desc,
                    lookup.clone(),
                ));
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
    let member_info_type =
        dotnet_vm_ops::vm_try!(ctx.loader().corlib_type("System.Reflection.MemberInfo"));
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
    let type_type = dotnet_vm_ops::vm_try!(ctx.loader().corlib_type("System.Type"));
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

pub fn handle_get_fields<'gc, T: ReflectionIntrinsicHost<'gc>>(
    ctx: &mut T,
    _generics: &GenericLookup,
) -> StepResult {
    let flags = ctx.pop_i32();
    let obj = ctx.pop_obj();
    let target_type = crate::common::resolve_runtime_type(ctx, obj);

    const BINDING_FLAGS_INSTANCE: i32 = 4;
    const BINDING_FLAGS_STATIC: i32 = 8;
    const BINDING_FLAGS_PUBLIC: i32 = 16;
    const BINDING_FLAGS_NON_PUBLIC: i32 = 32;

    let mut fields_objs = Vec::new();
    if let RuntimeType::Type(ref td) | RuntimeType::Generic(ref td, _) = target_type {
        let lookup = build_generic_lookup_from_runtime_type(ctx, &target_type);
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
                fields_objs.push(crate::common::get_runtime_field_obj(
                    ctx,
                    desc,
                    lookup.clone(),
                ));
            }
        }
    }

    let field_info_type =
        dotnet_vm_ops::vm_try!(ctx.loader().corlib_type("System.Reflection.FieldInfo"));
    populate_reflection_array(ctx, fields_objs, ConcreteType::from(field_info_type))
}

pub fn handle_get_field<'gc, T: ReflectionIntrinsicHost<'gc>>(
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
    let target_type = crate::common::resolve_runtime_type(ctx, obj);

    const BINDING_FLAGS_INSTANCE: i32 = 4;
    const BINDING_FLAGS_STATIC: i32 = 8;
    const BINDING_FLAGS_PUBLIC: i32 = 16;
    const BINDING_FLAGS_NON_PUBLIC: i32 = 32;

    let mut found_field = None;
    if let RuntimeType::Type(ref td) | RuntimeType::Generic(ref td, _) = target_type {
        let lookup = build_generic_lookup_from_runtime_type(ctx, &target_type);
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
                found_field = Some(crate::common::get_runtime_field_obj(
                    ctx,
                    desc,
                    lookup.clone(),
                ));
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
    let property_info_type =
        dotnet_vm_ops::vm_try!(ctx.loader().corlib_type("System.Reflection.PropertyInfo"));
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

pub fn handle_get_constructors<'gc, T: ReflectionIntrinsicHost<'gc>>(
    ctx: &mut T,
    _generics: &GenericLookup,
) -> StepResult {
    let flags = ctx.pop_i32();
    let obj = ctx.pop_obj();
    let target_type = crate::common::resolve_runtime_type(ctx, obj);

    const BINDING_FLAGS_INSTANCE: i32 = 4;
    const BINDING_FLAGS_STATIC: i32 = 8;
    const BINDING_FLAGS_PUBLIC: i32 = 16;
    const BINDING_FLAGS_NON_PUBLIC: i32 = 32;

    let mut constructors_objs = Vec::new();
    if let RuntimeType::Type(ref td) | RuntimeType::Generic(ref td, _) = target_type {
        let lookup = build_generic_lookup_from_runtime_type(ctx, &target_type);
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
                let desc = MethodDescription::new(
                    td.clone(),
                    lookup.clone(),
                    td.resolution.clone(),
                    MethodMemberIndex::Method(idx),
                );
                constructors_objs.push(crate::common::get_runtime_method_obj(
                    ctx,
                    desc,
                    lookup.clone(),
                ));
            }
        }
    }

    let constructor_info_type = dotnet_vm_ops::vm_try!(
        ctx.loader()
            .corlib_type("System.Reflection.ConstructorInfo")
    );
    populate_reflection_array(
        ctx,
        constructors_objs,
        ConcreteType::from(constructor_info_type),
    )
}

use crate::{
    ReflectionIntrinsicHost,
    types::{
        build_generic_lookup_from_runtime_type, populate_reflection_array, string_from_heap_obj,
    },
};
use dotnet_assemblies::SUPPORT_ASSEMBLY;
use dotnet_macros::dotnet_intrinsic;
use dotnet_types::{
    TypeDescription,
    error::VmError,
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
use dotnetdll::prelude::{
    Accessibility, AlwaysFailsResolver, BaseType, FixedArg, IntegralParam, MemberAccessibility,
    MemberType, MethodMemberIndex, MethodReferenceParent, TypeDefinition, TypeSource, UserMethod,
};

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
    let num_args = method.signature().parameters.len()
        + if method.signature().instance {
            1
        } else {
            0
        };
    for _ in 0..num_args {
        let _ = ctx.pop();
    }

    let attribute_type = dotnet_vm_ops::vm_try!(ctx.loader().corlib_type("System.Attribute"));
    let array = dotnet_vm_ops::vm_try!(ctx.new_vector(attribute_type.into(), 0));
    let obj = ObjectRef::new(gc, HeapStorage::Vec(Box::new(array)));
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
    let num_args = method.signature().parameters.len()
        + if method.signature().instance {
            1
        } else {
            0
        };
    for _ in 0..num_args {
        let _ = ctx.pop();
    }

    let attribute_type = dotnet_vm_ops::vm_try!(ctx.loader().corlib_type("System.Attribute"));
    let array = dotnet_vm_ops::vm_try!(ctx.new_vector(attribute_type.into(), 0));
    let obj = ObjectRef::new(gc, HeapStorage::Vec(Box::new(array)));
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

    let target_type = dotnet_vm_ops::vm_try!(crate::common::resolve_runtime_type(ctx, obj));
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
    let v = ObjectRef::new(gc, HeapStorage::Obj(Box::new(asm_handle)));
    ctx.register_new_object(&v);

    ctx.reflection_cache_runtime_assembly(resolution, v);
    ctx.push_obj(v);
    StepResult::Continue
}

fn resolve_attribute_declaring_type<'gc, T: ReflectionIntrinsicHost<'gc>>(
    ctx: &T,
    owner: &TypeDescription,
    attribute: &dotnetdll::prelude::Attribute<'_>,
) -> Option<TypeDescription> {
    match attribute.constructor {
        UserMethod::Definition(method_index) => Some(TypeDescription::new(
            owner.resolution.clone(),
            method_index.parent_type(),
        )),
        UserMethod::Reference(method_ref_index) => {
            let method_ref = &owner.resolution.definition()[method_ref_index];
            match &method_ref.parent {
                MethodReferenceParent::Type(method_type) => {
                    let dotnetdll::prelude::MethodType::Base(base_type) = method_type else {
                        return None;
                    };

                    let BaseType::Type {
                        source: TypeSource::User(user_type),
                        ..
                    } = base_type.as_ref()
                    else {
                        return None;
                    };

                    ctx.loader()
                        .locate_type(owner.resolution.clone(), *user_type)
                        .ok()
                }
                _ => None,
            }
        }
    }
}

fn write_constructor_arg_to_field<'gc>(
    instance: &dotnet_value::object::Object<'gc>,
    owner: &TypeDescription,
    field_name: &str,
    field_type: &MemberType,
    arg: &FixedArg<'_>,
) -> bool {
    match (field_type, arg) {
        (MemberType::Base(base), FixedArg::Boolean(value)) if matches!(&**base, BaseType::Boolean) => {
            if let Some(field) = instance.instance_storage.field::<u8>(owner.clone(), field_name) {
                field.write(u8::from(*value));
                true
            } else {
                false
            }
        }
        (MemberType::Base(base), FixedArg::Char(value)) if matches!(&**base, BaseType::Char) => {
            if let Some(field) = instance.instance_storage.field::<u16>(owner.clone(), field_name) {
                field.write(*value as u16);
                true
            } else {
                false
            }
        }
        (MemberType::Base(base), FixedArg::Float32(value))
            if matches!(&**base, BaseType::Float32) =>
        {
            if let Some(field) = instance.instance_storage.field::<f32>(owner.clone(), field_name) {
                field.write(*value);
                true
            } else {
                false
            }
        }
        (MemberType::Base(base), FixedArg::Float64(value))
            if matches!(&**base, BaseType::Float64) =>
        {
            if let Some(field) = instance.instance_storage.field::<f64>(owner.clone(), field_name) {
                field.write(*value);
                true
            } else {
                false
            }
        }
        (MemberType::Base(base), FixedArg::Integral(IntegralParam::Int8(value)))
            if matches!(&**base, BaseType::Int8) =>
        {
            if let Some(field) = instance.instance_storage.field::<i8>(owner.clone(), field_name) {
                field.write(*value);
                true
            } else {
                false
            }
        }
        (MemberType::Base(base), FixedArg::Integral(IntegralParam::UInt8(value)))
            if matches!(&**base, BaseType::UInt8) =>
        {
            if let Some(field) = instance.instance_storage.field::<u8>(owner.clone(), field_name) {
                field.write(*value);
                true
            } else {
                false
            }
        }
        (MemberType::Base(base), FixedArg::Integral(IntegralParam::Int16(value)))
            if matches!(&**base, BaseType::Int16) =>
        {
            if let Some(field) = instance.instance_storage.field::<i16>(owner.clone(), field_name) {
                field.write(*value);
                true
            } else {
                false
            }
        }
        (MemberType::Base(base), FixedArg::Integral(IntegralParam::UInt16(value)))
            if matches!(&**base, BaseType::UInt16) =>
        {
            if let Some(field) = instance.instance_storage.field::<u16>(owner.clone(), field_name) {
                field.write(*value);
                true
            } else {
                false
            }
        }
        (MemberType::Base(base), FixedArg::Integral(IntegralParam::Int32(value)))
            if matches!(&**base, BaseType::Int32) =>
        {
            if let Some(field) = instance.instance_storage.field::<i32>(owner.clone(), field_name) {
                field.write(*value);
                true
            } else {
                false
            }
        }
        (MemberType::Base(base), FixedArg::Integral(IntegralParam::UInt32(value)))
            if matches!(&**base, BaseType::UInt32) =>
        {
            if let Some(field) = instance.instance_storage.field::<u32>(owner.clone(), field_name) {
                field.write(*value);
                true
            } else {
                false
            }
        }
        (MemberType::Base(base), FixedArg::Integral(IntegralParam::Int64(value)))
            if matches!(&**base, BaseType::Int64) =>
        {
            if let Some(field) = instance.instance_storage.field::<i64>(owner.clone(), field_name) {
                field.write(*value);
                true
            } else {
                false
            }
        }
        (MemberType::Base(base), FixedArg::Integral(IntegralParam::UInt64(value)))
            if matches!(&**base, BaseType::UInt64) =>
        {
            if let Some(field) = instance.instance_storage.field::<u64>(owner.clone(), field_name) {
                field.write(*value);
                true
            } else {
                false
            }
        }
        _ => false,
    }
}

/// Materialize a single decoded custom attribute into a managed object, applying the optional
/// `attribute_filter`. Returns `Ok(None)` when the attribute should be skipped (declaring type
/// unresolvable, filtered out, blob decode failed, or the constructor args do not map onto the
/// attribute's instance fields). `owner` supplies the resolution context used to resolve the
/// attribute's declaring type and to decode its instantiation blob; for a method/field attribute
/// pass the declaring type, since members share their declaring type's resolution.
fn materialize_custom_attribute<'gc, T: ReflectionIntrinsicHost<'gc>>(
    ctx: &mut T,
    owner: &TypeDescription,
    attribute: &dotnetdll::prelude::Attribute<'_>,
    attribute_filter: &Option<TypeDescription>,
) -> Result<Option<ObjectRef<'gc>>, VmError> {
    let owner_resolution = owner.resolution.definition();

    let Some(attribute_type) = resolve_attribute_declaring_type(ctx, owner, attribute) else {
        return Ok(None);
    };

    if let Some(filter_type) = attribute_filter {
        let is_match = attribute_type == *filter_type
            || ctx
                .is_a(attribute_type.clone().into(), filter_type.clone().into())
                .unwrap_or(false);
        if !is_match {
            return Ok(None);
        }
    }

    let data = match attribute.instantiation_data(&AlwaysFailsResolver, owner_resolution) {
        Ok(data) => data,
        Err(_) => return Ok(None),
    };

    let instance = ctx.new_object(attribute_type.clone())?;

    let instance_fields = attribute_type
        .definition()
        .fields
        .iter()
        .filter(|field| !field.static_member)
        .collect::<Vec<_>>();

    if data.constructor_args.len() > instance_fields.len() {
        return Ok(None);
    }

    let can_materialize = data
        .constructor_args
        .iter()
        .zip(instance_fields.iter())
        .all(|(arg, field)| {
            write_constructor_arg_to_field(
                &instance,
                &attribute_type,
                field.name.as_ref(),
                &field.return_type,
                arg,
            )
        });

    if !can_materialize {
        return Ok(None);
    }

    let gc = ctx.gc_with_token(&ctx.no_active_borrows_token());
    let attr_obj = ObjectRef::new(gc, HeapStorage::Obj(Box::new(instance)));
    ctx.register_new_object(&attr_obj);
    Ok(Some(attr_obj))
}

fn collect_type_custom_attributes<'gc, T: ReflectionIntrinsicHost<'gc>>(
    ctx: &mut T,
    target_runtime_type: RuntimeType,
    attribute_filter: Option<TypeDescription>,
) -> Result<Vec<ObjectRef<'gc>>, VmError> {
    let owner_type = match target_runtime_type {
        RuntimeType::Type(td) | RuntimeType::Generic(td, _) => td,
        _ => return Ok(Vec::new()),
    };

    let mut attributes = Vec::new();

    let type_attributes = owner_type
        .resolution
        .definition()
        .type_attributes(owner_type.index)
        .unwrap_or_default();

    for attribute in &type_attributes {
        if let Some(attr_obj) =
            materialize_custom_attribute(ctx, &owner_type, attribute, &attribute_filter)?
        {
            attributes.push(attr_obj);
        }
    }

    Ok(attributes)
}

/// Collects the materialized custom attributes declared directly on `method`, mirroring
/// [`collect_type_custom_attributes`] for the method-level metadata table. The attributes share
/// the resolution of the method's declaring type (`method.parent`).
pub(crate) fn collect_method_custom_attributes<'gc, T: ReflectionIntrinsicHost<'gc>>(
    ctx: &mut T,
    method: MethodDescription,
    attribute_filter: Option<TypeDescription>,
) -> Result<Vec<ObjectRef<'gc>>, VmError> {
    let mut attributes = Vec::new();

    let owner_type = method.parent.clone();
    let method_attributes = method
        .resolution()
        .definition()
        .method_attributes(method.method_index())
        .unwrap_or_default();

    for attribute in &method_attributes {
        if let Some(attr_obj) =
            materialize_custom_attribute(ctx, &owner_type, attribute, &attribute_filter)?
        {
            attributes.push(attr_obj);
        }
    }

    Ok(attributes)
}

pub fn handle_get_custom_attributes_bool<'gc, T: ReflectionIntrinsicHost<'gc>>(
    ctx: &mut T,
    _generics: &GenericLookup,
) -> StepResult {
    let _inherit = ctx.pop_i32();
    let obj = ctx.pop_obj();

    let target_runtime_type = dotnet_vm_ops::vm_try!(crate::common::resolve_runtime_type(ctx, obj));
    let custom_attributes =
        dotnet_vm_ops::vm_try!(collect_type_custom_attributes(ctx, target_runtime_type, None));

    let object_type = dotnet_vm_ops::vm_try!(ctx.loader().corlib_type("System.Object"));
    populate_reflection_array(ctx, custom_attributes, ConcreteType::from(object_type))
}

pub fn handle_get_custom_attributes_typed<'gc, T: ReflectionIntrinsicHost<'gc>>(
    ctx: &mut T,
    _generics: &GenericLookup,
) -> StepResult {
    let _inherit = ctx.pop_i32();
    let attribute_type_obj = ctx.pop_obj();
    let obj = ctx.pop_obj();

    let attribute_filter = if attribute_type_obj.0.is_some() {
        let filter_runtime_type =
            dotnet_vm_ops::vm_try!(crate::common::resolve_runtime_type(ctx, attribute_type_obj));
        match filter_runtime_type {
            RuntimeType::Type(td) | RuntimeType::Generic(td, _) => Some(td),
            _ => None,
        }
    } else {
        None
    };

    let target_runtime_type = dotnet_vm_ops::vm_try!(crate::common::resolve_runtime_type(ctx, obj));
    let custom_attributes = dotnet_vm_ops::vm_try!(collect_type_custom_attributes(
        ctx,
        target_runtime_type,
        attribute_filter,
    ));

    let object_type = dotnet_vm_ops::vm_try!(ctx.loader().corlib_type("System.Object"));
    populate_reflection_array(ctx, custom_attributes, ConcreteType::from(object_type))
}

pub fn handle_get_methods<'gc, T: ReflectionIntrinsicHost<'gc>>(
    ctx: &mut T,
    _generics: &GenericLookup,
) -> StepResult {
    let flags = ctx.pop_i32();
    let obj = ctx.pop_obj();
    let target_type = dotnet_vm_ops::vm_try!(crate::common::resolve_runtime_type(ctx, obj));

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

fn get_runtime_method_metadata<'gc, T: ReflectionIntrinsicHost<'gc>>(
    ctx: &mut T,
    td: &TypeDescription,
    lookup: &GenericLookup,
    idx: usize,
) -> ObjectRef<'gc> {
    let desc = MethodDescription::new(
        td.clone(),
        lookup.clone(),
        td.resolution.clone(),
        MethodMemberIndex::Method(idx),
    );
    crate::common::get_runtime_method_obj(ctx, desc, lookup.clone())
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

    let name = dotnet_vm_ops::vm_try!(string_from_heap_obj(name_obj));
    let target_type = dotnet_vm_ops::vm_try!(crate::common::resolve_runtime_type(ctx, obj));

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
                found_method = Some(get_runtime_method_metadata(ctx, td, &lookup, idx));
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

    let target_type = dotnet_vm_ops::vm_try!(crate::common::resolve_runtime_type(ctx, obj));

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
                found_constructor = Some(get_runtime_method_metadata(ctx, td, &lookup, idx));
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
    let target_type = dotnet_vm_ops::vm_try!(crate::common::resolve_runtime_type(ctx, obj));

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

    let name = dotnet_vm_ops::vm_try!(string_from_heap_obj(name_obj));
    let target_type = dotnet_vm_ops::vm_try!(crate::common::resolve_runtime_type(ctx, obj));

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
    let target_type = dotnet_vm_ops::vm_try!(crate::common::resolve_runtime_type(ctx, obj));

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

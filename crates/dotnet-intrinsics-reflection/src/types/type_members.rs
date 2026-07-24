use crate::{
    ReflectionIntrinsicHost,
    types::{
        build_generic_lookup_from_runtime_type, populate_reflection_array, string_from_heap_obj,
    },
};
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
    MemberType, MethodMemberIndex, MethodReferenceParent, ParameterType, TypeSource, UserMethod,
};

#[dotnet_intrinsic(
    "System.Reflection.AssemblyName System.Reflection.RuntimeAssembly::GetName(bool)"
)]
pub fn intrinsic_runtime_assembly_get_name<'gc, T: ReflectionIntrinsicHost<'gc>>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let _copied_name = ctx.pop_i32();
    let runtime_assembly_obj = ctx.pop_obj();

    let Some(resolution) = ctx.reflection_runtime_assembly_resolution(runtime_assembly_obj) else {
        return ctx.throw_by_name_with_message(
            "System.InvalidOperationException",
            "RuntimeAssembly resolution is unavailable.",
        );
    };

    let assembly_name_value = resolution
        .definition()
        .assembly
        .as_ref()
        .map(|assembly| assembly.name.to_string())
        .unwrap_or_default();

    let assembly_name_type =
        dotnet_vm_ops::vm_try!(ctx.loader().corlib_type("System.Reflection.AssemblyName"));
    let assembly_name_instance = dotnet_vm_ops::vm_try!(ctx.new_object(assembly_name_type.clone()));

    let gc = ctx.gc_with_token(&ctx.no_active_borrows_token());
    let assembly_name_obj = ObjectRef::new(gc, HeapStorage::Obj(Box::new(assembly_name_instance)));
    ctx.register_new_object(&assembly_name_obj);

    let name_obj = ObjectRef::new(gc, HeapStorage::Str(assembly_name_value.into()));
    ctx.register_new_object(&name_obj);

    assembly_name_obj.as_object_mut(gc, |instance| {
        instance
            .instance_storage
            .field::<ObjectRef<'gc>>(assembly_name_type.clone(), "_name")
            .expect("AssemblyName._name field must exist")
            .write(name_obj);
    });

    ctx.push_obj(assembly_name_obj);
    StepResult::Continue
}

#[dotnet_intrinsic("object[] System.Reflection.Assembly::GetCustomAttributes(System.Type, bool)")]
#[dotnet_intrinsic(
    "object[] System.Reflection.RuntimeAssembly::GetCustomAttributes(System.Type, bool)"
)]
#[dotnet_intrinsic("object[] System.Reflection.RuntimeAssembly::GetCustomAttributes(bool)")]
#[dotnet_intrinsic(
    "static System.Attribute[] System.Attribute::GetCustomAttributes(System.Reflection.Assembly, System.Type, bool)"
)]
pub fn intrinsic_assembly_get_custom_attributes<'gc, T: ReflectionIntrinsicHost<'gc>>(
    ctx: &mut T,
    method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let (assembly_obj, attribute_filter, element_type) = match method.signature().parameters.len() {
        1 => {
            let _inherit = ctx.pop_i32();
            let assembly_obj = ctx.pop_obj();
            let object_type = dotnet_vm_ops::vm_try!(ctx.loader().corlib_type("System.Object"));
            (
                assembly_obj,
                None,
                RuntimeType::Type(object_type).to_concrete(ctx.loader().as_ref()),
            )
        }
        _ => {
            let _inherit = ctx.pop_i32();
            let attribute_type_obj = ctx.pop_obj();
            let assembly_obj = ctx.pop_obj();

            if attribute_type_obj.0.is_some() {
                let attribute_runtime_type = dotnet_vm_ops::vm_try!(
                    crate::common::resolve_runtime_type(ctx, attribute_type_obj)
                );
                let attribute_filter = match &attribute_runtime_type {
                    RuntimeType::Type(td) | RuntimeType::Generic(td, _) => Some(td.clone()),
                    _ => None,
                };

                (
                    assembly_obj,
                    attribute_filter,
                    attribute_runtime_type.to_concrete(ctx.loader().as_ref()),
                )
            } else {
                let attribute_type =
                    dotnet_vm_ops::vm_try!(ctx.loader().corlib_type("System.Attribute"));
                (
                    assembly_obj,
                    None,
                    RuntimeType::Type(attribute_type).to_concrete(ctx.loader().as_ref()),
                )
            }
        }
    };

    if method.parent.type_name() == "System.Attribute" && assembly_obj.0.is_some() {
        let assembly_base =
            dotnet_vm_ops::vm_try!(ctx.loader().corlib_type("System.Reflection.Assembly"));
        let assembly_base_concrete =
            RuntimeType::Type(assembly_base).to_concrete(ctx.loader().as_ref());
        let argument_type = dotnet_vm_ops::vm_try!(ctx.get_heap_description(assembly_obj));
        let argument_concrete = RuntimeType::Type(argument_type).to_concrete(ctx.loader().as_ref());

        if !ctx
            .loader()
            .comparer()
            .is_assignable_to(&argument_concrete, &assembly_base_concrete)
        {
            return populate_reflection_array(ctx, Vec::new(), element_type);
        }
    }

    let Some(resolution) = ctx.reflection_runtime_assembly_resolution(assembly_obj) else {
        return ctx.throw_by_name_with_message(
            "System.InvalidOperationException",
            "RuntimeAssembly resolution is unavailable.",
        );
    };

    let attrs = dotnet_vm_ops::vm_try!(collect_assembly_custom_attributes(
        ctx,
        resolution,
        attribute_filter
    ));
    populate_reflection_array(ctx, attrs, element_type)
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
    let num_args =
        method.signature().parameters.len() + if method.signature().instance { 1 } else { 0 };
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

    let runtime_assembly_type = dotnet_vm_ops::vm_try!(
        ctx.loader()
            .corlib_type("System.Reflection.RuntimeAssembly")
    );
    let asm_handle = dotnet_vm_ops::vm_try!(ctx.new_object(runtime_assembly_type));
    let assembly_obj = ObjectRef::new(gc, HeapStorage::Obj(Box::new(asm_handle)));
    ctx.register_new_object(&assembly_obj);

    ctx.reflection_cache_runtime_assembly(resolution, assembly_obj);
    ctx.push_obj(assembly_obj);
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

fn write_constructor_arg_to_field<'gc, T: ReflectionIntrinsicHost<'gc>>(
    ctx: &mut T,
    instance: &dotnet_value::object::Object<'gc>,
    owner: &TypeDescription,
    field_name: &str,
    field_type: &MemberType,
    arg: &FixedArg<'_>,
) -> bool {
    match (field_type, arg) {
        (MemberType::Base(base), FixedArg::Boolean(value))
            if matches!(&**base, BaseType::Boolean) =>
        {
            if let Some(field) = instance
                .instance_storage
                .field::<u8>(owner.clone(), field_name)
            {
                field.write(u8::from(*value));
                true
            } else {
                false
            }
        }
        (MemberType::Base(base), FixedArg::Char(value)) if matches!(&**base, BaseType::Char) => {
            if let Some(field) = instance
                .instance_storage
                .field::<u16>(owner.clone(), field_name)
            {
                field.write(*value as u16);
                true
            } else {
                false
            }
        }
        (MemberType::Base(base), FixedArg::Float32(value))
            if matches!(&**base, BaseType::Float32) =>
        {
            if let Some(field) = instance
                .instance_storage
                .field::<f32>(owner.clone(), field_name)
            {
                field.write(*value);
                true
            } else {
                false
            }
        }
        (MemberType::Base(base), FixedArg::Float64(value))
            if matches!(&**base, BaseType::Float64) =>
        {
            if let Some(field) = instance
                .instance_storage
                .field::<f64>(owner.clone(), field_name)
            {
                field.write(*value);
                true
            } else {
                false
            }
        }
        (MemberType::Base(base), FixedArg::String(value))
            if matches!(&**base, BaseType::String) =>
        {
            if let Some(field) = instance
                .instance_storage
                .field::<ObjectRef<'gc>>(owner.clone(), field_name)
            {
                let value = match value {
                    Some(value) => {
                        let gc = ctx.gc_with_token(&ctx.no_active_borrows_token());
                        let value_obj =
                            ObjectRef::new(gc, HeapStorage::Str(value.to_string().into()));
                        ctx.register_new_object(&value_obj);
                        value_obj
                    }
                    None => ObjectRef(None),
                };
                field.write(value);
                true
            } else {
                false
            }
        }
        (MemberType::Base(base), FixedArg::Integral(IntegralParam::Int8(value)))
            if matches!(&**base, BaseType::Int8) =>
        {
            if let Some(field) = instance
                .instance_storage
                .field::<i8>(owner.clone(), field_name)
            {
                field.write(*value);
                true
            } else {
                false
            }
        }
        (MemberType::Base(base), FixedArg::Integral(IntegralParam::UInt8(value)))
            if matches!(&**base, BaseType::UInt8) =>
        {
            if let Some(field) = instance
                .instance_storage
                .field::<u8>(owner.clone(), field_name)
            {
                field.write(*value);
                true
            } else {
                false
            }
        }
        (MemberType::Base(base), FixedArg::Integral(IntegralParam::Int16(value)))
            if matches!(&**base, BaseType::Int16) =>
        {
            if let Some(field) = instance
                .instance_storage
                .field::<i16>(owner.clone(), field_name)
            {
                field.write(*value);
                true
            } else {
                false
            }
        }
        (MemberType::Base(base), FixedArg::Integral(IntegralParam::UInt16(value)))
            if matches!(&**base, BaseType::UInt16) =>
        {
            if let Some(field) = instance
                .instance_storage
                .field::<u16>(owner.clone(), field_name)
            {
                field.write(*value);
                true
            } else {
                false
            }
        }
        (MemberType::Base(base), FixedArg::Integral(IntegralParam::Int32(value)))
            if matches!(&**base, BaseType::Int32) =>
        {
            if let Some(field) = instance
                .instance_storage
                .field::<i32>(owner.clone(), field_name)
            {
                field.write(*value);
                true
            } else {
                false
            }
        }
        (MemberType::Base(base), FixedArg::Integral(IntegralParam::UInt32(value)))
            if matches!(&**base, BaseType::UInt32) =>
        {
            if let Some(field) = instance
                .instance_storage
                .field::<u32>(owner.clone(), field_name)
            {
                field.write(*value);
                true
            } else {
                false
            }
        }
        (MemberType::Base(base), FixedArg::Integral(IntegralParam::Int64(value)))
            if matches!(&**base, BaseType::Int64) =>
        {
            if let Some(field) = instance
                .instance_storage
                .field::<i64>(owner.clone(), field_name)
            {
                field.write(*value);
                true
            } else {
                false
            }
        }
        (MemberType::Base(base), FixedArg::Integral(IntegralParam::UInt64(value)))
            if matches!(&**base, BaseType::UInt64) =>
        {
            if let Some(field) = instance
                .instance_storage
                .field::<u64>(owner.clone(), field_name)
            {
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
                ctx,
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

/// Collects the materialized custom attributes declared directly on `field`,
/// mirroring [`collect_method_custom_attributes`] for the field-level metadata
/// table.
pub(crate) fn collect_field_custom_attributes<'gc, T: ReflectionIntrinsicHost<'gc>>(
    ctx: &mut T,
    field: dotnet_types::members::FieldDescription,
    attribute_filter: Option<TypeDescription>,
) -> Result<Vec<ObjectRef<'gc>>, VmError> {
    let owner_type = field.parent.clone();
    // `field()` dereferences directly into the type definition's field list;
    // for lazy assemblies the attributes may not be loaded yet so we fall back
    // to an empty list (same behaviour as collect_method_custom_attributes when
    // the method has no attributes).
    let field_attrs = field.field().attributes.clone();
    let mut attributes = Vec::new();
    for attribute in &field_attrs {
        if let Some(attr_obj) =
            materialize_custom_attribute(ctx, &owner_type, attribute, &attribute_filter)?
        {
            attributes.push(attr_obj);
        }
    }
    Ok(attributes)
}

fn collect_assembly_custom_attributes<'gc, T: ReflectionIntrinsicHost<'gc>>(
    ctx: &mut T,
    resolution: dotnet_types::resolution::ResolutionS,
    attribute_filter: Option<TypeDescription>,
) -> Result<Vec<ObjectRef<'gc>>, VmError> {
    let Some(module_type_index) = resolution.definition().type_definition_index(0) else {
        return Ok(Vec::new());
    };

    let owner_type = TypeDescription::new(resolution, module_type_index);
    let mut attributes = Vec::new();
    let assembly_attrs = owner_type
        .resolution
        .definition()
        .assembly_attributes()
        .unwrap_or_default();

    for attribute in &assembly_attrs {
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
    let custom_attributes = dotnet_vm_ops::vm_try!(collect_type_custom_attributes(
        ctx,
        target_runtime_type,
        None
    ));

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

    let attribute_filter = dotnet_vm_ops::vm_try!(crate::common::resolve_attribute_filter(
        ctx,
        attribute_type_obj
    ));

    let target_runtime_type = dotnet_vm_ops::vm_try!(crate::common::resolve_runtime_type(ctx, obj));
    let custom_attributes = dotnet_vm_ops::vm_try!(collect_type_custom_attributes(
        ctx,
        target_runtime_type,
        attribute_filter,
    ));

    let object_type = dotnet_vm_ops::vm_try!(ctx.loader().corlib_type("System.Object"));
    populate_reflection_array(ctx, custom_attributes, ConcreteType::from(object_type))
}

pub fn handle_is_defined<'gc, T: ReflectionIntrinsicHost<'gc>>(
    ctx: &mut T,
    _generics: &GenericLookup,
) -> StepResult {
    let _inherit = ctx.pop_i32();
    let attribute_type_obj = ctx.pop_obj();
    let obj = ctx.pop_obj();

    let attribute_filter = dotnet_vm_ops::vm_try!(crate::common::resolve_attribute_filter(
        ctx,
        attribute_type_obj
    ));

    let target_runtime_type = dotnet_vm_ops::vm_try!(crate::common::resolve_runtime_type(ctx, obj));
    let custom_attributes = dotnet_vm_ops::vm_try!(collect_type_custom_attributes(
        ctx,
        target_runtime_type,
        attribute_filter,
    ));

    ctx.push_i32(if custom_attributes.is_empty() { 0 } else { 1 });
    StepResult::Continue
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
    if let Some((td, lookup)) = member_lookup_target(ctx, &target_type) {
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

fn member_lookup_target<'gc, T: ReflectionIntrinsicHost<'gc>>(
    ctx: &mut T,
    runtime_type: &RuntimeType,
) -> Option<(TypeDescription, GenericLookup)> {
    match runtime_type {
        RuntimeType::Type(td) | RuntimeType::Generic(td, _) => Some((
            td.clone(),
            build_generic_lookup_from_runtime_type(ctx, runtime_type),
        )),
        _ => {
            let concrete = runtime_type.to_concrete(ctx.loader().as_ref());
            ctx.loader()
                .find_concrete_type(concrete)
                .ok()
                .map(|td| (td, GenericLookup::default()))
        }
    }
}

fn method_parameter_runtime_types<'gc, T: ReflectionIntrinsicHost<'gc>>(
    ctx: &T,
    method: &MethodDescription,
    lookup: &GenericLookup,
) -> Vec<RuntimeType> {
    method
        .signature()
        .parameters
        .iter()
        .map(|parameter| runtime_type_from_parameter(ctx, method, lookup, &parameter.1))
        .collect()
}

pub fn handle_get_method_impl<'gc, T: ReflectionIntrinsicHost<'gc>>(
    ctx: &mut T,
    _generics: &GenericLookup,
    has_generic_parameter_count: bool,
) -> StepResult {
    let _modifiers = ctx.pop();
    let types_obj = ctx.pop_obj();
    let _call_convention = ctx.pop();
    let _binder = ctx.pop();
    let flags = ctx.pop_i32();
    let generic_parameter_count = if has_generic_parameter_count {
        Some(ctx.pop_i32())
    } else {
        None
    };
    let name_obj = ctx.pop_obj();
    let obj = ctx.pop_obj();

    let name = dotnet_vm_ops::vm_try!(string_from_heap_obj(name_obj));
    let ignore_case = (flags & BINDING_FLAGS_IGNORE_CASE) != 0;
    let expected_parameter_types = if types_obj.0.is_some() {
        Some(dotnet_vm_ops::vm_try!(parse_runtime_type_array(
            ctx, types_obj
        )))
    } else {
        None
    };
    let target_type = dotnet_vm_ops::vm_try!(crate::common::resolve_runtime_type(ctx, obj));

    let mut found_method = None;
    if let Some((td, lookup)) = member_lookup_target(ctx, &target_type) {
        for (idx, m) in td.definition().methods.iter().enumerate() {
            let name_matches = if ignore_case {
                m.name.eq_ignore_ascii_case(&name)
            } else {
                m.name == name
            };
            if !name_matches {
                continue;
            }

            if generic_parameter_count
                .is_some_and(|count| count < 0 || m.generic_parameters.len() != count as usize)
            {
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

            if !match_public || !match_static {
                continue;
            }

            let desc = MethodDescription::new(
                td.clone(),
                lookup.clone(),
                td.resolution.clone(),
                MethodMemberIndex::Method(idx),
            );
            if let Some(expected) = expected_parameter_types.as_ref() {
                let actual = method_parameter_runtime_types(ctx, &desc, &lookup);
                if expected != &actual {
                    continue;
                }
            }

            found_method = Some(crate::common::get_runtime_method_obj(
                ctx,
                desc,
                lookup.clone(),
            ));
            break;
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

fn member_name_matches(name_filter: Option<&str>, candidate: &str, ignore_case: bool) -> bool {
    match name_filter {
        Some(name) if ignore_case => candidate.eq_ignore_ascii_case(name),
        Some(name) => candidate == name,
        None => true,
    }
}

fn member_matches_binding_flags(is_public: bool, is_static: bool, flags: i32) -> bool {
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

    match_public && match_static
}

fn push_method_members_if_match<'gc, T: ReflectionIntrinsicHost<'gc>>(
    ctx: &mut T,
    td: &TypeDescription,
    lookup: &GenericLookup,
    name_filter: Option<&str>,
    flags: i32,
    require_ctor: bool,
    member_objs: &mut Vec<ObjectRef<'gc>>,
) {
    let ignore_case = (flags & BINDING_FLAGS_IGNORE_CASE) != 0;

    for (idx, method) in td.definition().methods.iter().enumerate() {
        let matches_member_type = if require_ctor {
            method.name == ".ctor"
        } else {
            method.name != ".ctor" && method.name != ".cctor"
        };
        if !matches_member_type {
            continue;
        }

        let is_public = method.accessibility == MemberAccessibility::Access(Accessibility::Public);
        let is_static = !method.signature.instance;
        if member_name_matches(name_filter, &method.name, ignore_case)
            && member_matches_binding_flags(is_public, is_static, flags)
        {
            let desc = MethodDescription::new(
                td.clone(),
                lookup.clone(),
                td.resolution.clone(),
                MethodMemberIndex::Method(idx),
            );
            member_objs.push(crate::common::get_runtime_method_obj(
                ctx,
                desc,
                lookup.clone(),
            ));
        }
    }
}

fn collect_type_member_infos<'gc, T: ReflectionIntrinsicHost<'gc>>(
    ctx: &mut T,
    target_type: &RuntimeType,
    name_filter: Option<&str>,
    member_types: i32,
    flags: i32,
) -> Result<Vec<ObjectRef<'gc>>, VmError> {
    let ignore_case = (flags & BINDING_FLAGS_IGNORE_CASE) != 0;
    let mut member_objs = Vec::new();

    if let Some((td, lookup)) = member_lookup_target(ctx, target_type) {
        if (member_types & MEMBER_TYPES_METHOD) != 0 {
            push_method_members_if_match(
                ctx,
                &td,
                &lookup,
                name_filter,
                flags,
                false,
                &mut member_objs,
            );
        }

        if (member_types & MEMBER_TYPES_CONSTRUCTOR) != 0 {
            push_method_members_if_match(
                ctx,
                &td,
                &lookup,
                name_filter,
                flags,
                true,
                &mut member_objs,
            );
        }

        if (member_types & MEMBER_TYPES_FIELD) != 0 {
            for (index, field) in td.definition().fields.iter().enumerate() {
                let is_public =
                    field.accessibility == MemberAccessibility::Access(Accessibility::Public);
                let is_static = field.static_member;
                if member_name_matches(name_filter, &field.name, ignore_case)
                    && member_matches_binding_flags(is_public, is_static, flags)
                {
                    let desc = dotnet_types::members::FieldDescription::new(
                        td.clone(),
                        td.resolution.clone(),
                        index,
                    );
                    member_objs.push(crate::common::get_runtime_field_obj(
                        ctx,
                        desc,
                        lookup.clone(),
                    ));
                }
            }
        }

        if (member_types & MEMBER_TYPES_PROPERTY) != 0 {
            let properties = collect_property_candidates(&td, &lookup);
            for property in &properties {
                if member_name_matches(name_filter, &property.name, ignore_case)
                    && property_matches_binding_flags(property, flags)
                {
                    member_objs.push(create_runtime_property_obj(
                        ctx,
                        target_type,
                        property,
                        &lookup,
                    )?);
                }
            }
        }
    }

    Ok(member_objs)
}

pub fn handle_get_member<'gc, T: ReflectionIntrinsicHost<'gc>>(
    ctx: &mut T,
    _generics: &GenericLookup,
    has_member_types: bool,
) -> StepResult {
    let flags = ctx.pop_i32();
    let member_types = if has_member_types {
        ctx.pop_i32()
    } else {
        MEMBER_TYPES_ALL
    };
    let name_obj = ctx.pop_obj();
    let obj = ctx.pop_obj();

    let name = dotnet_vm_ops::vm_try!(string_from_heap_obj(name_obj));
    let target_type = dotnet_vm_ops::vm_try!(crate::common::resolve_runtime_type(ctx, obj));
    let member_objs = dotnet_vm_ops::vm_try!(collect_type_member_infos(
        ctx,
        &target_type,
        Some(&name),
        member_types,
        flags,
    ));

    let member_info_type =
        dotnet_vm_ops::vm_try!(ctx.loader().corlib_type("System.Reflection.MemberInfo"));
    populate_reflection_array(ctx, member_objs, ConcreteType::from(member_info_type))
}

pub fn handle_get_members<'gc, T: ReflectionIntrinsicHost<'gc>>(
    ctx: &mut T,
    _generics: &GenericLookup,
) -> StepResult {
    let flags = ctx.pop_i32();
    let obj = ctx.pop_obj();

    let target_type = dotnet_vm_ops::vm_try!(crate::common::resolve_runtime_type(ctx, obj));
    let member_objs = dotnet_vm_ops::vm_try!(collect_type_member_infos(
        ctx,
        &target_type,
        None,
        MEMBER_TYPES_ALL,
        flags,
    ));

    let member_info_type =
        dotnet_vm_ops::vm_try!(ctx.loader().corlib_type("System.Reflection.MemberInfo"));
    populate_reflection_array(ctx, member_objs, ConcreteType::from(member_info_type))
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

const BINDING_FLAGS_IGNORE_CASE: i32 = 1;
const BINDING_FLAGS_INSTANCE: i32 = 4;
const BINDING_FLAGS_STATIC: i32 = 8;
const BINDING_FLAGS_PUBLIC: i32 = 16;
const BINDING_FLAGS_NON_PUBLIC: i32 = 32;

const MEMBER_TYPES_CONSTRUCTOR: i32 = 0x01;
const MEMBER_TYPES_FIELD: i32 = 0x04;
const MEMBER_TYPES_METHOD: i32 = 0x08;
const MEMBER_TYPES_PROPERTY: i32 = 0x10;
const MEMBER_TYPES_ALL: i32 = 0xBF;

#[derive(Clone)]
struct PropertyCandidate {
    name: String,
    getter: Option<MethodDescription>,
    setter: Option<MethodDescription>,
}

fn runtime_type_from_parameter<'gc, T: ReflectionIntrinsicHost<'gc>>(
    ctx: &T,
    method: &MethodDescription,
    lookup: &GenericLookup,
    parameter: &ParameterType<dotnetdll::prelude::MethodType>,
) -> RuntimeType {
    match parameter {
        ParameterType::Value(t) => {
            ctx.reflection_make_runtime_type_for_method(method.clone(), lookup, t)
        }
        ParameterType::Ref(t) => RuntimeType::ByRef(Box::new(
            ctx.reflection_make_runtime_type_for_method(method.clone(), lookup, t),
        )),
        ParameterType::TypedReference => RuntimeType::TypedReference,
    }
}

fn property_accessor_descriptions(
    td: &TypeDescription,
    lookup: &GenericLookup,
    index: usize,
) -> (Option<MethodDescription>, Option<MethodDescription>) {
    let property = &td.definition().properties[index];

    let fallback_accessor = |prefix: &str| {
        let accessor_name = format!("{prefix}{}", property.name);
        td.definition()
            .methods
            .iter()
            .enumerate()
            .find_map(|(i, m)| {
                (m.name == accessor_name).then_some(MethodDescription::new(
                    td.clone(),
                    lookup.clone(),
                    td.resolution.clone(),
                    MethodMemberIndex::Method(i),
                ))
            })
    };

    let getter = if property.getter.is_some() {
        Some(MethodDescription::new(
            td.clone(),
            lookup.clone(),
            td.resolution.clone(),
            MethodMemberIndex::PropertyGetter(index),
        ))
    } else {
        fallback_accessor("get_")
    };

    let setter = if property.setter.is_some() {
        Some(MethodDescription::new(
            td.clone(),
            lookup.clone(),
            td.resolution.clone(),
            MethodMemberIndex::PropertySetter(index),
        ))
    } else {
        fallback_accessor("set_")
    };

    (getter, setter)
}

fn collect_property_candidates(
    td: &TypeDescription,
    lookup: &GenericLookup,
) -> Vec<PropertyCandidate> {
    if !td.definition().properties.is_empty() {
        return td
            .definition()
            .properties
            .iter()
            .enumerate()
            .map(|(index, property)| {
                let (getter, setter) = property_accessor_descriptions(td, lookup, index);
                PropertyCandidate {
                    name: property.name.to_string(),
                    getter,
                    setter,
                }
            })
            .collect();
    }

    let mut synthetic = std::collections::BTreeMap::<String, (Option<usize>, Option<usize>)>::new();
    for (index, method) in td.definition().methods.iter().enumerate() {
        if let Some(property_name) = method.name.strip_prefix("get_") {
            synthetic.entry(property_name.to_string()).or_default().0 = Some(index);
        } else if let Some(property_name) = method.name.strip_prefix("set_") {
            synthetic.entry(property_name.to_string()).or_default().1 = Some(index);
        }
    }

    synthetic
        .into_iter()
        .map(|(name, (getter_index, setter_index))| PropertyCandidate {
            name,
            getter: getter_index.map(|method_index| {
                MethodDescription::new(
                    td.clone(),
                    lookup.clone(),
                    td.resolution.clone(),
                    MethodMemberIndex::Method(method_index),
                )
            }),
            setter: setter_index.map(|method_index| {
                MethodDescription::new(
                    td.clone(),
                    lookup.clone(),
                    td.resolution.clone(),
                    MethodMemberIndex::Method(method_index),
                )
            }),
        })
        .collect()
}

fn property_signature_runtime_types<'gc, T: ReflectionIntrinsicHost<'gc>>(
    ctx: &T,
    getter: Option<&MethodDescription>,
    setter: Option<&MethodDescription>,
    lookup: &GenericLookup,
) -> Option<(RuntimeType, Vec<RuntimeType>)> {
    if let Some(getter) = getter {
        let getter_sig = getter.signature();
        let return_type = getter_sig
            .return_type
            .1
            .as_ref()
            .map(|parameter| runtime_type_from_parameter(ctx, getter, lookup, parameter))
            .unwrap_or(RuntimeType::Void);
        let index_params = getter_sig
            .parameters
            .iter()
            .map(|parameter| runtime_type_from_parameter(ctx, getter, lookup, &parameter.1))
            .collect();
        return Some((return_type, index_params));
    }

    if let Some(setter) = setter {
        let setter_sig = setter.signature();
        let (value_parameter, index_parameters) = setter_sig.parameters.split_last()?;
        let return_type = runtime_type_from_parameter(ctx, setter, lookup, &value_parameter.1);
        let index_params = index_parameters
            .iter()
            .map(|parameter| runtime_type_from_parameter(ctx, setter, lookup, &parameter.1))
            .collect();
        return Some((return_type, index_params));
    }

    None
}

fn property_matches_binding_flags(property: &PropertyCandidate, flags: i32) -> bool {
    let mut has_public_accessor = false;
    let mut has_non_public_accessor = false;
    let mut has_static_accessor = false;
    let mut has_instance_accessor = false;

    for accessor in [property.getter.as_ref(), property.setter.as_ref()]
        .into_iter()
        .flatten()
    {
        let method = accessor.method();
        let is_public = method.accessibility == MemberAccessibility::Access(Accessibility::Public);
        if is_public {
            has_public_accessor = true;
        } else {
            has_non_public_accessor = true;
        }

        if accessor.signature().instance {
            has_instance_accessor = true;
        } else {
            has_static_accessor = true;
        }
    }

    let match_public = (has_public_accessor && (flags & BINDING_FLAGS_PUBLIC) != 0)
        || (has_non_public_accessor && (flags & BINDING_FLAGS_NON_PUBLIC) != 0);
    let match_static = (has_static_accessor && (flags & BINDING_FLAGS_STATIC) != 0)
        || (has_instance_accessor && (flags & BINDING_FLAGS_INSTANCE) != 0);

    match_public && match_static
}

fn parse_runtime_type_array<'gc, T: ReflectionIntrinsicHost<'gc>>(
    ctx: &mut T,
    types_obj: ObjectRef<'gc>,
) -> Result<Vec<RuntimeType>, VmError> {
    let gc = ctx.gc_with_token(&ctx.no_active_borrows_token());
    let type_objs = types_obj
        .try_as_vector(|v: &dotnet_value::object::Vector<'gc>| {
            v.object_ref_elements(&gc).collect::<Vec<_>>()
        })
        .map_err(VmError::from)?;

    let mut runtime_types = Vec::with_capacity(type_objs.len());
    for type_obj in type_objs {
        runtime_types.push(crate::common::resolve_runtime_type(ctx, type_obj)?);
    }

    Ok(runtime_types)
}

fn create_runtime_property_obj<'gc, T: ReflectionIntrinsicHost<'gc>>(
    ctx: &mut T,
    declaring_runtime_type: &RuntimeType,
    property: &PropertyCandidate,
    lookup: &GenericLookup,
) -> Result<ObjectRef<'gc>, VmError> {
    let accessor = property.getter.clone().or_else(|| property.setter.clone());
    if let Some(accessor) = accessor.as_ref()
        && let Some(obj) = ctx.reflection_cached_runtime_property_obj(accessor, lookup)
    {
        return Ok(obj);
    }

    let property_type_runtime = property_signature_runtime_types(
        ctx,
        property.getter.as_ref(),
        property.setter.as_ref(),
        lookup,
    )
    .map(|(property_type, _)| property_type)
    .unwrap_or(RuntimeType::Object);

    let getter_obj = property
        .getter
        .clone()
        .map(|desc| crate::common::get_runtime_method_obj(ctx, desc, lookup.clone()))
        .unwrap_or_else(|| ObjectRef(None));
    let setter_obj = property
        .setter
        .clone()
        .map(|desc| crate::common::get_runtime_method_obj(ctx, desc, lookup.clone()))
        .unwrap_or_else(|| ObjectRef(None));

    let declaring_type_obj = crate::common::get_runtime_type(ctx, declaring_runtime_type.clone());
    let property_type_obj = crate::common::get_runtime_type(ctx, property_type_runtime);

    let property_info_type = ctx.loader().corlib_type("DotnetRs.PropertyInfo")?;
    let property_info_instance = ctx.new_object(property_info_type.clone())?;

    let gc = ctx.gc_with_token(&ctx.no_active_borrows_token());
    let property_info_obj = ObjectRef::new(gc, HeapStorage::Obj(Box::new(property_info_instance)));
    ctx.register_new_object(&property_info_obj);

    let name_obj = ObjectRef::new(gc, HeapStorage::Str(property.name.clone().into()));
    ctx.register_new_object(&name_obj);

    property_info_obj.as_object_mut(gc, |instance| {
        instance
            .instance_storage
            .field::<ObjectRef<'gc>>(property_info_type.clone(), "name")
            .unwrap()
            .write(name_obj);
        instance
            .instance_storage
            .field::<ObjectRef<'gc>>(property_info_type.clone(), "getter")
            .unwrap()
            .write(getter_obj);
        instance
            .instance_storage
            .field::<ObjectRef<'gc>>(property_info_type.clone(), "setter")
            .unwrap()
            .write(setter_obj);
        instance
            .instance_storage
            .field::<ObjectRef<'gc>>(property_info_type.clone(), "declaringType")
            .unwrap()
            .write(declaring_type_obj);
        instance
            .instance_storage
            .field::<ObjectRef<'gc>>(property_info_type.clone(), "propertyType")
            .unwrap()
            .write(property_type_obj);
    });

    if let Some(accessor) = accessor {
        ctx.reflection_cache_runtime_property_obj(accessor, lookup.clone(), property_info_obj);
    }

    Ok(property_info_obj)
}

pub fn handle_get_properties<'gc, T: ReflectionIntrinsicHost<'gc>>(
    ctx: &mut T,
    _generics: &GenericLookup,
) -> StepResult {
    let flags = ctx.pop_i32();
    let obj = ctx.pop_obj();
    let target_type = dotnet_vm_ops::vm_try!(crate::common::resolve_runtime_type(ctx, obj));

    let mut property_objs = Vec::new();
    if let RuntimeType::Type(ref td) | RuntimeType::Generic(ref td, _) = target_type {
        let lookup = build_generic_lookup_from_runtime_type(ctx, &target_type);
        let properties = collect_property_candidates(td, &lookup);

        for property in &properties {
            if !property_matches_binding_flags(property, flags) {
                continue;
            }

            property_objs.push(dotnet_vm_ops::vm_try!(create_runtime_property_obj(
                ctx,
                &target_type,
                property,
                &lookup,
            )));
        }
    }

    let property_info_type =
        dotnet_vm_ops::vm_try!(ctx.loader().corlib_type("System.Reflection.PropertyInfo"));
    populate_reflection_array(ctx, property_objs, ConcreteType::from(property_info_type))
}

pub fn handle_get_property_impl<'gc, T: ReflectionIntrinsicHost<'gc>>(
    ctx: &mut T,
    _generics: &GenericLookup,
) -> StepResult {
    let _modifiers = ctx.pop();
    let types_obj = ctx.pop_obj();
    let return_type_obj = ctx.pop_obj();
    let _binder = ctx.pop();
    let flags = ctx.pop_i32();
    let name_obj = ctx.pop_obj();
    let obj = ctx.pop_obj();

    let name = dotnet_vm_ops::vm_try!(string_from_heap_obj(name_obj));
    let ignore_case = (flags & BINDING_FLAGS_IGNORE_CASE) != 0;

    let expected_return_type = if return_type_obj.0.is_some() {
        Some(dotnet_vm_ops::vm_try!(crate::common::resolve_runtime_type(
            ctx,
            return_type_obj,
        )))
    } else {
        None
    };

    let expected_index_types = if types_obj.0.is_some() {
        Some(dotnet_vm_ops::vm_try!(parse_runtime_type_array(
            ctx, types_obj
        )))
    } else {
        None
    };

    let target_type = dotnet_vm_ops::vm_try!(crate::common::resolve_runtime_type(ctx, obj));

    let mut found_property = None;
    if let RuntimeType::Type(ref td) | RuntimeType::Generic(ref td, _) = target_type {
        let lookup = build_generic_lookup_from_runtime_type(ctx, &target_type);
        let properties = collect_property_candidates(td, &lookup);

        for property in &properties {
            let name_matches = if ignore_case {
                property.name.eq_ignore_ascii_case(&name)
            } else {
                property.name == name
            };

            if !name_matches || !property_matches_binding_flags(property, flags) {
                continue;
            }

            let Some((property_type, index_types)) = property_signature_runtime_types(
                ctx,
                property.getter.as_ref(),
                property.setter.as_ref(),
                &lookup,
            ) else {
                continue;
            };

            if expected_return_type
                .as_ref()
                .is_some_and(|expected| *expected != property_type)
            {
                continue;
            }

            if expected_index_types
                .as_ref()
                .is_some_and(|expected| expected != &index_types)
            {
                continue;
            }

            found_property = Some(dotnet_vm_ops::vm_try!(create_runtime_property_obj(
                ctx,
                &target_type,
                property,
                &lookup,
            )));
            break;
        }
    }

    if let Some(property_obj) = found_property {
        ctx.push_obj(property_obj);
    } else {
        ctx.push(StackValue::null());
    }
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

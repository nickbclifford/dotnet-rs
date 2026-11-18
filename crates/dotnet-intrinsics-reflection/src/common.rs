use crate::{ReflectionIntrinsicHost, RuntimeTypeContext};
use dotnet_types::{
    TypeDescription, WellKnown,
    comparer::decompose_type_source,
    error::{ExecutionError, TypeResolutionError, VmError},
    generics::GenericLookup,
    members::{FieldDescription, MethodDescription},
    runtime::{RuntimeMethodSignature, RuntimeType},
};
use dotnet_value::{
    StackValue,
    object::{Object, ObjectRef},
    storage::FieldRef,
};
use dotnet_vm_ops::SupportSlotOps;
use dotnetdll::{
    binary::signature::kinds::StandAloneCallingConvention,
    prelude::{
        BaseType, CallingConvention, Constant, MethodMemberIndex, MethodType, ParameterType,
    },
};

pub fn find_parameterless_ctor(
    td: &TypeDescription,
    lookup: &GenericLookup,
) -> Option<MethodDescription> {
    td.definition()
        .methods
        .iter()
        .enumerate()
        .find_map(|(i, method)| {
            if method.name != ".ctor" {
                return None;
            }

            let description = MethodDescription::new(
                td.clone(),
                lookup.clone(),
                td.resolution.clone(),
                MethodMemberIndex::Method(i),
            );
            let signature = description.signature();
            (signature.instance && signature.parameters.is_empty()).then_some(description)
        })
}

/// Convert a metadata constant's numeric/bool/char payload to its evaluation-stack
/// representation. Returns `None` for `String`/`Null`, which callers materialize
/// separately when appropriate.
pub(crate) fn constant_to_stack_value<'gc>(constant: &Constant) -> Option<StackValue<'gc>> {
    Some(match constant {
        Constant::Boolean(v) => StackValue::Int32(i32::from(*v)),
        Constant::Char(v) => StackValue::Int32(*v as i32),
        Constant::Int8(v) => StackValue::Int32(*v as i32),
        Constant::UInt8(v) => StackValue::Int32(*v as i32),
        Constant::Int16(v) => StackValue::Int32(*v as i32),
        Constant::UInt16(v) => StackValue::Int32(*v as i32),
        Constant::Int32(v) => StackValue::Int32(*v),
        Constant::UInt32(v) => StackValue::NativeInt(*v as isize),
        Constant::Int64(v) => StackValue::Int64(*v),
        Constant::UInt64(v) => StackValue::Int64(*v as i64),
        Constant::Float32(v) => StackValue::NativeFloat((*v).into()),
        Constant::Float64(v) => StackValue::NativeFloat(*v),
        Constant::String(_) | Constant::Null => return None,
    })
}

pub fn pre_initialize_reflection<'gc>(ctx: &mut impl ReflectionIntrinsicHost<'gc>) {
    let _gc = ctx.gc_with_token(&ctx.no_active_borrows_token());
    let blessed = [
        RuntimeType::Void,
        RuntimeType::Boolean,
        RuntimeType::Char,
        RuntimeType::Int8,
        RuntimeType::UInt8,
        RuntimeType::Int16,
        RuntimeType::UInt16,
        RuntimeType::Int32,
        RuntimeType::UInt32,
        RuntimeType::Int64,
        RuntimeType::UInt64,
        RuntimeType::Float32,
        RuntimeType::Float64,
        RuntimeType::IntPtr,
        RuntimeType::UIntPtr,
        RuntimeType::Object,
        RuntimeType::String,
    ];

    for t in blessed {
        get_runtime_type(ctx, t);
    }
}

// The returned field borrows only the object's storage; the loader validates owner identity.
fn reflection_index_field<'storage, 'gc>(
    ctx: &impl ReflectionIntrinsicHost<'gc>,
    instance: &'storage Object<'gc>,
    owner: TypeDescription,
) -> Option<FieldRef<'storage, usize>> {
    match ctx.loader().canonical_type_name(&owner.type_name()) {
        "System.RuntimeType" => ctx
            .loader()
            .runtime_type_index_field(&instance.instance_storage, owner),
        "DotnetRs.MethodInfo" => ctx
            .loader()
            .method_info_index_field(&instance.instance_storage, owner),
        "DotnetRs.ConstructorInfo" => ctx
            .loader()
            .constructor_info_index_field(&instance.instance_storage, owner),
        "DotnetRs.FieldInfo" => ctx
            .loader()
            .field_info_index_field(&instance.instance_storage, owner),
        _ => None,
    }
}

/// Reads the validated runtime-registry index slot from a reflection wrapper object.
pub(crate) fn read_reflection_index<'gc>(
    ctx: &impl ReflectionIntrinsicHost<'gc>,
    instance: &Object<'gc>,
) -> Result<usize, ExecutionError> {
    reflection_index_field(ctx, instance, instance.description.clone())
        .map(|field| field.read())
        .ok_or_else(|| {
            ExecutionError::InternalError(
                format!(
                    "reflection object {} missing a validated runtime-registry index slot",
                    instance.description.type_name()
                )
                .into(),
            )
        })
}

/// Writes the validated index slot on a freshly allocated reflection object.
pub(crate) fn write_reflection_index<'gc>(
    ctx: &impl ReflectionIntrinsicHost<'gc>,
    instance: &mut Object<'gc>,
    owner: TypeDescription,
    index: usize,
) {
    reflection_index_field(ctx, instance, owner)
        .expect("validated reflection runtime-registry index support slot")
        .write(index);
}

fn alloc_reflection_obj<'gc>(
    ctx: &mut impl ReflectionIntrinsicHost<'gc>,
    corlib_type: WellKnown,
    index: usize,
) -> ObjectRef<'gc> {
    let gc = ctx.gc_with_token(&ctx.no_active_borrows_token());
    let rt = ctx
        .loader()
        .corlib_wkt(corlib_type)
        .unwrap_or_else(|_| panic!("{} not found", corlib_type.name()));
    let rt_obj = ctx
        .new_object(rt.clone())
        .expect("Failed to create reflection object");
    let obj_ref = ctx.alloc_obj_ref(gc, rt_obj);

    obj_ref.as_object_mut(gc, |instance| {
        write_reflection_index(ctx, instance, rt, index);
    });

    obj_ref
}

pub fn get_runtime_type<'gc>(
    ctx: &mut impl ReflectionIntrinsicHost<'gc>,
    target: RuntimeType,
) -> ObjectRef<'gc> {
    if let Some(obj) = ctx.reflection_cached_runtime_type(&target) {
        return obj;
    }

    let index = ctx.reflection_runtime_type_index_get_or_insert(target.clone());
    let obj_ref = alloc_reflection_obj(ctx, WellKnown::RuntimeType, index);

    ctx.reflection_cache_runtime_type(target, obj_ref)
}

#[cfg(feature = "multithreading")]
fn try_alloc_reflection_obj<'gc>(
    ctx: &mut impl ReflectionIntrinsicHost<'gc>,
    corlib_type: WellKnown,
    index: usize,
) -> Result<ObjectRef<'gc>, TypeResolutionError> {
    let gc = ctx.gc_with_token(&ctx.no_active_borrows_token());
    let loader = ctx.loader();
    // Host materialization can run without a managed frame. Check the context prerequisite
    // and ancestry using fallible resolution before the layout code traverses those types.
    loader.corlib_wkt(WellKnown::Object)?;
    let rt = loader.corlib_wkt(corlib_type)?;
    validate_reflection_ancestry(loader, rt.clone())?;
    let rt_obj = ctx.new_object(rt.clone())?;
    let obj_ref = ctx.alloc_obj_ref(gc, rt_obj);
    #[cfg(debug_assertions)]
    ctx.reflection_assert_canonical_owner(obj_ref);
    obj_ref.as_object_mut(gc, |instance| {
        write_reflection_index(ctx, instance, rt, index);
    });
    Ok(obj_ref)
}

#[cfg(feature = "multithreading")]
#[cold]
#[inline(never)]
fn validate_reflection_ancestry(
    loader: &dotnet_assemblies::AssemblyLoader,
    mut ty: TypeDescription,
) -> Result<(), TypeResolutionError> {
    use dotnetdll::prelude::TypeSource;
    let mut seen = Vec::new();
    loop {
        if seen.contains(&ty) {
            return Err(TypeResolutionError::InvalidLayout(
                "cyclic reflection ancestry".into(),
            ));
        }
        seen.push(ty.clone());
        let Some(source) = &ty.definition().extends else {
            return Ok(());
        };
        let (TypeSource::User(parent) | TypeSource::Generic { base: parent, .. }) = source;
        let name = parent.type_name(ty.resolution.definition());
        ty = match loader.canonical_type_name(&name) {
            "System.Delegate" => loader.corlib_wkt(WellKnown::Delegate)?,
            "System.MulticastDelegate" => loader.corlib_wkt(WellKnown::MulticastDelegate)?,
            _ => loader.locate_type(ty.resolution.clone(), *parent)?,
        };
    }
}

/// Materialize a complete runtime type in the caller's arena without guest execution.
#[cfg(feature = "multithreading")]
pub fn materialize_runtime_type<'gc>(
    ctx: &mut impl ReflectionIntrinsicHost<'gc>,
    target: RuntimeType,
) -> Result<ObjectRef<'gc>, TypeResolutionError> {
    if let Some(obj) = ctx.reflection_cached_runtime_type(&target) {
        return Ok(obj);
    }
    let index = ctx.reflection_runtime_type_index_get_or_insert(target.clone());
    let obj_ref = try_alloc_reflection_obj(ctx, WellKnown::RuntimeType, index)?;
    Ok(ctx.reflection_cache_runtime_type(target, obj_ref))
}

pub fn resolve_runtime_type<'gc>(
    ctx: &impl ReflectionIntrinsicHost<'gc>,
    obj: ObjectRef<'gc>,
) -> Result<RuntimeType, ExecutionError> {
    obj.try_as_object(|instance| {
        if let Ok(index) = read_reflection_index(ctx, instance) {
            return Ok(ctx.reflection_runtime_type_by_index(index));
        }

        let index_field = ctx
            .loader()
            .corlib_wkt(WellKnown::RuntimeType)
            .ok()
            .and_then(|owner| {
                ctx.loader()
                    .runtime_type_index_field(&instance.instance_storage, owner)
            });

        if let Some(index_field) = index_field {
            return Ok(ctx.reflection_runtime_type_by_index(index_field.read()));
        }

        // `TypeDelegator.typeImpl` is private BCL layout, so it is looked up by name at run time.
        if let Some(type_impl) = ctx
            .loader()
            .corlib_wkt(WellKnown::ReflectionTypeDelegator)
            .ok()
            .and_then(|owner| {
                instance
                    .instance_storage
                    .field::<ObjectRef<'gc>>(owner, "typeImpl")
            })
            .map(|f| f.read())
        {
            return resolve_runtime_type(ctx, type_impl);
        }

        Err(ExecutionError::InternalError(
            format!(
                "Reflection runtime type object missing index field: {}",
                instance.description.type_name()
            )
            .into(),
        ))
    })?
}

pub fn resolve_attribute_filter<'gc, T: ReflectionIntrinsicHost<'gc>>(
    ctx: &mut T,
    type_obj: ObjectRef<'gc>,
) -> Result<Option<TypeDescription>, VmError> {
    if type_obj.0.is_none() {
        return Ok(None);
    }

    Ok(match resolve_runtime_type(ctx, type_obj)? {
        RuntimeType::Type(td) | RuntimeType::Generic(td, _) => Some(td),
        _ => None,
    })
}

pub fn resolve_runtime_method<'gc>(
    ctx: &impl ReflectionIntrinsicHost<'gc>,
    obj: ObjectRef<'gc>,
) -> Result<(MethodDescription, GenericLookup), ExecutionError> {
    let index = obj.try_as_object(|instance| read_reflection_index(ctx, instance))??;
    Ok(ctx.reflection_runtime_method_by_index(index))
}

pub fn resolve_runtime_field<'gc>(
    ctx: &impl ReflectionIntrinsicHost<'gc>,
    obj: ObjectRef<'gc>,
) -> Result<(FieldDescription, GenericLookup), ExecutionError> {
    let index = obj.try_as_object(|instance| read_reflection_index(ctx, instance))??;
    Ok(ctx.reflection_runtime_field_by_index(index))
}

pub fn make_runtime_type(
    res_ctx: &impl RuntimeTypeContext,
    t: &MethodType,
) -> Result<RuntimeType, TypeResolutionError> {
    let runtime_type = match t {
        MethodType::Base(b) => match &**b {
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
            BaseType::Object => RuntimeType::Object,
            BaseType::String => RuntimeType::String,
            BaseType::Type { source, .. } => {
                let (ut, generics) = decompose_type_source::<MethodType>(source);
                let td = res_ctx.reflection_locate_type(ut)?;
                if generics.is_empty() {
                    // If the type itself declares generic parameters, this is a generic type
                    // definition (e.g. `typeof(IDictionary<,>)`). Represent it as
                    // `RuntimeType::Generic` with open `TypeParameter` args so that
                    // `IsGenericTypeDefinition` can recognise it correctly.
                    let n_params = td.definition().generic_parameters.len();
                    if n_params > 0 {
                        let params = (0..n_params as u16)
                            .map(|index| RuntimeType::TypeParameter {
                                owner: td.clone(),
                                index,
                            })
                            .collect();
                        return Ok(RuntimeType::Generic(td, params));
                    }
                    match (
                        td.definition().namespace.as_deref(),
                        td.definition().name.as_ref(),
                    ) {
                        (Some("System"), "Void") => RuntimeType::Void,
                        (Some("System"), "TypedReference") => RuntimeType::TypedReference,
                        (Some("System"), "Boolean") => RuntimeType::Boolean,
                        (Some("System"), "Char") => RuntimeType::Char,
                        (Some("System"), "SByte") => RuntimeType::Int8,
                        (Some("System"), "Byte") => RuntimeType::UInt8,
                        (Some("System"), "Int16") => RuntimeType::Int16,
                        (Some("System"), "UInt16") => RuntimeType::UInt16,
                        (Some("System"), "Int32") => RuntimeType::Int32,
                        (Some("System"), "UInt32") => RuntimeType::UInt32,
                        (Some("System"), "Int64") => RuntimeType::Int64,
                        (Some("System"), "UInt64") => RuntimeType::UInt64,
                        (Some("System"), "Single") => RuntimeType::Float32,
                        (Some("System"), "Double") => RuntimeType::Float64,
                        (Some("System"), "IntPtr") => RuntimeType::IntPtr,
                        (Some("System"), "UIntPtr") => RuntimeType::UIntPtr,
                        (Some("System"), "Object") => RuntimeType::Object,
                        (Some("System"), "String") => RuntimeType::String,
                        _ => RuntimeType::Type(td),
                    }
                } else {
                    RuntimeType::Generic(
                        td,
                        generics
                            .iter()
                            .map(|g| make_runtime_type(res_ctx, g))
                            .collect::<Result<Vec<_>, _>>()?,
                    )
                }
            }
            BaseType::Vector(_, t) => {
                RuntimeType::Vector(Box::new(make_runtime_type(res_ctx, &t.clone())?))
            }
            BaseType::Array(t, shape) => RuntimeType::Array(
                Box::new(make_runtime_type(res_ctx, &t.clone())?),
                shape.rank as u32,
            ),
            BaseType::ValuePointer(_, t) => match t {
                Some(inner) => {
                    RuntimeType::Pointer(Box::new(make_runtime_type(res_ctx, &inner.clone())?))
                }
                None => RuntimeType::IntPtr,
            },
            BaseType::FunctionPointer(sig) => {
                let convert_param = |p: &ParameterType<MethodType>| match p {
                    ParameterType::Value(t) | ParameterType::Ref(t) => {
                        make_runtime_type(res_ctx, t)
                    }
                    ParameterType::TypedReference => Ok(RuntimeType::TypedReference),
                };

                RuntimeType::FunctionPointer(RuntimeMethodSignature {
                    instance: sig.instance,
                    explicit_this: sig.explicit_this,
                    calling_convention: match sig.calling_convention {
                        StandAloneCallingConvention::DefaultManaged => CallingConvention::Default,
                        StandAloneCallingConvention::Vararg => CallingConvention::Vararg,
                        _ => CallingConvention::Default,
                    },
                    return_type: Box::new(match &sig.return_type.1 {
                        Some(t) => convert_param(t)?,
                        None => RuntimeType::Void,
                    }),
                    parameters: sig
                        .parameters
                        .iter()
                        .map(|p| convert_param(&p.1))
                        .collect::<Result<Vec<_>, _>>()?,
                })
            }
        },
        MethodType::TypeGeneric(i) => RuntimeType::TypeParameter {
            owner: res_ctx.reflection_type_owner().expect("missing type owner"),
            index: *i as u16,
        },
        MethodType::MethodGeneric(i) => RuntimeType::MethodParameter {
            owner: res_ctx
                .reflection_method_owner()
                .expect("missing method owner"),
            index: *i as u16,
        },
    };

    Ok(runtime_type)
}

pub fn get_runtime_method_index<'gc>(
    ctx: &mut impl ReflectionIntrinsicHost<'gc>,
    method: MethodDescription,
    lookup: GenericLookup,
) -> usize {
    ctx.reflection_runtime_method_index_get_or_insert(method, lookup)
}

pub fn get_runtime_field_index<'gc>(
    ctx: &mut impl ReflectionIntrinsicHost<'gc>,
    field: FieldDescription,
    lookup: GenericLookup,
) -> u16 {
    ctx.reflection_runtime_field_index_get_or_insert(field, lookup) as u16
}

/// Retry the current intrinsic before it consumes operands when a shared canonical reflection
/// object must be materialized by the long-lived service. Cache hits, ST and inactive-service
/// paths remain synchronous.
#[cfg(feature = "multithreading")]
pub fn await_canonical_object<'gc>(
    ctx: &mut impl ReflectionIntrinsicHost<'gc>,
    key: dotnet_vm_ops::effects::CanonicalKey,
) -> Option<dotnet_vm_data::StepResult> {
    ctx.reflection_await_canonical(key)
}

pub fn get_runtime_method_obj<'gc>(
    ctx: &mut impl ReflectionIntrinsicHost<'gc>,
    method: MethodDescription,
    lookup: GenericLookup,
) -> ObjectRef<'gc> {
    if let Some(obj) = ctx.reflection_cached_runtime_method_obj(&method, &lookup) {
        return obj;
    }

    let index = get_runtime_method_index(ctx, method.clone(), lookup.clone());

    let is_ctor = method.method().runtime_special_name
        && (method.method().name == ".ctor" || method.method().name == ".cctor");
    let class = if is_ctor {
        WellKnown::SupportConstructorInfo
    } else {
        WellKnown::SupportMethodInfo
    };

    let obj_ref = alloc_reflection_obj(ctx, class, index);

    ctx.reflection_cache_runtime_method_obj(method, lookup, obj_ref)
}

pub fn get_runtime_field_obj<'gc>(
    ctx: &mut impl ReflectionIntrinsicHost<'gc>,
    field: FieldDescription,
    lookup: GenericLookup,
) -> ObjectRef<'gc> {
    if let Some(obj) = ctx.reflection_cached_runtime_field_obj(&field, &lookup) {
        return obj;
    }

    let index = get_runtime_field_index(ctx, field.clone(), lookup.clone()) as usize;
    let obj_ref = alloc_reflection_obj(ctx, WellKnown::SupportFieldInfo, index);

    ctx.reflection_cache_runtime_field_obj(field, lookup, obj_ref)
}

/// Materialize a complete runtime method in the caller's arena without guest execution.
#[cfg(feature = "multithreading")]
pub fn materialize_runtime_method_obj<'gc>(
    ctx: &mut impl ReflectionIntrinsicHost<'gc>,
    method: MethodDescription,
    lookup: GenericLookup,
) -> Result<ObjectRef<'gc>, TypeResolutionError> {
    if let Some(obj) = ctx.reflection_cached_runtime_method_obj(&method, &lookup) {
        return Ok(obj);
    }

    let index = get_runtime_method_index(ctx, method.clone(), lookup.clone());

    let is_ctor = method.method().runtime_special_name
        && (method.method().name == ".ctor" || method.method().name == ".cctor");
    let class = if is_ctor {
        WellKnown::SupportConstructorInfo
    } else {
        WellKnown::SupportMethodInfo
    };

    let obj_ref = try_alloc_reflection_obj(ctx, class, index)?;

    Ok(ctx.reflection_cache_runtime_method_obj(method, lookup, obj_ref))
}

/// Materialize a complete runtime field in the caller's arena without guest execution.
#[cfg(feature = "multithreading")]
pub fn materialize_runtime_field_obj<'gc>(
    ctx: &mut impl ReflectionIntrinsicHost<'gc>,
    field: FieldDescription,
    lookup: GenericLookup,
) -> Result<ObjectRef<'gc>, TypeResolutionError> {
    if let Some(obj) = ctx.reflection_cached_runtime_field_obj(&field, &lookup) {
        return Ok(obj);
    }

    let index = get_runtime_field_index(ctx, field.clone(), lookup.clone()) as usize;
    let obj_ref = try_alloc_reflection_obj(ctx, WellKnown::SupportFieldInfo, index)?;

    Ok(ctx.reflection_cache_runtime_field_obj(field, lookup, obj_ref))
}

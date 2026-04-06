use crate::{ReflectionIntrinsicHost, RuntimeTypeContext};
use dotnet_types::{
    comparer::decompose_type_source,
    error::TypeResolutionError,
    generics::GenericLookup,
    members::{FieldDescription, MethodDescription},
    runtime::{RuntimeMethodSignature, RuntimeType},
};
use dotnet_value::object::{HeapStorage, ObjectRef};
use dotnetdll::{
    binary::signature::kinds::StandAloneCallingConvention,
    prelude::{BaseType, CallingConvention, MethodType, ParameterType},
};

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

pub fn get_runtime_type<'gc>(
    ctx: &mut impl ReflectionIntrinsicHost<'gc>,
    target: RuntimeType,
) -> ObjectRef<'gc> {
    let gc = ctx.gc_with_token(&ctx.no_active_borrows_token());
    if let Some(obj) = ctx.reflection_cached_runtime_type(&target) {
        return obj;
    }

    let index = ctx.reflection_runtime_type_index_get_or_insert(target.clone());

    let rt = ctx
        .loader()
        .corlib_type("System.RuntimeType")
        .expect("System.RuntimeType not found");
    let rt_obj = ctx
        .new_object(rt.clone())
        .expect("Failed to create RuntimeType object");
    let obj_ref = ObjectRef::new(gc, HeapStorage::Obj(rt_obj));
    ctx.register_new_object(&obj_ref);

    obj_ref.as_object_mut(gc, |instance| {
        instance
            .instance_storage
            .field::<usize>(rt, "index")
            .unwrap()
            .write(index);
    });

    ctx.reflection_cache_runtime_type(target, obj_ref);
    obj_ref
}

pub fn resolve_runtime_type<'gc>(
    ctx: &impl ReflectionIntrinsicHost<'gc>,
    obj: ObjectRef<'gc>,
) -> RuntimeType {
    obj.as_object(|instance| {
        let index = instance
            .instance_storage
            .field::<usize>(instance.description.clone(), "index")
            .unwrap()
            .read();
        ctx.reflection_runtime_type_by_index(index)
    })
}

pub fn resolve_runtime_method<'gc>(
    ctx: &impl ReflectionIntrinsicHost<'gc>,
    obj: ObjectRef<'gc>,
) -> (MethodDescription, GenericLookup) {
    obj.as_object(|instance| {
        let index = instance
            .instance_storage
            .field::<usize>(instance.description.clone(), "index")
            .unwrap()
            .read();
        ctx.reflection_runtime_method_by_index(index)
    })
}

pub fn resolve_runtime_field<'gc>(
    ctx: &impl ReflectionIntrinsicHost<'gc>,
    obj: ObjectRef<'gc>,
) -> (FieldDescription, GenericLookup) {
    obj.as_object(|instance| {
        let index = instance
            .instance_storage
            .field::<usize>(instance.description.clone(), "index")
            .unwrap()
            .read();
        ctx.reflection_runtime_field_by_index(index)
    })
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
                    RuntimeType::Type(td)
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
) -> u16 {
    ctx.reflection_runtime_method_index_get_or_insert(method, lookup) as u16
}

pub fn get_runtime_field_index<'gc>(
    ctx: &mut impl ReflectionIntrinsicHost<'gc>,
    field: FieldDescription,
    lookup: GenericLookup,
) -> u16 {
    ctx.reflection_runtime_field_index_get_or_insert(field, lookup) as u16
}

pub fn get_runtime_method_obj<'gc>(
    ctx: &mut impl ReflectionIntrinsicHost<'gc>,
    method: MethodDescription,
    lookup: GenericLookup,
) -> ObjectRef<'gc> {
    let gc = ctx.gc_with_token(&ctx.no_active_borrows_token());
    if let Some(obj) = ctx.reflection_cached_runtime_method_obj(&method, &lookup) {
        return obj;
    }

    let index = get_runtime_method_index(ctx, method.clone(), lookup.clone()) as usize;

    let is_ctor = method.method().name == ".ctor" || method.method().name == ".cctor";
    let class_name = if is_ctor {
        "DotnetRs.ConstructorInfo"
    } else {
        "DotnetRs.MethodInfo"
    };

    let rt = ctx
        .loader()
        .corlib_type(class_name)
        .expect("reflection type not found");
    let rt_obj = ctx
        .new_object(rt.clone())
        .expect("Failed to create reflection object");
    let obj_ref = ObjectRef::new(gc, HeapStorage::Obj(rt_obj));
    ctx.register_new_object(&obj_ref);

    obj_ref.as_object_mut(gc, |instance| {
        instance
            .instance_storage
            .field::<usize>(rt, "index")
            .unwrap()
            .write(index);
    });

    ctx.reflection_cache_runtime_method_obj(method, lookup, obj_ref);
    obj_ref
}

pub fn get_runtime_field_obj<'gc>(
    ctx: &mut impl ReflectionIntrinsicHost<'gc>,
    field: FieldDescription,
    lookup: GenericLookup,
) -> ObjectRef<'gc> {
    let gc = ctx.gc_with_token(&ctx.no_active_borrows_token());
    if let Some(obj) = ctx.reflection_cached_runtime_field_obj(&field, &lookup) {
        return obj;
    }

    let index = get_runtime_field_index(ctx, field.clone(), lookup.clone()) as usize;

    let rt = ctx
        .loader()
        .corlib_type("DotnetRs.FieldInfo")
        .expect("FieldInfo not found");
    let rt_obj = ctx
        .new_object(rt.clone())
        .expect("Failed to create FieldInfo object");
    let obj_ref = ObjectRef::new(gc, HeapStorage::Obj(rt_obj));
    ctx.register_new_object(&obj_ref);

    obj_ref.as_object_mut(gc, |instance| {
        instance
            .instance_storage
            .field::<usize>(rt, "index")
            .unwrap()
            .write(index);
    });

    ctx.reflection_cache_runtime_field_obj(field, lookup, obj_ref);
    obj_ref
}

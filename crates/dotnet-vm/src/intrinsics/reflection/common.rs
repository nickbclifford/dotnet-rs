use crate::{
    context::ResolutionContext,
    stack::ops::{LoaderOps, ReflectionOps},
};
use dotnet_types::{
    comparer::decompose_type_source,
    generics::GenericLookup,
    members::{FieldDescription, MethodDescription},
    runtime::{RuntimeMethodSignature, RuntimeType},
};
use dotnet_value::object::{HeapStorage, ObjectRef};
use dotnetdll::prelude::{BaseType, MethodType};

#[cfg(feature = "multithreading")]
use dotnet_utils::sync::Ordering;

#[cfg(not(feature = "multithreading"))]
pub(crate) fn get_runtime_member_index<T: PartialEq>(
    members: &mut Vec<(T, GenericLookup)>,
    member: T,
    lookup: GenericLookup,
) -> usize {
    members
        .iter()
        .position(|(m, g)| *m == member && *g == lookup)
        .unwrap_or_else(|| {
            members.push((member, lookup));
            members.len() - 1
        })
}

pub(crate) fn pre_initialize_reflection<'gc, 'm: 'gc>(
    ctx: &mut (impl ReflectionOps<'gc, 'm> + LoaderOps<'m>),
) {
    let _gc = ctx.gc_with_token(&dotnet_utils::NoActiveBorrows::new());
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

pub(crate) fn get_runtime_type<'gc, 'm: 'gc>(
    ctx: &mut (impl ReflectionOps<'gc, 'm> + LoaderOps<'m>),
    target: RuntimeType,
) -> ObjectRef<'gc> {
    let gc = ctx.gc_with_token(&dotnet_utils::NoActiveBorrows::new());
    if let Some(obj) = ctx.reflection().types_read().get(&target) {
        return *obj;
    }

    #[cfg(feature = "multithreading")]
    let index = *ctx
        .shared()
        .shared_runtime_types
        .entry(target.clone())
        .or_insert_with(|| {
            let idx = ctx
                .shared()
                .next_runtime_type_index
                .fetch_add(1, Ordering::Relaxed);
            ctx.shared()
                .shared_runtime_types_rev
                .insert(idx, target.clone());
            idx
        });

    #[cfg(not(feature = "multithreading"))]
    let index = {
        let mut list = ctx.reflection().types_list_write();
        let index = list.len();
        list.push(target.clone());
        index
    };

    let rt = ctx
        .loader()
        .corlib_type("System.RuntimeType")
        .expect("System.RuntimeType not found");
    let rt_obj = ctx
        .new_object(rt)
        .expect("Failed to create RuntimeType object");
    let obj_ref = ObjectRef::new(gc, HeapStorage::Obj(rt_obj));
    ctx.register_new_object(&obj_ref);

    // Set the index field
    obj_ref.as_object_mut(gc, |instance| {
        instance
            .instance_storage
            .field::<usize>(rt, "index")
            .unwrap()
            .write(index);
    });

    ctx.reflection().types_write().insert(target, obj_ref);
    obj_ref
}

pub(crate) fn resolve_runtime_type<'gc, 'm: 'gc>(
    ctx: &(impl ReflectionOps<'gc, 'm> + LoaderOps<'m>),
    obj: ObjectRef<'gc>,
) -> RuntimeType {
    obj.as_object(|instance| {
        let index = instance
            .instance_storage
            .field::<usize>(instance.description, "index")
            .unwrap()
            .read();
        #[cfg(feature = "multithreading")]
        return ctx
            .shared()
            .shared_runtime_types_rev
            .get(&index)
            .map(|e: dashmap::mapref::one::Ref<usize, RuntimeType>| e.clone())
            .expect("invalid runtime type index");

        #[cfg(not(feature = "multithreading"))]
        ctx.reflection().types_list_read()[index].clone()
    })
}

pub(crate) fn resolve_runtime_method<'gc, 'm: 'gc>(
    ctx: &(impl ReflectionOps<'gc, 'm> + LoaderOps<'m>),
    obj: ObjectRef<'gc>,
) -> (MethodDescription, GenericLookup) {
    obj.as_object(|instance| {
        let index = instance
            .instance_storage
            .field::<usize>(instance.description, "index")
            .unwrap()
            .read();
        #[cfg(feature = "multithreading")]
        return ctx
            .shared()
            .shared_runtime_methods_rev
            .get(&index)
            .map(
                |e: dashmap::mapref::one::Ref<usize, (MethodDescription, GenericLookup)>| e.clone(),
            )
            .expect("invalid runtime method index");

        #[cfg(not(feature = "multithreading"))]
        ctx.reflection().methods_read()[index].clone()
    })
}

pub(crate) fn resolve_runtime_field<'gc, 'm: 'gc>(
    ctx: &(impl ReflectionOps<'gc, 'm> + LoaderOps<'m>),
    obj: ObjectRef<'gc>,
) -> (FieldDescription, GenericLookup) {
    obj.as_object(|instance| {
        let index = instance
            .instance_storage
            .field::<usize>(instance.description, "index")
            .unwrap()
            .read();
        #[cfg(feature = "multithreading")]
        return ctx
            .shared()
            .shared_runtime_fields_rev
            .get(&index)
            .map(|e: dashmap::mapref::one::Ref<usize, (FieldDescription, GenericLookup)>| e.clone())
            .expect("invalid runtime field index");

        #[cfg(not(feature = "multithreading"))]
        ctx.reflection().fields_read()[index].clone()
    })
}

pub(crate) fn make_runtime_type(res_ctx: &ResolutionContext, t: &MethodType) -> RuntimeType {
    match t {
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
                let td = res_ctx.locate_type(ut).expect("failed to locate type");
                if generics.is_empty() {
                    RuntimeType::Type(td)
                } else {
                    RuntimeType::Generic(
                        td,
                        generics
                            .iter()
                            .map(|g| make_runtime_type(res_ctx, g))
                            .collect(),
                    )
                }
            }
            BaseType::Vector(_, t) => {
                RuntimeType::Vector(Box::new(make_runtime_type(res_ctx, &t.clone())))
            }
            BaseType::Array(t, shape) => RuntimeType::Array(
                Box::new(make_runtime_type(res_ctx, &t.clone())),
                shape.rank as u32,
            ),
            BaseType::ValuePointer(_, t) => match t {
                Some(inner) => {
                    RuntimeType::Pointer(Box::new(make_runtime_type(res_ctx, &inner.clone())))
                }
                None => RuntimeType::IntPtr,
            },
            BaseType::FunctionPointer(_sig) => RuntimeType::FunctionPointer(RuntimeMethodSignature),
        },
        MethodType::TypeGeneric(i) => RuntimeType::TypeParameter {
            owner: res_ctx.type_owner.expect("missing type owner"),
            index: *i as u16,
        },
        MethodType::MethodGeneric(i) => RuntimeType::MethodParameter {
            owner: res_ctx.method_owner.expect("missing method owner"),
            index: *i as u16,
        },
    }
}

pub(crate) fn get_runtime_method_index<'gc, 'm: 'gc>(
    ctx: &mut (impl ReflectionOps<'gc, 'm> + LoaderOps<'m>),
    method: MethodDescription,
    lookup: GenericLookup,
) -> u16 {
    #[cfg(feature = "multithreading")]
    {
        let index = *ctx
            .shared()
            .shared_runtime_methods
            .entry((method, lookup.clone()))
            .or_insert_with(|| {
                let idx = ctx
                    .shared()
                    .next_runtime_method_index
                    .fetch_add(1, Ordering::Relaxed);
                ctx.shared()
                    .shared_runtime_methods_rev
                    .insert(idx, (method, lookup.clone()));
                idx
            });
        index as u16
    }

    #[cfg(not(feature = "multithreading"))]
    {
        let mut methods = ctx.reflection().methods_write();
        let idx = get_runtime_member_index(&mut methods, method, lookup);
        idx as u16
    }
}

pub(crate) fn get_runtime_field_index<'gc, 'm: 'gc>(
    ctx: &mut (impl ReflectionOps<'gc, 'm> + LoaderOps<'m>),
    field: FieldDescription,
    lookup: GenericLookup,
) -> u16 {
    #[cfg(feature = "multithreading")]
    {
        let index = *ctx
            .shared()
            .shared_runtime_fields
            .entry((field, lookup.clone()))
            .or_insert_with(|| {
                let idx = ctx
                    .shared()
                    .next_runtime_field_index
                    .fetch_add(1, Ordering::Relaxed);
                ctx.shared()
                    .shared_runtime_fields_rev
                    .insert(idx, (field, lookup.clone()));
                idx
            });
        index as u16
    }

    #[cfg(not(feature = "multithreading"))]
    {
        let mut fields = ctx.reflection().fields_write();
        let idx = get_runtime_member_index(&mut fields, field, lookup);
        idx as u16
    }
}

pub(crate) fn get_runtime_method_obj<'gc, 'm: 'gc>(
    ctx: &mut (impl ReflectionOps<'gc, 'm> + LoaderOps<'m>),
    method: MethodDescription,
    lookup: GenericLookup,
) -> ObjectRef<'gc> {
    let gc = ctx.gc_with_token(&dotnet_utils::NoActiveBorrows::new());
    if let Some(obj) = ctx
        .reflection()
        .method_objs_read()
        .get(&(method, lookup.clone()))
    {
        return *obj;
    }

    let index = get_runtime_method_index(ctx, method, lookup.clone()) as usize;

    let is_ctor = method.method.name == ".ctor" || method.method.name == ".cctor";
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
        .new_object(rt)
        .expect("Failed to create reflection object");
    let obj_ref = ObjectRef::new(gc, HeapStorage::Obj(rt_obj));
    ctx.register_new_object(&obj_ref);

    // Set the index field
    obj_ref.as_object_mut(gc, |instance| {
        instance
            .instance_storage
            .field::<usize>(rt, "index")
            .unwrap()
            .write(index);
    });

    ctx.reflection()
        .method_objs_write()
        .insert((method, lookup), obj_ref);
    obj_ref
}

pub(crate) fn get_runtime_field_obj<'gc, 'm: 'gc>(
    ctx: &mut (impl ReflectionOps<'gc, 'm> + LoaderOps<'m>),
    field: FieldDescription,
    lookup: GenericLookup,
) -> ObjectRef<'gc> {
    let gc = ctx.gc_with_token(&dotnet_utils::NoActiveBorrows::new());
    if let Some(obj) = ctx
        .reflection()
        .field_objs_read()
        .get(&(field, lookup.clone()))
    {
        return *obj;
    }

    let index = get_runtime_field_index(ctx, field, lookup.clone()) as usize;

    let rt = ctx
        .loader()
        .corlib_type("DotnetRs.FieldInfo")
        .expect("FieldInfo not found");
    let rt_obj = ctx
        .new_object(rt)
        .expect("Failed to create FieldInfo object");
    let obj_ref = ObjectRef::new(gc, HeapStorage::Obj(rt_obj));
    ctx.register_new_object(&obj_ref);

    // Set the index field
    obj_ref.as_object_mut(gc, |instance| {
        instance
            .instance_storage
            .field::<usize>(rt, "index")
            .unwrap()
            .write(index);
    });

    ctx.reflection()
        .field_objs_write()
        .insert((field, lookup), obj_ref);
    obj_ref
}

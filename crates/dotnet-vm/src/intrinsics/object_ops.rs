use crate::{
    StepResult,
    error::ExecutionError,
    stack::ops::{
        EvalStackOps, ExceptionOps, LoaderOps, ReflectionOps, TypedStackOps, VmReflectionOps,
    },
};
use dotnet_assemblies::AssemblyLoader;
use dotnet_macros::dotnet_intrinsic;
use dotnet_runtime_memory::ops::MemoryOps;
use dotnet_types::{
    TypeDescription,
    generics::{ConcreteType, GenericLookup},
    members::MethodDescription,
    runtime::{RuntimeType, runtime_type_from_concrete},
};
use dotnet_value::{
    StackValue,
    object::{HeapStorage, Object, ObjectRef},
    string::CLRString,
};
use dotnet_vm_ops::NULL_REF_MSG;
use dotnetdll::prelude::{BaseType, TypeSource, UserType};

#[dotnet_intrinsic("object System.Object::MemberwiseClone()")]
pub fn object_memberwise_clone<'gc, T: TypedStackOps<'gc> + ExceptionOps<'gc> + MemoryOps<'gc>>(
    ctx: &mut T,
    _method: MethodDescription,
    _lookup: &GenericLookup,
) -> StepResult {
    let obj = ctx.pop_obj();
    if obj.0.is_none() {
        return ctx.throw_by_name_with_message("System.NullReferenceException", NULL_REF_MSG);
    }

    let clone = ctx.clone_object(obj);
    ctx.push_obj(clone);
    StepResult::Continue
}

#[dotnet_intrinsic(
    "static int System.Runtime.CompilerServices.RuntimeHelpers::TryGetHashCode(object)"
)]
pub fn runtime_helpers_try_get_hash_code<'gc, T: TypedStackOps<'gc>>(
    ctx: &mut T,
    _method: MethodDescription,
    _lookup: &GenericLookup,
) -> StepResult {
    let obj = ctx.pop_obj();
    let hash = obj.as_ptr().map_or(0, |ptr| {
        let addr = ptr.as_ptr() as usize;
        let mixed = (addr ^ (addr >> 32)) as u32;
        let non_zero = if mixed == 0 { 1 } else { mixed };
        non_zero as i32
    });
    ctx.push_i32(hash);
    StepResult::Continue
}

#[dotnet_intrinsic("string System.Object::ToString()")]
pub(super) fn object_to_string<
    'gc,
    T: EvalStackOps<'gc> + TypedStackOps<'gc> + MemoryOps<'gc> + ExceptionOps<'gc>,
>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let this = ctx.pop();

    let type_name = if let StackValue::ObjectRef(obj_ref) = this {
        if obj_ref.0.is_some() {
            obj_ref.as_heap_storage(|storage| match storage {
                HeapStorage::Obj(o) => o.description.type_name(),
                HeapStorage::Str(_) => "System.String".to_string(),
                HeapStorage::Vec(_) => "System.Array".to_string(),
                HeapStorage::Boxed(_) => "System.ValueType".to_string(),
            })
        } else {
            return ctx.throw_by_name_with_message("System.NullReferenceException", NULL_REF_MSG);
        }
    } else {
        "System.Object".to_string()
    };

    let str_val = CLRString::from(type_name);
    let storage = HeapStorage::Str(str_val);
    let obj_ref = ObjectRef::new(ctx.gc_with_token(&ctx.no_active_borrows_token()), storage);
    ctx.push_obj(obj_ref);
    StepResult::Continue
}

fn integral_constant_as_i128(c: &dotnetdll::prelude::Constant) -> Option<i128> {
    use dotnetdll::prelude::Constant;
    match c {
        Constant::Boolean(v) => Some(i128::from(u8::from(*v))),
        Constant::Char(v) => Some(i128::from(*v)),
        Constant::Int8(v) => Some(i128::from(*v)),
        Constant::UInt8(v) => Some(i128::from(*v)),
        Constant::Int16(v) => Some(i128::from(*v)),
        Constant::UInt16(v) => Some(i128::from(*v)),
        Constant::Int32(v) => Some(i128::from(*v)),
        Constant::UInt32(v) => Some(i128::from(*v)),
        Constant::Int64(v) => Some(i128::from(*v)),
        Constant::UInt64(v) => Some(i128::from(*v)),
        Constant::Float32(_) | Constant::Float64(_) | Constant::String(_) | Constant::Null => None,
    }
}

fn read_enum_value(obj: &Object<'_>) -> Option<(i128, bool)> {
    let enum_type = obj.description.clone();
    let underlying = enum_type.is_enum()?;
    let method_type: dotnetdll::prelude::MethodType = underlying.clone().into();
    let dotnetdll::prelude::MethodType::Base(base) = method_type else {
        return None;
    };

    match &*base {
        // BCL-dynamic layout probe — see REVIEW.md §4 (F-SCOPE-001)
        dotnetdll::prelude::BaseType::Int8 => Some((
            i128::from(
                obj.instance_storage
                    .field::<i8>(enum_type, "value__")?
                    .read(),
            ),
            true,
        )),
        dotnetdll::prelude::BaseType::UInt8 => Some((
            i128::from(
                obj.instance_storage
                    .field::<u8>(enum_type, "value__")?
                    .read(),
            ),
            false,
        )),
        dotnetdll::prelude::BaseType::Int16 => Some((
            i128::from(
                obj.instance_storage
                    .field::<i16>(enum_type, "value__")?
                    .read(),
            ),
            true,
        )),
        dotnetdll::prelude::BaseType::UInt16 => Some((
            i128::from(
                obj.instance_storage
                    .field::<u16>(enum_type, "value__")?
                    .read(),
            ),
            false,
        )),
        dotnetdll::prelude::BaseType::Int32 => Some((
            i128::from(
                obj.instance_storage
                    .field::<i32>(enum_type, "value__")?
                    .read(),
            ),
            true,
        )),
        dotnetdll::prelude::BaseType::UInt32 => Some((
            i128::from(
                obj.instance_storage
                    .field::<u32>(enum_type, "value__")?
                    .read(),
            ),
            false,
        )),
        dotnetdll::prelude::BaseType::Int64 => Some((
            i128::from(
                obj.instance_storage
                    .field::<i64>(enum_type, "value__")?
                    .read(),
            ),
            true,
        )),
        dotnetdll::prelude::BaseType::UInt64 => Some((
            i128::from(
                obj.instance_storage
                    .field::<u64>(enum_type, "value__")?
                    .read(),
            ),
            false,
        )),
        dotnetdll::prelude::BaseType::IntPtr => Some((
            obj.instance_storage
                .field::<isize>(enum_type, "value__")?
                .read() as i128,
            true,
        )),
        dotnetdll::prelude::BaseType::UIntPtr => Some((
            obj.instance_storage
                .field::<usize>(enum_type, "value__")?
                .read() as i128,
            false,
        )),
        _ => None,
    }
}

fn read_enum_bits(obj: &Object<'_>) -> Option<(TypeDescription, u128)> {
    let enum_type = obj.description.clone();
    let underlying = enum_type.is_enum()?;
    let method_type: dotnetdll::prelude::MethodType = underlying.clone().into();
    let dotnetdll::prelude::MethodType::Base(base) = method_type else {
        return None;
    };

    let bits = match &*base {
        BaseType::Int8 => u128::from(
            obj.instance_storage
                .field::<i8>(enum_type.clone(), "value__")?
                .read() as u8,
        ),
        BaseType::UInt8 => u128::from(
            obj.instance_storage
                .field::<u8>(enum_type.clone(), "value__")?
                .read(),
        ),
        BaseType::Int16 => u128::from(
            obj.instance_storage
                .field::<i16>(enum_type.clone(), "value__")?
                .read() as u16,
        ),
        BaseType::UInt16 => u128::from(
            obj.instance_storage
                .field::<u16>(enum_type.clone(), "value__")?
                .read(),
        ),
        BaseType::Int32 => u128::from(
            obj.instance_storage
                .field::<i32>(enum_type.clone(), "value__")?
                .read() as u32,
        ),
        BaseType::UInt32 => u128::from(
            obj.instance_storage
                .field::<u32>(enum_type.clone(), "value__")?
                .read(),
        ),
        BaseType::Int64 => u128::from(
            obj.instance_storage
                .field::<i64>(enum_type.clone(), "value__")?
                .read() as u64,
        ),
        BaseType::UInt64 => u128::from(
            obj.instance_storage
                .field::<u64>(enum_type.clone(), "value__")?
                .read(),
        ),
        BaseType::IntPtr => obj
            .instance_storage
            .field::<isize>(enum_type.clone(), "value__")?
            .read() as usize as u128,
        BaseType::UIntPtr => obj
            .instance_storage
            .field::<usize>(enum_type.clone(), "value__")?
            .read() as u128,
        _ => return None,
    };

    Some((enum_type, bits))
}

fn object_from_heap_storage<'gc>(storage: &HeapStorage<'gc>) -> Option<Object<'gc>> {
    match storage {
        HeapStorage::Boxed(obj) | HeapStorage::Obj(obj) => Some((**obj).clone()),
        HeapStorage::Str(_) | HeapStorage::Vec(_) => None,
    }
}

fn type_from_heap_storage(storage: &HeapStorage<'_>) -> Option<TypeDescription> {
    match storage {
        HeapStorage::Boxed(obj) | HeapStorage::Obj(obj) => Some(obj.description.clone()),
        HeapStorage::Str(_) | HeapStorage::Vec(_) => None,
    }
}

fn enum_object_from_stack<'gc, T: ExceptionOps<'gc>>(
    ctx: &mut T,
    value: StackValue<'gc>,
) -> Result<Object<'gc>, StepResult> {
    let enum_obj = match value {
        StackValue::ObjectRef(obj_ref) => {
            if obj_ref.0.is_none() {
                return Err(
                    ctx.throw_by_name_with_message("System.NullReferenceException", NULL_REF_MSG)
                );
            }

            obj_ref.as_heap_storage(object_from_heap_storage)
        }
        StackValue::ManagedPtr(ptr) => {
            if ptr.is_null() {
                return Err(
                    ctx.throw_by_name_with_message("System.NullReferenceException", NULL_REF_MSG)
                );
            }

            ptr.owner()
                .and_then(|owner| owner.as_heap_storage(object_from_heap_storage))
        }
        StackValue::ValueType(vt) => Some(vt),
        _ => None,
    };

    enum_obj.ok_or_else(|| {
        ctx.throw_by_name_with_message(
            "System.InvalidCastException",
            "Specified cast is not valid.",
        )
    })
}

pub(super) fn format_enum_value_from_type(
    enum_type: &TypeDescription,
    raw_value: i128,
    signed: bool,
) -> String {
    for field in &enum_type.definition().fields {
        if !field.literal {
            continue;
        }

        let Some(constant) = field.default.as_ref() else {
            continue;
        };

        if integral_constant_as_i128(constant) == Some(raw_value) {
            return field.name.to_string();
        }
    }

    if signed {
        raw_value.to_string()
    } else {
        (raw_value as u128).to_string()
    }
}

pub(super) fn scalar_enum_value_from_stack<'gc>(value: StackValue<'gc>) -> Option<(i128, bool)> {
    match value.coerce_enum_to_underlying() {
        StackValue::Int32(v) => Some((i128::from(v), true)),
        StackValue::Int64(v) => Some((i128::from(v), true)),
        StackValue::NativeInt(v) => Some((v as i128, true)),
        StackValue::ValueType(vt) => read_enum_value(&vt),
        _ => None,
    }
}

pub(super) fn enum_type_from_stack_value<'gc>(value: &StackValue<'gc>) -> Option<TypeDescription> {
    match value {
        StackValue::ValueType(vt) => Some(vt.description.clone()),
        StackValue::ObjectRef(obj_ref) => obj_ref.as_heap_storage(type_from_heap_storage),
        StackValue::ManagedPtr(ptr) => ptr
            .owner()
            .and_then(|owner| owner.as_heap_storage(type_from_heap_storage)),
        _ => None,
    }
}

pub(super) fn generic_enum_type(generics: &GenericLookup) -> Option<TypeDescription> {
    let concrete = generics
        .method_generics
        .first()
        .or_else(|| generics.type_generics.first())?;

    let dotnetdll::prelude::BaseType::Type {
        source: TypeSource::User(UserType::Definition(index)),
        ..
    } = concrete.get()
    else {
        return None;
    };

    Some(TypeDescription::new(concrete.resolution(), *index))
}

pub(super) fn format_enum_value<'gc, T: ExceptionOps<'gc>>(
    ctx: &mut T,
    value: StackValue<'gc>,
) -> Result<String, StepResult> {
    let enum_obj = enum_object_from_stack(ctx, value)?;
    let enum_type = enum_obj.description.clone();
    let Some((raw_value, signed)) = read_enum_value(&enum_obj) else {
        return Err(ctx.throw_by_name_with_message(
            "System.InvalidCastException",
            "Specified cast is not valid.",
        ));
    };

    Ok(format_enum_value_from_type(&enum_type, raw_value, signed))
}

#[dotnet_intrinsic("bool System.Enum::HasFlag(System.Enum)")]
pub(super) fn enum_has_flag<'gc, T: TypedStackOps<'gc> + ExceptionOps<'gc>>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let flag = ctx.pop();
    let this = ctx.pop();

    let flag_obj = match enum_object_from_stack(ctx, flag) {
        Ok(v) => v,
        Err(step) => return step,
    };
    let this_obj = match enum_object_from_stack(ctx, this) {
        Ok(v) => v,
        Err(step) => return step,
    };

    let Some((flag_type, flag_bits)) = read_enum_bits(&flag_obj) else {
        return ctx.throw_by_name_with_message(
            "System.ArgumentException",
            "Object must be an enum type.",
        );
    };
    let Some((this_type, this_bits)) = read_enum_bits(&this_obj) else {
        return ctx.throw_by_name_with_message(
            "System.ArgumentException",
            "Object must be an enum type.",
        );
    };

    if this_type != flag_type {
        return ctx.throw_by_name_with_message(
            "System.ArgumentException",
            "The argument type must be the same as the enum type.",
        );
    }

    ctx.push_i32(i32::from((this_bits & flag_bits) == flag_bits));
    StepResult::Continue
}

#[dotnet_intrinsic("System.Type System.Object::GetType()")]
pub(super) fn object_get_type<
    'gc,
    T: TypedStackOps<'gc> + ExceptionOps<'gc> + ReflectionOps<'gc> + VmReflectionOps<'gc> + LoaderOps,
>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    fn runtime_type_from_desc(td: dotnet_types::TypeDescription) -> RuntimeType {
        match td.type_name().as_str() {
            "System.Boolean" => RuntimeType::Boolean,
            "System.Char" => RuntimeType::Char,
            "System.SByte" => RuntimeType::Int8,
            "System.Byte" => RuntimeType::UInt8,
            "System.Int16" => RuntimeType::Int16,
            "System.UInt16" => RuntimeType::UInt16,
            "System.Int32" => RuntimeType::Int32,
            "System.UInt32" => RuntimeType::UInt32,
            "System.Int64" => RuntimeType::Int64,
            "System.UInt64" => RuntimeType::UInt64,
            "System.Single" => RuntimeType::Float32,
            "System.Double" => RuntimeType::Float64,
            "System.IntPtr" => RuntimeType::IntPtr,
            "System.UIntPtr" => RuntimeType::UIntPtr,
            _ => RuntimeType::Type(td),
        }
    }

    fn runtime_type_from_object(loader: &AssemblyLoader, object: &Object<'_>) -> RuntimeType {
        let type_arity = object.description.definition().generic_parameters.len();
        if type_arity == 0 {
            return runtime_type_from_desc(object.description.clone());
        }

        let type_args: Vec<_> = object
            .generics
            .type_generics
            .iter()
            .take(type_arity)
            .cloned()
            .collect();

        if type_args.len() != type_arity {
            return RuntimeType::Type(object.description.clone());
        }

        let concrete = ConcreteType::new(
            object.description.resolution.clone(),
            BaseType::Type {
                source: TypeSource::Generic {
                    base: UserType::Definition(object.description.index),
                    parameters: type_args,
                },
                value_kind: None,
            },
        );

        runtime_type_from_concrete(loader, &concrete)
            .unwrap_or_else(|| RuntimeType::Type(object.description.clone()))
    }

    fn runtime_type_from_heap(loader: &AssemblyLoader, object_ref: ObjectRef<'_>) -> RuntimeType {
        object_ref.as_heap_storage(|storage| match storage {
            HeapStorage::Obj(o) => runtime_type_from_object(loader, o),
            HeapStorage::Str(_) => RuntimeType::String,
            HeapStorage::Vec(v) => {
                let element_rt =
                    runtime_type_from_concrete(loader, &v.element).unwrap_or(RuntimeType::Object);
                if v.dims.len() <= 1 {
                    RuntimeType::Vector(Box::new(element_rt))
                } else {
                    RuntimeType::Array(Box::new(element_rt), v.dims.len() as u32)
                }
            }
            HeapStorage::Boxed(o) => runtime_type_from_object(loader, o),
        })
    }

    let this = ctx.pop();
    let rt = match this {
        StackValue::ObjectRef(this_ref) => {
            if this_ref.0.is_none() {
                return ctx
                    .throw_by_name_with_message("System.NullReferenceException", NULL_REF_MSG);
            }
            runtime_type_from_heap(ctx.loader().as_ref(), this_ref)
        }
        StackValue::ManagedPtr(this_ptr) => {
            if this_ptr.is_null() {
                return ctx
                    .throw_by_name_with_message("System.NullReferenceException", NULL_REF_MSG);
            }

            if let Some(owner) = this_ptr.owner() {
                runtime_type_from_heap(ctx.loader().as_ref(), owner)
            } else {
                runtime_type_from_desc(this_ptr.inner_type())
            }
        }
        StackValue::ValueType(value) => runtime_type_from_desc(value.description.clone()),
        other => {
            return StepResult::Error(
                ExecutionError::TypeMismatch {
                    expected: "ObjectRef/ManagedPtr/ValueType",
                    actual: format!("{other:?}").into(),
                }
                .into(),
            );
        }
    };

    let typ_obj = ctx.get_runtime_type(rt);
    ctx.push_obj(typ_obj);
    StepResult::Continue
}

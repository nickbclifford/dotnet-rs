use crate::ReflectionIntrinsicHost;
use dotnet_macros::dotnet_intrinsic;
use dotnet_types::{
    TypeDescription,
    generics::{ConcreteType, GenericLookup},
    members::MethodDescription,
    runtime::{RuntimeType, runtime_type_from_concrete},
};
use dotnet_value::{
    CLRString, StackValue,
    object::{CTSValue, HeapStorage, ObjectRef},
};
use dotnet_vm_ops::{
    StepResult,
    ops::{LoaderOps, MemoryOps, TypedStackOps},
};
use dotnetdll::prelude::Constant;

#[dotnet_intrinsic("bool DotnetRs.FieldInfo::IsDefined(System.Type, bool)")]
pub fn intrinsic_field_info_is_defined<'gc, T: ReflectionIntrinsicHost<'gc>>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let _inherit = ctx.pop_i32();
    let attribute_type_obj = ctx.pop_obj();
    let this = ctx.pop_obj();
    let attribute_filter = if attribute_type_obj.0.is_some() {
        let filter_rt =
            dotnet_vm_ops::vm_try!(crate::common::resolve_runtime_type(ctx, attribute_type_obj));
        match filter_rt {
            RuntimeType::Type(td) | RuntimeType::Generic(td, _) => Some(td),
            _ => None,
        }
    } else {
        None
    };
    let (field, _) = dotnet_vm_ops::vm_try!(crate::common::resolve_runtime_field(ctx, this));
    let attrs = dotnet_vm_ops::vm_try!(crate::types::collect_field_custom_attributes(
        ctx,
        field,
        attribute_filter
    ));
    ctx.push_i32(if attrs.is_empty() { 0 } else { 1 });
    StepResult::Continue
}

#[dotnet_intrinsic("object[] DotnetRs.FieldInfo::GetCustomAttributes(bool)")]
pub fn intrinsic_field_info_get_custom_attributes_bool<'gc, T: ReflectionIntrinsicHost<'gc>>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let _inherit = ctx.pop_i32();
    let this = ctx.pop_obj();
    let (field, _) = dotnet_vm_ops::vm_try!(crate::common::resolve_runtime_field(ctx, this));
    let attrs = dotnet_vm_ops::vm_try!(crate::types::collect_field_custom_attributes(
        ctx, field, None
    ));
    let object_type = dotnet_vm_ops::vm_try!(ctx.loader().corlib_type("System.Object"));
    crate::types::populate_reflection_array(ctx, attrs, ConcreteType::from(object_type))
}

#[dotnet_intrinsic("object[] DotnetRs.FieldInfo::GetCustomAttributes(System.Type, bool)")]
pub fn intrinsic_field_info_get_custom_attributes_typed<'gc, T: ReflectionIntrinsicHost<'gc>>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let _inherit = ctx.pop_i32();
    let attribute_type_obj = ctx.pop_obj();
    let this = ctx.pop_obj();
    let attribute_filter = if attribute_type_obj.0.is_some() {
        let filter_rt =
            dotnet_vm_ops::vm_try!(crate::common::resolve_runtime_type(ctx, attribute_type_obj));
        match filter_rt {
            RuntimeType::Type(td) | RuntimeType::Generic(td, _) => Some(td),
            _ => None,
        }
    } else {
        None
    };
    let (field, _) = dotnet_vm_ops::vm_try!(crate::common::resolve_runtime_field(ctx, this));
    let attrs = dotnet_vm_ops::vm_try!(crate::types::collect_field_custom_attributes(
        ctx,
        field,
        attribute_filter
    ));
    let object_type = dotnet_vm_ops::vm_try!(ctx.loader().corlib_type("System.Object"));
    crate::types::populate_reflection_array(ctx, attrs, ConcreteType::from(object_type))
}

#[dotnet_intrinsic("string DotnetRs.FieldInfo::GetName()")]
pub fn intrinsic_field_info_get_name<'gc, T: ReflectionIntrinsicHost<'gc>>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let obj_ref = ctx.pop_obj();
    let (field, _) = dotnet_vm_ops::vm_try!(crate::common::resolve_runtime_field(ctx, obj_ref));
    ctx.push_string(field.field().name.clone().into());
    StepResult::Continue
}

#[dotnet_intrinsic("System.Type DotnetRs.FieldInfo::GetDeclaringType()")]
pub fn intrinsic_field_info_get_declaring_type<'gc, T: ReflectionIntrinsicHost<'gc>>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let obj_ref = ctx.pop_obj();
    let (field, _) = dotnet_vm_ops::vm_try!(crate::common::resolve_runtime_field(ctx, obj_ref));
    let rt_obj = crate::common::get_runtime_type(ctx, RuntimeType::Type(field.parent));
    ctx.push_obj(rt_obj);
    StepResult::Continue
}

#[dotnet_intrinsic("System.Type DotnetRs.FieldInfo::GetFieldType()")]
pub fn intrinsic_field_info_get_field_type<'gc, T: ReflectionIntrinsicHost<'gc>>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let obj_ref = ctx.pop_obj();
    let (field, lookup) =
        dotnet_vm_ops::vm_try!(crate::common::resolve_runtime_field(ctx, obj_ref));
    let field_type: dotnetdll::prelude::MethodType = field.field().return_type.clone().into();

    let runtime_type = lookup
        .make_concrete(
            field.resolution(),
            field_type.clone(),
            ctx.loader().as_ref(),
        )
        .ok()
        .and_then(|concrete| runtime_type_from_concrete(ctx.loader().as_ref(), &concrete))
        .unwrap_or_else(|| ctx.reflection_make_runtime_type_with_lookup(&field_type, &lookup));

    let rt_obj = crate::common::get_runtime_type(ctx, runtime_type);
    ctx.push_obj(rt_obj);
    StepResult::Continue
}

fn push_boxed_constant<'gc, T: ReflectionIntrinsicHost<'gc>>(
    ctx: &mut T,
    type_name: &str,
    value: StackValue<'gc>,
) -> StepResult {
    let t = dotnet_vm_ops::vm_try!(ctx.loader().corlib_type(type_name));
    let boxed = dotnet_vm_ops::vm_try!(ctx.box_value(&ConcreteType::from(t), value));
    ctx.push_obj(boxed);
    StepResult::Continue
}

#[dotnet_intrinsic("object DotnetRs.FieldInfo::GetRawConstantValue()")]
pub fn intrinsic_field_info_get_raw_constant_value<'gc, T: ReflectionIntrinsicHost<'gc>>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    const INVALID_OPERATION_MESSAGE: &str =
        "Operation is not valid due to the current state of the object.";

    let obj_ref = ctx.pop_obj();
    let (field, _) = dotnet_vm_ops::vm_try!(crate::common::resolve_runtime_field(ctx, obj_ref));
    let field_def = field.field();

    if !field_def.literal {
        return ctx.throw_by_name_with_message(
            "System.InvalidOperationException",
            INVALID_OPERATION_MESSAGE,
        );
    }

    let Some(constant) = field_def.default.as_ref() else {
        return ctx.throw_by_name_with_message(
            "System.InvalidOperationException",
            INVALID_OPERATION_MESSAGE,
        );
    };

    match constant {
        Constant::Boolean(v) => {
            push_boxed_constant(ctx, "System.Boolean", StackValue::Int32(i32::from(*v)))
        }
        Constant::Char(v) => push_boxed_constant(ctx, "System.Char", StackValue::Int32(*v as i32)),
        Constant::Int8(v) => push_boxed_constant(ctx, "System.SByte", StackValue::Int32(*v as i32)),
        Constant::UInt8(v) => push_boxed_constant(ctx, "System.Byte", StackValue::Int32(*v as i32)),
        Constant::Int16(v) => {
            push_boxed_constant(ctx, "System.Int16", StackValue::Int32(*v as i32))
        }
        Constant::UInt16(v) => {
            push_boxed_constant(ctx, "System.UInt16", StackValue::Int32(*v as i32))
        }
        Constant::Int32(v) => push_boxed_constant(ctx, "System.Int32", StackValue::Int32(*v)),
        Constant::UInt32(v) => {
            push_boxed_constant(ctx, "System.UInt32", StackValue::NativeInt(*v as isize))
        }
        Constant::Int64(v) => push_boxed_constant(ctx, "System.Int64", StackValue::Int64(*v)),
        Constant::UInt64(v) => {
            push_boxed_constant(ctx, "System.UInt64", StackValue::Int64(*v as i64))
        }
        Constant::Float32(v) => {
            push_boxed_constant(ctx, "System.Single", StackValue::NativeFloat((*v).into()))
        }
        Constant::Float64(v) => {
            push_boxed_constant(ctx, "System.Double", StackValue::NativeFloat(*v))
        }
        Constant::String(chars) => {
            ctx.push_string(CLRString::new(chars.clone()));
            StepResult::Continue
        }
        Constant::Null => {
            ctx.push(StackValue::null());
            StepResult::Continue
        }
    }
}

/// Convert a metadata constant's numeric/bool/char payload to its evaluation-stack
/// representation. Returns `None` for `String`/`Null`, which are materialized
/// directly by the caller rather than boxed.
fn numeric_constant_to_stack<'gc>(constant: &Constant) -> Option<StackValue<'gc>> {
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

/// `FieldInfo.GetValue(obj)` — supports literal constants and instance fields.
/// Unlike `GetRawConstantValue`, which returns the boxed underlying primitive,
/// `GetValue` boxes constants as the field's declared type, so an enum member
/// yields a boxed value of the enum type.
#[dotnet_intrinsic("object DotnetRs.FieldInfo::GetValue(object)")]
pub fn intrinsic_field_info_get_value<'gc, T: ReflectionIntrinsicHost<'gc>>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    // Argument layout: `this` (FieldInfo), then `obj`. Pop in reverse order.
    let obj = ctx.pop();
    let this = ctx.pop_obj();
    let (field, lookup) = dotnet_vm_ops::vm_try!(crate::common::resolve_runtime_field(ctx, this));
    let field_def = field.field();

    if !field_def.literal {
        if field_def.static_member {
            // Static non-literal fields are accessible via normal field access at runtime;
            // reflection-based reads are not yet fully implemented. Return null for now
            // so attribute-default-lookup paths (e.g. AttributeCollection.GetDefaultAttribute)
            // fall through to their null-check fallback rather than throwing.
            ctx.push(StackValue::null());
            return StepResult::Continue;
        }
        let StackValue::ObjectRef(target_obj) = obj else {
            return ctx.throw_by_name_with_message(
                "System.ArgumentException",
                "FieldInfo.GetValue requires an object instance for instance fields.",
            );
        };

        let field_type: dotnetdll::prelude::MethodType = field_def.return_type.clone().into();
        let concrete_field_type = dotnet_vm_ops::vm_try!(lookup.make_concrete(
            field.resolution(),
            field_type,
            ctx.loader().as_ref(),
        ));
        let field_bytes = dotnet_vm_ops::vm_try!(read_instance_field_bytes(
            target_obj,
            field.parent.clone(),
            field_def.name.as_ref(),
        ));
        let value = dotnet_vm_ops::vm_try!(
            ctx.read_cts_value(&concrete_field_type, &field_bytes)
                .map(|value| value.into_stack())
        );

        if let StackValue::ObjectRef(obj) = value {
            ctx.push_obj(obj);
        } else {
            let boxed = dotnet_vm_ops::vm_try!(ctx.box_value(&concrete_field_type, value));
            ctx.push_obj(boxed);
        }
        return StepResult::Continue;
    }

    let Some(constant) = field_def.default.as_ref() else {
        return ctx.throw_by_name_with_message(
            "System.InvalidOperationException",
            "Operation is not valid due to the current state of the object.",
        );
    };

    match constant {
        Constant::String(chars) => {
            ctx.push_string(CLRString::new(chars.clone()));
            StepResult::Continue
        }
        Constant::Null => {
            ctx.push(StackValue::null());
            StepResult::Continue
        }
        _ => {
            let value = numeric_constant_to_stack(constant)
                .expect("non-string/null constant must produce a stack value");
            // Box as the field's declared type (the enum type for enum members).
            let field_type: dotnetdll::prelude::MethodType = field_def.return_type.clone().into();
            let field_ct = dotnet_vm_ops::vm_try!(ctx.make_concrete(&field_type));
            let boxed = dotnet_vm_ops::vm_try!(ctx.box_value(&field_ct, value));
            ctx.push_obj(boxed);
            StepResult::Continue
        }
    }
}

#[dotnet_intrinsic(
    "void DotnetRs.FieldInfo::SetValue(object, object, System.Reflection.BindingFlags, System.Reflection.Binder, System.Globalization.CultureInfo)"
)]
pub fn intrinsic_field_info_set_value<'gc, T: ReflectionIntrinsicHost<'gc>>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    // Argument layout: `this` (FieldInfo), then `obj`, `value`, `invokeAttr`, `binder`, `culture`.
    // Pop in reverse order.
    let _culture = ctx.pop();
    let _binder = ctx.pop();
    let _invoke_attr = ctx.pop_i32();
    let value = ctx.pop();
    let obj = ctx.pop();
    let this = ctx.pop_obj();

    let (field, lookup) = dotnet_vm_ops::vm_try!(crate::common::resolve_runtime_field(ctx, this));
    let field_def = field.field();

    if field_def.literal {
        return ctx.throw_by_name_with_message(
            "System.FieldAccessException",
            "Cannot set a literal field.",
        );
    }

    if field_def.static_member {
        return ctx.throw_by_name_with_message(
            "System.NotImplementedException",
            "DotnetRs.FieldInfo.SetValue does not yet support static field writes.",
        );
    }

    let StackValue::ObjectRef(target_obj) = obj else {
        return ctx.throw_by_name_with_message(
            "System.ArgumentException",
            "FieldInfo.SetValue requires an object instance for instance fields.",
        );
    };

    if target_obj.0.is_none() {
        return ctx.throw_by_name_with_message(
            "System.ArgumentException",
            "FieldInfo.SetValue requires a non-null object instance for instance fields.",
        );
    }

    let field_type: dotnetdll::prelude::MethodType = field_def.return_type.clone().into();
    let concrete_field_type = dotnet_vm_ops::vm_try!(lookup.make_concrete(
        field.resolution(),
        field_type,
        ctx.loader().as_ref(),
    ));

    let field_is_value_type = concrete_field_type.is_value_type(ctx.loader().as_ref());
    let writes_default_zero =
        field_is_value_type && matches!(value, StackValue::ObjectRef(ObjectRef(None)));

    if writes_default_zero {
        dotnet_vm_ops::vm_try!(write_instance_field_value(
            target_obj,
            field.parent.clone(),
            field_def.name.as_ref(),
            None,
        ));
        return StepResult::Continue;
    }

    let value = if field_is_value_type {
        dotnet_vm_ops::vm_try!(coerce_field_set_value_argument(
            ctx,
            value,
            &concrete_field_type,
        ))
    } else {
        value
    };

    let cts_value = dotnet_vm_ops::vm_try!(ctx.new_cts_value(&concrete_field_type, value));
    dotnet_vm_ops::vm_try!(write_instance_field_value(
        target_obj,
        field.parent.clone(),
        field_def.name.as_ref(),
        Some(&cts_value),
    ));

    StepResult::Continue
}

fn coerce_field_set_value_argument<'gc, T: ReflectionIntrinsicHost<'gc>>(
    ctx: &mut T,
    value: StackValue<'gc>,
    field_type: &ConcreteType,
) -> Result<StackValue<'gc>, dotnet_types::error::VmError> {
    let StackValue::ObjectRef(value_obj) = value else {
        return Ok(value);
    };

    let Some(_) = value_obj.0 else {
        return Ok(value);
    };

    let boxed_value = value_obj.as_heap_storage(|storage| match storage {
        HeapStorage::Boxed(obj) => Ok(obj
            .instance_storage
            .with_data(|data| ctx.read_cts_value(field_type, data))),
        HeapStorage::Obj(_) => Err(dotnet_types::error::ExecutionError::TypeMismatch {
            expected: "boxed value type",
            actual: "object".into(),
        }),
        HeapStorage::Vec(_) => Err(dotnet_types::error::ExecutionError::TypeMismatch {
            expected: "boxed value type",
            actual: "vector".into(),
        }),
        HeapStorage::Str(_) => Err(dotnet_types::error::ExecutionError::TypeMismatch {
            expected: "boxed value type",
            actual: "string".into(),
        }),
    });

    match boxed_value {
        Ok(Ok(v)) => Ok(v.into_stack()),
        Ok(Err(e)) => Err(e.into()),
        Err(e) => Err(e.into()),
    }
}

fn read_instance_field_bytes<'gc>(
    target_obj: ObjectRef<'gc>,
    field_parent: TypeDescription,
    field_name: &str,
) -> Result<Vec<u8>, dotnet_types::error::ExecutionError> {
    target_obj.try_as_heap_storage(|storage| match storage {
        HeapStorage::Obj(instance) | HeapStorage::Boxed(instance) => Ok(instance
            .instance_storage
            .get_field_local(field_parent, field_name)
            .to_vec()),
        HeapStorage::Vec(_) => Err(dotnet_types::error::ExecutionError::TypeMismatch {
            expected: "object with fields",
            actual: "Vector".into(),
        }),
        HeapStorage::Str(_) => Err(dotnet_types::error::ExecutionError::TypeMismatch {
            expected: "object with fields",
            actual: "String".into(),
        }),
    })?
}

fn write_instance_field_value<'gc>(
    target_obj: ObjectRef<'gc>,
    field_parent: TypeDescription,
    field_name: &str,
    value: Option<&CTSValue<'gc>>,
) -> Result<(), dotnet_types::error::ExecutionError> {
    target_obj.try_as_heap_storage(|storage| match storage {
        HeapStorage::Obj(instance) | HeapStorage::Boxed(instance) => {
            let mut field_data = instance
                .instance_storage
                .get_field_mut_local(field_parent, field_name);

            if let Some(value) = value {
                value.write(&mut field_data);
            } else {
                field_data.fill(0);
            }

            Ok(())
        }
        HeapStorage::Vec(_) => Err(dotnet_types::error::ExecutionError::TypeMismatch {
            expected: "object with fields",
            actual: "Vector".into(),
        }),
        HeapStorage::Str(_) => Err(dotnet_types::error::ExecutionError::TypeMismatch {
            expected: "object with fields",
            actual: "String".into(),
        }),
    })?
}

#[dotnet_intrinsic("System.RuntimeFieldHandle DotnetRs.FieldInfo::GetFieldHandle()")]
pub fn intrinsic_field_info_get_field_handle<
    'gc,
    T: TypedStackOps<'gc> + MemoryOps<'gc> + LoaderOps,
>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let obj_ref = ctx.pop_obj();

    let rfh = dotnet_vm_ops::vm_try!(ctx.loader().corlib_type("System.RuntimeFieldHandle"));
    let instance = dotnet_vm_ops::vm_try!(ctx.new_object(rfh.clone()));
    obj_ref.write(&mut instance.instance_storage.get_field_mut_local(rfh, "_value"));

    ctx.push(StackValue::ValueType(instance));
    StepResult::Continue
}

/// `DotnetRs.FieldInfo.GetFieldAttributes()` — returns the `System.Reflection.FieldAttributes`
/// flags value for the field. These flags are defined in ECMA-335 §II.23.1.5.
#[dotnet_intrinsic(
    "valuetype System.Reflection.FieldAttributes DotnetRs.FieldInfo::GetFieldAttributes()"
)]
pub fn intrinsic_field_info_get_field_attributes<'gc, T: ReflectionIntrinsicHost<'gc>>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let obj_ref = ctx.pop_obj();
    let (field, _) = dotnet_vm_ops::vm_try!(crate::common::resolve_runtime_field(ctx, obj_ref));
    let field_def = field.field();

    // Reconstruct FieldAttributes flags from the field's metadata bits.
    // Accessibility bits occupy positions 0–2 (field-access mask = 0x07).
    let mut flags = field_def.accessibility.to_mask() as i32;

    if field_def.static_member {
        flags |= 0x10; // FieldAttributes.Static
    }
    if field_def.init_only {
        flags |= 0x20; // FieldAttributes.InitOnly
    }
    if field_def.literal {
        flags |= 0x40; // FieldAttributes.Literal
        flags |= 0x8000; // FieldAttributes.HasDefault (literals always have a default value)
    }
    if field_def.default.is_some() && !field_def.literal {
        flags |= 0x8000; // FieldAttributes.HasDefault
    }
    if field_def.runtime_special_name {
        flags |= 0x0400; // FieldAttributes.RTSpecialName
    }
    if field_def.special_name {
        flags |= 0x0200; // FieldAttributes.SpecialName
    }

    ctx.push_i32(flags);
    StepResult::Continue
}

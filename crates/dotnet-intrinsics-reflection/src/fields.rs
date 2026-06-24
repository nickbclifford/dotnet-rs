use crate::ReflectionIntrinsicHost;
use dotnet_macros::dotnet_intrinsic;
use dotnet_types::{
    generics::{ConcreteType, GenericLookup},
    members::MethodDescription,
    runtime::RuntimeType,
};
use dotnet_value::{CLRString, StackValue};
use dotnet_vm_ops::{
    StepResult,
    ops::{LoaderOps, MemoryOps, TypedStackOps},
};
use dotnetdll::prelude::Constant;

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

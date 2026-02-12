use crate::{StepResult, stack::ops::VesOps};
use dotnet_macros::dotnet_intrinsic;
use dotnet_types::{generics::GenericLookup, members::MethodDescription, runtime::RuntimeType};
use dotnet_value::StackValue;

#[dotnet_intrinsic("string DotnetRs.FieldInfo::GetName()")]
pub fn intrinsic_field_info_get_name<'gc, 'm: 'gc>(
    ctx: &mut dyn VesOps<'gc, 'm>,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let obj_ref = ctx.pop_obj();
    let (field, _) = ctx.resolve_runtime_field(obj_ref);
    ctx.push_string(field.field.name.clone().into());
    StepResult::Continue
}

#[dotnet_intrinsic("System.Type DotnetRs.FieldInfo::GetDeclaringType()")]
pub fn intrinsic_field_info_get_declaring_type<'gc, 'm: 'gc>(
    ctx: &mut dyn VesOps<'gc, 'm>,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let obj_ref = ctx.pop_obj();
    let (field, _) = ctx.resolve_runtime_field(obj_ref);
    let rt_obj = ctx.get_runtime_type(RuntimeType::Type(field.parent));
    ctx.push_obj(rt_obj);
    StepResult::Continue
}

#[dotnet_intrinsic("System.RuntimeFieldHandle DotnetRs.FieldInfo::GetFieldHandle()")]
pub fn intrinsic_field_info_get_field_handle<'gc, 'm: 'gc>(
    ctx: &mut dyn VesOps<'gc, 'm>,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let obj_ref = ctx.pop_obj();

    let rfh = vm_try!(ctx.loader().corlib_type("System.RuntimeFieldHandle"));
    let instance = vm_try!(ctx.new_object(rfh));
    obj_ref.write(&mut instance.instance_storage.get_field_mut_local(rfh, "_value"));

    ctx.push(StackValue::ValueType(instance));
    StepResult::Continue
}

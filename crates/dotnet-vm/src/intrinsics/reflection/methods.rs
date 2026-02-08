use crate::{
    StepResult,
    stack::ops::VesOps,
};
use dotnet_macros::dotnet_intrinsic;
use dotnet_types::{
    generics::GenericLookup,
    members::MethodDescription,
    runtime::RuntimeType,
};
use dotnet_utils::gc::GCHandle;
use dotnet_value::{StackValue, object::ObjectRef};


#[dotnet_intrinsic("string System.Reflection.MethodInfo::get_Name()")]
#[dotnet_intrinsic("System.Type System.Reflection.MethodInfo::get_DeclaringType()")]
#[dotnet_intrinsic("System.Type System.Reflection.MethodInfo::get_ReturnType()")]
#[dotnet_intrinsic(
    "System.Reflection.MethodAttributes System.Reflection.MethodBase::get_Attributes()"
)]
#[dotnet_intrinsic(
    "System.Reflection.CallingConventions System.Reflection.MethodBase::get_CallingConvention()"
)]
#[dotnet_intrinsic("bool System.Reflection.MethodBase::get_IsGenericMethod()")]
#[dotnet_intrinsic("bool System.Reflection.MethodBase::get_IsGenericMethodDefinition()")]
#[dotnet_intrinsic("bool System.Reflection.MethodBase::get_ContainsGenericParameters()")]
#[dotnet_intrinsic("System.Type[] System.Reflection.MethodBase::GetGenericArguments()")]
#[dotnet_intrinsic("System.RuntimeMethodHandle System.Reflection.MethodBase::get_MethodHandle()")]
#[dotnet_intrinsic("string System.Reflection.MethodInfo::ToString()")]
#[dotnet_intrinsic("string DotnetRs.RuntimeMethodInfo::get_Name()")]
#[dotnet_intrinsic("string DotnetRs.RuntimeMethodInfo::GetName()")]
#[dotnet_intrinsic("System.Type DotnetRs.RuntimeMethodInfo::get_DeclaringType()")]
#[dotnet_intrinsic("System.Type DotnetRs.RuntimeMethodInfo::GetDeclaringType()")]
#[dotnet_intrinsic("System.Type DotnetRs.RuntimeMethodInfo::get_ReturnType()")]
#[dotnet_intrinsic("System.Type DotnetRs.RuntimeMethodInfo::GetReturnType()")]
#[dotnet_intrinsic(
    "System.Reflection.MethodAttributes DotnetRs.RuntimeMethodInfo::get_Attributes()"
)]
#[dotnet_intrinsic(
    "System.Reflection.MethodAttributes DotnetRs.RuntimeMethodInfo::GetAttributes()"
)]
#[dotnet_intrinsic(
    "System.Reflection.CallingConventions DotnetRs.RuntimeMethodInfo::get_CallingConvention()"
)]
#[dotnet_intrinsic(
    "System.Reflection.CallingConventions DotnetRs.RuntimeMethodInfo::GetCallingConvention()"
)]
#[dotnet_intrinsic("bool DotnetRs.RuntimeMethodInfo::get_IsGenericMethod()")]
#[dotnet_intrinsic("bool DotnetRs.RuntimeMethodInfo::GetIsGenericMethod()")]
#[dotnet_intrinsic("bool DotnetRs.RuntimeMethodInfo::get_IsGenericMethodDefinition()")]
#[dotnet_intrinsic("bool DotnetRs.RuntimeMethodInfo::GetIsGenericMethodDefinition()")]
#[dotnet_intrinsic("bool DotnetRs.RuntimeMethodInfo::get_ContainsGenericParameters()")]
#[dotnet_intrinsic("bool DotnetRs.RuntimeMethodInfo::GetContainsGenericParameters()")]
#[dotnet_intrinsic("System.Type[] DotnetRs.RuntimeMethodInfo::GetGenericArguments()")]
#[dotnet_intrinsic("System.RuntimeMethodHandle DotnetRs.RuntimeMethodInfo::get_MethodHandle()")]
#[dotnet_intrinsic("System.RuntimeMethodHandle DotnetRs.RuntimeMethodInfo::GetMethodHandle()")]
#[dotnet_intrinsic("string DotnetRs.RuntimeMethodInfo::ToString()")]
pub fn runtime_method_info_intrinsic_call<'gc, 'm: 'gc>(
    ctx: &mut dyn VesOps<'gc, 'm>,
    gc: GCHandle<'gc>,
    method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let method_name = &*method.method.name;
    let param_count = method.method.signature.parameters.len();

    let result = match (method_name, param_count) {
        ("GetName" | "get_Name", 0) => {
            let obj = ctx.pop_obj(gc);
            let (method, _) = ctx.resolve_runtime_method(obj);
            ctx.push_string(gc, method.method.name.clone().into());
            Some(StepResult::Continue)
        }
        ("GetDeclaringType" | "get_DeclaringType", 0) => {
            let obj = ctx.pop_obj(gc);
            let (method, _) = ctx.resolve_runtime_method(obj);
            let rt_obj = ctx.get_runtime_type(gc, RuntimeType::Type(method.parent));
            ctx.push_obj(gc, rt_obj);
            Some(StepResult::Continue)
        }
        ("GetMethodHandle" | "get_MethodHandle", 0) => {
            let obj = ctx.pop_obj(gc);

            let rmh = ctx.loader().corlib_type("System.RuntimeMethodHandle");
            let instance = ctx.new_object(rmh);
            obj.write(&mut instance.instance_storage.get_field_mut_local(rmh, "_value"));

            ctx.push(gc, StackValue::ValueType(instance));
            Some(StepResult::Continue)
        }
        _ => None,
    };

    result.expect("unimplemented method info intrinsic");
    StepResult::Continue
}

#[dotnet_intrinsic(
    "static System.IntPtr System.RuntimeMethodHandle::GetFunctionPointer(System.RuntimeMethodHandle)"
)]
#[dotnet_intrinsic(
    "static System.IntPtr DotnetRs.RuntimeMethodHandle::GetFunctionPointer(DotnetRs.RuntimeMethodHandle)"
)]
pub fn intrinsic_method_handle_get_function_pointer<'gc, 'm: 'gc>(
    ctx: &mut dyn VesOps<'gc, 'm>,
    gc: GCHandle<'gc>,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let handle = ctx.pop_value_type(gc);
    let method_obj = unsafe {
        ObjectRef::read_branded(
            &handle
                .instance_storage
                .get_field_local(handle.description, "_value"),
            gc,
        )
    };
    let (method, lookup) = ctx.resolve_runtime_method(method_obj);
    let index = ctx.get_runtime_method_index(method, lookup);
    ctx.push_isize(gc, index as isize);
    StepResult::Continue
}

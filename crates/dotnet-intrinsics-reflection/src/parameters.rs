use crate::ReflectionIntrinsicHost;
use dotnet_macros::dotnet_intrinsic;
use dotnet_types::{generics::GenericLookup, members::MethodDescription, runtime::RuntimeType};
use dotnet_value::object::ObjectRef;
use dotnet_vm_ops::{
    StepResult,
    ops::{LoaderOps, TypedStackOps},
};
use dotnetdll::resolved::signature::ParameterType;

#[dotnet_intrinsic("string DotnetRs.ParameterInfo::GetName()")]
#[dotnet_intrinsic("System.Type DotnetRs.ParameterInfo::GetParameterType()")]
pub fn runtime_parameter_info_intrinsic_call<'gc, T: ReflectionIntrinsicHost<'gc>>(
    ctx: &mut T,
    method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let method_name = &*method.method().name;

    match method_name {
        "GetName" => {
            let obj = ctx.pop_obj();
            let (_m_desc, _lookup, position) = resolve_runtime_parameter(ctx, obj);
            ctx.push_string(format!("p{}", position).into());
            StepResult::Continue
        }
        "GetParameterType" => {
            let obj = ctx.pop_obj();
            let (m_desc, lookup, position) = resolve_runtime_parameter(ctx, obj);
            let param_type = &m_desc.method().signature.parameters[position].1;

            let rt = match param_type {
                ParameterType::Value(t) => {
                    ctx.reflection_make_runtime_type_for_method(m_desc.clone(), &lookup, t)
                }
                ParameterType::Ref(t) => RuntimeType::ByRef(Box::new(
                    ctx.reflection_make_runtime_type_for_method(m_desc.clone(), &lookup, t),
                )),
                ParameterType::TypedReference => RuntimeType::TypedReference,
            };
            let rt_obj = crate::common::get_runtime_type(ctx, rt);
            ctx.push_obj(rt_obj);
            StepResult::Continue
        }
        _ => unreachable!("unhandled ParameterInfo intrinsic: {}", method_name),
    }
}

pub(crate) fn resolve_runtime_parameter<'gc>(
    ctx: &(impl TypedStackOps<'gc> + LoaderOps + crate::ReflectionRegistryHost<'gc>),
    obj: ObjectRef<'gc>,
) -> (MethodDescription, GenericLookup, usize) {
    obj.as_object(|instance| {
        let method_index = instance
            .instance_storage
            .field::<usize>(instance.description.clone(), "method_index")
            .unwrap()
            .read();
        let position = instance
            .instance_storage
            .field::<i32>(instance.description.clone(), "position")
            .unwrap()
            .read();
        let (m_desc, lookup) = ctx.reflection_runtime_method_by_index(method_index);
        (m_desc, lookup, position as usize)
    })
}

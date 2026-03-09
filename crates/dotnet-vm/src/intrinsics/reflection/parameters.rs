use crate::{
    StepResult,
    context::ResolutionContext,
    intrinsics::reflection::common::make_runtime_type,
    stack::ops::{LoaderOps, ReflectionOps, TypedStackOps},
};
use dotnet_macros::dotnet_intrinsic;
use dotnet_types::{generics::GenericLookup, members::MethodDescription, runtime::RuntimeType};
use dotnet_value::object::ObjectRef;
use dotnetdll::resolved::signature::ParameterType;

#[dotnet_intrinsic("string DotnetRs.ParameterInfo::GetName()")]
#[dotnet_intrinsic("System.Type DotnetRs.ParameterInfo::GetParameterType()")]
pub fn runtime_parameter_info_intrinsic_call<
    'gc,
    T: TypedStackOps<'gc> + ReflectionOps<'gc> + LoaderOps,
>(
    ctx: &mut T,
    method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let method_name = &*method.method().name;

    match method_name {
        "GetName" => {
            let obj = ctx.pop_obj();
            let (_m_desc, _lookup, position) = resolve_runtime_parameter(ctx, obj);
            // Metadata may not have parameter names; use a generic name if missing
            ctx.push_string(format!("p{}", position).into());
            StepResult::Continue
        }
        "GetParameterType" => {
            let obj = ctx.pop_obj();
            let (m_desc, lookup, position) = resolve_runtime_parameter(ctx, obj);
            let param_type = &m_desc.method().signature.parameters[position].1;

            let res_ctx = ResolutionContext::for_method(
                m_desc,
                ctx.loader_arc(),
                &lookup,
                ctx.shared().caches.clone(),
                Some(dotnet_utils::sync::Arc::downgrade(ctx.shared())),
            );
            let rt = match param_type {
                ParameterType::Value(t) => make_runtime_type(&res_ctx, t),
                ParameterType::Ref(t) => {
                    RuntimeType::ByRef(Box::new(make_runtime_type(&res_ctx, t)))
                }
                ParameterType::TypedReference => RuntimeType::TypedReference,
            };
            let rt_obj = ctx.get_runtime_type(rt);
            ctx.push_obj(rt_obj);
            StepResult::Continue
        }
        _ => unreachable!("unhandled ParameterInfo intrinsic: {}", method_name),
    }
}

pub(crate) fn resolve_runtime_parameter<'gc>(
    ctx: &(impl ReflectionOps<'gc> + LoaderOps),
    obj: ObjectRef<'gc>,
) -> (MethodDescription, GenericLookup, usize) {
    obj.as_object(|instance| {
        let method_index = instance
            .instance_storage
            .field::<usize>(instance.description, "method_index")
            .unwrap()
            .read();
        let position = instance
            .instance_storage
            .field::<i32>(instance.description, "position")
            .unwrap()
            .read();
        let (m_desc, lookup) = ctx.lookup_method_by_index(method_index);
        (m_desc, lookup, position as usize)
    })
}

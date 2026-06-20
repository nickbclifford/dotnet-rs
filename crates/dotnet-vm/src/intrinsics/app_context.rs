use crate::{
    StepResult,
    instructions::objects::get_ptr_info,
    stack::ops::{ExceptionOps, RawMemoryOps, TypedStackOps, VmLoaderOps},
};
use dotnet_macros::dotnet_intrinsic;
use dotnet_types::{generics::GenericLookup, members::MethodDescription};
use dotnet_value::{
    StackValue,
    layout::{LayoutManager, Scalar},
    object::ObjectRef,
    with_string,
};

#[dotnet_intrinsic("static bool System.AppContext::TryGetSwitch(string, bool)")]
pub fn intrinsic_app_context_try_get_switch<
    'gc,
    T: TypedStackOps<'gc> + ExceptionOps<'gc> + RawMemoryOps<'gc> + VmLoaderOps,
>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let is_enabled_out_ptr = ctx.pop();
    let switch_name = with_string!(ctx, ctx.pop(), |s| s.as_string().to_string());

    let switch_value = ctx
        .shared()
        .app_context_switches
        .get(&switch_name)
        .map(|entry| *entry.value());

    let is_enabled = switch_value.unwrap_or(false);
    let (origin, offset) = match get_ptr_info(ctx, &is_enabled_out_ptr) {
        Ok(v) => v,
        Err(e) => return e,
    };

    let layout = LayoutManager::Scalar(Scalar::UInt8);
    dotnet_vm_ops::vm_try!(unsafe {
        ctx.write_unaligned(origin, offset, StackValue::Int32(i32::from(is_enabled)), &layout)
    });

    ctx.push_i32(i32::from(switch_value.is_some()));
    StepResult::Continue
}

#[dotnet_intrinsic("static object System.AppContext::GetData(string)")]
pub fn intrinsic_app_context_get_data<'gc, T: TypedStackOps<'gc>>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let _name = ctx.pop();
    ctx.push_obj(ObjectRef(None));
    StepResult::Continue
}

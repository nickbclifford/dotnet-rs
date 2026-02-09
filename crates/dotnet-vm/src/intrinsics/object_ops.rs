use crate::{StepResult, stack::ops::VesOps};
use dotnet_macros::dotnet_intrinsic;
use dotnet_types::{generics::GenericLookup, members::MethodDescription};
use dotnet_utils::gc::GCHandle;

#[dotnet_intrinsic("object System.Object::MemberwiseClone()")]
pub fn object_memberwise_clone<'gc, 'm: 'gc>(
    ctx: &mut dyn VesOps<'gc, 'm>,
    gc: GCHandle<'gc>,
    _method: MethodDescription,
    _lookup: &GenericLookup,
) -> StepResult {
    let obj = ctx.pop_obj(gc);
    if obj.0.is_none() {
        return ctx.throw_by_name(gc, "System.NullReferenceException");
    }

    let clone = ctx.clone_object(gc, obj);
    ctx.push_obj(gc, clone);
    StepResult::Continue
}

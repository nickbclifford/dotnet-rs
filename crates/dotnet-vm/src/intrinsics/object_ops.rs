use crate::{StepResult, stack::ops::{ExceptionOps, TypedStackOps}};
use crate::memory::ops::MemoryOps;
use dotnet_macros::dotnet_intrinsic;
use dotnet_types::{generics::GenericLookup, members::MethodDescription};

const NULL_REF_MSG: &str = "Object reference not set to an instance of an object.";

#[dotnet_intrinsic("object System.Object::MemberwiseClone()")]
pub fn object_memberwise_clone<'gc, 'm: 'gc, T: TypedStackOps<'gc> + ExceptionOps<'gc> + MemoryOps<'gc>>(
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

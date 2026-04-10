use crate::{
    StepResult,
    stack::ops::{ExceptionOps, TypedStackOps},
};
use dotnet_macros::dotnet_intrinsic;
use dotnet_runtime_memory::ops::MemoryOps;
use dotnet_types::{generics::GenericLookup, members::MethodDescription};

const NULL_REF_MSG: &str = "Object reference not set to an instance of an object.";

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

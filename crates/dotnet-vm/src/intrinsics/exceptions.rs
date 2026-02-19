use crate::{StepResult, error::ExecutionError, stack::ops::VesOps};
use dotnet_macros::dotnet_intrinsic;
use dotnet_types::{generics::GenericLookup, members::MethodDescription};
use dotnet_value::{
    StackValue,
    layout::{LayoutManager, Scalar},
};

#[dotnet_intrinsic(
    "static void System.Exception::GetStackTracesDeepCopy(System.Exception, byte[]&, object[]&)"
)]
pub fn intrinsic_get_stack_traces_deep_copy<'gc, 'm: 'gc>(
    ctx: &mut dyn VesOps<'gc, 'm>,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    // Arguments: Exception, out byte[], out object[]
    let dynamic_method_array_ptr = ctx.pop();
    let current_stack_trace_ptr = ctx.pop();
    let _exception = ctx.pop_obj();

    let layout = LayoutManager::Scalar(Scalar::ObjectRef);

    let (origin, offset) =
        match crate::instructions::objects::get_ptr_info(ctx, &current_stack_trace_ptr) {
            Ok(v) => v,
            Err(e) => return e,
        };
    vm_try!(unsafe {
        ctx.write_unaligned(origin, offset, StackValue::null(), &layout)
            .map_err(ExecutionError::NotImplemented)
    });

    let (origin, offset) =
        match crate::instructions::objects::get_ptr_info(ctx, &dynamic_method_array_ptr) {
            Ok(v) => v,
            Err(e) => return e,
        };
    vm_try!(unsafe {
        ctx.write_unaligned(origin, offset, StackValue::null(), &layout)
            .map_err(ExecutionError::NotImplemented)
    });

    StepResult::Continue
}

#[dotnet_intrinsic(
    "static void System.Exception::SaveStackTracesFromDeepCopy(System.Exception, byte[], object[])"
)]
pub fn intrinsic_save_stack_traces_from_deep_copy<'gc, 'm: 'gc>(
    ctx: &mut dyn VesOps<'gc, 'm>,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    // Arguments: Exception, byte[], object[]
    let _dynamic_method_array = ctx.pop();
    let _current_stack_trace = ctx.pop();
    let _exception = ctx.pop_obj();

    // Stub: do nothing
    StepResult::Continue
}

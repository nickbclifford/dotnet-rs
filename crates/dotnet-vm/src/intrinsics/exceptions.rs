use crate::{
    StepResult,
    instructions::objects::get_ptr_info,
    stack::ops::{EvalStackOps, ExceptionOps, LoaderOps, MemoryOps, RawMemoryOps, TypedStackOps},
};
use dotnet_macros::dotnet_intrinsic;
use dotnet_types::{generics::GenericLookup, members::MethodDescription};
use dotnet_value::{
    StackValue,
    layout::{LayoutManager, Scalar},
};

#[dotnet_intrinsic(
    "static void System.Exception::GetStackTracesDeepCopy(System.Exception, byte[]&, object[]&)"
)]
pub fn intrinsic_get_stack_traces_deep_copy<
    'gc,
    T: EvalStackOps<'gc> + TypedStackOps<'gc> + ExceptionOps<'gc> + RawMemoryOps<'gc>,
>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    // Arguments: Exception, out byte[], out object[]
    let dynamic_method_array_ptr = ctx.pop();
    let current_stack_trace_ptr = ctx.pop();
    let _exception = ctx.pop_obj();

    let layout = LayoutManager::Scalar(Scalar::ObjectRef);

    let (origin, offset) = match get_ptr_info(ctx, &current_stack_trace_ptr) {
        Ok(v) => v,
        Err(e) => return e,
    };
    dotnet_vm_ops::vm_try!(unsafe {
        ctx.write_unaligned(origin, offset, StackValue::null(), &layout)
    });

    let (origin, offset) = match get_ptr_info(ctx, &dynamic_method_array_ptr) {
        Ok(v) => v,
        Err(e) => return e,
    };
    dotnet_vm_ops::vm_try!(unsafe {
        ctx.write_unaligned(origin, offset, StackValue::null(), &layout)
    });

    StepResult::Continue
}

#[dotnet_intrinsic(
    "static void System.Exception::SaveStackTracesFromDeepCopy(System.Exception, byte[], object[])"
)]
pub fn intrinsic_save_stack_traces_from_deep_copy<'gc, T: TypedStackOps<'gc>>(
    ctx: &mut T,
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

#[dotnet_intrinsic("System.Exception/DispatchState System.Exception::CaptureDispatchState()")]
pub fn intrinsic_exception_capture_dispatch_state<
    'gc,
    T: TypedStackOps<'gc> + LoaderOps + MemoryOps<'gc>,
>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let _this = ctx.pop_obj();

    let dispatch_state_type = dotnet_vm_ops::vm_try!(
        ctx.loader()
            .corlib_type("System.Exception/DispatchState")
            .or_else(|_| ctx.loader().corlib_type("System.Exception+DispatchState"))
    );
    let dispatch_state = dotnet_vm_ops::vm_try!(ctx.new_object(dispatch_state_type));
    ctx.push_value_type(dispatch_state);
    StepResult::Continue
}

#[dotnet_intrinsic("static bool System.Exception::IsImmutableAgileException(System.Exception)")]
pub fn intrinsic_exception_is_immutable_agile_exception<'gc, T: TypedStackOps<'gc>>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let _exception = ctx.pop_obj();
    ctx.push_i32(0);
    StepResult::Continue
}

#[dotnet_intrinsic("static void System.Exception::PrepareForForeignExceptionRaise()")]
pub fn intrinsic_exception_prepare_for_foreign_exception_raise<'gc, T: TypedStackOps<'gc>>(
    _ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    StepResult::Continue
}

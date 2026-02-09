use crate::{
    StepResult,
    stack::ops::{ResolutionOps, StackOps},
};
use dotnet_macros::dotnet_instruction;
use dotnet_utils::gc::GCHandle;
use dotnet_value::StackValue;
use dotnetdll::prelude::*;

#[dotnet_instruction(NoOperation)]
pub fn nop<'gc, 'm: 'gc>(
    _ctx: &mut dyn crate::stack::ops::VesOps<'gc, 'm>,
    _gc: GCHandle<'gc>,
) -> StepResult {
    StepResult::Continue
}
#[dotnet_instruction(Pop)]
pub fn pop<'gc, 'm: 'gc, T: StackOps<'gc, 'm> + ?Sized>(
    ctx: &mut T,
    gc: GCHandle<'gc>,
) -> StepResult {
    match ctx.pop_safe(gc) {
        Ok(_) => StepResult::Continue,
        Err(e) => StepResult::Error(e),
    }
}

#[dotnet_instruction(Duplicate)]
pub fn duplicate<'gc, 'm: 'gc, T: StackOps<'gc, 'm> + ?Sized>(
    ctx: &mut T,
    gc: GCHandle<'gc>,
) -> StepResult {
    match ctx.pop_safe(gc) {
        Ok(val) => {
            ctx.push(gc, val.clone());
            ctx.push(gc, val);
            StepResult::Continue
        }
        Err(e) => StepResult::Error(e),
    }
}

load_const!(
    #[dotnet_instruction(LoadConstantInt32(val))]
    ldc_i4,
    i32,
    StackValue::Int32
);
load_const!(
    #[dotnet_instruction(LoadConstantInt64(val))]
    ldc_i8,
    i64,
    StackValue::Int64
);
load_const!(
    #[dotnet_instruction(LoadConstantFloat32(val))]
    ldc_r4,
    f32,
    |v| StackValue::NativeFloat(v as f64)
);
load_const!(
    #[dotnet_instruction(LoadConstantFloat64(val))]
    ldc_r8,
    f64,
    StackValue::NativeFloat
);
#[dotnet_instruction(LoadNull)]
pub fn ldnull<'gc, 'm: 'gc, T: StackOps<'gc, 'm> + ?Sized>(
    ctx: &mut T,
    gc: GCHandle<'gc>,
) -> StepResult {
    ctx.push(gc, StackValue::null());
    StepResult::Continue
}

load_var!(
    #[dotnet_instruction(LoadArgument(index))]
    ldarg,
    get_argument
);

#[dotnet_instruction(LoadArgumentAddress(index))]
pub fn ldarga<'gc, 'm: 'gc, T: StackOps<'gc, 'm> + ResolutionOps<'gc, 'm> + ?Sized>(
    ctx: &mut T,
    gc: GCHandle<'gc>,
    index: u16,
) -> StepResult {
    let frame = ctx.current_frame();
    let abs_idx = frame.base.arguments + index as usize;
    let arg = ctx.get_argument(index as usize);
    let live_type = ctx.stack_value_type(&arg);
    ctx.push(
        gc,
        StackValue::managed_stack_ptr(
            abs_idx,
            0,
            ctx.get_argument_address(index as usize).as_ptr() as *mut _,
            live_type,
            true,
        ),
    );
    StepResult::Continue
}

store_var!(
    #[dotnet_instruction(StoreArgument(index))]
    starg,
    set_argument
);
load_var!(
    #[dotnet_instruction(LoadLocal(index))]
    ldloc,
    get_local
);

#[dotnet_instruction(LoadLocalAddress(index))]
pub fn ldloca<'gc, 'm: 'gc, T: StackOps<'gc, 'm> + ResolutionOps<'gc, 'm> + ?Sized>(
    ctx: &mut T,
    gc: GCHandle<'gc>,
    index: u16,
) -> StepResult {
    let frame = ctx.current_frame();
    let abs_idx = frame.base.locals + index as usize;
    let local = ctx.get_local(index as usize);
    let live_type = ctx.stack_value_type(&local);

    let (ptr, pinned) = ctx.get_local_info_for_managed_ptr(index as usize);

    ctx.push(
        gc,
        StackValue::managed_stack_ptr(abs_idx, 0, ptr.as_ptr() as *mut _, live_type, pinned),
    );
    StepResult::Continue
}

store_var!(
    #[dotnet_instruction(StoreLocal(index))]
    stloc,
    set_local
);

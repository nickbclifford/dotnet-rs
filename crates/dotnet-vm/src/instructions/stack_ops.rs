use crate::{
    ArgumentIndex, LocalIndex, StepResult,
    stack::ops::{ArgumentOps, EvalStackOps, ExceptionOps, LocalOps, VmStackOps},
};
use dotnet_macros::dotnet_instruction;
use dotnet_value::StackValue;

#[dotnet_instruction(NoOperation)]
pub fn nop<T: ?Sized>(_ctx: &mut T) -> StepResult {
    StepResult::Continue
}
#[dotnet_instruction(Pop)]
pub fn pop<'gc, T: EvalStackOps<'gc>>(ctx: &mut T) -> StepResult {
    match ctx.pop_safe() {
        Ok(_) => StepResult::Continue,
        Err(e) => StepResult::Error(e),
    }
}

#[dotnet_instruction(Duplicate)]
pub fn duplicate<'gc, T: EvalStackOps<'gc>>(ctx: &mut T) -> StepResult {
    match ctx.pop_safe() {
        Ok(val) => {
            ctx.push(val.clone());
            ctx.push(val);
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
pub fn ldnull<'gc, T: EvalStackOps<'gc>>(ctx: &mut T) -> StepResult {
    ctx.push(StackValue::null());
    StepResult::Continue
}

load_var!(
    #[dotnet_instruction(LoadArgument(index))]
    ldarg,
    get_argument,
    ArgumentIndex
);

#[dotnet_instruction(LoadArgumentAddress(index))]
pub fn ldarga<'gc, T: VmStackOps<'gc> + EvalStackOps<'gc> + ExceptionOps<'gc>>(
    ctx: &mut T,
    index: u16,
) -> StepResult {
    let pointer = match ctx.argument_address(ArgumentIndex::new(index as usize)) {
        Ok(pointer) => pointer,
        Err(crate::error::TypeResolutionError::InvalidLayout(_)) => {
            return ctx.throw_by_name_with_message(
                "System.InvalidProgramException",
                "Common Language Runtime detected an invalid program.",
            );
        }
        Err(error) => return StepResult::Error(error.into()),
    };
    ctx.push(pointer);
    StepResult::Continue
}

#[dotnet_instruction(StoreArgument(index))]
pub fn starg<'gc, T: ArgumentOps<'gc> + EvalStackOps<'gc>>(ctx: &mut T, index: u16) -> StepResult {
    let value = vm_pop!(ctx);
    match ctx.set_argument(ArgumentIndex::new(index as usize), value) {
        Ok(()) => StepResult::Continue,
        Err(error) => StepResult::Error(error),
    }
}
load_var!(
    #[dotnet_instruction(LoadLocal(index))]
    ldloc,
    get_local,
    LocalIndex
);

#[dotnet_instruction(LoadLocalAddress(index))]
pub fn ldloca<'gc, T: VmStackOps<'gc> + EvalStackOps<'gc> + ExceptionOps<'gc>>(
    ctx: &mut T,
    index: u16,
) -> StepResult {
    let pointer = match ctx.local_address(LocalIndex::new(index as usize)) {
        Ok(pointer) => pointer,
        Err(crate::error::TypeResolutionError::InvalidLayout(_)) => {
            return ctx.throw_by_name_with_message(
                "System.InvalidProgramException",
                "Common Language Runtime detected an invalid program.",
            );
        }
        Err(error) => return StepResult::Error(error.into()),
    };
    ctx.push(pointer);
    StepResult::Continue
}

#[dotnet_instruction(StoreLocal(index))]
pub fn stloc<'gc, T: LocalOps<'gc> + EvalStackOps<'gc>>(ctx: &mut T, index: u16) -> StepResult {
    let value = vm_pop!(ctx);
    match ctx.set_local(LocalIndex::new(index as usize), value) {
        Ok(()) => StepResult::Continue,
        Err(error) => StepResult::Error(error),
    }
}

use crate::{instructions::macros::*, resolution::ValueResolution, CallStack, StepResult};
use dotnet_macros::dotnet_instruction;
use dotnet_utils::gc::GCHandle;
use dotnet_value::StackValue;
use dotnetdll::prelude::*;

#[dotnet_instruction(NoOperation)]
pub fn nop<'gc, 'm: 'gc>(_gc: GCHandle<'gc>, _stack: &mut CallStack<'gc, 'm>) -> StepResult {
    StepResult::Continue
}
#[dotnet_instruction(Pop)]
pub fn pop<'gc, 'm: 'gc>(gc: GCHandle<'gc>, stack: &mut CallStack<'gc, 'm>) -> StepResult {
    stack.pop(gc);
    StepResult::Continue
}

#[dotnet_instruction(Duplicate)]
pub fn duplicate<'gc, 'm: 'gc>(gc: GCHandle<'gc>, stack: &mut CallStack<'gc, 'm>) -> StepResult {
    let val = stack.pop(gc);
    stack.push(gc, val.clone());
    stack.push(gc, val);
    StepResult::Continue
}

load_const!(
    #[dotnet_instruction(LoadConstantInt32)]
    ldc_i4,
    i32,
    StackValue::Int32
);
load_const!(
    #[dotnet_instruction(LoadConstantInt64)]
    ldc_i8,
    i64,
    StackValue::Int64
);
load_const!(
    #[dotnet_instruction(LoadConstantFloat32)]
    ldc_r4,
    f32,
    |v| StackValue::NativeFloat(v as f64)
);
load_const!(
    #[dotnet_instruction(LoadConstantFloat64)]
    ldc_r8,
    f64,
    StackValue::NativeFloat
);
#[dotnet_instruction(LoadNull)]
pub fn ldnull<'gc, 'm: 'gc>(gc: GCHandle<'gc>, stack: &mut CallStack<'gc, 'm>) -> StepResult {
    stack.push(gc, StackValue::null());
    StepResult::Continue
}

load_var!(
    #[dotnet_instruction(LoadArgument)]
    ldarg,
    get_argument
);

#[dotnet_instruction(LoadArgumentAddress)]
pub fn ldarga<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    index: u16,
) -> StepResult {
    let arg = stack.get_argument(index as usize);
    let ctx = stack.current_context();
    let live_type = ctx.stack_value_type(&arg);
    stack.push(
        gc,
        StackValue::managed_ptr(
            stack.get_argument_address(index as usize).as_ptr() as *mut _,
            live_type,
            true,
        ),
    );
    StepResult::Continue
}

store_var!(
    #[dotnet_instruction(StoreArgument)]
    starg,
    set_argument
);
load_var!(
    #[dotnet_instruction(LoadLocal)]
    ldloc,
    get_local
);

#[dotnet_instruction(LoadLocalAddress)]
pub fn ldloca<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    index: u16,
) -> StepResult {
    let local = stack.get_local(index as usize);
    let ctx = stack.current_context();
    let live_type = ctx.stack_value_type(&local);

    let (ptr, pinned) = stack.get_local_info_for_managed_ptr(index as usize);

    stack.push(
        gc,
        StackValue::managed_ptr_with_owner(ptr.as_ptr() as *mut _, live_type, None, pinned),
    );
    StepResult::Continue
}

store_var!(
    #[dotnet_instruction(StoreLocal)]
    stloc,
    set_local
);

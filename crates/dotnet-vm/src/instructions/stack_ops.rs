use crate::resolution::ValueResolution;
use crate::{CallStack, StepResult};
use dotnet_macros::dotnet_instruction;
use dotnet_utils::gc::GCHandle;
use dotnet_value::StackValue;
use dotnetdll::prelude::*;

#[dotnet_instruction(NoOperation)]
pub fn nop<'gc, 'm: 'gc>(
    _gc: GCHandle<'gc>,
    _stack: &mut CallStack<'gc, 'm>,
) -> StepResult {
    StepResult::InstructionStepped
}

#[dotnet_instruction(Pop)]
pub fn pop<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
) -> StepResult {
    stack.pop(gc);
    StepResult::InstructionStepped
}

#[dotnet_instruction(Duplicate)]
pub fn duplicate<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
) -> StepResult {
    let val = stack.pop(gc);
    stack.push(gc, val.clone());
    stack.push(gc, val);
    StepResult::InstructionStepped
}

#[dotnet_instruction(LoadConstantInt32)]
pub fn ldc_i4<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    i: i32,
) -> StepResult {
    stack.push(gc, StackValue::Int32(i));
    StepResult::InstructionStepped
}

#[dotnet_instruction(LoadConstantInt64)]
pub fn ldc_i8<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    i: i64,
) -> StepResult {
    stack.push(gc, StackValue::Int64(i));
    StepResult::InstructionStepped
}

#[dotnet_instruction(LoadConstantFloat32)]
pub fn ldc_r4<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    f: f32,
) -> StepResult {
    stack.push(gc, StackValue::NativeFloat(f as f64));
    StepResult::InstructionStepped
}

#[dotnet_instruction(LoadConstantFloat64)]
pub fn ldc_r8<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    f: f64,
) -> StepResult {
    stack.push(gc, StackValue::NativeFloat(f));
    StepResult::InstructionStepped
}

#[dotnet_instruction(LoadNull)]
pub fn ldnull<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
) -> StepResult {
    stack.push(gc, StackValue::null());
    StepResult::InstructionStepped
}

#[dotnet_instruction(LoadArgument)]
pub fn ldarg<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    index: u16,
) -> StepResult {
    let val = stack.get_argument(index as usize);
    stack.push(gc, val);
    StepResult::InstructionStepped
}

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
    StepResult::InstructionStepped
}

#[dotnet_instruction(StoreArgument)]
pub fn starg<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    index: u16,
) -> StepResult {
    let val = stack.pop(gc);
    stack.set_argument(gc, index as usize, val);
    StepResult::InstructionStepped
}

#[dotnet_instruction(LoadLocal)]
pub fn ldloc<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    index: u16,
) -> StepResult {
    let val = stack.get_local(index as usize);
    stack.push(gc, val);
    StepResult::InstructionStepped
}

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
        StackValue::managed_ptr_with_owner(
            ptr.as_ptr() as *mut _,
            live_type,
            None,
            pinned,
        ),
    );
    StepResult::InstructionStepped
}

#[dotnet_instruction(StoreLocal)]
pub fn stloc<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    index: u16,
) -> StepResult {
    let val = stack.pop(gc);
    stack.set_local(gc, index as usize, val);
    StepResult::InstructionStepped
}

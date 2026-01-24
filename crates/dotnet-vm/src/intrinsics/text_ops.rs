use crate::{vm_pop, vm_push, CallStack, StepResult};
use dotnet_types::{generics::GenericLookup, members::MethodDescription};
use dotnet_utils::gc::GCHandle;
use dotnet_value::StackValue;

pub fn intrinsic_unicode_utility_is_ascii_code_point<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let val = vm_pop!(stack, gc);
    let is_ascii = match val {
        StackValue::Int32(i) => (0..=0x7F).contains(&i),
        StackValue::Int64(i) => (0..=0x7F).contains(&i),
        StackValue::NativeInt(i) => (0..=0x7F).contains(&i),
        _ => false,
    };
    vm_push!(stack, gc, Int32(if is_ascii { 1 } else { 0 }));
    StepResult::InstructionStepped
}

pub fn intrinsic_unicode_utility_is_in_range_inclusive<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let high = vm_pop!(stack, gc);
    let low = vm_pop!(stack, gc);
    let value = vm_pop!(stack, gc);

    let result = match (value, low, high) {
        (StackValue::Int32(v), StackValue::Int32(l), StackValue::Int32(h)) => v >= l && v <= h,
        (StackValue::Int64(v), StackValue::Int64(l), StackValue::Int64(h)) => v >= l && v <= h,
        (StackValue::NativeInt(v), StackValue::NativeInt(l), StackValue::NativeInt(h)) => {
            v >= l && v <= h
        }
        _ => panic!("IsInRangeInclusive: mismatched types"),
    };
    vm_push!(stack, gc, Int32(if result { 1 } else { 0 }));
    StepResult::InstructionStepped
}

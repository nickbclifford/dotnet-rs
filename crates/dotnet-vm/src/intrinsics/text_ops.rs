use crate::{CallStack, StepResult};
use dotnet_macros::dotnet_intrinsic;
use dotnet_types::{generics::GenericLookup, members::MethodDescription};
use dotnet_utils::gc::GCHandle;
use dotnet_value::StackValue;

#[dotnet_intrinsic("static bool System.Text.UnicodeUtility::IsAsciiCodePoint(uint)")]
pub fn intrinsic_unicode_utility_is_ascii_code_point<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let val = stack.pop(gc);
    let is_ascii = match val {
        StackValue::Int32(i) => (0..=0x7F).contains(&i),
        StackValue::Int64(i) => (0..=0x7F).contains(&i),
        StackValue::NativeInt(i) => (0..=0x7F).contains(&i),
        _ => false,
    };
    stack.push_i32(gc, if is_ascii { 1 } else { 0 });
    StepResult::Continue
}

#[dotnet_intrinsic("static bool System.Text.UnicodeUtility::IsInRangeInclusive(uint, uint, uint)")]
pub fn intrinsic_unicode_utility_is_in_range_inclusive<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let high = stack.pop(gc);
    let low = stack.pop(gc);
    let value = stack.pop(gc);

    let result = match (value, low, high) {
        (StackValue::Int32(v), StackValue::Int32(l), StackValue::Int32(h)) => v >= l && v <= h,
        (StackValue::Int64(v), StackValue::Int64(l), StackValue::Int64(h)) => v >= l && v <= h,
        (StackValue::NativeInt(v), StackValue::NativeInt(l), StackValue::NativeInt(h)) => {
            v >= l && v <= h
        }
        _ => panic!("IsInRangeInclusive: mismatched types"),
    };
    stack.push_i32(gc, if result { 1 } else { 0 });
    StepResult::Continue
}

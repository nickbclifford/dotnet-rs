use crate::{StepResult, stack::ops::VesOps};
use dotnet_macros::dotnet_intrinsic;
use dotnet_types::{generics::GenericLookup, members::MethodDescription};
use dotnet_utils::gc::GCHandle;
use dotnet_value::StackValue;

#[dotnet_intrinsic("static bool System.Text.UnicodeUtility::IsAsciiCodePoint(uint)")]
pub fn intrinsic_unicode_utility_is_ascii_code_point<'gc, 'm: 'gc>(
    ctx: &mut dyn VesOps<'gc, 'm>,
    gc: GCHandle<'gc>,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let val = ctx.pop(gc);
    let is_ascii = match val {
        StackValue::Int32(i) => (0..=0x7F).contains(&i),
        StackValue::Int64(i) => (0..=0x7F).contains(&i),
        StackValue::NativeInt(i) => (0..=0x7F).contains(&i),
        _ => false,
    };
    ctx.push_i32(gc, if is_ascii { 1 } else { 0 });
    StepResult::Continue
}

#[dotnet_intrinsic("static bool System.Text.UnicodeUtility::IsInRangeInclusive(uint, uint, uint)")]
pub fn intrinsic_unicode_utility_is_in_range_inclusive<'gc, 'm: 'gc>(
    ctx: &mut dyn VesOps<'gc, 'm>,
    gc: GCHandle<'gc>,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let high = ctx.pop(gc);
    let low = ctx.pop(gc);
    let value = ctx.pop(gc);

    let result = match (value, low, high) {
        (StackValue::Int32(v), StackValue::Int32(l), StackValue::Int32(h)) => v >= l && v <= h,
        (StackValue::Int64(v), StackValue::Int64(l), StackValue::Int64(h)) => v >= l && v <= h,
        (StackValue::NativeInt(v), StackValue::NativeInt(l), StackValue::NativeInt(h)) => {
            v >= l && v <= h
        }
        _ => panic!("IsInRangeInclusive: mismatched types"),
    };
    ctx.push_i32(gc, if result { 1 } else { 0 });
    StepResult::Continue
}

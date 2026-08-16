use crate::{
    StepResult,
    instructions::objects::get_ptr_info,
    stack::ops::{EvalStackOps, ExceptionOps, LoaderOps, MemoryOps, RawMemoryOps, TypedStackOps},
};
use dotnet_macros::dotnet_intrinsic;
use dotnet_types::{
    TypeDescription,
    error::{ExecutionError, VmError},
    generics::GenericLookup,
    members::MethodDescription,
};
use dotnet_value::{
    StackValue,
    layout::{LayoutManager, Scalar},
    pointer::ManagedPtr,
    string::CLRString,
};

use super::object_ops::{
    enum_type_from_stack_value, format_enum_value, format_enum_value_from_type, generic_enum_type,
    scalar_enum_value_from_stack,
};

#[dotnet_intrinsic("static bool System.Text.UnicodeUtility::IsAsciiCodePoint(uint)")]
pub fn intrinsic_unicode_utility_is_ascii_code_point<
    'gc,
    T: EvalStackOps<'gc> + TypedStackOps<'gc> + MemoryOps<'gc>,
>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let _gc = ctx.gc_with_token(&ctx.no_active_borrows_token());
    let val = ctx.pop();
    let is_ascii = match val {
        StackValue::Int32(i) => (0..=0x7F).contains(&i),
        StackValue::Int64(i) => (0..=0x7F).contains(&i),
        StackValue::NativeInt(i) => (0..=0x7F).contains(&i),
        _ => false,
    };
    ctx.push_i32(if is_ascii { 1 } else { 0 });
    StepResult::Continue
}

#[dotnet_intrinsic("static bool System.Text.UnicodeUtility::IsInRangeInclusive(uint, uint, uint)")]
pub fn intrinsic_unicode_utility_is_in_range_inclusive<
    'gc,
    T: EvalStackOps<'gc> + TypedStackOps<'gc> + MemoryOps<'gc>,
>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let _gc = ctx.gc_with_token(&ctx.no_active_borrows_token());
    let high = ctx.pop();
    let low = ctx.pop();
    let value = ctx.pop();

    let result = match (value, low, high) {
        (StackValue::Int32(v), StackValue::Int32(l), StackValue::Int32(h)) => v >= l && v <= h,
        (StackValue::Int64(v), StackValue::Int64(l), StackValue::Int64(h)) => v >= l && v <= h,
        (StackValue::NativeInt(v), StackValue::NativeInt(l), StackValue::NativeInt(h)) => {
            v >= l && v <= h
        }
        (actual_value, actual_low, actual_high) => {
            return StepResult::Error(VmError::Execution(ExecutionError::TypeMismatch {
                expected: "all Int32, all Int64, or all NativeInt",
                actual: format!("value={actual_value:?}, low={actual_low:?}, high={actual_high:?}")
                    .into(),
            }));
        }
    };
    ctx.push_i32(if result { 1 } else { 0 });
    StepResult::Continue
}

fn write_i32_out_arg<'gc, T: ExceptionOps<'gc> + RawMemoryOps<'gc>>(
    ctx: &mut T,
    out_arg: &StackValue<'gc>,
    value: i32,
) -> Result<(), StepResult> {
    let (origin, offset) = get_ptr_info(ctx, out_arg)?;
    let layout = LayoutManager::Scalar(Scalar::Int32);
    // SAFETY: F2.DescriptorMatchesEcmaLayout — `get_ptr_info` decoded the managed Int32 out parameter, and `layout` matches the
    // four-byte value written through that origin and offset.
    unsafe {
        ctx.write_unaligned(origin, offset, StackValue::Int32(value), &layout)
            .map_err(|e| StepResult::Error(e.into()))
    }
}

fn try_write_utf16_to_span<
    'gc,
    T: RawMemoryOps<'gc> + dotnet_value::pointer::ManagedPtrResolver<'gc> + LoaderOps,
>(
    ctx: &mut T,
    destination: &StackValue<'gc>,
    chars: &[u16],
) -> Result<bool, StepResult> {
    let span = match destination {
        StackValue::ValueType(span) => span.clone(),
        other => {
            return Err(StepResult::type_error(
                "System.Span<char>",
                format!("{:?}", other),
            ));
        }
    };

    let span_len = dotnet_intrinsics_span::helpers::read_span_length(&span, ctx.loader().as_ref())
        .map_err(|e| StepResult::Error(e.into()))?;
    if span_len < 0 {
        return Ok(false);
    }

    let required_len = chars.len();
    if (span_len as usize) < required_len {
        return Ok(false);
    }

    if required_len == 0 {
        return Ok(true);
    }

    let span_ref =
        dotnet_intrinsics_span::helpers::read_span_reference(&span, ctx, ctx.loader().as_ref())
            .map_err(|e| StepResult::Error(e.into()))?;
    let span_ptr = ManagedPtr::from_info_full(span_ref, TypeDescription::NULL, false);

    let mut bytes = Vec::with_capacity(required_len * 2);
    for ch in chars {
        bytes.extend_from_slice(&ch.to_ne_bytes());
    }

    // SAFETY: F10.RawMemoryAccessValid — `read_span_reference` decoded the live span origin, the length check proves room for
    // every UTF-16 code unit, and `bytes` contains exactly `chars.len() * 2` bytes.
    unsafe {
        ctx.write_bytes(span_ptr.origin().clone(), span_ptr.byte_offset(), &bytes)
            .map_err(|e| StepResult::Error(e.into()))?;
    }

    Ok(true)
}

#[dotnet_intrinsic("string System.Enum::ToString()")]
pub(super) fn enum_to_string<'gc, T: TypedStackOps<'gc> + ExceptionOps<'gc>>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let this = ctx.pop();
    let formatted = match format_enum_value(ctx, this) {
        Ok(v) => v,
        Err(step) => return step,
    };

    ctx.push_string(CLRString::from(formatted));
    StepResult::Continue
}

#[dotnet_intrinsic(
    "static bool System.Enum::TryFormatUnconstrained<M0>(M0, System.Span<char>, int&, System.ReadOnlySpan<char>)"
)]
pub(super) fn enum_try_format_unconstrained<
    'gc,
    T: TypedStackOps<'gc>
        + ExceptionOps<'gc>
        + RawMemoryOps<'gc>
        + LoaderOps
        + dotnet_value::pointer::ManagedPtrResolver<'gc>,
>(
    ctx: &mut T,
    _method: MethodDescription,
    generics: &GenericLookup,
) -> StepResult {
    let _format = ctx.pop();
    let chars_written_out = ctx.pop();
    let destination = ctx.pop();
    let value = ctx.pop();

    let Some((raw_value, signed)) = scalar_enum_value_from_stack(value.clone()) else {
        if let Err(step) = write_i32_out_arg(ctx, &chars_written_out, 0) {
            return step;
        }
        ctx.push_i32(0);
        return StepResult::Continue;
    };

    let enum_type = enum_type_from_stack_value(&value).or_else(|| generic_enum_type(generics));
    let formatted = if let Some(enum_type) = enum_type {
        format_enum_value_from_type(&enum_type, raw_value, signed)
    } else if signed {
        raw_value.to_string()
    } else {
        (raw_value as u128).to_string()
    };

    let formatted_utf16: Vec<u16> = formatted.encode_utf16().collect();
    let wrote = match try_write_utf16_to_span(ctx, &destination, &formatted_utf16) {
        Ok(wrote) => wrote,
        Err(step) => return step,
    };

    if wrote {
        let chars_written = i32::try_from(formatted_utf16.len()).unwrap_or(i32::MAX);
        if let Err(step) = write_i32_out_arg(ctx, &chars_written_out, chars_written) {
            return step;
        }
        ctx.push_i32(1);
    } else {
        if let Err(step) = write_i32_out_arg(ctx, &chars_written_out, 0) {
            return step;
        }
        ctx.push_i32(0);
    }

    StepResult::Continue
}

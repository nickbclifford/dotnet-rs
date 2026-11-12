use crate::{
    IntrinsicStringHost, NULL_REF_MSG, extend_from_utf16_ne_bytes, simd::probe_sequence_equal_u16,
};
use dotnet_macros::dotnet_intrinsic;
use dotnet_types::{
    TypeDescription, error::IntrinsicError, generics::GenericLookup, members::MethodDescription,
};
use dotnet_utils::GcScopeGuard;
use dotnet_value::{
    StackValue,
    object::{HeapStorage, Object, ObjectHandle, ObjectRef},
    string::CLRString,
};
use dotnet_vm_data::StepResult;
use dotnet_vm_ops::effects::{
    NativeInvocation, NativeOperandView, NativeResult, NativeSelection, VmEffects,
};
use dotnet_vm_ops::ops::{ExceptionOps, MemoryOps, RawMemoryOps, TypedStackOps};
use std::hash::{DefaultHasher, Hash, Hasher};

const SPAN_COPY_CHUNK_SIZE: usize = 1024;

/// Copies `len` chars in bounded chunks without polling for a collection. The operands are
/// ref-struct spans, which native transport excludes, and a synchronous handler that yields is
/// retried from its start; one linear pass runs to completion instead, as `cpblk` does.
fn run_chunked_copy<C>(len: usize, mut copy_chunk: C) -> Result<(), StepResult>
where
    C: FnMut(usize, usize) -> Result<(), IntrinsicError>,
{
    let mut offset = 0usize;
    while offset < len {
        let chunk_len = std::cmp::min(SPAN_COPY_CHUNK_SIZE, len - offset);
        copy_chunk(offset, chunk_len).map_err(|e| StepResult::Error(e.into()))?;
        offset += chunk_len;
    }

    Ok(())
}

fn extend_from_string_chunk<'gc, T: RawMemoryOps<'gc>>(
    ctx: &T,
    handle: ObjectHandle<'gc>,
    offset: usize,
    chunk_len: usize,
    dest: &mut Vec<u16>,
) -> bool {
    let _gc_scope = GcScopeGuard::enter(
        ctx.as_borrow_scope(),
        ctx.as_borrow_scope().gc_ready_token(),
    );
    let inner = handle.borrow();
    match &inner.storage {
        HeapStorage::Str(s) => {
            dest.extend_from_slice(&s[offset..offset + chunk_len]);
            true
        }
        _ => false,
    }
}

fn heap_string_len(handle: ObjectHandle<'_>) -> (usize, bool) {
    let heap = handle.borrow();
    if let HeapStorage::Str(s) = &heap.storage {
        (s.len(), true)
    } else {
        (0, false)
    }
}

fn string_comparison_ignore_case(comparison: StackValue<'_>) -> bool {
    matches!(
        comparison.coerce_enum_to_underlying(),
        StackValue::Int32(1 | 3 | 5)
    )
}

fn string_slices_equal_with_comparison(a: &[u16], b: &[u16], ignore_case: bool) -> bool {
    if ignore_case {
        a.iter().zip(b.iter()).all(|(ac, bc)| {
            char::from_u32(u32::from(*ac)).map(|c| c.to_ascii_uppercase())
                == char::from_u32(u32::from(*bc)).map(|c| c.to_ascii_uppercase())
        })
    } else {
        a == b
    }
}

/// System.String::Equals(string, string)
/// System.String::Equals(string)
/// System.String::Equals(object)
// Bound each poll to the same byte count as the bulk memory intrinsics.
const EQUALITY_CHUNK_SIZE: usize = dotnet_vm_ops::effects::BULK_CHUNK_SIZE / 2;

/// Only equal-length, distinct large strings need a resumable comparison.
pub fn string_equals_select<'gc, T: TypedStackOps<'gc> + RawMemoryOps<'gc>>(
    _ctx: &T,
    operands: NativeOperandView<'_, 'gc>,
    _method: &MethodDescription,
    _generics: &GenericLookup,
) -> NativeSelection {
    let (StackValue::ObjectRef(ObjectRef(Some(a))), StackValue::ObjectRef(ObjectRef(Some(b)))) =
        (operands.peek_at(1), operands.peek_at(0))
    else {
        return NativeSelection::Fallback;
    };
    if gc_arena::Gc::as_ptr(a) == gc_arena::Gc::as_ptr(b) {
        return NativeSelection::Fallback;
    }
    let (a_len, a_is_str) = heap_string_len(a);
    let (b_len, b_is_str) = heap_string_len(b);
    if a_is_str && b_is_str && a_len == b_len && a_len > EQUALITY_CHUNK_SIZE {
        NativeSelection::Async(NativeInvocation {
            start: 0,
            length: a_len,
            parameter_count: 2,
        })
    } else {
        NativeSelection::Fallback
    }
}

#[dotnet_intrinsic("static bool System.String::Equals(string, string)", select = string_equals_select, fallback = intrinsic_string_equals)]
#[dotnet_intrinsic("bool System.String::Equals(string)", select = string_equals_select, fallback = intrinsic_string_equals)]
#[dotnet_intrinsic("bool System.String::Equals(object)", select = string_equals_select, fallback = intrinsic_string_equals)]
pub async fn string_equals_async(mut fx: VmEffects) -> NativeResult {
    let length = fx.invocation().length;
    let (a, b) = {
        let mut vm = fx.vm().await?;
        let b = vm.pop_string()?;
        let a = vm.pop_string()?;
        (a, b)
    };
    let mut offset = 0;
    while offset < length {
        let chunk = (length - offset).min(EQUALITY_CHUNK_SIZE);
        let equal = {
            let mut vm = fx.vm().await?;
            vm.with(|region| {
                let (
                    StackValue::ObjectRef(ObjectRef(Some(a))),
                    StackValue::ObjectRef(ObjectRef(Some(b))),
                ) = (region.fetch(&a)?, region.fetch(&b)?)
                else {
                    return Err(dotnet_types::error::ExecutionError::InternalError(
                        "native string equality operands invalid".into(),
                    )
                    .into());
                };
                let a_heap = a.borrow();
                let b_heap = b.borrow();
                let (HeapStorage::Str(a), HeapStorage::Str(b)) = (&a_heap.storage, &b_heap.storage)
                else {
                    return Err(dotnet_types::error::ExecutionError::InternalError(
                        "native string equality operands invalid".into(),
                    )
                    .into());
                };
                let a = &a[offset..offset + chunk];
                let b = &b[offset..offset + chunk];
                Ok::<bool, dotnet_types::error::VmError>(
                    probe_sequence_equal_u16(a, b).unwrap_or(a == b),
                )
            })??
        };
        if !equal {
            return fx.vm().await?.push_bool(false);
        }
        offset += chunk;
        if offset < length {
            fx.yield_now().await?;
        }
    }
    fx.vm().await?.push_bool(true)
}

pub fn intrinsic_string_equals<'gc, T: TypedStackOps<'gc> + RawMemoryOps<'gc>>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let b_val = ctx.peek_stack();
    let a_val = ctx.peek_stack_at(1);

    let res = match (a_val, b_val) {
        (StackValue::ObjectRef(ObjectRef(None)), StackValue::ObjectRef(ObjectRef(None))) => true,
        (StackValue::ObjectRef(ObjectRef(None)), _)
        | (_, StackValue::ObjectRef(ObjectRef(None))) => false,
        (
            StackValue::ObjectRef(ObjectRef(Some(a_handle))),
            StackValue::ObjectRef(ObjectRef(Some(b_handle))),
        ) => {
            // Compare the actual object pointers (for reference equality/interning check)
            use gc_arena::Gc;
            if Gc::as_ptr(a_handle) == Gc::as_ptr(b_handle) {
                true
            } else {
                let (a_len, a_is_str) = heap_string_len(a_handle);
                let (b_len, b_is_str) = heap_string_len(b_handle);

                if !a_is_str || !b_is_str || a_len != b_len {
                    false
                } else {
                    let length = a_len;
                    let mut offset = 0;
                    let mut equal = true;

                    while offset < length {
                        let current_chunk = std::cmp::min(EQUALITY_CHUNK_SIZE, length - offset);

                        let chunk_equal = {
                            let a_heap = a_handle.borrow();
                            let b_heap = b_handle.borrow();
                            if let (HeapStorage::Str(a), HeapStorage::Str(b)) =
                                (&a_heap.storage, &b_heap.storage)
                            {
                                let a_chunk = &a[offset..offset + current_chunk];
                                let b_chunk = &b[offset..offset + current_chunk];
                                probe_sequence_equal_u16(a_chunk, b_chunk)
                                    .unwrap_or(a_chunk == b_chunk)
                            } else {
                                false
                            }
                        };

                        if !chunk_equal {
                            equal = false;
                            break;
                        }

                        offset += current_chunk;
                    }
                    equal
                }
            }
        }
        _ => false,
    };

    ctx.drop_top(2);
    ctx.push_i32(if res { 1 } else { 0 });
    StepResult::Continue
}

/// `String.Equals(string, string, StringComparison)` and
/// `String.Equals(string, StringComparison)` — compares strings using the given comparison
/// mode. dotnet-rs maps all comparison modes to ordinal (byte-level) comparison because
/// locale-sensitive collation is not yet implemented; this is correct for `Ordinal` and
/// `OrdinalIgnoreCase`, and a conservative approximation for culture-specific modes.
#[dotnet_intrinsic(
    "static bool System.String::Equals(string, string, valuetype System.StringComparison)"
)]
#[dotnet_intrinsic("bool System.String::Equals(string, valuetype System.StringComparison)")]
pub fn intrinsic_string_equals_comparison<'gc, T: TypedStackOps<'gc> + RawMemoryOps<'gc>>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let comparison = ctx.pop();
    let b_val = ctx.pop();
    let a_val = ctx.pop();

    let ignore_case = string_comparison_ignore_case(comparison);

    let res = match (a_val, b_val) {
        (StackValue::ObjectRef(ObjectRef(None)), StackValue::ObjectRef(ObjectRef(None))) => true,
        (StackValue::ObjectRef(ObjectRef(None)), _)
        | (_, StackValue::ObjectRef(ObjectRef(None))) => false,
        (
            StackValue::ObjectRef(ObjectRef(Some(a_handle))),
            StackValue::ObjectRef(ObjectRef(Some(b_handle))),
        ) => {
            let a_heap = a_handle.borrow();
            let b_heap = b_handle.borrow();
            match (&a_heap.storage, &b_heap.storage) {
                (HeapStorage::Str(a), HeapStorage::Str(b)) => {
                    if a.len() != b.len() {
                        false
                    } else {
                        string_slices_equal_with_comparison(a, b, ignore_case)
                    }
                }
                _ => false,
            }
        }
        _ => false,
    };

    ctx.push_i32(if res { 1 } else { 0 });
    StepResult::Continue
}

/// `String.StartsWith(string, StringComparison)` — checks whether the current string begins
/// with `value` using the provided comparison mode. dotnet-rs currently maps all comparison
/// modes to ordinal behavior with optional ASCII case-folding for `*IgnoreCase` variants.
#[dotnet_intrinsic("bool System.String::StartsWith(string, valuetype System.StringComparison)")]
pub fn intrinsic_string_starts_with_comparison<
    'gc,
    T: TypedStackOps<'gc> + ExceptionOps<'gc> + RawMemoryOps<'gc>,
>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let comparison = ctx.pop();
    let value = ctx.pop();
    let this = ctx.pop();

    let ignore_case = string_comparison_ignore_case(comparison);

    let this_handle = match this {
        StackValue::ObjectRef(ObjectRef(Some(handle))) => handle,
        StackValue::ObjectRef(ObjectRef(None)) => {
            return ctx.throw_by_name_with_message("System.NullReferenceException", NULL_REF_MSG);
        }
        _ => {
            return StepResult::type_error(
                "string",
                format!("invalid this for System.String::StartsWith: {:?}", this),
            );
        }
    };

    let value_handle = match value {
        StackValue::ObjectRef(ObjectRef(Some(handle))) => handle,
        StackValue::ObjectRef(ObjectRef(None)) => {
            return ctx.throw_by_name_with_message("System.ArgumentNullException", "value");
        }
        _ => {
            return StepResult::type_error(
                "string",
                format!(
                    "invalid argument for System.String::StartsWith: {:?}",
                    value
                ),
            );
        }
    };

    let res = {
        let this_heap = this_handle.borrow();
        let value_heap = value_handle.borrow();

        match (&this_heap.storage, &value_heap.storage) {
            (HeapStorage::Str(this_str), HeapStorage::Str(value_str)) => {
                if value_str.len() > this_str.len() {
                    false
                } else {
                    let this_prefix = &this_str[..value_str.len()];
                    string_slices_equal_with_comparison(this_prefix, value_str, ignore_case)
                }
            }
            _ => {
                return StepResult::internal_error(
                    "System.String::StartsWith called on non-string object",
                );
            }
        }
    };

    ctx.push_i32(res as i32);
    StepResult::Continue
}

/// `String.EndsWith(string, StringComparison)` — checks whether the current string ends
/// with `value` using the provided comparison mode. dotnet-rs currently maps all comparison
/// modes to ordinal behavior with optional ASCII case-folding for `*IgnoreCase` variants.
#[dotnet_intrinsic("bool System.String::EndsWith(string, valuetype System.StringComparison)")]
pub fn intrinsic_string_ends_with_comparison<
    'gc,
    T: TypedStackOps<'gc> + ExceptionOps<'gc> + RawMemoryOps<'gc>,
>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let comparison = ctx.pop();
    let value = ctx.pop();
    let this = ctx.pop();

    let ignore_case = string_comparison_ignore_case(comparison);

    let this_handle = match this {
        StackValue::ObjectRef(ObjectRef(Some(handle))) => handle,
        StackValue::ObjectRef(ObjectRef(None)) => {
            return ctx.throw_by_name_with_message("System.NullReferenceException", NULL_REF_MSG);
        }
        _ => {
            return StepResult::type_error(
                "string",
                format!("invalid this for System.String::EndsWith: {:?}", this),
            );
        }
    };

    let value_handle = match value {
        StackValue::ObjectRef(ObjectRef(Some(handle))) => handle,
        StackValue::ObjectRef(ObjectRef(None)) => {
            return ctx.throw_by_name_with_message("System.ArgumentNullException", "value");
        }
        _ => {
            return StepResult::type_error(
                "string",
                format!("invalid argument for System.String::EndsWith: {:?}", value),
            );
        }
    };

    let res = {
        let this_heap = this_handle.borrow();
        let value_heap = value_handle.borrow();

        match (&this_heap.storage, &value_heap.storage) {
            (HeapStorage::Str(this_str), HeapStorage::Str(value_str)) => {
                if value_str.len() > this_str.len() {
                    false
                } else {
                    let suffix_start = this_str.len() - value_str.len();
                    let this_suffix = &this_str[suffix_start..];
                    string_slices_equal_with_comparison(this_suffix, value_str, ignore_case)
                }
            }
            _ => {
                return StepResult::internal_error(
                    "System.String::EndsWith called on non-string object",
                );
            }
        }
    };

    ctx.push_i32(res as i32);
    StepResult::Continue
}

/// `System.String::Concat(ReadOnlySpan<char>, ReadOnlySpan<char>, ReadOnlySpan<char>)`
#[dotnet_intrinsic(
    "static string System.String::Concat(System.ReadOnlySpan<char>, System.ReadOnlySpan<char>, System.ReadOnlySpan<char>)"
)]
pub fn intrinsic_string_concat_three_spans<'gc, T: IntrinsicStringHost<'gc>>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let arg2 = ctx.peek_stack();
    let arg1 = ctx.peek_stack_at(1);
    let arg0 = ctx.peek_stack_at(2);

    let copy_span_chunked =
        |ctx: &mut T, span: Object, dest: &mut Vec<u16>| -> Result<(), StepResult> {
            let len = ctx
                .string_with_span_data(span.clone(), TypeDescription::NULL, 2, |slice| {
                    slice.len() / 2
                })
                .map_err(|e| StepResult::Error(e.into()))?;

            run_chunked_copy(len, |offset, chunk_len| {
                ctx.string_with_span_data(span.clone(), TypeDescription::NULL, 2, |slice| {
                    let start = offset * 2;
                    let end = start + (chunk_len * 2);
                    if let Some(chunk_bytes) = slice.get(start..end) {
                        extend_from_utf16_ne_bytes(dest, chunk_bytes);
                    }
                })
            })?;

            Ok(())
        };

    let extract_chars = |ctx: &mut T, arg: StackValue<'gc>| -> Result<Vec<u16>, StepResult> {
        match arg {
            StackValue::ValueType(span) => {
                let mut chars = Vec::new();
                copy_span_chunked(ctx, span, &mut chars)?;
                Ok(chars)
            }
            StackValue::ObjectRef(ObjectRef(None)) => Ok(Vec::new()),
            StackValue::ObjectRef(ObjectRef(Some(handle))) => {
                let chars = {
                    let inner = handle.borrow();
                    match &inner.storage {
                        HeapStorage::Str(s) => s.to_vec(),
                        _ => {
                            return Err(StepResult::type_error(
                                "ReadOnlySpan<char> or string",
                                "non-string object reference".to_string(),
                            ));
                        }
                    }
                };
                Ok(chars)
            }
            other => Err(StepResult::type_error(
                "ReadOnlySpan<char> or string",
                format!("{:?}", other),
            )),
        }
    };

    let data0 = match extract_chars(ctx, arg0) {
        Ok(chars) => chars,
        Err(step) => return step,
    };
    let data1 = match extract_chars(ctx, arg1) {
        Ok(chars) => chars,
        Err(step) => return step,
    };
    let data2 = match extract_chars(ctx, arg2) {
        Ok(chars) => chars,
        Err(step) => return step,
    };

    let value = CLRString::new(data0.into_iter().chain(data1).chain(data2).collect());
    ctx.drop_top(3);
    ctx.push_string(value);
    StepResult::Continue
}

/// System.String::GetHashCodeOrdinalIgnoreCase()
const HASH_CHUNK_SIZE: usize = dotnet_vm_ops::effects::BULK_CHUNK_SIZE / 2;

pub fn string_hash_select<'gc, T: TypedStackOps<'gc> + RawMemoryOps<'gc>>(
    _ctx: &T,
    operands: NativeOperandView<'_, 'gc>,
    _method: &MethodDescription,
    _generics: &GenericLookup,
) -> NativeSelection {
    if let StackValue::ObjectRef(ObjectRef(Some(handle))) = operands.peek_at(0) {
        let inner = handle.borrow();
        if let HeapStorage::Str(chars) = &inner.storage
            && chars.len() > HASH_CHUNK_SIZE
        {
            return NativeSelection::Async(NativeInvocation {
                start: 0,
                length: chars.len(),
                parameter_count: 1,
            });
        }
    }
    NativeSelection::Fallback
}

#[dotnet_intrinsic("int System.String::GetHashCodeOrdinalIgnoreCase()", select = string_hash_select, fallback = intrinsic_string_get_hash_code_ordinal_ignore_case)]
pub async fn string_hash_async(mut fx: VmEffects) -> NativeResult {
    let length = fx.invocation().length;
    let source = fx.vm().await?.pop_string()?;
    let mut chars = Vec::with_capacity(length);
    while chars.len() < length {
        {
            let mut vm = fx.vm().await?;
            vm.with(|region| {
                let StackValue::ObjectRef(ObjectRef(Some(handle))) = region.fetch(&source)? else {
                    return Err(dotnet_types::error::ExecutionError::InternalError(
                        "native string hash operand invalid".into(),
                    )
                    .into());
                };
                let inner = handle.borrow();
                let HeapStorage::Str(source) = &inner.storage else {
                    return Err(dotnet_types::error::ExecutionError::InternalError(
                        "native string hash operand invalid".into(),
                    )
                    .into());
                };
                let end = (chars.len() + HASH_CHUNK_SIZE).min(length);
                chars.extend_from_slice(&source[chars.len()..end]);
                Ok::<(), dotnet_types::error::VmError>(())
            })??;
        }
        if chars.len() < length {
            fx.yield_now().await?;
        }
    }
    let mut h = DefaultHasher::new();
    String::from_utf16_lossy(&chars)
        .to_uppercase()
        .into_bytes()
        .hash(&mut h);
    fx.vm()
        .await?
        .with(|region| region.push_int(h.finish() as i32))??;
    Ok(())
}

pub fn intrinsic_string_get_hash_code_ordinal_ignore_case<
    'gc,
    T: TypedStackOps<'gc> + RawMemoryOps<'gc>,
>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let val = ctx.pop();
    let mut h = DefaultHasher::new();

    let chars = match &val {
        StackValue::ObjectRef(ObjectRef(Some(handle))) => {
            let len = {
                let inner = handle.borrow();
                match &inner.storage {
                    HeapStorage::Str(s) => s.len(),
                    _ => {
                        return StepResult::internal_error(
                            "System.String::GetHashCodeOrdinalIgnoreCase called on non-string object",
                        );
                    }
                }
            };

            let mut result = Vec::with_capacity(len);
            let mut offset = 0;
            while offset < len {
                let chunk_len = std::cmp::min(HASH_CHUNK_SIZE, len - offset);
                if !extend_from_string_chunk(ctx, *handle, offset, chunk_len, &mut result) {
                    break;
                }
                offset += chunk_len;
            }
            result
        }
        _ => Vec::new(),
    };

    let value = String::from_utf16_lossy(&chars).to_uppercase().into_bytes();
    value.hash(&mut h);
    let code = h.finish();

    ctx.push_i32(code as i32);
    StepResult::Continue
}

/// System.String::CopyStringContent(string, int, string)
#[dotnet_intrinsic("static void System.String::CopyStringContent(string, int, string)")]
pub fn intrinsic_string_copy_string_content<
    'gc,
    T: TypedStackOps<'gc> + ExceptionOps<'gc> + MemoryOps<'gc> + RawMemoryOps<'gc>,
>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let src_val = ctx.pop_obj();
    let dest_pos = ctx.pop_i32() as usize;
    let dest_val = ctx.pop_obj();

    let src_handle = match src_val.0 {
        Some(h) => h,
        None => {
            return ctx.throw_by_name_with_message("System.NullReferenceException", NULL_REF_MSG);
        }
    };
    let dest_handle = match dest_val.0 {
        Some(h) => h,
        None => {
            return ctx.throw_by_name_with_message("System.NullReferenceException", NULL_REF_MSG);
        }
    };

    let src_len = {
        let heap = src_handle.borrow();
        match &heap.storage {
            HeapStorage::Str(s) => s.len(),
            _ => 0,
        }
    };

    let mut offset = 0;
    const CHUNK_SIZE: usize = 1024;
    while offset < src_len {
        let chunk_len = std::cmp::min(CHUNK_SIZE, src_len - offset);
        let mut chunk_buf = Vec::with_capacity(chunk_len);
        if !extend_from_string_chunk(ctx, src_handle, offset, chunk_len, &mut chunk_buf) {
            break;
        }

        let mut err = false;
        {
            let gc = ctx.gc_with_token(&ctx.no_active_borrows_token());
            let _gc_scope = GcScopeGuard::enter(
                ctx.as_borrow_scope(),
                ctx.as_borrow_scope().gc_ready_token(),
            );
            let mut dest_heap =
                dotnet_utils::gc::ThreadSafeLock::borrow_mut(dest_handle, gc.mutation());
            match &mut dest_heap.storage {
                HeapStorage::Str(dest) => {
                    let d_pos = dest_pos + offset;
                    if d_pos + chunk_len > dest.len() {
                        err = true;
                    } else {
                        dest.as_mut_slice()[d_pos..d_pos + chunk_len].copy_from_slice(&chunk_buf);
                    }
                }
                _ => break,
            }
        }
        if err {
            return ctx.throw_by_name_with_message(
                "System.ArgumentOutOfRangeException",
                "Destination is too short.",
            );
        }

        // No collection poll: a retry would restart this copy from the beginning.
        offset += chunk_len;
    }
    StepResult::Continue
}

#[dotnet_intrinsic("static System.ReadOnlySpan<char> System.String::op_Implicit(string)")]
pub fn intrinsic_string_implicit_to_span<'gc, T: IntrinsicStringHost<'gc>>(
    ctx: &mut T,
    method: MethodDescription,
    generics: &GenericLookup,
) -> StepResult {
    ctx.string_intrinsic_as_span(method, generics)
}

#[cfg(test)]
mod tests {
    use super::*;
    use dotnet_types::error::VmError;
    use dotnet_utils::{ArenaId, gc::ThreadSafeLock};
    use dotnet_value::object::ObjectInner;
    use dotnet_vm_ops::effects::{
        NativeBackend, NativeControl, NativeEffect, NativeRegion, poll_native,
    };
    use gc_arena::{Arena, Collect, DynamicRootSet, Gc, GcWeak, Rootable};
    use std::task::Poll;

    #[derive(Collect)]
    #[collect(no_drop)]
    struct EqualityRoots<'gc> {
        roots: DynamicRootSet<'gc>,
        a: GcWeak<'gc, ThreadSafeLock<ObjectInner<'gc>>>,
        b: GcWeak<'gc, ThreadSafeLock<ObjectInner<'gc>>>,
    }

    #[derive(Default)]
    struct EqualityBackend<'gc> {
        operands: Vec<StackValue<'gc>>,
        result: Option<i32>,
        stop: bool,
    }
    impl<'gc> NativeBackend<'gc> for EqualityBackend<'gc> {
        fn stop_requested(&self) -> bool {
            self.stop
        }
        fn active_scope_depth(&self) -> usize {
            0
        }
        fn operands(&mut self) -> &mut Vec<StackValue<'gc>> {
            &mut self.operands
        }
        fn push_new_string(&mut self, _chars: Vec<u16>) -> Result<(), VmError> {
            unreachable!()
        }
        fn push_output(&mut self, value: StackValue<'gc>) -> Result<(), VmError> {
            self.result = Some(value.as_i32());
            Ok(())
        }
    }

    #[test]
    fn equality_roots_both_strings_and_resumes_after_collection() {
        for different in [false, true] {
            let length = EQUALITY_CHUNK_SIZE * 2 + 1;
            let mut arena = Arena::<Rootable![EqualityRoots<'_>]>::new(|mc| {
                let chars = vec![42; length];
                let mut other = chars.clone();
                if different {
                    other[length - 1] = 43;
                }
                let a = Gc::new(
                    mc,
                    ThreadSafeLock::new(ObjectInner::new(
                        HeapStorage::Str(CLRString::new(chars)),
                        ArenaId::new(1),
                    )),
                );
                let b = Gc::new(
                    mc,
                    ThreadSafeLock::new(ObjectInner::new(
                        HeapStorage::Str(CLRString::new(other)),
                        ArenaId::new(1),
                    )),
                );
                EqualityRoots {
                    roots: DynamicRootSet::new(mc),
                    a: Gc::downgrade(a),
                    b: Gc::downgrade(b),
                }
            });
            let control = NativeControl::new();
            let fx = control
                .effects(NativeInvocation {
                    start: 0,
                    length,
                    parameter_count: 2,
                })
                .unwrap();
            let mut future = Box::pin(string_equals_async(fx));
            arena.mutate(|mc, roots| {
                let mut backend = EqualityBackend::default();
                backend.operands.push(StackValue::ObjectRef(ObjectRef(Some(
                    roots.a.upgrade(mc).unwrap(),
                ))));
                backend.operands.push(StackValue::ObjectRef(ObjectRef(Some(
                    roots.b.upgrade(mc).unwrap(),
                ))));
                let mut region = NativeRegion::new(mc, roots.roots, &mut backend);
                assert!(matches!(
                    poll_native(&control, future.as_mut(), &mut region),
                    Ok(Poll::Pending)
                ));
                assert!(backend.operands.is_empty());
            });
            assert_eq!(control.take_effect(), Some(NativeEffect::Yield));
            arena.finish_cycle();
            arena.mutate(|mc, roots| {
                assert!(roots.a.upgrade(mc).is_some() && roots.b.upgrade(mc).is_some());
                let mut backend = EqualityBackend {
                    stop: true,
                    ..Default::default()
                };
                let mut region = NativeRegion::new(mc, roots.roots, &mut backend);
                assert!(matches!(
                    poll_native(&control, future.as_mut(), &mut region),
                    Ok(Poll::Pending)
                ));
            });
            assert_eq!(control.take_effect(), Some(NativeEffect::Yield));
            arena.mutate(|mc, roots| {
                let mut backend = EqualityBackend::default();
                let mut region = NativeRegion::new(mc, roots.roots, &mut backend);
                assert!(matches!(
                    poll_native(&control, future.as_mut(), &mut region),
                    Ok(Poll::Pending)
                ));
            });
            assert_eq!(control.take_effect(), Some(NativeEffect::Yield));
            arena.finish_cycle();
            arena.mutate(|mc, roots| {
                let mut backend = EqualityBackend::default();
                let mut region = NativeRegion::new(mc, roots.roots, &mut backend);
                assert!(matches!(
                    poll_native(&control, future.as_mut(), &mut region),
                    Ok(Poll::Ready(Ok(())))
                ));
                assert_eq!(backend.result, Some(i32::from(!different)));
            });
            drop(future);
            arena.finish_cycle();
            arena.mutate(|mc, roots| {
                assert!(roots.a.upgrade(mc).is_none() && roots.b.upgrade(mc).is_none())
            });
        }
    }

    #[test]
    fn hash_keeps_utf16_and_progress_across_collection() {
        let length = HASH_CHUNK_SIZE * 2 + 1;
        let mut chars = vec![b'a' as u16; length];
        chars[HASH_CHUNK_SIZE - 1] = 0xd83d;
        chars[HASH_CHUNK_SIZE] = 0xde00;
        let mut expected = DefaultHasher::new();
        String::from_utf16_lossy(&chars)
            .to_uppercase()
            .into_bytes()
            .hash(&mut expected);
        let expected = expected.finish() as i32;
        let mut arena = Arena::<Rootable![EqualityRoots<'_>]>::new(|mc| {
            let a = Gc::new(
                mc,
                ThreadSafeLock::new(ObjectInner::new(
                    HeapStorage::Str(CLRString::new(chars)),
                    ArenaId::new(1),
                )),
            );
            EqualityRoots {
                roots: DynamicRootSet::new(mc),
                a: Gc::downgrade(a),
                b: Gc::downgrade(a),
            }
        });
        let control = NativeControl::new();
        let fx = control
            .effects(NativeInvocation {
                start: 0,
                length,
                parameter_count: 1,
            })
            .unwrap();
        let mut future = Box::pin(string_hash_async(fx));
        arena.mutate(|mc, roots| {
            let mut backend = EqualityBackend::default();
            backend.operands.push(StackValue::ObjectRef(ObjectRef(Some(
                roots.a.upgrade(mc).unwrap(),
            ))));
            let mut region = NativeRegion::new(mc, roots.roots, &mut backend);
            assert!(matches!(
                poll_native(&control, future.as_mut(), &mut region),
                Ok(Poll::Pending)
            ));
        });
        assert_eq!(control.take_effect(), Some(NativeEffect::Yield));
        arena.finish_cycle();
        arena.mutate(|mc, roots| {
            assert!(roots.a.upgrade(mc).is_some());
            let mut backend = EqualityBackend::default();
            let mut region = NativeRegion::new(mc, roots.roots, &mut backend);
            assert!(matches!(
                poll_native(&control, future.as_mut(), &mut region),
                Ok(Poll::Pending)
            ));
        });
        assert_eq!(control.take_effect(), Some(NativeEffect::Yield));
        arena.finish_cycle();
        arena.mutate(|mc, roots| {
            let mut backend = EqualityBackend::default();
            let mut region = NativeRegion::new(mc, roots.roots, &mut backend);
            assert!(matches!(
                poll_native(&control, future.as_mut(), &mut region),
                Ok(Poll::Ready(Ok(())))
            ));
            assert_eq!(backend.result, Some(expected));
        });
        drop(future);
        arena.finish_cycle();
        arena.mutate(|mc, roots| assert!(roots.a.upgrade(mc).is_none()));
    }

    #[test]
    fn run_chunked_copy_completes_in_one_call() {
        let len = SPAN_COPY_CHUNK_SIZE * 2 + 8;
        let mut copied = 0usize;
        let res = run_chunked_copy(len, |_offset, chunk_len| {
            copied += chunk_len;
            Ok(())
        });

        assert_eq!(res, Ok(()));
        assert_eq!(copied, len);
    }
}

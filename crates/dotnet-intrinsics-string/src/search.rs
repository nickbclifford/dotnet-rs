use crate::{
    IntrinsicStringHost, NULL_REF_MSG,
    simd::{probe_all_ascii_whitespace, probe_index_of_char},
};
use dotnet_macros::dotnet_intrinsic;
use dotnet_types::{generics::GenericLookup, members::MethodDescription};
use dotnet_utils::GcScopeGuard;
use dotnet_value::{
    StackValue,
    object::{HeapStorage, ObjectRef},
    string::CLRString,
    with_string,
};
use dotnet_vm_data::StepResult;
use dotnet_vm_ops::ops::{ExceptionOps, RawMemoryOps, TypedStackOps};

/// System.String::IndexOf(char)
/// System.String::IndexOf(char, int)
#[dotnet_intrinsic("int System.String::IndexOf(char)")]
#[dotnet_intrinsic("int System.String::IndexOf(char, int)")]
pub fn intrinsic_string_index_of<
    'gc,
    T: TypedStackOps<'gc> + ExceptionOps<'gc> + RawMemoryOps<'gc>,
>(
    ctx: &mut T,
    method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let (c, start_at) = if method.signature().parameters.len() == 1 {
        let c = ctx.pop_i32();
        (c as u16, 0usize)
    } else {
        let start_at = ctx.pop_i32();
        let c = ctx.pop_i32();
        (c as u16, start_at as usize)
    };

    let val = ctx.pop();
    let index = with_string!(ctx, val, |s| {
        if start_at >= s.len() {
            None
        } else {
            let haystack = &s[start_at..];
            probe_index_of_char(haystack, c)
                .unwrap_or_else(|| haystack.iter().position(|x| *x == c))
        }
    });

    ctx.push_i32(match index {
        None => -1,
        Some(i) => (i + start_at) as i32,
    });
    StepResult::Continue
}

/// Maximum number of UTF-16 code units copied in one substring chunk.
pub const SUBSTRING_CHUNK_SIZE: usize = 64 * 1024;

/// Append one bounded chunk of a previously validated substring.
pub fn copy_substring_chunk(
    source: &[u16],
    start: usize,
    length: usize,
    offset: usize,
    destination: &mut Vec<u16>,
) -> usize {
    let count = (length - offset).min(SUBSTRING_CHUNK_SIZE);
    destination.extend_from_slice(&source[start + offset..start + offset + count]);
    count
}

#[cfg(test)]
mod substring_chunk_tests {
    use super::{SUBSTRING_CHUNK_SIZE, copy_substring_chunk};

    #[test]
    fn copies_bounded_chunks_without_altering_utf16() {
        let mut source = vec![0x1234; SUBSTRING_CHUNK_SIZE + 4];
        source[SUBSTRING_CHUNK_SIZE] = 0xd800;
        source[SUBSTRING_CHUNK_SIZE + 1] = 0xdc00;
        source[SUBSTRING_CHUNK_SIZE + 2] = 0xdfff;
        let length = SUBSTRING_CHUNK_SIZE + 2;
        let mut result = Vec::new();
        let first = copy_substring_chunk(&source, 1, length, 0, &mut result);
        assert_eq!(first, SUBSTRING_CHUNK_SIZE);
        let second = copy_substring_chunk(&source, 1, length, first, &mut result);
        assert_eq!(second, 2);
        assert_eq!(result, source[1..1 + length]);
    }
}

/// System.String::Substring(int)
/// System.String::Substring(int, int)
#[dotnet_intrinsic("string System.String::Substring(int)")]
#[dotnet_intrinsic("string System.String::Substring(int, int)")]
pub fn intrinsic_string_substring<'gc, T: IntrinsicStringHost<'gc>>(
    ctx: &mut T,
    method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let (start_at, length) = if method.signature().parameters.len() == 1 {
        let start_at = ctx.peek_stack().as_i32();
        (start_at, None)
    } else {
        let length = ctx.peek_stack().as_i32();
        let start_at = ctx.peek_stack_at(1).as_i32();
        (start_at, Some(length))
    };

    let val = ctx.peek_stack_at(if length.is_some() { 2 } else { 1 });

    let result_vec = match val {
        StackValue::ObjectRef(source @ ObjectRef(Some(handle))) => {
            let s_len = {
                let inner = handle.borrow();
                match &inner.storage {
                    HeapStorage::Str(s) => s.len(),
                    _ => {
                        return ctx.throw_by_name_with_message(
                            "System.ArgumentException",
                            "The argument must be a string.",
                        );
                    }
                }
            };

            if start_at < 0 || start_at as usize > s_len {
                return ctx.throw_argument_out_of_range("startIndex");
            }

            let start_at = start_at as usize;
            let l = match length {
                None => s_len - start_at,
                Some(l) => {
                    if l < 0 || l as usize > s_len - start_at {
                        return ctx.throw_argument_out_of_range("length");
                    }
                    l as usize
                }
            };

            let operand_count = if length.is_some() { 3 } else { 2 };
            if l > SUBSTRING_CHUNK_SIZE
                && let Some(result) =
                    ctx.offer_substring_continuation(source, start_at, l, operand_count)
            {
                return result;
            }

            let mut result_vec = Vec::with_capacity(l);
            let mut offset = 0;
            while offset < l {
                let current_chunk = {
                    let _gc_scope = GcScopeGuard::enter(
                        ctx.as_borrow_scope(),
                        ctx.as_borrow_scope().gc_ready_token(),
                    );
                    let inner = handle.borrow();
                    match &inner.storage {
                        HeapStorage::Str(s) => {
                            copy_substring_chunk(s, start_at, l, offset, &mut result_vec)
                        }
                        _ => break,
                    }
                };
                offset += current_chunk;
                if offset < l && ctx.check_gc_safe_point() {
                    return StepResult::Yield;
                }
            }
            result_vec
        }
        StackValue::ObjectRef(ObjectRef(None)) => {
            return ctx.throw_by_name_with_message("System.NullReferenceException", NULL_REF_MSG);
        }
        _ => {
            return ctx.throw_by_name_with_message(
                "System.ArgumentException",
                "The argument must be a string.",
            );
        }
    };

    ctx.drop_top(if length.is_some() { 3 } else { 2 });
    ctx.push_string(CLRString::new(result_vec));
    StepResult::Continue
}

/// System.String::IsNullOrEmpty(string)
#[dotnet_intrinsic("static bool System.String::IsNullOrEmpty(string)")]
pub fn intrinsic_string_is_null_or_empty<'gc, T: TypedStackOps<'gc>>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let str_val = ctx.pop();
    let is_null_or_empty = match &str_val {
        StackValue::ObjectRef(ObjectRef(None)) => true,
        StackValue::ObjectRef(ObjectRef(Some(obj))) => {
            let heap = obj.borrow();
            match &heap.storage {
                HeapStorage::Str(s) => s.is_empty(),
                _ => {
                    return StepResult::internal_error(
                        "System.String::IsNullOrEmpty called on non-string object",
                    );
                }
            }
        }
        _ => {
            return StepResult::internal_error(
                "System.String::IsNullOrEmpty called on invalid stack value",
            );
        }
    };
    ctx.push_i32(is_null_or_empty as i32);
    StepResult::Continue
}

/// System.String::IsNullOrWhiteSpace(string)
#[dotnet_intrinsic("static bool System.String::IsNullOrWhiteSpace(string)")]
pub fn intrinsic_string_is_null_or_white_space<'gc, T: TypedStackOps<'gc> + RawMemoryOps<'gc>>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let str_val = ctx.peek_stack();
    let is_null_or_white_space = match &str_val {
        StackValue::ObjectRef(ObjectRef(None)) => true,
        _ => {
            let obj_ref = str_val.as_object_ref();
            let Some(handle) = obj_ref.0 else {
                // This shouldn't happen given the match arms but we must return StepResult
                ctx.pop();
                ctx.push_i32(1);
                return StepResult::Continue;
            };

            let len = {
                let inner = handle.borrow();
                match &inner.storage {
                    HeapStorage::Str(s) => s.len(),
                    _ => {
                        return StepResult::internal_error(
                            "System.String::IsNullOrWhiteSpace called on non-string object",
                        );
                    }
                }
            };

            let mut offset = 0;
            let mut all_white = true;
            const CHUNK_SIZE: usize = 1024;
            while offset < len {
                let current_chunk_size = std::cmp::min(len - offset, CHUNK_SIZE);
                {
                    let _gc_scope = GcScopeGuard::enter(
                        ctx.as_borrow_scope(),
                        ctx.as_borrow_scope().gc_ready_token(),
                    );
                    let inner = handle.borrow();
                    match &inner.storage {
                        HeapStorage::Str(s) => {
                            let chunk = &s[offset..offset + current_chunk_size];
                            let chunk_all_white =
                                probe_all_ascii_whitespace(chunk).unwrap_or_else(|| {
                                    chunk.iter().all(|&c| {
                                        char::from_u32(c as u32)
                                            .map(|c| c.is_whitespace())
                                            .unwrap_or(false)
                                    })
                                });
                            if !chunk_all_white {
                                all_white = false;
                            }
                        }
                        _ => break,
                    }
                }

                if !all_white {
                    break;
                }

                offset += current_chunk_size;
                if offset < len && ctx.check_gc_safe_point() {
                    return StepResult::Yield;
                }
            }
            all_white
        }
    };
    ctx.pop();
    ctx.push_i32(is_null_or_white_space as i32);
    StepResult::Continue
}

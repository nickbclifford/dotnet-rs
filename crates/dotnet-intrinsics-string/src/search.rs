use dotnet_macros::dotnet_intrinsic;
use dotnet_types::{generics::GenericLookup, members::MethodDescription};
use dotnet_utils::GcScopeGuard;
use dotnet_value::{
    StackValue,
    object::{HeapStorage, ObjectRef},
    string::CLRString,
    with_string,
};
use dotnet_vm_ops::{
    StepResult,
    ops::{ExceptionOps, RawMemoryOps, TypedStackOps},
};

use crate::NULL_REF_MSG;

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
    let (c, start_at) = if method.method().signature.parameters.len() == 1 {
        let c = ctx.pop_i32();
        (c as u16, 0usize)
    } else {
        let start_at = ctx.pop_i32();
        let c = ctx.pop_i32();
        (c as u16, start_at as usize)
    };

    let val = ctx.pop();
    let index = with_string!(ctx, val, |s| s.iter().skip(start_at).position(|x| *x == c));

    ctx.push_i32(match index {
        None => -1,
        Some(i) => (i + start_at) as i32,
    });
    StepResult::Continue
}

/// System.String::Substring(int)
/// System.String::Substring(int, int)
#[dotnet_intrinsic("string System.String::Substring(int)")]
#[dotnet_intrinsic("string System.String::Substring(int, int)")]
pub fn intrinsic_string_substring<
    'gc,
    T: TypedStackOps<'gc> + ExceptionOps<'gc> + RawMemoryOps<'gc>,
>(
    ctx: &mut T,
    method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let (start_at, length) = if method.method().signature.parameters.len() == 1 {
        let start_at = ctx.peek_stack().as_i32();
        (start_at as usize, None)
    } else {
        let length = ctx.peek_stack().as_i32();
        let start_at = ctx.peek_stack_at(1).as_i32();
        (start_at as usize, Some(length as usize))
    };

    let val = ctx.peek_stack_at(if length.is_some() { 2 } else { 1 });

    let result_vec: Result<Vec<u16>, String> = match &val {
        StackValue::ObjectRef(ObjectRef(Some(handle))) => {
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

            if start_at > s_len {
                return ctx.throw_by_name_with_message(
                    "System.ArgumentOutOfRangeException",
                    "startIndex",
                );
            }

            let l = match length {
                None => s_len - start_at,
                Some(l) => {
                    if start_at + l > s_len {
                        return ctx.throw_by_name_with_message(
                            "System.ArgumentOutOfRangeException",
                            "length",
                        );
                    }
                    l
                }
            };

            let mut result_vec = Vec::with_capacity(l);
            let mut offset = 0;
            const CHUNK_SIZE: usize = 1024 * 64; // 64K characters
            while offset < l {
                let current_chunk = std::cmp::min(l - offset, CHUNK_SIZE);
                {
                    let _gc_scope = GcScopeGuard::enter(
                        ctx.as_borrow_scope(),
                        ctx.as_borrow_scope().gc_ready_token(),
                    );
                    let inner = handle.borrow();
                    match &inner.storage {
                        HeapStorage::Str(s) => {
                            result_vec.extend_from_slice(
                                &s[start_at + offset..start_at + offset + current_chunk],
                            );
                        }
                        _ => break,
                    }
                }
                offset += current_chunk;
                if offset < l && ctx.check_gc_safe_point() {
                    return StepResult::Yield;
                }
            }
            Ok(result_vec)
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

    let value = match result_vec {
        Ok(v) => v,
        Err(e) => return StepResult::internal_error(e),
    };

    ctx.pop_multiple(if length.is_some() { 3 } else { 2 });
    ctx.push_string(CLRString::new(value));
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
                            for i in 0..current_chunk_size {
                                let c = s[offset + i];
                                if !char::from_u32(c as u32)
                                    .map(|c| c.is_whitespace())
                                    .unwrap_or(false)
                                {
                                    all_white = false;
                                    break;
                                }
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

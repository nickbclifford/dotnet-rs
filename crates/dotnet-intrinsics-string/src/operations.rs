use crate::IntrinsicStringHost;
use dotnet_macros::dotnet_intrinsic;
use dotnet_types::{
    TypeDescription, error::IntrinsicError, generics::GenericLookup, members::MethodDescription,
};
use dotnet_utils::GcScopeGuard;
use dotnet_value::{
    StackValue,
    object::{HeapStorage, Object, ObjectRef},
    string::CLRString,
};
use dotnet_vm_ops::{
    StepResult,
    ops::{ExceptionOps, MemoryOps, RawMemoryOps, TypedStackOps},
};
use std::hash::{DefaultHasher, Hash, Hasher};

const NULL_REF_MSG: &str = "Object reference not set to an instance of an object.";

/// System.String::Equals(string, string)
/// System.String::Equals(string)
/// System.String::Equals(object)
#[dotnet_intrinsic("static bool System.String::Equals(string, string)")]
#[dotnet_intrinsic("bool System.String::Equals(string)")]
#[dotnet_intrinsic("bool System.String::Equals(object)")]
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
                let (a_len, a_is_str) = {
                    let a_heap = a_handle.borrow();
                    if let HeapStorage::Str(s) = &a_heap.storage {
                        (s.len(), true)
                    } else {
                        (0, false)
                    }
                };
                let (b_len, b_is_str) = {
                    let b_heap = b_handle.borrow();
                    if let HeapStorage::Str(s) = &b_heap.storage {
                        (s.len(), true)
                    } else {
                        (0, false)
                    }
                };

                if !a_is_str || !b_is_str || a_len != b_len {
                    false
                } else {
                    let length = a_len;
                    let mut offset = 0;
                    const CHUNK_SIZE: usize = 4096; // 4K characters (8KB)
                    let mut equal = true;

                    while offset < length {
                        let current_chunk = std::cmp::min(CHUNK_SIZE, length - offset);

                        let chunk_equal = {
                            let a_heap = a_handle.borrow();
                            let b_heap = b_handle.borrow();
                            if let (HeapStorage::Str(a), HeapStorage::Str(b)) =
                                (&a_heap.storage, &b_heap.storage)
                            {
                                a[offset..offset + current_chunk]
                                    == b[offset..offset + current_chunk]
                            } else {
                                false
                            }
                        };

                        if !chunk_equal {
                            equal = false;
                            break;
                        }

                        offset += current_chunk;
                        if offset < length && ctx.check_gc_safe_point() {
                            return StepResult::Yield;
                        }
                    }
                    equal
                }
            }
        }
        _ => false,
    };

    ctx.pop_multiple(2);
    ctx.push_i32(if res { 1 } else { 0 });
    StepResult::Continue
}

/// System.String::Concat(ReadOnlySpan<char>, ReadOnlySpan<char>, ReadOnlySpan<char>)
#[dotnet_intrinsic(
    "static string System.String::Concat(System.ReadOnlySpan<char>, System.ReadOnlySpan<char>, System.ReadOnlySpan<char>)"
)]
pub fn intrinsic_string_concat_three_spans<'gc, T: IntrinsicStringHost<'gc>>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let span2 = ctx.peek_stack().as_value_type();
    let span1 = ctx.peek_stack_at(1).as_value_type();
    let span0 = ctx.peek_stack_at(2).as_value_type();

    let mut data0 = Vec::new();
    let mut data1 = Vec::new();
    let mut data2 = Vec::new();

    let copy_span_chunked =
        |ctx: &mut T, span: Object, dest: &mut Vec<u16>| -> Result<(), IntrinsicError> {
            let len =
                ctx.string_with_span_data(span.clone(), TypeDescription::NULL, 2, |slice| {
                    slice.len() / 2
                })?;
            let mut offset = 0;
            const CHUNK_SIZE: usize = 1024;

            while offset < len {
                let chunk_len = std::cmp::min(CHUNK_SIZE, len - offset);
                ctx.string_with_span_data(span.clone(), TypeDescription::NULL, 2, |slice| {
                    for i in 0..chunk_len {
                        let c_idx = (offset + i) * 2;
                        dest.push(u16::from_ne_bytes([slice[c_idx], slice[c_idx + 1]]));
                    }
                })?;
                offset += chunk_len;
                if offset < len {
                    let _ = ctx.check_gc_safe_point();
                }
            }
            Ok(())
        };

    crate::vm_try!(copy_span_chunked(ctx, span0, &mut data0));
    crate::vm_try!(copy_span_chunked(ctx, span1, &mut data1));
    crate::vm_try!(copy_span_chunked(ctx, span2, &mut data2));

    let total_length = data0.len() + data1.len() + data2.len();
    const LARGE_STRING_CONCAT_THRESHOLD: usize = 1024;
    if total_length > LARGE_STRING_CONCAT_THRESHOLD && ctx.check_gc_safe_point() {
        return StepResult::Yield;
    }

    let value = CLRString::new(data0.into_iter().chain(data1).chain(data2).collect());
    ctx.pop_multiple(3);
    ctx.push_string(value);
    StepResult::Continue
}

/// System.String::GetHashCodeOrdinalIgnoreCase()
#[dotnet_intrinsic("int System.String::GetHashCodeOrdinalIgnoreCase()")]
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
            const CHUNK_SIZE: usize = 1024;

            while offset < len {
                let chunk_len = std::cmp::min(CHUNK_SIZE, len - offset);
                {
                    let _gc_scope = GcScopeGuard::enter(
                        ctx.as_borrow_scope(),
                        ctx.as_borrow_scope().gc_ready_token(),
                    );
                    let inner = handle.borrow();
                    match &inner.storage {
                        HeapStorage::Str(s) => {
                            result.extend_from_slice(&s[offset..offset + chunk_len]);
                        }
                        _ => break,
                    }
                }
                offset += chunk_len;
                if offset < len && ctx.check_gc_safe_point() {
                    return StepResult::Yield;
                }
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
        {
            let _gc_scope = GcScopeGuard::enter(
                ctx.as_borrow_scope(),
                ctx.as_borrow_scope().gc_ready_token(),
            );
            let src_heap = src_handle.borrow();
            match &src_heap.storage {
                HeapStorage::Str(s) => {
                    chunk_buf.extend_from_slice(&s[offset..offset + chunk_len]);
                }
                _ => break,
            }
        }

        let mut err = false;
        {
            let gc = ctx.gc_with_token(&ctx.no_active_borrows_token());
            let _gc_scope = GcScopeGuard::enter(
                ctx.as_borrow_scope(),
                ctx.as_borrow_scope().gc_ready_token(),
            );
            let mut dest_heap = dest_handle.borrow_mut(gc.mutation());
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

        offset += chunk_len;
        if offset < src_len && ctx.check_gc_safe_point() {
            return StepResult::Yield;
        }
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

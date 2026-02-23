use crate::{
    StepResult,
    intrinsics::span::{intrinsic_as_span, with_span_data},
    stack::ops::VesOps,
};
use dotnet_macros::{dotnet_intrinsic, dotnet_intrinsic_field};
use dotnet_types::{
    TypeDescription,
    generics::{ConcreteType, GenericLookup},
    members::{FieldDescription, MethodDescription},
};
use dotnet_value::{
    StackValue,
    object::{HeapStorage, Object, ObjectRef},
    string::CLRString,
    with_string,
};
use std::{
    hash::{DefaultHasher, Hash, Hasher},
    sync::Arc,
};

const NULL_REF_MSG: &str = "Object reference not set to an instance of an object.";

/// System.String::Equals(string, string)
/// System.String::Equals(string)
/// System.String::Equals(object)
#[dotnet_intrinsic("static bool System.String::Equals(string, string)")]
#[dotnet_intrinsic("bool System.String::Equals(string)")]
#[dotnet_intrinsic("bool System.String::Equals(object)")]
pub fn intrinsic_string_equals<'gc, 'm: 'gc>(
    ctx: &mut dyn VesOps<'gc, 'm>,
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

/// System.String::FastAllocateString(int)
#[dotnet_intrinsic("static string System.String::FastAllocateString(int)")]
#[dotnet_intrinsic("static string System.String::FastAllocateString(int, IntPtr)")]
pub fn intrinsic_string_fast_allocate_string<'gc, 'm: 'gc>(
    ctx: &mut dyn VesOps<'gc, 'm>,
    method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let len = if method.method.signature.parameters.len() == 1 {
        let i = ctx.pop_i32();
        if i < 0 {
            return ctx.throw_by_name_with_message(
                "System.OverflowException",
                "Arithmetic operation resulted in an overflow.",
            );
        }
        i as usize
    } else {
        // Overload with MethodTable* as first param
        let i = ctx.pop_isize();
        let _ = ctx.pop(); // pop method table pointer
        if i < 0 {
            return ctx.throw_by_name_with_message(
                "System.OverflowException",
                "Arithmetic operation resulted in an overflow.",
            );
        }
        i as usize
    };

    // Defensive check: limit string size to 512MB characters
    if len > 0x2000_0000 {
        return ctx.throw_by_name_with_message(
            "System.OutOfMemoryException",
            "Insufficient memory to continue the execution of the program.",
        );
    }

    // Check GC safe point before allocating large strings
    const LARGE_STRING_THRESHOLD: usize = 1024;
    if len > LARGE_STRING_THRESHOLD && ctx.check_gc_safe_point() {
        return StepResult::Yield;
    }

    let value = CLRString::new(vec![0u16; len]);
    ctx.push_string(value);
    StepResult::Continue
}

/// System.String::.ctor(char[])
#[dotnet_intrinsic("void System.String::.ctor(char[])")]
pub fn intrinsic_string_ctor_char_array<'gc, 'm: 'gc>(
    ctx: &mut dyn VesOps<'gc, 'm>,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let arg = ctx.pop();
    let chars: Vec<u16> = match arg {
        StackValue::ObjectRef(ObjectRef(Some(handle))) => {
            let len = {
                let obj = handle.borrow();
                match &obj.storage {
                    HeapStorage::Vec(v) => v.layout.length,
                    _ => 0,
                }
            };

            let mut result = Vec::with_capacity(len);
            let mut offset = 0;
            const CHUNK_SIZE: usize = 1024;

            while offset < len {
                let chunk_len = std::cmp::min(CHUNK_SIZE, len - offset);
                {
                    let _guard = dotnet_value::BorrowGuard::new(ctx);
                    let obj = handle.borrow();
                    match &obj.storage {
                        HeapStorage::Vec(v) => {
                            let bytes = v.get();
                            for i in 0..chunk_len {
                                let c_idx = (offset + i) * 2;
                                if c_idx + 1 < bytes.len() {
                                    result
                                        .push(u16::from_ne_bytes([bytes[c_idx], bytes[c_idx + 1]]));
                                }
                            }
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
        _ => Vec::new(), // null or invalid
    };

    let value = CLRString::new(chars);
    ctx.push_string(value);
    StepResult::Continue
}

/// System.String::.ctor(char*)
#[dotnet_intrinsic("void System.String::.ctor(char*)")]
pub fn intrinsic_string_ctor_char_ptr<'gc, 'm: 'gc>(
    ctx: &mut dyn VesOps<'gc, 'm>,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let arg = ctx.pop();
    let ptr = match arg {
        StackValue::NativeInt(p) => p as *const u16,
        StackValue::Int32(p) => p as usize as *const u16,
        StackValue::Int64(p) => p as usize as *const u16,
        _ => std::ptr::null(),
    };

    let value = if !ptr.is_null() {
        let mut chars = Vec::new();
        let mut i = 0;
        // SAFETY: We assume the pointer is valid and points to a null-terminated string.
        // This is a standard assumption for string constructors taking a pointer.
        unsafe {
            loop {
                let c = *ptr.add(i);
                if c == 0 {
                    break;
                }
                chars.push(c);
                i += 1;

                if i % 1024 == 0 && ctx.check_gc_safe_point() {
                    return StepResult::Yield;
                }
            }
        }
        CLRString::new(chars)
    } else {
        CLRString::new(Vec::new())
    };

    ctx.push_string(value);
    StepResult::Continue
}

/// System.String::get_Chars(int)
#[dotnet_intrinsic("char System.String::get_Chars(int)")]
pub fn intrinsic_string_get_chars<'gc, 'm: 'gc>(
    ctx: &mut dyn VesOps<'gc, 'm>,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let index = ctx.pop_i32();
    let val = ctx.pop();
    if val.as_object_ref().0.is_none() {
        return ctx.throw_by_name_with_message("System.NullReferenceException", NULL_REF_MSG);
    }
    let char_opt = with_string!(ctx, val, |s| if index >= 0 && (index as usize) < s.len() {
        Some(s[index as usize])
    } else {
        None
    });
    if let Some(value) = char_opt {
        ctx.push_i32(value as i32);
        StepResult::Continue
    } else {
        ctx.throw_by_name_with_message(
            "System.IndexOutOfRangeException",
            "Index was out of range. Must be non-negative and less than the size of the collection.",
        )
    }
}

/// System.String::get_Length()
#[dotnet_intrinsic("int System.String::get_Length()")]
pub fn intrinsic_string_get_length<'gc, 'm: 'gc>(
    ctx: &mut dyn VesOps<'gc, 'm>,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let val = ctx.pop();
    let len = with_string!(ctx, val, |s| s.len());
    ctx.push_i32(len as i32);
    StepResult::Continue
}

/// System.String::Concat(ReadOnlySpan<char>, ReadOnlySpan<char>, ReadOnlySpan<char>)
#[dotnet_intrinsic(
    "static string System.String::Concat(System.ReadOnlySpan<char>, System.ReadOnlySpan<char>, System.ReadOnlySpan<char>)"
)]
pub fn intrinsic_string_concat_three_spans<'gc, 'm: 'gc>(
    ctx: &mut dyn VesOps<'gc, 'm>,
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
        |ctx: &mut dyn VesOps<'gc, 'm>, span: Object, dest: &mut Vec<u16>| -> Result<(), String> {
            let len = with_span_data(ctx, span.clone(), TypeDescription::NULL, 2, |slice| {
                slice.len() / 2
            })?;
            let mut offset = 0;
            const CHUNK_SIZE: usize = 1024;

            while offset < len {
                let chunk_len = std::cmp::min(CHUNK_SIZE, len - offset);
                with_span_data(ctx, span.clone(), TypeDescription::NULL, 2, |slice| {
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

    vm_try!(
        copy_span_chunked(ctx, span0, &mut data0)
            .map_err(crate::error::ExecutionError::NotImplemented)
    );
    vm_try!(
        copy_span_chunked(ctx, span1, &mut data1)
            .map_err(crate::error::ExecutionError::NotImplemented)
    );
    vm_try!(
        copy_span_chunked(ctx, span2, &mut data2)
            .map_err(crate::error::ExecutionError::NotImplemented)
    );

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
pub fn intrinsic_string_get_hash_code_ordinal_ignore_case<'gc, 'm: 'gc>(
    ctx: &mut dyn VesOps<'gc, 'm>,
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
                        return StepResult::Error(
                            crate::error::ExecutionError::InternalError(
                                "System.String::GetHashCodeOrdinalIgnoreCase called on non-string object"
                                    .to_string(),
                            )
                            .into(),
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
                    let _guard = dotnet_value::BorrowGuard::new(ctx);
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

/// System.String::GetPinnableReference()
/// System.String::GetRawStringData()
#[dotnet_intrinsic("char& System.String::GetPinnableReference()")]
#[dotnet_intrinsic("char& System.String::GetRawStringData()")]
pub fn intrinsic_string_get_raw_data<'gc, 'm: 'gc>(
    ctx: &mut dyn VesOps<'gc, 'm>,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let val = ctx.pop();
    let char_type = vm_try!(ctx.loader().corlib_type("System.Char"));

    let obj = val.as_object_ref();
    if let Some(handle) = obj.0 {
        let (ptr, is_str) = {
            let heap = handle.borrow();
            if let HeapStorage::Str(_) = &heap.storage {
                (unsafe { heap.storage.raw_data_ptr() }, true)
            } else {
                (std::ptr::null_mut(), false)
            }
        };

        if is_str {
            ctx.push_ptr(
                ptr,
                char_type,
                false,
                Some(obj),
                Some(dotnet_value::ByteOffset(0)),
            );
            StepResult::Continue
        } else {
            let heap = handle.borrow();
            StepResult::Error(
                crate::error::ExecutionError::InternalError(format!(
                    "invalid type on stack, expected string, received {:?}",
                    heap.storage
                ))
                .into(),
            )
        }
    } else {
        ctx.throw_by_name_with_message("System.NullReferenceException", NULL_REF_MSG)
    }
}

/// System.String::IndexOf(char)
/// System.String::IndexOf(char, int)
#[dotnet_intrinsic("int System.String::IndexOf(char)")]
#[dotnet_intrinsic("int System.String::IndexOf(char, int)")]
pub fn intrinsic_string_index_of<'gc, 'm: 'gc>(
    ctx: &mut dyn VesOps<'gc, 'm>,
    method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let (c, start_at) = if method.method.signature.parameters.len() == 1 {
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
pub fn intrinsic_string_substring<'gc, 'm: 'gc>(
    ctx: &mut dyn VesOps<'gc, 'm>,
    method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let (start_at, length) = if method.method.signature.parameters.len() == 1 {
        let start_at = ctx.peek_stack().as_i32();
        (start_at as usize, None)
    } else {
        let length = ctx.peek_stack().as_i32();
        let start_at = ctx.peek_stack_at(1).as_i32();
        (start_at as usize, Some(length as usize))
    };

    let val = ctx.peek_stack_at(if length.is_some() { 2 } else { 1 });

    let result_vec = match &val {
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
                    let _guard = dotnet_value::BorrowGuard::new(ctx);
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
        Err(e) => return StepResult::Error(crate::error::ExecutionError::InternalError(e).into()),
    };

    ctx.pop_multiple(if length.is_some() { 3 } else { 2 });
    ctx.push_string(CLRString::new(value));
    StepResult::Continue
}

/// System.String::IsNullOrEmpty(string)
#[dotnet_intrinsic("static bool System.String::IsNullOrEmpty(string)")]
pub fn intrinsic_string_is_null_or_empty<'gc, 'm: 'gc>(
    ctx: &mut dyn VesOps<'gc, 'm>,
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
                    return StepResult::Error(
                        crate::error::ExecutionError::InternalError(
                            "System.String::IsNullOrEmpty called on non-string object".to_string(),
                        )
                        .into(),
                    );
                }
            }
        }
        _ => {
            return StepResult::Error(
                crate::error::ExecutionError::InternalError(
                    "System.String::IsNullOrEmpty called on invalid stack value".to_string(),
                )
                .into(),
            );
        }
    };
    ctx.push_i32(is_null_or_empty as i32);
    StepResult::Continue
}

/// System.String::IsNullOrWhiteSpace(string)
#[dotnet_intrinsic("static bool System.String::IsNullOrWhiteSpace(string)")]
pub fn intrinsic_string_is_null_or_white_space<'gc, 'm: 'gc>(
    ctx: &mut dyn VesOps<'gc, 'm>,
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
                        return StepResult::Error(
                            crate::error::ExecutionError::InternalError(
                                "System.String::IsNullOrWhiteSpace called on non-string object"
                                    .to_string(),
                            )
                            .into(),
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
                    let _guard = dotnet_value::BorrowGuard::new(ctx);
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

#[dotnet_intrinsic_field("static string System.String::Empty")]
pub fn intrinsic_field_string_empty<'gc, 'm: 'gc>(
    ctx: &mut dyn VesOps<'gc, 'm>,
    _field: FieldDescription,
    _type_generics: Arc<[ConcreteType]>,
    _is_address: bool,
) -> StepResult {
    ctx.push_string(CLRString::new(vec![]));
    StepResult::Continue
}

#[dotnet_intrinsic_field("int System.String::_stringLength")]
pub fn intrinsic_field_string_length<'gc, 'm: 'gc>(
    ctx: &mut dyn VesOps<'gc, 'm>,
    _field: FieldDescription,
    _type_generics: Arc<[ConcreteType]>,
    is_address: bool,
) -> StepResult {
    if is_address {
        return StepResult::Error(
            crate::error::ExecutionError::NotImplemented(
                "taking address of _stringLength is not supported".to_string(),
            )
            .into(),
        );
    }
    let val = ctx.pop();
    let len = with_string!(ctx, val, |s| s.len());

    ctx.push_i32(len as i32);
    StepResult::Continue
}

/// System.String::CopyStringContent(string, int, string)
#[dotnet_intrinsic("static void System.String::CopyStringContent(string, int, string)")]
pub fn intrinsic_string_copy_string_content<'gc, 'm: 'gc>(
    ctx: &mut dyn VesOps<'gc, 'm>,
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
            let _guard = dotnet_value::BorrowGuard::new(ctx);
            let src_heap = src_handle.borrow();
            match &src_heap.storage {
                HeapStorage::Str(s) => {
                    chunk_buf.extend_from_slice(&s[offset..offset + chunk_len]);
                }
                _ => break,
            }
        }

        {
            let _guard = dotnet_value::BorrowGuard::new(ctx);
            let mut dest_heap = dest_handle.borrow_mut(ctx.gc().mutation());
            match &mut dest_heap.storage {
                HeapStorage::Str(dest) => {
                    let d_pos = dest_pos + offset;
                    if d_pos + chunk_len > dest.len() {
                        return ctx.throw_by_name_with_message(
                            "System.ArgumentOutOfRangeException",
                            "Destination is too short.",
                        );
                    }
                    dest.as_mut_slice()[d_pos..d_pos + chunk_len].copy_from_slice(&chunk_buf);
                }
                _ => break,
            }
        }

        offset += chunk_len;
        if offset < src_len && ctx.check_gc_safe_point() {
            return StepResult::Yield;
        }
    }
    StepResult::Continue
}

#[dotnet_intrinsic("static System.ReadOnlySpan<char> System.String::op_Implicit(string)")]
pub fn intrinsic_string_implicit_to_span<'gc, 'm: 'gc>(
    ctx: &mut dyn VesOps<'gc, 'm>,
    method: MethodDescription,
    generics: &GenericLookup,
) -> StepResult {
    intrinsic_as_span(ctx, method, generics)
}

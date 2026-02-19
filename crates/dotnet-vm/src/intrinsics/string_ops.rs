use crate::{StepResult, intrinsics::span::{intrinsic_as_span, with_span_data}, stack::ops::VesOps};
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
    with_string, with_string_mut,
};
use std::{
    hash::{DefaultHasher, Hash, Hasher},
    sync::Arc,
};

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
    let b_val = ctx.pop();
    let a_val = ctx.pop();

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
            let ptr_equal = Gc::as_ptr(a_handle) == Gc::as_ptr(b_handle);
            if ptr_equal {
                true
            } else {
                let a_heap = a_handle.borrow();
                let b_heap = b_handle.borrow();
                match (&a_heap.storage, &b_heap.storage) {
                    (HeapStorage::Str(a), HeapStorage::Str(b)) => a == b,
                    _ => false,
                }
            }
        }
        _ => false,
    };

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
            return ctx.throw_by_name("System.OverflowException");
        }
        i as usize
    } else {
        // Overload with MethodTable* as first param
        let i = ctx.pop_isize();
        let _ = ctx.pop(); // pop method table pointer
        if i < 0 {
            return ctx.throw_by_name("System.OverflowException");
        }
        i as usize
    };

    // Defensive check: limit string size to 512MB characters
    if len > 0x2000_0000 {
        return ctx.throw_by_name("System.OutOfMemoryException");
    }

    // Check GC safe point before allocating large strings
    const LARGE_STRING_THRESHOLD: usize = 1024;
    if len > LARGE_STRING_THRESHOLD {
        // ctx.check_gc_safe_point();
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
            let obj = handle.borrow();
            match &obj.storage {
                HeapStorage::Vec(v) => {
                    // Assuming u16 data (char is 2 bytes)
                    // We need to interpret bytes as u16.
                    // Vector stores data as Vec<u8>.
                    let bytes = v.get();
                    bytes
                        .chunks_exact(2)
                        .map(|c| u16::from_ne_bytes([c[0], c[1]]))
                        .collect()
                }
                _ => Vec::new(), // Should not happen for char[]
            }
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
        return ctx.throw_by_name("System.NullReferenceException");
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
        ctx.throw_by_name("System.IndexOutOfRangeException")
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
    let span2 = ctx.pop_value_type();
    let span1 = ctx.pop_value_type();
    let span0 = ctx.pop_value_type();

    fn char_span_into_str(span: Object) -> Result<Vec<u16>, String> {
        with_span_data(span, TypeDescription::NULL, 2, |slice| {
            slice
                .chunks_exact(2)
                .map(|c| u16::from_ne_bytes([c[0], c[1]]))
                .collect::<Vec<_>>()
        })
    }

    let data0 =
        vm_try!(char_span_into_str(span0).map_err(crate::error::ExecutionError::NotImplemented));
    let data1 =
        vm_try!(char_span_into_str(span1).map_err(crate::error::ExecutionError::NotImplemented));
    let data2 =
        vm_try!(char_span_into_str(span2).map_err(crate::error::ExecutionError::NotImplemented));

    let total_length = data0.len() + data1.len() + data2.len();
    const LARGE_STRING_CONCAT_THRESHOLD: usize = 1024;
    if total_length > LARGE_STRING_CONCAT_THRESHOLD {
        // ctx.check_gc_safe_point();
    }

    let value = CLRString::new(data0.into_iter().chain(data1).chain(data2).collect());
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
    let value = with_string!(ctx, val, |s| String::from_utf16_lossy(s)
        .to_uppercase()
        .into_bytes());
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
        let heap = handle.borrow();
        if let HeapStorage::Str(_) = &heap.storage {
            let ptr = unsafe { heap.storage.raw_data_ptr() };
            ctx.push_ptr(
                ptr,
                char_type,
                false,
                Some(obj),
                Some(dotnet_value::ByteOffset(0)),
            );
            StepResult::Continue
        } else {
            StepResult::Error(
                crate::error::ExecutionError::InternalError(format!(
                    "invalid type on stack, expected string, received {:?}",
                    heap.storage
                ))
                .into(),
            )
        }
    } else {
        ctx.throw_by_name("System.NullReferenceException")
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
        let start_at = ctx.pop_i32();
        (start_at as usize, None)
    } else {
        let length = ctx.pop_i32();
        let start_at = ctx.pop_i32();
        (start_at as usize, Some(length as usize))
    };

    let val = ctx.pop();
    let value = match with_string!(ctx, val, |s| {
        if start_at > s.len() {
            Err("start_at out of range".to_string())
        } else {
            let sub = &s[start_at..];
            match length {
                None => Ok(sub.to_vec()),
                Some(l) => {
                    if l > sub.len() {
                        Err("length out of range".to_string())
                    } else {
                        Ok(sub[..l].to_vec())
                    }
                }
            }
        }
    }) {
        Ok(v) => v,
        Err(_) => return ctx.throw_by_name("System.ArgumentOutOfRangeException"),
    };

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
    let str_val = ctx.pop();
    let is_null_or_white_space = match &str_val {
        StackValue::ObjectRef(ObjectRef(None)) => true,
        _ => with_string!(ctx, str_val, |s| {
            s.iter().all(|&c| {
                char::from_u32(c as u32)
                    .map(|c| c.is_whitespace())
                    .unwrap_or(false)
            })
        }),
    };
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
    let dest_pos = ctx.pop_i32();
    let dest_val = ctx.pop_obj();

    let src = with_string!(ctx, StackValue::ObjectRef(src_val), |s| s.to_vec());
    let res = with_string_mut!(ctx, StackValue::ObjectRef(dest_val), |dest| {
        let dest_pos = dest_pos as usize;
        let len = src.len();
        if dest_pos + len > dest.len() {
            Err("destination too small".to_string())
        } else {
            dest.as_mut_slice()[dest_pos..dest_pos + len].copy_from_slice(&src);
            Ok(())
        }
    });
    if res.is_err() {
        return ctx.throw_by_name("System.ArgumentOutOfRangeException");
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
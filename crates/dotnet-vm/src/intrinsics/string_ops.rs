use crate::{StepResult, intrinsics::span::span_to_slice, stack::ops::VesOps};
use dotnet_macros::{dotnet_intrinsic, dotnet_intrinsic_field};
use dotnet_types::{
    generics::{ConcreteType, GenericLookup},
    members::{FieldDescription, MethodDescription},
};
use dotnet_utils::gc::GCHandle;
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
#[dotnet_intrinsic("static bool System.String::Equals(string, string)")]
#[dotnet_intrinsic("bool System.String::Equals(string)")]
pub fn intrinsic_string_equals<'gc, 'm: 'gc>(
    ctx: &mut dyn VesOps<'gc, 'm>,
    gc: GCHandle<'gc>,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let b_val = ctx.pop(gc);
    let a_val = ctx.pop(gc);

    let res = match (a_val, b_val) {
        (StackValue::ObjectRef(ObjectRef(None)), StackValue::ObjectRef(ObjectRef(None))) => true,
        (StackValue::ObjectRef(ObjectRef(None)), _)
        | (_, StackValue::ObjectRef(ObjectRef(None))) => false,
        (
            StackValue::ObjectRef(ObjectRef(Some(a_handle))),
            StackValue::ObjectRef(ObjectRef(Some(b_handle))),
        ) => {
            if unsafe { std::ptr::eq(a_handle.as_ptr(), b_handle.as_ptr()) } {
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

    ctx.push_i32(gc, if res { 1 } else { 0 });
    StepResult::Continue
}

/// System.String::FastAllocateString(int)
#[dotnet_intrinsic("static string System.String::FastAllocateString(int)")]
#[dotnet_intrinsic("static string System.String::FastAllocateString(int, IntPtr)")]
pub fn intrinsic_string_fast_allocate_string<'gc, 'm: 'gc>(
    ctx: &mut dyn VesOps<'gc, 'm>,
    gc: GCHandle<'gc>,
    method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let len = if method.method.signature.parameters.len() == 1 {
        let i = ctx.pop_i32(gc);
        if i < 0 {
            return ctx.throw_by_name(gc, "System.OverflowException");
        }
        i as usize
    } else {
        // Overload with MethodTable* as first param
        let i = ctx.pop_isize(gc);
        ctx.pop(gc); // pop method table pointer
        if i < 0 {
            return ctx.throw_by_name(gc, "System.OverflowException");
        }
        i as usize
    };

    // Defensive check: limit string size to 512MB characters
    if len > 0x2000_0000 {
        return ctx.throw_by_name(gc, "System.OutOfMemoryException");
    }

    // Check GC safe point before allocating large strings
    const LARGE_STRING_THRESHOLD: usize = 1024;
    if len > LARGE_STRING_THRESHOLD {
        ctx.check_gc_safe_point();
    }

    let value = CLRString::new(vec![0u16; len]);
    ctx.push_string(gc, value);
    StepResult::Continue
}

/// System.String::.ctor(char[])
#[dotnet_intrinsic("void System.String::.ctor(char[])")]
pub fn intrinsic_string_ctor_char_array<'gc, 'm: 'gc>(
    ctx: &mut dyn VesOps<'gc, 'm>,
    gc: GCHandle<'gc>,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let arg = ctx.pop(gc);
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
    ctx.push_string(gc, value);
    StepResult::Continue
}

/// System.String::.ctor(char*)
#[dotnet_intrinsic("void System.String::.ctor(char*)")]
pub fn intrinsic_string_ctor_char_ptr<'gc, 'm: 'gc>(
    ctx: &mut dyn VesOps<'gc, 'm>,
    gc: GCHandle<'gc>,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let arg = ctx.pop(gc);
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

    ctx.push_string(gc, value);
    StepResult::Continue
}

/// System.String::get_Chars(int)
#[dotnet_intrinsic("char System.String::get_Chars(int)")]
pub fn intrinsic_string_get_chars<'gc, 'm: 'gc>(
    ctx: &mut dyn VesOps<'gc, 'm>,
    gc: GCHandle<'gc>,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let index = ctx.pop_i32(gc);
    let val = ctx.pop(gc);
    let value = with_string!(ctx, gc, val, |s| s[index as usize]);
    ctx.push_i32(gc, value as i32);
    StepResult::Continue
}

/// System.String::get_Length()
#[dotnet_intrinsic("int System.String::get_Length()")]
pub fn intrinsic_string_get_length<'gc, 'm: 'gc>(
    ctx: &mut dyn VesOps<'gc, 'm>,
    gc: GCHandle<'gc>,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let val = ctx.pop(gc);
    let len = with_string!(ctx, gc, val, |s| s.len());
    ctx.push_i32(gc, len as i32);
    StepResult::Continue
}

/// System.String::Concat(ReadOnlySpan<char>, ReadOnlySpan<char>, ReadOnlySpan<char>)
#[dotnet_intrinsic(
    "static string System.String::Concat(System.ReadOnlySpan<char>, System.ReadOnlySpan<char>, System.ReadOnlySpan<char>)"
)]
pub fn intrinsic_string_concat_three_spans<'gc, 'm: 'gc>(
    ctx: &mut dyn VesOps<'gc, 'm>,
    gc: GCHandle<'gc>,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let span2 = ctx.pop_value_type(gc);
    let span1 = ctx.pop_value_type(gc);
    let span0 = ctx.pop_value_type(gc);

    fn char_span_into_str(span: Object) -> Vec<u16> {
        span_to_slice(span, 2)
            .chunks_exact(2)
            .map(|c| u16::from_ne_bytes([c[0], c[1]]))
            .collect::<Vec<_>>()
    }

    let data0 = char_span_into_str(span0);
    let data1 = char_span_into_str(span1);
    let data2 = char_span_into_str(span2);

    let total_length = data0.len() + data1.len() + data2.len();
    const LARGE_STRING_CONCAT_THRESHOLD: usize = 1024;
    if total_length > LARGE_STRING_CONCAT_THRESHOLD {
        ctx.check_gc_safe_point();
    }

    let value = CLRString::new(data0.into_iter().chain(data1).chain(data2).collect());
    ctx.push_string(gc, value);
    StepResult::Continue
}

/// System.String::GetHashCodeOrdinalIgnoreCase()
#[dotnet_intrinsic("int System.String::GetHashCodeOrdinalIgnoreCase()")]
pub fn intrinsic_string_get_hash_code_ordinal_ignore_case<'gc, 'm: 'gc>(
    ctx: &mut dyn VesOps<'gc, 'm>,
    gc: GCHandle<'gc>,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let val = ctx.pop(gc);
    let mut h = DefaultHasher::new();
    let value = with_string!(ctx, gc, val, |s| String::from_utf16_lossy(s)
        .to_uppercase()
        .into_bytes());
    value.hash(&mut h);
    let code = h.finish();

    ctx.push_i32(gc, code as i32);
    StepResult::Continue
}

/// System.String::GetPinnableReference()
/// System.String::GetRawStringData()
#[dotnet_intrinsic("char& System.String::GetPinnableReference()")]
#[dotnet_intrinsic("char& System.String::GetRawStringData()")]
pub fn intrinsic_string_get_raw_data<'gc, 'm: 'gc>(
    ctx: &mut dyn VesOps<'gc, 'm>,
    gc: GCHandle<'gc>,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let val = ctx.pop(gc);
    let ptr = with_string!(ctx, gc, val, |s| s.as_ptr() as *mut u8);
    ctx.push_ptr(gc, ptr, ctx.loader().corlib_type("System.Char"), false);
    StepResult::Continue
}

/// System.String::IndexOf(char)
/// System.String::IndexOf(char, int)
#[dotnet_intrinsic("int System.String::IndexOf(char)")]
#[dotnet_intrinsic("int System.String::IndexOf(char, int)")]
pub fn intrinsic_string_index_of<'gc, 'm: 'gc>(
    ctx: &mut dyn VesOps<'gc, 'm>,
    gc: GCHandle<'gc>,
    method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let (c, start_at) = if method.method.signature.parameters.len() == 1 {
        let c = ctx.pop_i32(gc);
        (c as u16, 0usize)
    } else {
        let start_at = ctx.pop_i32(gc);
        let c = ctx.pop_i32(gc);
        (c as u16, start_at as usize)
    };

    let val = ctx.pop(gc);
    let index = with_string!(ctx, gc, val, |s| s
        .iter()
        .skip(start_at)
        .position(|x| *x == c));

    ctx.push_i32(
        gc,
        match index {
            None => -1,
            Some(i) => (i + start_at) as i32,
        },
    );
    StepResult::Continue
}

/// System.String::Substring(int)
/// System.String::Substring(int, int)
#[dotnet_intrinsic("string System.String::Substring(int)")]
#[dotnet_intrinsic("string System.String::Substring(int, int)")]
pub fn intrinsic_string_substring<'gc, 'm: 'gc>(
    ctx: &mut dyn VesOps<'gc, 'm>,
    gc: GCHandle<'gc>,
    method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let (start_at, length) = if method.method.signature.parameters.len() == 1 {
        let start_at = ctx.pop_i32(gc);
        (start_at as usize, None)
    } else {
        let length = ctx.pop_i32(gc);
        let start_at = ctx.pop_i32(gc);
        (start_at as usize, Some(length as usize))
    };

    let val = ctx.pop(gc);
    let value = with_string!(ctx, gc, val, |s| {
        let sub = &s[start_at..];
        match length {
            None => sub.to_vec(),
            Some(l) => sub[..l].to_vec(),
        }
    });

    ctx.push_string(gc, CLRString::new(value));
    StepResult::Continue
}

/// System.String::IsNullOrEmpty(string)
#[dotnet_intrinsic("static bool System.String::IsNullOrEmpty(string)")]
pub fn intrinsic_string_is_null_or_empty<'gc, 'm: 'gc>(
    ctx: &mut dyn VesOps<'gc, 'm>,
    gc: GCHandle<'gc>,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let str_val = ctx.pop(gc);
    let is_null_or_empty = match &str_val {
        StackValue::ObjectRef(ObjectRef(None)) => true,
        StackValue::ObjectRef(ObjectRef(Some(obj))) => {
            let heap = obj.borrow();
            match &heap.storage {
                HeapStorage::Str(s) => s.is_empty(),
                _ => panic!("System.String::IsNullOrEmpty called on non-string object"),
            }
        }
        _ => panic!("System.String::IsNullOrEmpty called on invalid stack value"),
    };
    ctx.push_i32(gc, is_null_or_empty as i32);
    StepResult::Continue
}

/// System.String::IsNullOrWhiteSpace(string)
#[dotnet_intrinsic("static bool System.String::IsNullOrWhiteSpace(string)")]
pub fn intrinsic_string_is_null_or_white_space<'gc, 'm: 'gc>(
    ctx: &mut dyn VesOps<'gc, 'm>,
    gc: GCHandle<'gc>,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let str_val = ctx.pop(gc);
    let is_null_or_white_space = match &str_val {
        StackValue::ObjectRef(ObjectRef(None)) => true,
        _ => with_string!(ctx, gc, str_val, |s| {
            s.iter().all(|&c| {
                char::from_u32(c as u32)
                    .map(|c| c.is_whitespace())
                    .unwrap_or(false)
            })
        }),
    };
    ctx.push_i32(gc, is_null_or_white_space as i32);
    StepResult::Continue
}

#[dotnet_intrinsic_field("static string System.String::Empty")]
pub fn intrinsic_field_string_empty<'gc, 'm: 'gc>(
    ctx: &mut dyn VesOps<'gc, 'm>,
    gc: GCHandle<'gc>,
    _field: FieldDescription,
    _type_generics: Arc<[ConcreteType]>,
    _is_address: bool,
) -> StepResult {
    ctx.push_string(gc, CLRString::new(vec![]));
    StepResult::Continue
}

#[dotnet_intrinsic_field("int System.String::_stringLength")]
pub fn intrinsic_field_string_length<'gc, 'm: 'gc>(
    ctx: &mut dyn VesOps<'gc, 'm>,
    gc: GCHandle<'gc>,
    _field: FieldDescription,
    _type_generics: Arc<[ConcreteType]>,
    is_address: bool,
) -> StepResult {
    if is_address {
        panic!("taking address of _stringLength is not supported");
    }

    let val = ctx.pop(gc);
    let len = with_string!(ctx, gc, val, |s| s.len());

    ctx.push_i32(gc, len as i32);
    StepResult::Continue
}

/// System.String::CopyStringContent(string, int, string)
#[dotnet_intrinsic("static void System.String::CopyStringContent(string, int, string)")]
pub fn intrinsic_string_copy_string_content<'gc, 'm: 'gc>(
    ctx: &mut dyn VesOps<'gc, 'm>,
    gc: GCHandle<'gc>,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let src_val = ctx.pop_obj(gc);
    let dest_pos = ctx.pop_i32(gc);
    let dest_val = ctx.pop_obj(gc);

    let src = with_string!(ctx, gc, StackValue::ObjectRef(src_val), |s| s.to_vec());
    with_string_mut!(ctx, gc, StackValue::ObjectRef(dest_val), |dest| {
        let dest_pos = dest_pos as usize;
        let len = src.len();
        dest.as_mut_slice()[dest_pos..dest_pos + len].copy_from_slice(&src);
    });
    StepResult::Continue
}

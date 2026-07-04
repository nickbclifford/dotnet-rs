use crate::{IntrinsicStringHost, extend_from_utf16_ne_bytes};
use dotnet_macros::dotnet_intrinsic;
use dotnet_types::{TypeDescription, generics::GenericLookup, members::MethodDescription};
use dotnet_value::{
    StackValue,
    object::{HeapStorage, ObjectRef},
    string::CLRString,
    with_vector,
};
use dotnet_vm_ops::{
    StepResult,
    ops::{ExceptionOps, RawMemoryOps, TypedStackOps},
};

fn parse_string_length<'gc, T: TypedStackOps<'gc> + ExceptionOps<'gc> + RawMemoryOps<'gc>>(
    ctx: &mut T,
    method: &MethodDescription,
) -> Result<usize, StepResult> {
    use dotnetdll::prelude::*;

    let params_count = method.signature().parameters.len();
    let len_res = if params_count == 1 {
        let param = &method.signature().parameters[0].1;
        match param {
            ParameterType::Value(MethodType::Base(b))
                if matches!(b.as_ref(), BaseType::IntPtr | BaseType::UIntPtr) =>
            {
                ctx.pop_isize()
            }
            _ => ctx.pop_i32() as isize,
        }
    } else {
        // Overload with length as first param, MethodTable* as second param
        let _p_mt = ctx.pop();
        let param = &method.signature().parameters[0].1;
        match param {
            ParameterType::Value(MethodType::Base(b))
                if matches!(b.as_ref(), BaseType::IntPtr | BaseType::UIntPtr) =>
            {
                ctx.pop_isize()
            }
            _ => ctx.pop_i32() as isize,
        }
    };

    if len_res < 0 {
        return Err(ctx.throw_by_name_with_message(
            "System.OverflowException",
            "Arithmetic operation resulted in an overflow.",
        ));
    }
    let len = len_res as usize;

    // Defensive check: limit string size to 512MB characters
    if len > 0x2000_0000 {
        return Err(ctx.throw_by_name_with_message(
            "System.OutOfMemoryException",
            "Insufficient memory to continue the execution of the program.",
        ));
    }

    // Check GC safe point before allocating large strings
    const LARGE_STRING_THRESHOLD: usize = 1024;
    if len > LARGE_STRING_THRESHOLD && ctx.check_gc_safe_point() {
        return Err(StepResult::Yield);
    }

    Ok(len)
}

/// System.String::FastAllocateString(int)
#[dotnet_intrinsic("static string System.String::FastAllocateString(int)")]
#[dotnet_intrinsic("static string System.String::FastAllocateString(IntPtr)")]
#[dotnet_intrinsic("static string System.String::FastAllocateString(int, IntPtr)")]
#[dotnet_intrinsic("static string System.String::FastAllocateString(IntPtr, IntPtr)")]
pub fn intrinsic_string_fast_allocate_string<
    'gc,
    T: TypedStackOps<'gc> + ExceptionOps<'gc> + RawMemoryOps<'gc>,
>(
    ctx: &mut T,
    method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let len = match parse_string_length(ctx, &method) {
        Ok(len) => len,
        Err(step) => return step,
    };

    let value = CLRString::new(vec![0u16; len]);
    ctx.push_string(value);
    StepResult::Continue
}

/// System.String::.ctor(char[])
#[dotnet_intrinsic("void System.String::.ctor(char[])")]
pub fn intrinsic_string_ctor_char_array<'gc, T: IntrinsicStringHost<'gc>>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let arg = ctx.pop();
    let chars: Vec<u16> = match arg {
        StackValue::ObjectRef(ObjectRef(Some(handle))) => {
            let arg_obj = StackValue::ObjectRef(ObjectRef(Some(handle)));
            if !arg_obj
                .as_object_ref()
                .try_as_heap_storage(|storage| matches!(storage, HeapStorage::Vec(_)))
                .unwrap_or(false)
            {
                Vec::new()
            } else {
                let len = with_vector!(ctx, &arg_obj, |v| v.layout.length);

                let mut result = Vec::with_capacity(len);
                let mut offset = 0;
                const CHUNK_SIZE: usize = 1024;

                while offset < len {
                    let chunk_len = std::cmp::min(CHUNK_SIZE, len - offset);
                    with_vector!(ctx, &arg_obj, |v| {
                        let bytes = v.get();
                        let start = offset * 2;
                        let end = start + (chunk_len * 2);
                        if let Some(chunk_bytes) = bytes.get(start..end) {
                            crate::extend_from_utf16_ne_bytes(&mut result, chunk_bytes);
                        }
                    });
                    offset += chunk_len;
                    if offset < len && ctx.check_gc_safe_point() {
                        return StepResult::Yield;
                    }
                }
                result
            }
        }
        StackValue::ValueType(span) => {
            let len =
                match ctx.string_with_span_data(span.clone(), TypeDescription::NULL, 2, |slice| {
                    slice.len() / 2
                }) {
                    Ok(len) => len,
                    Err(e) => return StepResult::Error(e.into()),
                };

            let mut result = Vec::with_capacity(len);
            let mut offset = 0usize;
            const CHUNK_SIZE: usize = 1024;

            while offset < len {
                let chunk_len = std::cmp::min(CHUNK_SIZE, len - offset);
                let copy_res =
                    ctx.string_with_span_data(span.clone(), TypeDescription::NULL, 2, |slice| {
                        let start = offset * 2;
                        let end = start + (chunk_len * 2);
                        if let Some(chunk_bytes) = slice.get(start..end) {
                            extend_from_utf16_ne_bytes(&mut result, chunk_bytes);
                        }
                    });

                if let Err(e) = copy_res {
                    return StepResult::Error(e.into());
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

/// System.String::.ctor(System.ReadOnlySpan<char>)
#[dotnet_intrinsic("void System.String::.ctor(System.ReadOnlySpan<char>)")]
pub fn intrinsic_string_ctor_readonly_span_char<'gc, T: IntrinsicStringHost<'gc>>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let arg = ctx.pop();
    let chars: Vec<u16> = match arg {
        StackValue::ValueType(span) => {
            let len =
                match ctx.string_with_span_data(span.clone(), TypeDescription::NULL, 2, |slice| {
                    slice.len() / 2
                }) {
                    Ok(len) => len,
                    Err(e) => return StepResult::Error(e.into()),
                };

            let mut result = Vec::with_capacity(len);
            let mut offset = 0usize;
            const CHUNK_SIZE: usize = 1024;

            while offset < len {
                let chunk_len = std::cmp::min(CHUNK_SIZE, len - offset);
                let copy_res =
                    ctx.string_with_span_data(span.clone(), TypeDescription::NULL, 2, |slice| {
                        let start = offset * 2;
                        let end = start + (chunk_len * 2);
                        if let Some(chunk_bytes) = slice.get(start..end) {
                            extend_from_utf16_ne_bytes(&mut result, chunk_bytes);
                        }
                    });

                if let Err(e) = copy_res {
                    return StepResult::Error(e.into());
                }

                offset += chunk_len;
                if offset < len && ctx.check_gc_safe_point() {
                    return StepResult::Yield;
                }
            }

            result
        }
        StackValue::ObjectRef(ObjectRef(None)) => Vec::new(),
        other => {
            return StepResult::type_error("System.ReadOnlySpan<char>", format!("{:?}", other));
        }
    };

    let value = CLRString::new(chars);
    ctx.push_string(value);
    StepResult::Continue
}

/// System.String::.ctor(char*)
#[dotnet_intrinsic("void System.String::.ctor(char*)")]
pub fn intrinsic_string_ctor_char_ptr<'gc, T: TypedStackOps<'gc> + RawMemoryOps<'gc>>(
    ctx: &mut T,
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

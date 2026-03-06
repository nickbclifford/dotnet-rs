use crate::{
    StepResult,
    stack::ops::{
        CallOps, EvalStackOps, ExceptionOps, LoaderOps, MemoryOps, RawMemoryOps, ReflectionOps,
        ResolutionOps, ThreadOps, TypedStackOps,
    },
};
use dotnet_macros::dotnet_intrinsic;
use dotnet_types::{generics::GenericLookup, members::MethodDescription};
use dotnet_utils::{BorrowGuardHandle, NoActiveBorrows};
use dotnet_value::{
    StackValue,
    object::{HeapStorage, ObjectRef},
    string::CLRString,
};

#[allow(dead_code)]
const NULL_REF_MSG: &str = "Object reference not set to an instance of an object.";

/// System.String::FastAllocateString(int)
#[dotnet_intrinsic("static string System.String::FastAllocateString(int)")]
#[dotnet_intrinsic("static string System.String::FastAllocateString(IntPtr)")]
#[dotnet_intrinsic("static string System.String::FastAllocateString(int, IntPtr)")]
#[dotnet_intrinsic("static string System.String::FastAllocateString(IntPtr, IntPtr)")]
pub fn intrinsic_string_fast_allocate_string<
    'gc,
    T: EvalStackOps<'gc>
        + TypedStackOps<'gc>
        + MemoryOps<'gc>
        + CallOps<'gc>
        + ResolutionOps<'gc>
        + RawMemoryOps<'gc>
        + ExceptionOps<'gc>
        + LoaderOps
        + ThreadOps
        + ReflectionOps<'gc>,
>(
    ctx: &mut T,
    method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    use dotnetdll::prelude::*;

    let params_count = method.method().signature.parameters.len();
    let len_res = if params_count == 1 {
        let param = &method.method().signature.parameters[0].1;
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
        let param = &method.method().signature.parameters[0].1;
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
        return ctx.throw_by_name_with_message(
            "System.OverflowException",
            "Arithmetic operation resulted in an overflow.",
        );
    }
    let len = len_res as usize;

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
pub fn intrinsic_string_ctor_char_array<
    'gc,
    T: EvalStackOps<'gc>
        + TypedStackOps<'gc>
        + MemoryOps<'gc>
        + CallOps<'gc>
        + ResolutionOps<'gc>
        + RawMemoryOps<'gc>
        + ExceptionOps<'gc>
        + LoaderOps
        + ThreadOps
        + ReflectionOps<'gc>,
>(
    ctx: &mut T,
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
                    let (_active, _guard) =
                        BorrowGuardHandle::new(ctx.as_borrow_scope(), NoActiveBorrows::new());
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
pub fn intrinsic_string_ctor_char_ptr<
    'gc,
    T: EvalStackOps<'gc>
        + TypedStackOps<'gc>
        + MemoryOps<'gc>
        + CallOps<'gc>
        + ResolutionOps<'gc>
        + RawMemoryOps<'gc>
        + ExceptionOps<'gc>
        + LoaderOps
        + ThreadOps
        + ReflectionOps<'gc>,
>(
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

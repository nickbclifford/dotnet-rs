use crate::{IntrinsicStringHost, extend_from_utf16_ne_bytes};
use dotnet_macros::dotnet_intrinsic;
use dotnet_types::{TypeDescription, generics::GenericLookup, members::MethodDescription};
use dotnet_value::{
    StackValue,
    object::{HeapStorage, Object, ObjectRef},
    pointer::unmanaged_ptr_from_addr,
    string::CLRString,
    with_vector,
};
use dotnet_vm_data::StepResult;
use dotnet_vm_ops::{
    effects::{NativeInvocation, NativeOperandView, NativeResult, NativeSelection, VmEffects},
    ops::{ExceptionOps, RawMemoryOps, TypedStackOps},
};

fn parse_string_length<'gc, T: TypedStackOps<'gc> + ExceptionOps<'gc> + RawMemoryOps<'gc>>(
    ctx: &mut T,
    method: &MethodDescription,
) -> Result<usize, StepResult> {
    use dotnetdll::prelude::*;

    let params_count = method.signature().parameters.len();
    if params_count != 1 {
        // Overload with length as first param, MethodTable* as second param
        let _p_mt = ctx.pop();
    }

    let param = &method.signature().parameters[0].1;
    let len_res = match param {
        ParameterType::Value(MethodType::Base(b))
            if matches!(b.as_ref(), BaseType::IntPtr | BaseType::UIntPtr) =>
        {
            ctx.pop_isize()
        }
        _ => ctx.pop_i32() as isize,
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

    Ok(len)
}

fn readonly_span_to_vec<'gc, T: IntrinsicStringHost<'gc>>(
    ctx: &mut T,
    span: Object<'gc>,
) -> Result<Vec<u16>, StepResult> {
    let len = ctx
        .string_with_span_data(span.clone(), TypeDescription::NULL, 2, |slice| {
            slice.len() / 2
        })
        .map_err(|e| StepResult::Error(e.into()))?;

    let mut result = Vec::with_capacity(len);
    let mut offset = 0usize;
    const CHUNK_SIZE: usize = 1024;

    while offset < len {
        let chunk_len = std::cmp::min(CHUNK_SIZE, len - offset);
        ctx.string_with_span_data(span.clone(), TypeDescription::NULL, 2, |slice| {
            let start = offset * 2;
            let end = start + (chunk_len * 2);
            if let Some(chunk_bytes) = slice.get(start..end) {
                extend_from_utf16_ne_bytes(&mut result, chunk_bytes);
            }
        })
        .map_err(|e| StepResult::Error(e.into()))?;

        // No collection poll: the span operand is a ref struct, which native transport
        // excludes, and a yield would retry the constructor from its start.
        offset += chunk_len;
    }

    Ok(result)
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

/// Select large arrays; spans and invalid inputs retain the existing validation path.
pub fn string_array_select<'gc, T: IntrinsicStringHost<'gc>>(
    _ctx: &T,
    operands: NativeOperandView<'_, 'gc>,
    _method: &MethodDescription,
    _generics: &GenericLookup,
) -> NativeSelection {
    if let StackValue::ObjectRef(ObjectRef(Some(handle))) = operands.peek_at(0) {
        let inner = handle.borrow();
        if let HeapStorage::Vec(vector) = &inner.storage
            && vector.layout.length > REPEAT_CHUNK_SIZE
        {
            return NativeSelection::Async(NativeInvocation {
                start: 0,
                length: vector.layout.length,
                parameter_count: 1,
            });
        }
    }
    NativeSelection::Fallback
}

/// System.String::.ctor(char[])
#[dotnet_intrinsic("void System.String::.ctor(char[])", select = string_array_select, fallback = intrinsic_string_ctor_char_array)]
pub async fn string_array_async(mut fx: VmEffects) -> NativeResult {
    let length = fx.invocation().length;
    let source = fx.vm().await?.pop_object()?;
    let mut chars = Vec::with_capacity(length);
    while chars.len() < length {
        {
            let mut vm = fx.vm().await?;
            vm.with(|region| {
                let StackValue::ObjectRef(ObjectRef(Some(handle))) = region.fetch(&source)? else {
                    return Err(dotnet_types::error::ExecutionError::InternalError(
                        "native string array source invalid".into(),
                    )
                    .into());
                };
                let inner = handle.borrow();
                let HeapStorage::Vec(vector) = &inner.storage else {
                    return Err(dotnet_types::error::ExecutionError::InternalError(
                        "native string array source invalid".into(),
                    )
                    .into());
                };
                let end = (chars.len() + REPEAT_CHUNK_SIZE).min(length);
                if let Some(bytes) = vector.get().get(chars.len() * 2..end * 2) {
                    extend_from_utf16_ne_bytes(&mut chars, bytes);
                } else {
                    return Err(dotnet_types::error::ExecutionError::InternalError(
                        "native string array is shorter than its length".into(),
                    )
                    .into());
                }
                Ok::<(), dotnet_types::error::VmError>(())
            })??;
        }
        if chars.len() < length {
            fx.yield_now().await?;
        }
    }
    fx.vm().await?.push_new_string(chars)
}

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
                            extend_from_utf16_ne_bytes(&mut result, chunk_bytes);
                        }
                    });
                    offset += chunk_len;
                }
                result
            }
        }
        StackValue::ValueType(span) => match readonly_span_to_vec(ctx, span) {
            Ok(chars) => chars,
            Err(step) => return step,
        },
        _ => Vec::new(), // null or invalid
    };

    let value = CLRString::new(chars);
    ctx.push_string(value);
    StepResult::Continue
}

/// `System.String::.ctor(char, int)` builds a string of `count` copies of `c`.
///
/// ECMA-335 does not define this BCL constructor; the CLR throws
/// `ArgumentOutOfRangeException` for a negative count.
const REPEAT_CHUNK_SIZE: usize = dotnet_vm_ops::effects::BULK_CHUNK_SIZE / 2;

pub fn string_repeat_select<'gc, T: TypedStackOps<'gc> + ExceptionOps<'gc> + RawMemoryOps<'gc>>(
    _ctx: &T,
    operands: NativeOperandView<'_, 'gc>,
    _method: &MethodDescription,
    _generics: &GenericLookup,
) -> NativeSelection {
    if let (StackValue::Int32(count), StackValue::Int32(c)) =
        (operands.peek_at(0), operands.peek_at(1))
        && count > REPEAT_CHUNK_SIZE as i32
        && count <= 0x2000_0000
    {
        NativeSelection::Async(NativeInvocation {
            start: c as u16 as usize,
            length: count as usize,
            parameter_count: 2,
        })
    } else {
        NativeSelection::Fallback
    }
}

#[dotnet_intrinsic("void System.String::.ctor(char, int)", select = string_repeat_select, fallback = intrinsic_string_ctor_char_count)]
pub async fn string_repeat_async(mut fx: VmEffects) -> NativeResult {
    let invocation = fx.invocation();
    fx.vm().await?.discard_scalars(2)?;
    let mut chars = Vec::with_capacity(invocation.length);
    while chars.len() < invocation.length {
        let end = (chars.len() + REPEAT_CHUNK_SIZE).min(invocation.length);
        chars.resize(end, invocation.start as u16);
        if end < invocation.length {
            fx.yield_now().await?;
        }
    }
    fx.vm().await?.push_new_string(chars)
}

pub fn intrinsic_string_ctor_char_count<
    'gc,
    T: TypedStackOps<'gc> + ExceptionOps<'gc> + RawMemoryOps<'gc>,
>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let count = ctx.pop_i32();
    let c = ctx.pop_i32() as u16;

    if count < 0 {
        return ctx.throw_by_name_with_message(
            "System.ArgumentOutOfRangeException",
            "Count cannot be less than zero. (Parameter 'count')",
        );
    }
    let len = count as usize;
    // Same size cap as `FastAllocateString`.
    if len > 0x2000_0000 {
        return ctx.throw_by_name_with_message(
            "System.OutOfMemoryException",
            "Insufficient memory to continue the execution of the program.",
        );
    }

    ctx.push_string(CLRString::new(vec![c; len]));
    StepResult::Continue
}

/// `System.String::.ctor(System.ReadOnlySpan<char>)`
#[dotnet_intrinsic("void System.String::.ctor(System.ReadOnlySpan<char>)")]
pub fn intrinsic_string_ctor_readonly_span_char<'gc, T: IntrinsicStringHost<'gc>>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let arg = ctx.pop();
    let chars: Vec<u16> = match arg {
        StackValue::ValueType(span) => match readonly_span_to_vec(ctx, span) {
            Ok(chars) => chars,
            Err(step) => return step,
        },
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
    let address = match arg {
        StackValue::NativeInt(p) => Some(p as usize),
        StackValue::Int32(p) => Some(p as usize),
        StackValue::Int64(p) => Some(p as usize),
        _ => None,
    };
    // SAFETY: F10.RawMemoryAccessValid — The native `String(char*)` constructor contract requires a live,
    // null-terminated unmanaged UTF-16 address; the loop below owns that proof.
    let ptr = address.map_or(std::ptr::null(), |address| unsafe {
        unmanaged_ptr_from_addr(address).cast_const().cast::<u16>()
    });

    let value = if !ptr.is_null() {
        let mut chars = Vec::new();
        let mut i = 0;
        #[expect(
            clippy::multiple_unsafe_ops_per_block,
            reason = "walking the caller-provided C string requires pointer addition and dereference"
        )]
        // SAFETY: F10.RawMemoryAccessValid — The CLR pointer overload contract supplies a valid, null-terminated u16 string;
        // each iteration remains within that string until the terminating zero is observed.
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

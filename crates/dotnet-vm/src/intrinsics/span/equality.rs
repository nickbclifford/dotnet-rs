use crate::{
    StepResult,
    error::ExecutionError,
    intrinsics::span::helpers::*,
    layout::type_layout,
    stack::ops::{
        CallOps, EvalStackOps, ExceptionOps, LoaderOps, MemoryOps, RawMemoryOps, ReflectionOps,
        ResolutionOps, TypedStackOps,
    },
};
use dotnet_macros::dotnet_intrinsic;
use dotnet_types::{TypeDescription, generics::GenericLookup, members::MethodDescription};
use dotnet_value::{
    layout::{HasLayout, LayoutManager, Scalar},
    pointer::ManagedPtr,
};

fn chunked_sequence_equal<
    'gc,
    T: EvalStackOps<'gc>
        + TypedStackOps<'gc>
        + ResolutionOps<'gc>
        + LoaderOps
        + ExceptionOps<'gc>
        + RawMemoryOps<'gc>
        + MemoryOps<'gc>
        + ReflectionOps<'gc>,
>(
    ctx: &mut T,
    a: &ManagedPtr<'gc>,
    b: &ManagedPtr<'gc>,
    total_bytes: usize,
) -> bool {
    let mut offset = 0;
    const CHUNK_SIZE: usize = 1024 * 1024; // 1MB chunks

    while offset < total_bytes {
        let current_chunk = std::cmp::min(CHUNK_SIZE, total_bytes - offset);

        let a_chunk = unsafe { a.clone().offset(offset as isize) };
        let b_chunk = unsafe { b.clone().offset(offset as isize) };

        let res = unsafe {
            a_chunk.with_data(current_chunk, |a_slice| {
                b_chunk.with_data(current_chunk, |b_slice| a_slice == b_slice)
            })
        };

        if !res {
            return false;
        }

        offset += current_chunk;
        if offset < total_bytes {
            let _ = ctx.check_gc_safe_point();
        }
    }

    true
}

#[dotnet_intrinsic(
    "static bool System.MemoryExtensions::SequenceEqual<T>(System.ReadOnlySpan<T>, System.ReadOnlySpan<T>)"
)]
pub fn intrinsic_memory_extensions_sequence_equal<
    'gc,
    T: EvalStackOps<'gc>
        + TypedStackOps<'gc>
        + ResolutionOps<'gc>
        + LoaderOps
        + ExceptionOps<'gc>
        + RawMemoryOps<'gc>
        + MemoryOps<'gc>
        + ReflectionOps<'gc>
        + CallOps<'gc>,
>(
    ctx: &mut T,
    _method: MethodDescription,
    generics: &GenericLookup,
) -> StepResult {
    let b_val = ctx.peek_stack();
    let a_val = ctx.peek_stack_at(1);
    let b = b_val.as_value_type();
    let a = a_val.as_value_type();

    let element_type = &generics.method_generics[0];

    // Check length
    let a_len = match read_span_length(&a) {
        Ok(l) => l,
        Err(e) => return StepResult::Error(e.into()),
    };
    let b_len = match read_span_length(&b) {
        Ok(l) => l,
        Err(e) => return StepResult::Error(e.into()),
    };

    if a_len != b_len {
        ctx.pop_multiple(2);
        ctx.push_i32(0);
        return StepResult::Continue;
    }

    if a_len == 0 {
        ctx.pop_multiple(2);
        ctx.push_i32(1);
        return StepResult::Continue;
    }

    let res_ctx = ctx.with_generics(generics);
    let layout = vm_try!(type_layout(element_type.clone(), &res_ctx));

    let is_bitwise = match &*layout {
        LayoutManager::Scalar(s) => !matches!(
            s,
            Scalar::ObjectRef | Scalar::ManagedPtr | Scalar::Float32 | Scalar::Float64
        ),
        _ => false,
    };

    if is_bitwise {
        let element_size = layout.size().as_usize();
        let element_desc = vm_try!(ctx.loader().find_concrete_type(element_type.clone()));

        let a_ptr_info = match read_span_reference(&a) {
            Ok(info) => info,
            Err(e) => {
                return StepResult::Error(e.into());
            }
        };
        let b_ptr_info = match read_span_reference(&b) {
            Ok(info) => info,
            Err(e) => {
                return StepResult::Error(e.into());
            }
        };

        let a_mptr = ManagedPtr::from_info_full(a_ptr_info, element_desc, false);
        let b_mptr = ManagedPtr::from_info_full(b_ptr_info, element_desc, false);

        let total_bytes = a_len as usize * element_size;
        let equal = chunked_sequence_equal(ctx, &a_mptr, &b_mptr, total_bytes);

        ctx.pop_multiple(2);
        ctx.push_i32(equal as i32);
        StepResult::Continue
    } else {
        // Slow path: dispatch to MemoryExtensions.SequenceEqualSlowPath<T>
        // Arguments (a, b) are already on the stack from the peek earlier
        let loader = ctx.loader();
        let memory_extensions_type = vm_try!(loader.corlib_type("System.MemoryExtensions"));
        let def = memory_extensions_type.definition();

        let (_method_idx, method_def) = match def
            .methods
            .iter()
            .enumerate()
            .find(|(_, m)| m.name == "SequenceEqualSlowPath")
        {
            Some(m) => m,
            None => {
                return StepResult::Error(
                    ExecutionError::InternalError("SequenceEqualSlowPath not found".to_string())
                        .into(),
                );
            }
        };

        let slow_path_method = MethodDescription::new(
            memory_extensions_type,
            memory_extensions_type.resolution,
            method_def,
        );

        ctx.dispatch_method(slow_path_method, generics.clone())
    }
}

#[dotnet_intrinsic(
    "static bool System.MemoryExtensions::Equals(System.ReadOnlySpan<char>, System.ReadOnlySpan<char>, System.StringComparison)"
)]
pub fn intrinsic_memory_extensions_equals_span_char<
    'gc,
    T: EvalStackOps<'gc>
        + TypedStackOps<'gc>
        + ResolutionOps<'gc>
        + LoaderOps
        + ExceptionOps<'gc>
        + RawMemoryOps<'gc>
        + MemoryOps<'gc>
        + ReflectionOps<'gc>,
>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let comparison_type = ctx.pop_i32();
    let b = ctx.pop_value_type();
    let a = ctx.pop_value_type();

    // TODO: Support non-ordinal comparison types if needed
    // In many .NET versions, Ordinal is 4. OrdinalIgnoreCase is 5.
    if comparison_type != 4 && comparison_type != 5 {
        // Fallback to ordinal for now but log a warning if we had a tracer
    }

    let a_len = match read_span_length(&a) {
        Ok(l) => l,
        Err(e) => return StepResult::Error(e.into()),
    };
    let b_len = match read_span_length(&b) {
        Ok(l) => l,
        Err(e) => return StepResult::Error(e.into()),
    };

    if a_len != b_len {
        ctx.push_i32(0);
        return StepResult::Continue;
    }

    if a_len == 0 {
        ctx.push_i32(1);
        return StepResult::Continue;
    }

    let a_ptr_info = match read_span_reference(&a) {
        Ok(info) => info,
        Err(e) => return StepResult::Error(e.into()),
    };
    let b_ptr_info = match read_span_reference(&b) {
        Ok(info) => info,
        Err(e) => return StepResult::Error(e.into()),
    };

    let a_mptr = ManagedPtr::from_info_full(a_ptr_info, TypeDescription::NULL, false);
    let b_mptr = ManagedPtr::from_info_full(b_ptr_info, TypeDescription::NULL, false);

    let total_bytes = a_len as usize * 2; // char is 2 bytes
    let equal = chunked_sequence_equal(ctx, &a_mptr, &b_mptr, total_bytes);

    ctx.push_i32(equal as i32);
    StepResult::Continue
}

#[dotnet_intrinsic("static bool System.SpanHelpers::SequenceEqual(ref byte, ref byte, nuint)")]
pub fn intrinsic_span_helpers_sequence_equal<
    'gc,
    T: EvalStackOps<'gc>
        + TypedStackOps<'gc>
        + ResolutionOps<'gc>
        + LoaderOps
        + ExceptionOps<'gc>
        + RawMemoryOps<'gc>
        + MemoryOps<'gc>
        + ReflectionOps<'gc>,
>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let length = ctx.pop_isize() as usize;
    let b = ctx.pop_managed_ptr();
    let a = ctx.pop_managed_ptr();

    if length == 0 {
        ctx.push_i32(1);
        return StepResult::Continue;
    }

    if a.is_null() || b.is_null() {
        // According to ECMA-335, this shouldn't happen if length > 0 for spans,
        // but we handle it defensively.
        ctx.push_i32(if a.is_null() && b.is_null() { 1 } else { 0 });
        return StepResult::Continue;
    }

    let equal = chunked_sequence_equal(ctx, &a, &b, length);

    ctx.push_i32(equal as i32);
    StepResult::Continue
}

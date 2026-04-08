use crate::{SpanIntrinsicHost, helpers::*, vm_try};
use dotnet_macros::dotnet_intrinsic;
use dotnet_types::{
    TypeDescription,
    generics::{ConcreteType, GenericLookup},
    members::MethodDescription,
};
use dotnet_value::{
    StackValue,
    layout::{HasLayout, LayoutManager, Scalar},
    pointer::ManagedPtr,
};
use dotnet_vm_ops::StepResult;
use dotnetdll::prelude::MethodMemberIndex;

fn chunked_sequence_equal<'gc, T: SpanIntrinsicHost<'gc>>(
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
pub fn intrinsic_memory_extensions_sequence_equal<'gc, T: SpanIntrinsicHost<'gc>>(
    ctx: &mut T,
    _method: MethodDescription,
    generics: &GenericLookup,
) -> StepResult {
    let b_val = ctx.peek_stack();
    let a_val = ctx.peek_stack_at(1);
    let (a, b) = match (&a_val, &b_val) {
        (StackValue::ValueType(a), StackValue::ValueType(b)) => (a.clone(), b.clone()),
        // Some runtime packs call SequenceEqualSlowPath with already-projected values
        // instead of ReadOnlySpan<T>. In that case, compare the values directly.
        _ => {
            ctx.pop_multiple(2);
            ctx.push_i32((a_val == b_val) as i32);
            return StepResult::Continue;
        }
    };

    let element_type = &generics.method_generics[0];

    // Check length
    let a_len = match read_span_length(&a) {
        Ok(l) => l,
        Err(_) => {
            ctx.pop_multiple(2);
            ctx.push_i32((a_val == b_val) as i32);
            return StepResult::Continue;
        }
    };
    let b_len = match read_span_length(&b) {
        Ok(l) => l,
        Err(_) => {
            ctx.pop_multiple(2);
            ctx.push_i32((a_val == b_val) as i32);
            return StepResult::Continue;
        }
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

    let layout = vm_try!(ctx.span_type_layout(element_type.clone()));

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

        let a_mptr = ManagedPtr::from_info_full(a_ptr_info, element_desc.clone(), false);
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

        let (method_idx, _) = match def
            .methods
            .iter()
            .enumerate()
            .find(|(_, m)| m.name == "SequenceEqualSlowPath")
        {
            Some(m) => m,
            None => {
                return StepResult::internal_error("SequenceEqualSlowPath not found");
            }
        };

        let slow_path_method = MethodDescription::new(
            memory_extensions_type.clone(),
            GenericLookup::default(),
            memory_extensions_type.resolution.clone(),
            MethodMemberIndex::Method(method_idx),
        );

        ctx.span_dispatch_method(slow_path_method, generics.clone())
    }
}

#[dotnet_intrinsic("int System.Span<T>::get_Length()")]
#[dotnet_intrinsic("int System.ReadOnlySpan<T>::get_Length()")]
pub fn intrinsic_span_get_length<'gc, T: SpanIntrinsicHost<'gc>>(
    ctx: &mut T,
    method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let this = ctx.pop();
    let len = match this {
        StackValue::ValueType(span) => match read_span_length(&span) {
            Ok(l) => l,
            Err(e) => return StepResult::Error(e.into()),
        },
        StackValue::ManagedPtr(span_ptr) => {
            let span_concrete: ConcreteType = method.parent.clone().into();
            let span_layout = vm_try!(ctx.span_type_layout(span_concrete));
            let field_layout = match &*span_layout {
                LayoutManager::Field(f) => f,
                _ => {
                    return StepResult::type_error(
                        "Span/ReadOnlySpan field layout",
                        format!("{:?}", span_layout),
                    );
                }
            };

            match read_span_length_from_ptr(&span_ptr, field_layout, ctx) {
                Ok(l) => l,
                Err(e) => return StepResult::Error(e.into()),
            }
        }
        other => {
            return StepResult::type_error("ValueType or ManagedPtr", format!("{:?}", other));
        }
    };

    ctx.push_i32(len);
    StepResult::Continue
}

#[dotnet_intrinsic("T& System.Span<T>::get_Item(int)")]
#[dotnet_intrinsic("T& System.ReadOnlySpan<T>::get_Item(int)")]
pub fn intrinsic_span_get_item<'gc, T: SpanIntrinsicHost<'gc>>(
    ctx: &mut T,
    method: MethodDescription,
    generics: &GenericLookup,
) -> StepResult {
    let index = ctx.pop_i32();
    let this = ctx.pop();

    let Some(element_type) = generics.type_generics.first().cloned() else {
        return StepResult::internal_error("Span.get_Item missing type generic parameter");
    };

    let span_concrete: ConcreteType = method.parent.clone().into();
    let span_layout = vm_try!(ctx.span_type_layout(span_concrete));
    let element_layout = vm_try!(ctx.span_type_layout(element_type.clone()));
    let element_size = element_layout.size().as_usize();
    let element_desc = vm_try!(ctx.loader().find_concrete_type(element_type));

    let (base_ptr, len) = match this {
        StackValue::ValueType(span) => {
            let len = match read_span_length(&span) {
                Ok(v) => v,
                Err(e) => return StepResult::Error(e.into()),
            };
            let info = match read_span_reference(&span) {
                Ok(v) => v,
                Err(e) => return StepResult::Error(e.into()),
            };
            (
                ManagedPtr::from_info_full(info, element_desc.clone(), false),
                len,
            )
        }
        StackValue::ManagedPtr(span_ptr) => {
            let field_layout = match &*span_layout {
                LayoutManager::Field(f) => f,
                _ => {
                    return StepResult::type_error(
                        "Span/ReadOnlySpan field layout",
                        format!("{:?}", span_layout),
                    );
                }
            };

            let len = match read_span_length_from_ptr(&span_ptr, field_layout, ctx) {
                Ok(v) => v,
                Err(e) => return StepResult::Error(e.into()),
            };
            let raw_ref_ptr = match read_span_reference_from_ptr(&span_ptr, field_layout, ctx) {
                Ok(v) => v,
                Err(e) => return StepResult::Error(e.into()),
            };
            (
                ManagedPtr::from_info_full(raw_ref_ptr.into_info(), element_desc.clone(), false),
                len,
            )
        }
        other => {
            return StepResult::type_error("ValueType or ManagedPtr", format!("{:?}", other));
        }
    };

    if index < 0 || index >= len {
        return ctx.throw_by_name_with_message(
            "System.IndexOutOfRangeException",
            "Index was outside the bounds of the span.",
        );
    }

    let ptr = unsafe { base_ptr.offset((index as usize * element_size) as isize) };
    ctx.push(StackValue::ManagedPtr(ptr.into()));
    StepResult::Continue
}

#[dotnet_intrinsic(
    "static bool System.MemoryExtensions::Equals(System.ReadOnlySpan<char>, System.ReadOnlySpan<char>, System.StringComparison)"
)]
pub fn intrinsic_memory_extensions_equals_span_char<'gc, T: SpanIntrinsicHost<'gc>>(
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
pub fn intrinsic_span_helpers_sequence_equal<'gc, T: SpanIntrinsicHost<'gc>>(
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

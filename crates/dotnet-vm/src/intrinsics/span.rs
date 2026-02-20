use crate::{StepResult, layout::type_layout, resolution::ValueResolution, stack::ops::VesOps};
use dotnet_types::{
    TypeDescription,
    generics::{ConcreteType, GenericLookup},
    members::{FieldDescription, MethodDescription},
};
use dotnet_utils::BorrowGuard;
use dotnet_value::{
    StackValue,
    layout::{FieldLayoutManager, HasLayout, LayoutManager, Scalar},
    object::{HeapStorage, Object, ObjectRef},
    pointer::{ManagedPtr, ManagedPtrInfo},
};
use dotnetdll::prelude::ParameterType;
use std::ptr::NonNull;

/// Read the _reference ManagedPtr from a Span/ReadOnlySpan value type.
pub fn read_span_reference<'gc>(span: &Object<'gc>) -> Result<ManagedPtrInfo<'gc>, String> {
    if !span
        .instance_storage
        .has_field(span.description, "_reference")
    {
        return Err("Span must have _reference field".to_string());
    }
    let ptr_data = span
        .instance_storage
        .get_field_local(span.description, "_reference");
    unsafe { ManagedPtr::read_unchecked(&ptr_data) }
        .map_err(|e| format!("Failed to deserialize span _reference: {:?}", e))
}

/// Read the _length from a Span/ReadOnlySpan value type.
pub fn read_span_length(span: &Object) -> Result<i32, String> {
    if !span.instance_storage.has_field(span.description, "_length") {
        return Err("Span must have _length field".to_string());
    }
    let len_data = span
        .instance_storage
        .get_field_local(span.description, "_length");
    Ok(i32::from_ne_bytes(len_data[..4].try_into().map_err(
        |_| "Failed to convert span length bytes".to_string(),
    )?))
}

/// Read the _reference ManagedPtr from a ManagedPtr that points to a Span.
pub fn read_span_reference_from_ptr<'gc, 'm>(
    span_ptr: &ManagedPtr<'gc>,
    layout: &FieldLayoutManager,
    ctx: &dyn VesOps<'gc, 'm>,
) -> Result<ManagedPtr<'gc>, String> {
    let ref_field = layout
        .get_field_by_name("_reference")
        .ok_or_else(|| "Span must have _reference field".to_string())?;

    // Read the raw bytes of the _reference field and deserialize properly.
    // We MUST NOT use ctx.read_unaligned with span_ptr.origin because that would
    // incorrectly tag the ManagedPtr with the Span's origin (e.g., Stack) instead of
    // the actual origin stored in the serialized bytes (e.g., Heap).
    let mut ptr_bytes = ManagedPtr::serialization_buffer();
    unsafe {
        ctx.read_bytes(
            span_ptr.origin.clone(),
            span_ptr.offset + ref_field.position,
            &mut ptr_bytes,
        )
    }
    .map_err(|e| format!("Failed to read span _reference bytes: {}", e))?;

    // Deserialize the ManagedPtrInfo from bytes
    let info = unsafe { ManagedPtr::read_branded(&ptr_bytes, &ctx.gc()) }
        .map_err(|e| format!("Failed to deserialize span _reference: {:?}", e))?;

    // Reconstruct with proper type - use NULL for now, caller can adjust if needed
    Ok(ManagedPtr::from_info_full(
        info,
        TypeDescription::NULL,
        false,
    ))
}

/// Read the _length from a ManagedPtr that points to a Span.
pub fn read_span_length_from_ptr<'gc, 'm>(
    span_ptr: &ManagedPtr<'gc>,
    layout: &FieldLayoutManager,
    ctx: &dyn VesOps<'gc, 'm>,
) -> Result<i32, String> {
    let length_field = layout
        .get_field_by_name("_length")
        .ok_or_else(|| "Span must have _length field".to_string())?;
    let val = unsafe {
        ctx.read_unaligned(
            span_ptr.origin.clone(),
            span_ptr.offset + length_field.position,
            &length_field.layout,
            None,
        )
        .map_err(|e| format!("Failed to read _length: {}", e))?
    };
    Ok(val.as_i32())
}

/// Write a ManagedPtr + length into a Span/ReadOnlySpan value type.
pub fn write_span_fields<'gc, 'm>(
    span_ptr: &ManagedPtr<'gc>,
    managed: &ManagedPtr<'gc>,
    length: i32,
    layout: &FieldLayoutManager,
    ctx: &mut dyn VesOps<'gc, 'm>,
) -> Result<(), String> {
    let ref_field = layout
        .get_field_by_name("_reference")
        .ok_or_else(|| "Span must have _reference field".to_string())?;
    let length_field = layout
        .get_field_by_name("_length")
        .ok_or_else(|| "Span must have _length field".to_string())?;

    // Write _reference
    unsafe {
        ctx.write_unaligned(
            span_ptr.origin.clone(),
            span_ptr.offset + ref_field.position,
            StackValue::ManagedPtr(managed.clone()),
            &ref_field.layout,
        )
    }
    .map_err(|e| format!("Failed to write _reference: {}", e))?;

    // Write _length
    unsafe {
        ctx.write_unaligned(
            span_ptr.origin.clone(),
            span_ptr.offset + length_field.position,
            StackValue::Int32(length),
            &length_field.layout,
        )
    }
    .map_err(|e| format!("Failed to write _length: {}", e))?;

    Ok(())
}

pub fn with_span_data<'gc, 'm, R>(
    ctx: &dyn VesOps<'gc, 'm>,
    span: Object,
    element_type: TypeDescription,
    element_size: usize,
    f: impl FnOnce(&[u8]) -> R,
) -> Result<R, String> {
    let _borrow = BorrowGuard::new(ctx);
    let info = read_span_reference(&span)?;
    let len = read_span_length(&span)? as usize;

    if len == 0 {
        return Ok(f(&[]));
    }

    let m_ptr = ManagedPtr::from_info_full(info, element_type, false);
    let total_size = len * element_size;

    // Bounds checking
    if let Some(owner) = m_ptr.owner() {
        let handle = owner
            .0
            .ok_or_else(|| "ManagedPtr::with_data: null owner handle".to_string())?;
        let inner = handle.borrow();

        match &inner.storage {
            HeapStorage::Vec(v) => {
                // Vector storage is external (on Rust heap), so we must check absolute addresses
                // rather than offsets relative to the object header.
                let data_ptr = unsafe { v.raw_data_ptr() } as usize;
                let elem_size = v.layout.element_layout.size().as_usize();
                let data_len = v.layout.length * elem_size;

                // Calculate pointer value from owner + offset
                let ptr_val = data_ptr + m_ptr.offset.as_usize();

                // Check if the range [ptr_val, ptr_val + total_size) is within [data_ptr, data_ptr + data_len)
                if ptr_val < data_ptr || ptr_val + total_size > data_ptr + data_len {
                    return Err(format!(
                        "Span bounds violation (Vector): ptr {:#x} + size {} not within buffer {:#x}..{:#x}",
                        ptr_val,
                        total_size,
                        data_ptr,
                        data_ptr + data_len
                    ));
                }
            }
            _ => {
                // Inline storage (Object/Boxed): check offset relative to object header
                let obj_size = inner.storage.size_bytes();
                if m_ptr.offset.as_usize() + total_size > obj_size {
                    return Err(format!(
                        "Span bounds violation: offset {} + size {} > owner size {}",
                        m_ptr.offset.as_usize(),
                        total_size,
                        obj_size
                    ));
                }
            }
        }
    } else if let dotnet_value::pointer::PointerOrigin::Transient(obj) = &m_ptr.origin {
        let mut bounds_error = None;
        obj.with_data(|data| {
            if m_ptr.offset.as_usize() + total_size > data.len() {
                bounds_error = Some(format!(
                    "Span bounds violation (transient): offset {} + size {} > owner size {}",
                    m_ptr.offset.as_usize(),
                    total_size,
                    data.len()
                ));
            }
        });
        if let Some(e) = bounds_error {
            return Err(e);
        }
    }

    Ok(unsafe { m_ptr.with_data(total_size, f) })
}

fn chunked_sequence_equal<'gc, 'm>(
    ctx: &mut dyn VesOps<'gc, 'm>,
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
            ctx.check_gc_safe_point();
        }
    }

    true
}

use dotnet_macros::dotnet_intrinsic;

#[dotnet_intrinsic("void System.Span<T>::.ctor(void*, int)")]
#[dotnet_intrinsic("void System.ReadOnlySpan<T>::.ctor(void*, int)")]
pub fn intrinsic_span_ctor_from_pointer<'gc, 'm: 'gc>(
    ctx: &mut dyn VesOps<'gc, 'm>,
    _method: MethodDescription,
    generics: &GenericLookup,
) -> StepResult {
    let length = ctx.pop_i32();
    let ptr_val = ctx.pop();
    let this_ptr = ctx.pop_managed_ptr();

    if length < 0 {
        return ctx.throw_by_name("System.ArgumentOutOfRangeException");
    }

    let element_type = &generics.type_generics[0];
    let element_desc = vm_try!(ctx.loader().find_concrete_type(element_type.clone()));

    let res_ctx = ctx.current_context();
    let element_layout = vm_try!(type_layout(element_type.clone(), &res_ctx));

    // Span<T>(void*, int) is only valid for types that do not contain references
    if element_layout.is_or_contains_refs() {
        return ctx.throw_by_name("System.ArgumentException");
    }

    // The pointer can be a NativeInt, ManagedPtr, or UnmanagedPtr
    let managed = match ptr_val {
        StackValue::NativeInt(ptr) => {
            if ptr == 0 && length > 0 {
                return ctx.throw_by_name("System.ArgumentOutOfRangeException");
            }
            // Create a ManagedPtr from the native pointer
            ManagedPtr::new(
                NonNull::new(ptr as *mut u8),
                element_desc,
                None, // unmanaged pointer
                false,
                None,
            )
        }
        StackValue::UnmanagedPtr(dotnet_value::pointer::UnmanagedPtr(ptr)) => {
            // Create a ManagedPtr from the unmanaged pointer
            ManagedPtr::new(
                Some(ptr),
                element_desc,
                None, // unmanaged pointer
                false,
                None,
            )
        }
        StackValue::ManagedPtr(m) => {
            if m.is_null() && length > 0 {
                return ctx.throw_by_name("System.ArgumentOutOfRangeException");
            }
            // Already a managed pointer, just use it
            m
        }
        StackValue::ObjectRef(ObjectRef(None)) => {
            if length > 0 {
                return ctx.throw_by_name("System.ArgumentOutOfRangeException");
            }
            ManagedPtr::new(None, element_desc, None, false, None)
        }
        _ => return ctx.throw_by_name("System.ArgumentException"),
    };

    let span_type = this_ptr.inner_type;
    let res_ctx = ctx.current_context();
    let layout = vm_try!(type_layout(ConcreteType::from(span_type), &res_ctx));

    let LayoutManager::Field(f) = &*layout else {
        return StepResult::Error(
            crate::error::ExecutionError::NotImplemented(
                "Expected Field layout for Span".to_string(),
            )
            .into(),
        );
    };

    if let Err(e) = write_span_fields(&this_ptr, &managed, length, f, ctx) {
        return StepResult::Error(crate::error::ExecutionError::NotImplemented(e).into());
    }

    StepResult::Continue
}

#[dotnet_intrinsic(
    "static bool System.MemoryExtensions::SequenceEqual<T>(System.ReadOnlySpan<T>, System.ReadOnlySpan<T>)"
)]
pub fn intrinsic_memory_extensions_sequence_equal<'gc, 'm: 'gc>(
    ctx: &mut dyn VesOps<'gc, 'm>,
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
        Err(e) => return StepResult::Error(crate::error::ExecutionError::InternalError(e).into()),
    };
    let b_len = match read_span_length(&b) {
        Ok(l) => l,
        Err(e) => return StepResult::Error(crate::error::ExecutionError::InternalError(e).into()),
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
                return StepResult::Error(crate::error::ExecutionError::InternalError(e).into());
            }
        };
        let b_ptr_info = match read_span_reference(&b) {
            Ok(info) => info,
            Err(e) => {
                return StepResult::Error(crate::error::ExecutionError::InternalError(e).into());
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
                    crate::error::ExecutionError::InternalError(
                        "SequenceEqualSlowPath not found".to_string(),
                    )
                    .into(),
                );
            }
        };

        let slow_path_method = MethodDescription {
            parent: memory_extensions_type,
            method_resolution: memory_extensions_type.resolution,
            method: method_def,
        };

        ctx.dispatch_method(slow_path_method, generics.clone())
    }
}

#[dotnet_intrinsic(
    "static bool System.MemoryExtensions::Equals(System.ReadOnlySpan<char>, System.ReadOnlySpan<char>, System.StringComparison)"
)]
pub fn intrinsic_memory_extensions_equals_span_char<'gc, 'm: 'gc>(
    ctx: &mut dyn VesOps<'gc, 'm>,
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
        Err(e) => return StepResult::Error(crate::error::ExecutionError::InternalError(e).into()),
    };
    let b_len = match read_span_length(&b) {
        Ok(l) => l,
        Err(e) => return StepResult::Error(crate::error::ExecutionError::InternalError(e).into()),
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
        Err(e) => return StepResult::Error(crate::error::ExecutionError::InternalError(e).into()),
    };
    let b_ptr_info = match read_span_reference(&b) {
        Ok(info) => info,
        Err(e) => return StepResult::Error(crate::error::ExecutionError::InternalError(e).into()),
    };

    let a_mptr = ManagedPtr::from_info_full(a_ptr_info, TypeDescription::NULL, false);
    let b_mptr = ManagedPtr::from_info_full(b_ptr_info, TypeDescription::NULL, false);

    let total_bytes = a_len as usize * 2; // char is 2 bytes
    let equal = chunked_sequence_equal(ctx, &a_mptr, &b_mptr, total_bytes);

    ctx.push_i32(equal as i32);
    StepResult::Continue
}

#[dotnet_intrinsic("static bool System.SpanHelpers::SequenceEqual(ref byte, ref byte, nuint)")]
pub fn intrinsic_span_helpers_sequence_equal<'gc, 'm: 'gc>(
    ctx: &mut dyn VesOps<'gc, 'm>,
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
fn pop_nonneg_usize<'gc, 'm>(ctx: &mut dyn VesOps<'gc, 'm>) -> Result<usize, StepResult> {
    match ctx.pop() {
        StackValue::Int32(i) => {
            if i < 0 {
                return Err(ctx.throw_by_name("System.ArgumentOutOfRangeException"));
            }
            Ok(i as usize)
        }
        _ => Err(ctx.throw_by_name("System.ArgumentException")),
    }
}

#[dotnet_intrinsic("static System.ReadOnlySpan<char> System.MemoryExtensions::AsSpan(string)")]
#[dotnet_intrinsic("static System.ReadOnlySpan<char> System.MemoryExtensions::AsSpan(string, int)")]
#[dotnet_intrinsic(
    "static System.ReadOnlySpan<char> System.MemoryExtensions::AsSpan(string, int, int)"
)]
#[dotnet_intrinsic("static System.Span<T> System.MemoryExtensions::AsSpan<T>(T[])")]
#[dotnet_intrinsic("static System.Span<T> System.MemoryExtensions::AsSpan<T>(T[], int)")]
#[dotnet_intrinsic("static System.Span<T> System.MemoryExtensions::AsSpan<T>(T[], int, int)")]
#[dotnet_intrinsic("static System.ReadOnlySpan<T> System.MemoryExtensions::AsSpan<T>(T[])")]
#[dotnet_intrinsic("static System.ReadOnlySpan<T> System.MemoryExtensions::AsSpan<T>(T[], int)")]
#[dotnet_intrinsic(
    "static System.ReadOnlySpan<T> System.MemoryExtensions::AsSpan<T>(T[], int, int)"
)]
pub fn intrinsic_as_span<'gc, 'm: 'gc>(
    ctx: &mut dyn VesOps<'gc, 'm>,
    method: MethodDescription,
    generics: &GenericLookup,
) -> StepResult {
    let _gc = ctx.gc();
    let param_count = method.method.signature.parameters.len();

    // AsSpan can have 1, 2, or 3 parameters:
    // - AsSpan(string) - whole string
    // - AsSpan(string, int start) - substring from start
    // - AsSpan(string, int start, int length) - substring with length
    // - AsSpan(T[]) - whole array
    // - AsSpan(T[], int start) - array slice from start
    // - AsSpan(T[], int start, int length) - array slice with length
    let (start, length_override) = match param_count {
        1 => (0, None),
        2 => {
            let start = match pop_nonneg_usize(ctx) {
                Ok(v) => v,
                Err(e) => return e,
            };
            (start, None)
        }
        3 => {
            let length = match pop_nonneg_usize(ctx) {
                Ok(v) => v,
                Err(e) => return e,
            };
            let start = match pop_nonneg_usize(ctx) {
                Ok(v) => v,
                Err(e) => return e,
            };
            (start, Some(length))
        }
        _ => return ctx.throw_by_name("System.ArgumentException"),
    };

    let source = ctx.pop();

    let res_ctx = ctx.with_generics(generics);

    let (origin, mut offset) = match crate::instructions::objects::get_ptr_info(ctx, &source) {
        Ok(v) => v,
        Err(e) => return e,
    };
    let h_opt = match origin {
        dotnet_value::pointer::PointerOrigin::Heap(ObjectRef(Some(h))) => Some(h),
        _ => None,
    };

    let (base_ptr, total_len, element_type, element_size) = match &source {
        StackValue::ObjectRef(ObjectRef(Some(h))) => {
            let heap = h.borrow();
            match &heap.storage {
                HeapStorage::Str(s) => (
                    unsafe { heap.storage.raw_data_ptr() },
                    s.len(),
                    vm_try!(res_ctx.make_concrete(&dotnetdll::prelude::BaseType::Char)),
                    2, // char is 2 bytes in .NET
                ),
                HeapStorage::Vec(a) => {
                    let elem_type = a.element.clone();
                    let elem_size = a.layout.element_layout.size();
                    (
                        unsafe { a.raw_data_ptr() },
                        a.layout.length,
                        elem_type,
                        elem_size.as_usize(),
                    )
                }
                _ => {
                    return StepResult::Error(
                        crate::error::ExecutionError::NotImplemented(format!(
                            "AsSpan called on non-string/non-array object: {:?}",
                            heap.storage
                        ))
                        .into(),
                    );
                }
            }
        }
        StackValue::ObjectRef(ObjectRef(None)) => {
            let element_type = if !generics.method_generics.is_empty() {
                generics.method_generics[0].clone()
            } else {
                vm_try!(res_ctx.make_concrete(&dotnetdll::prelude::BaseType::Char))
            };
            (std::ptr::null_mut(), 0, element_type, 2)
        }
        _ => return ctx.throw_by_name("System.ArgumentException"),
    };

    // Apply start and length_override
    if start > total_len {
        return ctx.throw_by_name("System.ArgumentOutOfRangeException");
    }
    let actual_length = if let Some(len) = length_override {
        if start + len > total_len {
            return ctx.throw_by_name("System.ArgumentOutOfRangeException");
        }
        len
    } else {
        total_len - start
    };

    let ptr = if base_ptr.is_null() {
        base_ptr
    } else {
        unsafe { base_ptr.add(start * element_size) }
    };
    offset += dotnet_utils::ByteOffset(start * element_size);
    let len = actual_length;

    let span_type_concrete = match &method.method.signature.return_type.1 {
        Some(ParameterType::Value(t)) => {
            vm_try!(generics.make_concrete(method.resolution(), t.clone()))
        }
        Some(_) => {
            return StepResult::Error(
                crate::error::ExecutionError::InternalError(
                    "AsSpan called on method with ref/typedref return".to_string(),
                )
                .into(),
            );
        }
        None => {
            return StepResult::Error(
                crate::error::ExecutionError::InternalError(
                    "AsSpan called on method returning void".to_string(),
                )
                .into(),
            );
        }
    };
    let span_type = vm_try!(ctx.loader().find_concrete_type(span_type_concrete.clone()));

    let layout = vm_try!(type_layout(span_type_concrete.clone(), &res_ctx));

    let (ref_offset_rel, length_offset_rel) = match &*layout {
        LayoutManager::Field(f) => {
            let ref_off = vm_try!(f.get_field_by_name("_reference").ok_or_else(|| {
                crate::error::ExecutionError::InternalError(
                    "Span must have _reference field".to_string(),
                )
            }))
            .position;
            let len_off = vm_try!(f.get_field_by_name("_length").ok_or_else(|| {
                crate::error::ExecutionError::InternalError(
                    "Span must have _length field".to_string(),
                )
            }))
            .position;
            (ref_off, len_off)
        }
        _ => {
            return StepResult::Error(
                crate::error::ExecutionError::InternalError(
                    "Expected Field layout for Span".to_string(),
                )
                .into(),
            );
        }
    };

    let new_lookup = GenericLookup::new(vec![element_type.clone()]);
    let res_ctx_generic = res_ctx.with_generics(&new_lookup);

    let span = vm_try!(res_ctx_generic.new_object(span_type));

    let element_type_desc = vm_try!(ctx.loader().find_concrete_type(element_type));
    let managed = ManagedPtr::new(
        NonNull::new(ptr),
        element_type_desc,
        h_opt.map(|h| ObjectRef(Some(h))),
        false,
        Some(offset),
    );
    {
        let mut data = span.instance_storage.get_mut();
        let start = ref_offset_rel.as_usize();
        managed.write(&mut data[start..start + ManagedPtr::SIZE]);
    }

    {
        let mut data = span.instance_storage.get_mut();
        let start = length_offset_rel.as_usize();
        data[start..start + 4].copy_from_slice(&(len as i32).to_ne_bytes());
    }

    ctx.push_value_type(span);
    StepResult::Continue
}

#[dotnet_intrinsic(
    "static System.Span<T> System.Runtime.CompilerServices.RuntimeHelpers::CreateSpan<T>(System.RuntimeFieldHandle)"
)]
pub fn intrinsic_runtime_helpers_create_span<'gc, 'm: 'gc>(
    ctx: &mut dyn VesOps<'gc, 'm>,
    _method: MethodDescription,
    generics: &GenericLookup,
) -> StepResult {
    let gc = ctx.gc();
    let element_type = &generics.method_generics[0];
    let res_ctx = ctx.with_generics(generics);
    let element_size = vm_try!(type_layout(element_type.clone(), &res_ctx)).size();

    let field_handle = ctx.pop_value_type();

    let res_ctx_handle = ctx.current_context();
    let handle_layout = vm_try!(type_layout(
        ConcreteType::from(field_handle.description),
        &res_ctx_handle
    ));
    let value_offset = match &*handle_layout {
        LayoutManager::Field(f) => vm_try!(f.get_field_by_name("_value").ok_or_else(|| {
            crate::error::ExecutionError::InternalError(
                "RuntimeFieldHandle must have _value field".to_string(),
            )
        }))
        .position
        .as_usize(),
        _ => {
            return StepResult::Error(
                crate::error::ExecutionError::InternalError(
                    "Expected Field layout for RuntimeFieldHandle".to_string(),
                )
                .into(),
            );
        }
    };

    let (
        FieldDescription {
            field,
            field_resolution,
            ..
        },
        lookup,
    ) = {
        let mut ptr_buf = [0u8; ObjectRef::SIZE];
        {
            let data = field_handle.instance_storage.get();
            ptr_buf.copy_from_slice(&data[value_offset..value_offset + ObjectRef::SIZE]);
        }
        let obj_ref = unsafe { ObjectRef::read_branded(&ptr_buf, &gc) };
        ctx.resolve_runtime_field(obj_ref)
    };
    let field_type = vm_try!(lookup.make_concrete(field_resolution, field.return_type.clone()));
    let field_desc = vm_try!(ctx.loader().find_concrete_type(field_type.clone()));

    let Some(initial_data) = &field.initial_value else {
        return ctx.throw_by_name("System.ArgumentException");
    };

    if field_desc
        .definition()
        .name
        .starts_with("__StaticArrayInitTypeSize=")
    {
        let prefix = "__StaticArrayInitTypeSize=";
        let size_str = &field_desc.definition().name[prefix.len()..];
        let size_end = size_str.find('_').unwrap_or(size_str.len());
        let array_size = vm_try!(size_str[..size_end].parse::<usize>().map_err(|e| {
            crate::error::ExecutionError::InternalError(format!(
                "Failed to parse array size: {}",
                e
            ))
        }));
        let data_slice = &initial_data[..array_size];

        let span_type = vm_try!(ctx.loader().corlib_type("System.ReadOnlySpan`1"));
        let span_lookup = GenericLookup::new(vec![element_type.clone()]);
        let span_res_ctx = res_ctx.with_generics(&span_lookup);
        let span_instance = vm_try!(span_res_ctx.new_object(span_type));

        let layout = vm_try!(type_layout(ConcreteType::from(span_type), &span_res_ctx));
        let (ref_offset, length_offset) = match &*layout {
            LayoutManager::Field(f) => {
                let ref_off = vm_try!(f.get_field_by_name("_reference").ok_or_else(|| {
                    crate::error::ExecutionError::NotImplemented(
                        "Span must have _reference field".to_string(),
                    )
                }))
                .position;
                let len_off = vm_try!(f.get_field_by_name("_length").ok_or_else(|| {
                    crate::error::ExecutionError::NotImplemented(
                        "Span must have _length field".to_string(),
                    )
                }))
                .position;
                (ref_off, len_off)
            }
            _ => {
                return StepResult::Error(
                    crate::error::ExecutionError::NotImplemented(
                        "Expected Field layout for Span".to_string(),
                    )
                    .into(),
                );
            }
        };

        let element_desc = vm_try!(ctx.loader().find_concrete_type(element_type.clone()));
        let managed = ManagedPtr::new(
            NonNull::new(data_slice.as_ptr() as *mut u8),
            element_desc,
            None,
            false,
            None,
        );
        {
            let mut data = span_instance.instance_storage.get_mut();
            managed
                .write(&mut data[ref_offset.as_usize()..ref_offset.as_usize() + ManagedPtr::SIZE]);
        }

        let element_count = (array_size / element_size.as_usize()) as i32;
        {
            let mut data = span_instance.instance_storage.get_mut();
            data[length_offset.as_usize()..length_offset.as_usize() + 4]
                .copy_from_slice(&element_count.to_ne_bytes());
        }

        ctx.push_value_type(span_instance);
        StepResult::Continue
    } else {
        todo!("initial field data for {:?}", field_desc);
    }
}

#[dotnet_intrinsic(
    "static T& System.Runtime.CompilerServices.RuntimeHelpers::GetSpanDataFrom<T>(T&, System.Type, int&)"
)]
pub fn intrinsic_runtime_helpers_get_span_data_from<'gc, 'm: 'gc>(
    ctx: &mut dyn VesOps<'gc, 'm>,
    _method: MethodDescription,
    generics: &GenericLookup,
) -> StepResult {
    let gc = ctx.gc();
    let length_ref = ctx.pop_managed_ptr();
    let type_handle = ctx.pop_value_type();
    let field_handle = ctx.pop_value_type();

    // Resolve field
    let res_ctx_field = ctx.current_context();
    let field_layout = vm_try!(type_layout(
        ConcreteType::from(field_handle.description),
        &res_ctx_field
    ));
    let field_value_offset = match &*field_layout {
        LayoutManager::Field(f) => vm_try!(f.get_field_by_name("_value").ok_or_else(|| {
            crate::error::ExecutionError::NotImplemented(
                "RuntimeFieldHandle must have _value field".to_string(),
            )
        }))
        .position
        .as_usize(),
        _ => {
            return StepResult::Error(
                crate::error::ExecutionError::NotImplemented(
                    "Expected Field layout for RuntimeFieldHandle".to_string(),
                )
                .into(),
            );
        }
    };

    let (FieldDescription { field, .. }, _) = {
        let mut ptr_buf = [0u8; ObjectRef::SIZE];
        {
            let data = field_handle.instance_storage.get();
            ptr_buf
                .copy_from_slice(&data[field_value_offset..field_value_offset + ObjectRef::SIZE]);
        }
        let obj_ref = unsafe { ObjectRef::read_branded(&ptr_buf, &gc) };
        ctx.resolve_runtime_field(obj_ref)
    };

    // Resolve type
    let res_ctx_type = ctx.current_context();
    let runtime_type_layout = vm_try!(type_layout(
        ConcreteType::from(type_handle.description),
        &res_ctx_type
    ));
    let type_value_offset = match &*runtime_type_layout {
        LayoutManager::Field(f) => vm_try!(f.get_field_by_name("_value").ok_or_else(|| {
            crate::error::ExecutionError::NotImplemented(
                "RuntimeTypeHandle must have _value field".to_string(),
            )
        }))
        .position
        .as_usize(),
        _ => {
            return StepResult::Error(
                crate::error::ExecutionError::NotImplemented(
                    "Expected Field layout for RuntimeTypeHandle".to_string(),
                )
                .into(),
            );
        }
    };

    let element_type_runtime = {
        let mut ptr_buf = [0u8; ObjectRef::SIZE];
        {
            let data = type_handle.instance_storage.get();
            ptr_buf.copy_from_slice(&data[type_value_offset..type_value_offset + ObjectRef::SIZE]);
        }
        let obj_ref = unsafe { ObjectRef::read_branded(&ptr_buf, &gc) };
        ctx.resolve_runtime_type(obj_ref)
    };

    let element_type: ConcreteType = element_type_runtime.to_concrete(ctx.loader());

    let res_ctx = ctx.with_generics(generics);
    let element_size = vm_try!(type_layout(element_type.clone(), &res_ctx)).size();

    let Some(initial_data) = &field.initial_value else {
        ctx.push_isize(0);
        return StepResult::Continue;
    };

    if field.name.starts_with("__StaticArrayInitTypeSize=") {
        let prefix = "__StaticArrayInitTypeSize=";
        let size_str = &field.name[prefix.len()..];
        let size_end = size_str.find('_').unwrap_or(size_str.len());
        let array_size = vm_try!(size_str[..size_end].parse::<usize>().map_err(|e| {
            crate::error::ExecutionError::InternalError(format!(
                "Failed to parse array size: {}",
                e
            ))
        }));

        let element_count = (array_size / element_size.as_usize()) as i32;
        vm_try!(
            unsafe {
                ctx.write_bytes(
                    length_ref.origin.clone(),
                    length_ref.offset,
                    &element_count.to_ne_bytes(),
                )
            }
            .map_err(|e| crate::error::ExecutionError::InternalError(e.to_string()))
        );

        let element_desc = vm_try!(ctx.loader().find_concrete_type(element_type.clone()));
        let managed = ManagedPtr::new(
            NonNull::new(initial_data.as_ptr() as *mut u8),
            element_desc,
            None,
            false,
            None,
        );
        ctx.push_managed_ptr(managed);
    } else {
        return ctx.throw_by_name("System.ArgumentException");
    }
    StepResult::Continue
}

#[dotnet_intrinsic("static byte& DotnetRs.Internal::GetArrayData(System.Array)")]
pub fn intrinsic_internal_get_array_data<'gc, 'm: 'gc>(
    ctx: &mut dyn VesOps<'gc, 'm>,
    _method: MethodDescription,
    generics: &GenericLookup,
) -> StepResult {
    let _gc = ctx.gc();
    let array_ref = ctx.pop_obj();

    let element_type = if !generics.method_generics.is_empty() {
        generics.method_generics[0].clone()
    } else {
        return StepResult::Error(
            crate::error::ExecutionError::NotImplemented(
                "GetArrayData expected generic argument".to_string(),
            )
            .into(),
        );
    };

    let element_type_desc = vm_try!(ctx.loader().find_concrete_type(element_type));

    if let Some(handle) = array_ref.0 {
        let inner = handle.borrow();
        if let HeapStorage::Vec(v) = &inner.storage {
            let ptr = unsafe { v.raw_data_ptr() };

            // For Vectors, the ManagedPtr offset must be relative to the raw data pointer
            // (returned by raw_data_ptr()), not the Object pointer.
            // Since we are pointing to the start of the data, the offset is 0.
            let offset = 0;

            let managed = ManagedPtr::new(
                NonNull::new(ptr),
                element_type_desc,
                Some(array_ref),
                false,
                Some(dotnet_value::ByteOffset(offset)),
            );
            ctx.push_managed_ptr(managed);
        } else {
            return StepResult::Error(
                crate::error::ExecutionError::NotImplemented(
                    "GetArrayData called on non-vector object".to_string(),
                )
                .into(),
            );
        }
    } else {
        let managed = ManagedPtr::new(None, element_type_desc, None, false, None);
        ctx.push_managed_ptr(managed);
    }
    StepResult::Continue
}

#[dotnet_intrinsic("T& System.Span<T>::GetPinnableReference()")]
#[dotnet_intrinsic("T& System.ReadOnlySpan<T>::GetPinnableReference()")]
pub fn intrinsic_span_get_pinnable_reference<'gc, 'm: 'gc>(
    ctx: &mut dyn VesOps<'gc, 'm>,
    _method: MethodDescription,
    generics: &GenericLookup,
) -> StepResult {
    let _gc = ctx.gc();
    let span = ctx.pop_managed_ptr();

    let element_type = &generics.type_generics[0];
    let element_desc = vm_try!(ctx.loader().find_concrete_type(element_type.clone()));

    let res_ctx = ctx.current_context();
    let layout = vm_try!(type_layout(ConcreteType::from(span.inner_type), &res_ctx));

    let LayoutManager::Field(f) = &*layout else {
        return StepResult::Error(
            crate::error::ExecutionError::NotImplemented(
                "Expected Field layout for Span".to_string(),
            )
            .into(),
        );
    };

    // Read fields using helpers
    let managed_ref = match read_span_reference_from_ptr(&span, f, ctx) {
        Ok(m) => m,
        Err(e) => return StepResult::Error(crate::error::ExecutionError::InternalError(e).into()),
    };
    let length = match read_span_length_from_ptr(&span, f, ctx) {
        Ok(l) => l,
        Err(e) => return StepResult::Error(crate::error::ExecutionError::InternalError(e).into()),
    };

    // If the span is empty, return a null reference
    if length == 0 {
        let null_ref = ManagedPtr::new(None, element_desc, None, false, None);
        ctx.push_managed_ptr(null_ref);
    } else {
        // Return a managed pointer to the first element
        let mut managed = managed_ref;
        managed.inner_type = element_desc;
        ctx.push_managed_ptr(managed);
    }

    StepResult::Continue
}

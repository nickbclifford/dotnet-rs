use dotnet_types::{
    TypeDescription,
    error::{IntrinsicError, MemoryAccessError},
};
use dotnet_utils::GcScopeGuard;
use dotnet_value::{
    StackValue,
    layout::{FieldLayoutManager, HasLayout},
    object::{HeapStorage, Object},
    pointer::{ManagedPtr, ManagedPtrInfo, ManagedPtrResolver, PointerOrigin},
};
use dotnet_vm_ops::SupportSlotOps;
use dotnet_vm_ops::ops::{LoaderOps, MemoryOps, RawMemoryOps};

/// Read the executable `_reference` ManagedPtr from a Span/ReadOnlySpan value type.
///
/// The field is stored in the three-word ManagedPtr wire format, so Stack and
/// Static origins require `resolver` to supply their currently live bases.
pub fn read_span_reference<'gc, R: ManagedPtrResolver<'gc> + ?Sized>(
    span: &Object<'gc>,
    resolver: &R,
    slots: &impl SupportSlotOps,
) -> Result<ManagedPtrInfo<'gc>, IntrinsicError> {
    let field = slots
        .span_or_readonly_span_reference_layout(span.instance_storage.layout())
        .ok_or(IntrinsicError::Static("Span must have _reference field"))?;
    span.instance_storage.with_data(|data| {
        // SAFETY: The field layout identifies one complete ManagedPtr representation in `data`.
        unsafe { ManagedPtr::read_resolved_unchecked(&data[field.position.as_usize()..], resolver) }
            .map_err(|e| {
                IntrinsicError::Message(
                    format!("Failed to deserialize span _reference: {e}").into(),
                )
            })
    })
}

/// Read the _length from a Span/ReadOnlySpan value type.
pub fn read_span_length(span: &Object, slots: &impl SupportSlotOps) -> Result<i32, IntrinsicError> {
    Ok(slots
        .span_or_readonly_span_length_field(&span.instance_storage, span.description.clone())
        .ok_or(IntrinsicError::Static("Span must have _length field"))?
        .read())
}

/// Read the _reference ManagedPtr from a ManagedPtr that points to a Span.
pub fn read_span_reference_from_ptr<'gc, T>(
    span_ptr: &ManagedPtr<'gc>,
    layout: &FieldLayoutManager,
    slots: &impl SupportSlotOps,
    ctx: &T,
) -> Result<ManagedPtr<'gc>, IntrinsicError>
where
    T: RawMemoryOps<'gc> + MemoryOps<'gc> + ManagedPtrResolver<'gc>,
{
    let ref_field = slots
        .span_or_readonly_span_reference_layout(layout)
        .ok_or(IntrinsicError::Static("Span must have _reference field"))?;

    // Read the raw bytes of the _reference field and deserialize properly.
    // We MUST NOT use ctx.read_unaligned with span_ptr.origin because that would
    // incorrectly tag the ManagedPtr with the Span's origin (e.g., Stack) instead of
    // the actual origin stored in the serialized bytes (e.g., Heap).
    // SAFETY: `span_ptr` points to a Span value validated by the caller and `ptr_bytes` matches
    // the serialized `ManagedPtr` width expected for the `_reference` field.
    let mut ptr_bytes = ManagedPtr::serialization_buffer();
    // SAFETY: The field position and serialization buffer were validated above; `read_bytes`
    // accesses exactly that in-bounds `_reference` representation.
    unsafe {
        ctx.read_bytes(
            span_ptr.origin().clone(),
            span_ptr.byte_offset() + ref_field.position,
            &mut ptr_bytes,
        )
    }
    .map_err(|e| {
        IntrinsicError::Message(format!("Failed to read span _reference bytes: {}", e).into())
    })?;

    // Deserialize the ManagedPtrInfo from bytes
    // SAFETY: `ptr_bytes` was just read from managed memory using `ManagedPtr` serialization size,
    // we pass the current branded GC token for lifetime validation, and `ctx`
    // resolves live Stack and Static bases for executable pointer recovery.
    let info = unsafe {
        ManagedPtr::read_resolved_branded(
            &ptr_bytes,
            &ctx.gc_with_token(&ctx.no_active_borrows_token()),
            ctx,
        )
    }
    .map_err(|e| {
        IntrinsicError::Message(format!("Failed to deserialize span _reference: {:?}", e).into())
    })?;

    // Reconstruct with proper type - use NULL for now, caller can adjust if needed
    Ok(ManagedPtr::from_info_full(
        info,
        TypeDescription::NULL,
        false,
    ))
}

/// Read the _length from a ManagedPtr that points to a Span.
pub fn read_span_length_from_ptr<'gc, T: RawMemoryOps<'gc>>(
    span_ptr: &ManagedPtr<'gc>,
    layout: &FieldLayoutManager,
    slots: &impl SupportSlotOps,
    ctx: &T,
) -> Result<i32, IntrinsicError> {
    let length_field = slots
        .span_or_readonly_span_length_layout(layout)
        .ok_or(IntrinsicError::Static("Span must have _length field"))?;
    // SAFETY: `_length` field offset/layout come from validated span layout metadata for `span_ptr`.
    let val = unsafe {
        ctx.read_unaligned(
            span_ptr.origin().clone(),
            span_ptr.byte_offset() + length_field.position,
            &length_field.layout,
            None,
        )
        .map_err(|e| IntrinsicError::Message(format!("Failed to read _length: {}", e).into()))?
    };
    Ok(val.as_i32())
}

/// Write a ManagedPtr + length into a Span/ReadOnlySpan value type.
pub fn write_span_fields<'gc, T: RawMemoryOps<'gc>>(
    span_ptr: &ManagedPtr<'gc>,
    managed: &ManagedPtr<'gc>,
    length: i32,
    layout: &FieldLayoutManager,
    slots: &impl SupportSlotOps,
    ctx: &mut T,
) -> Result<(), IntrinsicError> {
    let ref_field = slots
        .span_or_readonly_span_reference_layout(layout)
        .ok_or(IntrinsicError::Static("Span must have _reference field"))?;
    let length_field = slots
        .span_or_readonly_span_length_layout(layout)
        .ok_or(IntrinsicError::Static("Span must have _length field"))?;

    // Write _reference
    // SAFETY: `_reference` offset/layout are resolved from the span layout, and the serialized
    // `ManagedPtr` value matches that field's storage contract.
    unsafe {
        ctx.write_unaligned(
            span_ptr.origin().clone(),
            span_ptr.byte_offset() + ref_field.position,
            StackValue::ManagedPtr(managed.clone().into()),
            &ref_field.layout,
        )
    }
    .map_err(|e| IntrinsicError::Message(format!("Failed to write _reference: {}", e).into()))?;

    // Write _length
    // SAFETY: `_length` offset/layout are resolved from span metadata and we write a plain i32
    // value in the expected representation for that field.
    unsafe {
        ctx.write_unaligned(
            span_ptr.origin().clone(),
            span_ptr.byte_offset() + length_field.position,
            StackValue::Int32(length),
            &length_field.layout,
        )
    }
    .map_err(|e| IntrinsicError::Message(format!("Failed to write _length: {}", e).into()))?;

    Ok(())
}

pub fn with_span_data<
    'gc,
    'span,
    R,
    T: RawMemoryOps<'gc> + ManagedPtrResolver<'span> + LoaderOps,
>(
    ctx: &T,
    span: Object<'span>,
    element_type: TypeDescription,
    element_size: usize,
    f: impl FnOnce(&[u8]) -> R,
) -> Result<R, IntrinsicError> {
    let _gc_scope = GcScopeGuard::enter(
        ctx.as_borrow_scope(),
        ctx.as_borrow_scope().gc_ready_token(),
    );
    let info = read_span_reference(&span, ctx, ctx.loader().as_ref())?;
    let len = read_span_length(&span, ctx.loader().as_ref())? as usize;

    if len == 0 {
        return Ok(f(&[]));
    }

    let m_ptr = ManagedPtr::from_info_full(info, element_type, false);
    let total_size = len * element_size;

    // Bounds checking
    if let Some(owner) = m_ptr.owner() {
        let handle = owner.0.ok_or(IntrinsicError::Static(
            "ManagedPtr::with_data: null owner handle",
        ))?;
        let inner = handle.borrow();

        match &inner.storage {
            HeapStorage::Vec(v) => {
                // Vector storage is external (on Rust heap), so we must check absolute addresses
                // rather than offsets relative to the object header.
                // SAFETY: We only read the vector's backing data pointer for bounds computation.
                // `inner` borrow keeps the storage alive while this pointer is used.
                let data_ptr = unsafe { v.raw_data_ptr() } as usize;
                let elem_size = v.layout.element_layout.size().as_usize();
                let data_len = v.layout.length * elem_size;

                // Calculate pointer value from owner + offset
                let ptr_val = data_ptr + m_ptr.byte_offset().as_usize();

                // Check if the range [ptr_val, ptr_val + total_size) is within [data_ptr, data_ptr + data_len)
                if ptr_val < data_ptr || ptr_val + total_size > data_ptr + data_len {
                    return Err(IntrinsicError::Memory(MemoryAccessError::BoundsCheck {
                        offset: m_ptr.byte_offset().as_usize(),
                        size: total_size,
                        len: data_len,
                    }));
                }
            }
            _ => {
                // Inline storage (Object/Boxed): check offset relative to object header
                let obj_size = inner.storage.size_bytes();
                if m_ptr.byte_offset().as_usize() + total_size > obj_size {
                    return Err(IntrinsicError::Memory(MemoryAccessError::BoundsCheck {
                        offset: m_ptr.byte_offset().as_usize(),
                        size: total_size,
                        len: obj_size,
                    }));
                }
            }
        }
    } else if let PointerOrigin::Transient(obj) = &m_ptr.origin() {
        let mut bounds_error = None;
        obj.with_data(|data| {
            if m_ptr.byte_offset().as_usize() + total_size > data.len() {
                bounds_error = Some(IntrinsicError::Memory(MemoryAccessError::BoundsCheck {
                    offset: m_ptr.byte_offset().as_usize(),
                    size: total_size,
                    len: data.len(),
                }));
            }
        });
        if let Some(e) = bounds_error {
            return Err(e);
        }
    }

    // SAFETY: All size/offset bounds above guarantee the `[byte_offset, byte_offset + total_size)`
    // range is valid for `m_ptr`'s origin before exposing a byte slice to `f`.
    Ok(unsafe { m_ptr.with_data(total_size, f) })
}

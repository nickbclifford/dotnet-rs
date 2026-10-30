use dotnet_types::{
    TypeDescription,
    error::{IntrinsicError, MemoryAccessError},
};
use dotnet_utils::GcScopeGuard;
use dotnet_value::{
    StackValue,
    layout::FieldLayoutManager,
    object::Object,
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
        // SAFETY: F3.InteriorPointerRebased — The field layout identifies one complete ManagedPtr representation in `data`.
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
    // SAFETY: F3.InteriorPointerRebased — `span_ptr` points to a Span value validated by the caller and `ptr_bytes` matches
    // the serialized `ManagedPtr` width expected for the `_reference` field.
    let mut ptr_bytes = ManagedPtr::serialization_buffer();
    // SAFETY: F10.RawMemoryAccessValid — The field position and serialization buffer were validated above; `read_bytes`
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
    // SAFETY: F3.InteriorPointerRebased — `ptr_bytes` was just read from managed memory using `ManagedPtr` serialization size,
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
    // SAFETY: F10.RawMemoryAccessValid — `_length` field offset/layout come from validated span layout metadata for `span_ptr`.
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
    // SAFETY: F10.RawMemoryAccessValid — `_reference` offset/layout are resolved from the span layout, and the serialized
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
    // SAFETY: F10.RawMemoryAccessValid — `_length` offset/layout are resolved from span metadata and we write a plain i32
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

pub(crate) fn heap_payload_len(
    pointer: &ManagedPtr<'_>,
) -> Result<Option<usize>, MemoryAccessError> {
    Ok(match pointer.origin() {
        PointerOrigin::Heap(owner) => owner.0.map(|handle| handle.borrow().storage.payload_len()),
        #[cfg(feature = "multithreading")]
        PointerOrigin::CrossArenaObjectRef(ptr, tid) => {
            let _lease = dotnet_utils::gc::try_acquire_lease(*tid)
                .ok_or(MemoryAccessError::CrossArenaViolation)?;
            Some(ptr.as_heap_storage(|storage| storage.payload_len()))
        }
        _ => None,
    })
}

fn memory_bounds_error(offset: usize, size: usize, len: usize) -> IntrinsicError {
    IntrinsicError::Memory(MemoryAccessError::BoundsCheck { offset, size, len })
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
    if matches!(m_ptr.origin(), PointerOrigin::Heap(owner) if owner.0.is_none()) {
        return Err(IntrinsicError::Static(
            "ManagedPtr::with_data: null owner handle",
        ));
    }
    let offset = m_ptr.byte_offset().as_usize();
    let payload_len = heap_payload_len(&m_ptr).map_err(IntrinsicError::Memory)?;
    let total_size = if let Some(payload_len) = payload_len {
        len.checked_mul(element_size)
            .ok_or_else(|| memory_bounds_error(offset, usize::MAX, payload_len))?
    } else {
        // Preserve the pre-existing non-heap size arithmetic; this repair covers heap origins.
        len * element_size
    };

    // Heap and cross-arena ranges use actual initialized payload extent, not
    // `size_bytes()` (which includes allocation-accounting overhead). The
    // subsequent exact read revalidates the range while holding its own guard.
    if let Some(payload_len) = payload_len {
        if offset
            .checked_add(total_size)
            .is_none_or(|end| end > payload_len)
        {
            return Err(memory_bounds_error(offset, total_size, payload_len));
        }
    } else if let PointerOrigin::Transient(obj) = m_ptr.origin() {
        let mut bounds_error = None;
        obj.with_data(|data| {
            if offset + total_size > data.len() {
                bounds_error = Some(memory_bounds_error(offset, total_size, data.len()));
            }
        });
        if let Some(error) = bounds_error {
            return Err(error);
        }
    }

    if let Some(payload_len) = payload_len {
        // SAFETY: F10.RawMemoryAccessValid — the exact read validates the heap
        // range again under the storage guard before exposing the requested bytes.
        unsafe { m_ptr.try_with_data(total_size, f) }.ok_or_else(|| {
            match heap_payload_len(&m_ptr) {
                Err(error) => IntrinsicError::Memory(error),
                _ => memory_bounds_error(offset, total_size, payload_len),
            }
        })
    } else {
        // SAFETY: F10.RawMemoryAccessValid — Non-heap origins retain their legacy
        // access behavior and caller-provided validity contract. Stack/Static
        // migration belongs to M3; a Transient pointer retains its shared owner.
        Ok(unsafe { m_ptr.with_data(total_size, f) })
    }
}

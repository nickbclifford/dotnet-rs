use crate::error::MemoryAccessError;
use dotnet_value::{
    StackValue,
    layout::{HasLayout, LayoutManager, Scalar},
};

/// Integrity Checks
pub fn check_read_safety(
    result_layout: &LayoutManager,
    src_layout: Option<&LayoutManager>,
    src_ptr_offset: usize,
) -> Result<(), MemoryAccessError> {
    let mut err = Ok(());
    check_refs_in_layout(result_layout, 0, &mut |ref_offset| {
        if err.is_err() {
            return;
        }
        let target_src = src_ptr_offset + ref_offset;

        if let Some(sl) = src_layout {
            if !has_ref_at(sl, target_src) {
                err = Err(MemoryAccessError::TypeMismatch(format!(
                    "Heap Corruption: Reading ObjectRef from non-ref memory at offset {}",
                    target_src
                )));
            } else if !target_src.is_multiple_of(8) {
                err = Err(MemoryAccessError::UnalignedAccess(target_src));
            }
        }
    });
    err
}

pub fn has_ref_at(layout: &LayoutManager, offset: usize) -> bool {
    match layout {
        LayoutManager::Scalar(s) => match s {
            Scalar::ObjectRef | Scalar::ManagedPtr => offset == 0,
            _ => false,
        },
        LayoutManager::Field(fm) => {
            for f in fm.fields.values() {
                if offset >= f.position.as_usize()
                    && offset < (f.position + f.layout.size()).as_usize()
                {
                    return has_ref_at(&f.layout, offset - f.position.as_usize());
                }
            }
            false
        }
        LayoutManager::Array(am) => {
            let elem_size = am.element_layout.size();
            if elem_size.as_usize() == 0 {
                return false;
            }
            let idx = offset / elem_size.as_usize();
            if idx >= am.length {
                return false;
            }
            let rel = offset % elem_size.as_usize();
            has_ref_at(&am.element_layout, rel)
        }
    }
}

pub(crate) fn check_refs_in_layout<F>(layout: &LayoutManager, base: usize, callback: &mut F)
where
    F: FnMut(usize) + ?Sized,
{
    match layout {
        LayoutManager::Scalar(s) => match s {
            Scalar::ObjectRef | Scalar::ManagedPtr => callback(base),
            _ => {}
        },
        LayoutManager::Field(fm) => {
            for f in fm.fields.values() {
                check_refs_in_layout(&f.layout, base + f.position.as_usize(), callback);
            }
        }
        LayoutManager::Array(am) => {
            if am.element_layout.is_or_contains_refs() {
                let sz = am.element_layout.size();
                for i in 0..am.length {
                    check_refs_in_layout(&am.element_layout, base + (sz * i).as_usize(), callback);
                }
            }
        }
    }
}

pub(crate) fn validate_ref_integrity(
    dest_layout: &LayoutManager,
    base_offset: usize,
    range_start: usize,
    range_end: usize,
    src_layout: &LayoutManager,
) -> Result<(), MemoryAccessError> {
    match dest_layout {
        LayoutManager::Scalar(s) => match s {
            Scalar::ObjectRef | Scalar::ManagedPtr => {
                let ref_start = base_offset;
                let ref_end = base_offset + 8;

                if ref_start < range_end && ref_end > range_start {
                    if ref_start < range_start {
                        return Err(MemoryAccessError::TypeMismatch(format!(
                            "Heap Corruption: Write starts in the middle of an ObjectRef at {}",
                            ref_start
                        )));
                    }

                    if ref_end > range_end {
                        return Err(MemoryAccessError::TypeMismatch(format!(
                            "Heap Corruption: Write ends in the middle of an ObjectRef at {}",
                            ref_start
                        )));
                    }

                    let src_offset = ref_start - range_start;
                    if !has_ref_at(src_layout, src_offset) {
                        return Err(MemoryAccessError::TypeMismatch(format!(
                            "Heap Corruption: Writing non-ref data over ObjectRef at offset {}",
                            ref_start
                        )));
                    }

                    if !ref_start.is_multiple_of(8) {
                        return Err(MemoryAccessError::UnalignedAccess(ref_start));
                    }
                }
            }
            _ => {}
        },
        LayoutManager::Field(fm) => {
            for f in fm.fields.values() {
                let f_start = base_offset + f.position.as_usize();
                let f_end = f_start + f.layout.size().as_usize();
                if f_start < range_end && f_end > range_start {
                    validate_ref_integrity(&f.layout, f_start, range_start, range_end, src_layout)?;
                }
            }
        }
        LayoutManager::Array(am) => {
            if am.element_layout.is_or_contains_refs() {
                let elem_size = am.element_layout.size();
                if elem_size.as_usize() == 0 {
                    return Ok(());
                }

                let rel_start = range_start.saturating_sub(base_offset);
                let rel_end = range_end.saturating_sub(base_offset);

                let start_idx = rel_start / elem_size.as_usize();
                let end_idx = rel_end.div_ceil(elem_size.as_usize());
                let end_idx = end_idx.min(am.length);

                for i in start_idx..end_idx {
                    validate_ref_integrity(
                        &am.element_layout,
                        base_offset + (elem_size * i).as_usize(),
                        range_start,
                        range_end,
                        src_layout,
                    )?;
                }
            }
        }
    }
    Ok(())
}

pub(crate) fn extract_int(val: StackValue) -> Result<i32, MemoryAccessError> {
    match val {
        StackValue::Int32(v) => Ok(v),
        _ => Err(MemoryAccessError::TypeMismatch(format!(
            "Expected Int32, got {:?}",
            val
        ))),
    }
}

pub(crate) fn extract_long(val: StackValue) -> Result<i64, MemoryAccessError> {
    match val {
        StackValue::Int64(v) => Ok(v),
        _ => Err(MemoryAccessError::TypeMismatch(format!(
            "Expected Int64, got {:?}",
            val
        ))),
    }
}

pub(crate) fn extract_native_int(val: StackValue) -> Result<isize, MemoryAccessError> {
    match val {
        StackValue::NativeInt(v) => Ok(v),
        _ => Err(MemoryAccessError::TypeMismatch(format!(
            "Expected NativeInt, got {:?}",
            val
        ))),
    }
}

pub(crate) fn extract_float(val: StackValue) -> Result<f64, MemoryAccessError> {
    match val {
        StackValue::NativeFloat(v) => Ok(v),
        _ => Err(MemoryAccessError::TypeMismatch(format!(
            "Expected NativeFloat, got {:?}",
            val
        ))),
    }
}

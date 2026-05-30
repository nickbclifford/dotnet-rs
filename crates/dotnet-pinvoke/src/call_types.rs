use dotnet_types::error::ExecutionError;
use dotnet_utils::ByteOffset;
use dotnet_value::pointer::PointerOrigin;
use dotnet_vm_ops::ops::PInvokeContext;
use libffi::middle::Cif;
use std::{
    alloc::{Layout, alloc_zeroed, dealloc},
    mem::{align_of, size_of},
    ptr::NonNull,
};

pub(crate) enum WriteBackSource<'gc> {
    Managed(PointerOrigin<'gc>, ByteOffset),
    #[cfg(test)]
    Raw(NonNull<u8>),
}

pub(crate) enum TempBuffer {
    I32(Box<i32>),
    I64(Box<i64>),
    Isize(Box<isize>),
    F64(Box<f64>),
    Ptr(Box<*mut u8>),
    Bytes(Vec<u8>),
}

impl TempBuffer {
    fn kind_name(&self) -> &'static str {
        match self {
            TempBuffer::I32(_) => "i32",
            TempBuffer::I64(_) => "i64",
            TempBuffer::Isize(_) => "isize",
            TempBuffer::F64(_) => "f64",
            TempBuffer::Ptr(_) => "ptr",
            TempBuffer::Bytes(_) => "bytes",
        }
    }

    pub(crate) fn as_i32(&self) -> Result<&i32, ExecutionError> {
        match self {
            TempBuffer::I32(val) => Ok(val),
            _ => Err(ExecutionError::TypeMismatch {
                expected: "P/Invoke temp buffer i32".to_string(),
                actual: self.kind_name().to_string(),
            }),
        }
    }

    pub(crate) fn as_i64(&self) -> Result<&i64, ExecutionError> {
        match self {
            TempBuffer::I64(val) => Ok(val),
            _ => Err(ExecutionError::TypeMismatch {
                expected: "P/Invoke temp buffer i64".to_string(),
                actual: self.kind_name().to_string(),
            }),
        }
    }

    pub(crate) fn as_isize(&self) -> Result<&isize, ExecutionError> {
        match self {
            TempBuffer::Isize(val) => Ok(val),
            _ => Err(ExecutionError::TypeMismatch {
                expected: "P/Invoke temp buffer isize".to_string(),
                actual: self.kind_name().to_string(),
            }),
        }
    }

    pub(crate) fn as_f64(&self) -> Result<&f64, ExecutionError> {
        match self {
            TempBuffer::F64(val) => Ok(val),
            _ => Err(ExecutionError::TypeMismatch {
                expected: "P/Invoke temp buffer f64".to_string(),
                actual: self.kind_name().to_string(),
            }),
        }
    }

    pub(crate) fn as_ptr(&self) -> Result<&*mut u8, ExecutionError> {
        match self {
            TempBuffer::Ptr(val) => Ok(val),
            _ => Err(ExecutionError::TypeMismatch {
                expected: "P/Invoke temp buffer ptr".to_string(),
                actual: self.kind_name().to_string(),
            }),
        }
    }

    pub(crate) fn as_bytes(&self) -> Result<&[u8], ExecutionError> {
        match self {
            TempBuffer::Bytes(buf) => Ok(buf),
            _ => Err(ExecutionError::TypeMismatch {
                expected: "P/Invoke temp buffer bytes".to_string(),
                actual: self.kind_name().to_string(),
            }),
        }
    }
}

pub(crate) struct AlignedReturnBuffer {
    ptr: NonNull<u8>,
    len: usize,
    layout: Layout,
}

impl AlignedReturnBuffer {
    pub(crate) fn new_zeroed(len: usize, align: usize) -> Result<Self, ExecutionError> {
        if align == 0 || !align.is_power_of_two() {
            return Err(ExecutionError::InternalError(format!(
                "P/Invoke return buffer alignment is invalid: {}",
                align
            )));
        }

        let layout = Layout::from_size_align(len.max(1), align).map_err(|e| {
            ExecutionError::InternalError(format!(
                "P/Invoke return buffer layout is invalid (size={}, align={}): {}",
                len, align, e
            ))
        })?;

        // SAFETY: `layout` is validated above. A null return is handled as OOM.
        let ptr = unsafe { alloc_zeroed(layout) };
        let ptr = NonNull::new(ptr).ok_or_else(|| {
            ExecutionError::InternalError(format!(
                "P/Invoke return buffer allocation failed (size={}, align={})",
                len, align
            ))
        })?;

        Ok(Self { ptr, len, layout })
    }

    pub(crate) fn as_mut_ptr(&mut self) -> *mut u8 {
        self.ptr.as_ptr()
    }

    pub(crate) fn as_bytes(&self) -> &[u8] {
        // SAFETY: `ptr` points to `layout.size()` bytes for this allocation; callers only read
        // up to `len`, which is <= `layout.size()`.
        unsafe { std::slice::from_raw_parts(self.ptr.as_ptr(), self.len) }
    }
}

impl Drop for AlignedReturnBuffer {
    fn drop(&mut self) {
        // SAFETY: `ptr` was allocated with `alloc_zeroed` using `layout` in `new_zeroed`.
        unsafe { dealloc(self.ptr.as_ptr(), self.layout) };
    }
}

pub(crate) fn ffi_return_layout_from_raw(
    raw: &libffi::raw::ffi_type,
    unknown_align_fallback: usize,
) -> Result<(usize, usize), ExecutionError> {
    let size = raw.size;
    let align = usize::from(raw.alignment);

    let align = if align == 0 {
        unknown_align_fallback
    } else {
        align
    };

    if align == 0 || !align.is_power_of_two() {
        return Err(ExecutionError::InternalError(format!(
            "P/Invoke return ffi_type alignment is invalid: {}",
            align
        )));
    }

    Ok((size, align))
}

pub(crate) fn ffi_cif_return_layout(cif: &Cif) -> Result<(usize, usize), ExecutionError> {
    // SAFETY: `Cif` owns a prepared `ffi_cif`; rtype is set by `ffi_prep_cif`.
    let rtype = unsafe { (*cif.as_raw_ptr()).rtype };
    if rtype.is_null() {
        return Err(ExecutionError::InternalError(
            "P/Invoke ffi_cif has null return type".to_string(),
        ));
    }

    // Use a conservative fallback when libffi leaves alignment unset for aggregate metadata.
    // This keeps return buffers safely over-aligned even when rtype alignment is unknown.
    // SAFETY: null checked above.
    let raw = unsafe { &*rtype };
    ffi_return_layout_from_raw(raw, 64)
}

pub(crate) fn validate_typed_return_abi<T>(cif: &Cif) -> Result<(), ExecutionError> {
    let (ffi_size, ffi_align) = ffi_cif_return_layout(cif)?;
    let rust_size = size_of::<T>();
    let rust_align = align_of::<T>();
    if ffi_size != rust_size || ffi_align != rust_align {
        return Err(ExecutionError::InternalError(format!(
            "P/Invoke return ABI mismatch: ffi(size={}, align={}), rust(size={}, align={})",
            ffi_size, ffi_align, rust_size, rust_align
        )));
    }
    Ok(())
}

pub(crate) fn copy_value_type_return_data(dest: &mut [u8], src: &[u8]) {
    let copy_len = std::cmp::min(dest.len(), src.len());
    dest[..copy_len].copy_from_slice(&src[..copy_len]);
}

pub(crate) fn apply_write_backs_with<'gc, F>(
    write_backs: &[(WriteBackSource<'gc>, usize, usize)],
    temp_buffers: &[TempBuffer],
    mut write_managed: F,
) -> Result<(), dotnet_types::error::MemoryAccessError>
where
    F: FnMut(
        &PointerOrigin<'gc>,
        ByteOffset,
        &[u8],
    ) -> Result<(), dotnet_types::error::MemoryAccessError>,
{
    for (source, buf_idx, len) in write_backs {
        let Some(temp) = temp_buffers.get(*buf_idx) else {
            return Err(dotnet_types::error::MemoryAccessError::BoundsCheck {
                offset: *buf_idx,
                size: 1,
                len: temp_buffers.len(),
            });
        };
        let buf = match temp {
            TempBuffer::Bytes(buf) => buf.as_slice(),
            _ => {
                return Err(dotnet_types::error::MemoryAccessError::TypeMismatch(
                    format!(
                        "P/Invoke temp buffer bytes expected, got {}",
                        temp.kind_name()
                    ),
                ));
            }
        };
        if *len > buf.len() {
            return Err(dotnet_types::error::MemoryAccessError::BoundsCheck {
                offset: 0,
                size: *len,
                len: buf.len(),
            });
        }

        match source {
            WriteBackSource::Managed(origin, offset) => {
                write_managed(origin, *offset, &buf[..*len])?;
            }
            #[cfg(test)]
            // SAFETY: `dest_ptr` originates from validated argument marshalling for this call and
            // we copy exactly `len` bytes from an owned temporary buffer.
            WriteBackSource::Raw(dest_ptr) => unsafe {
                std::ptr::copy_nonoverlapping(buf.as_ptr(), dest_ptr.as_ptr(), *len);
            },
        }
    }

    Ok(())
}

pub(crate) fn apply_write_backs<'gc>(
    ctx: &mut dyn PInvokeContext<'gc>,
    write_backs: &[(WriteBackSource<'gc>, usize, usize)],
    temp_buffers: &[TempBuffer],
) -> Result<(), dotnet_types::error::MemoryAccessError> {
    apply_write_backs_with(write_backs, temp_buffers, |origin, offset, data| {
        // SAFETY: `origin` and `offset` were captured from validated managed pointers and `data`
        // references owned temporary storage.
        unsafe { ctx.write_bytes(origin.clone(), offset, data) }
    })
}

//! SIMD helper utilities with portable scalar fallback.
//!
//! This crate exposes byte-oriented operations that can transparently use an
//! architecture-specific SIMD backend when the `simd` feature is enabled.

mod scalar;

#[cfg(all(feature = "simd", target_arch = "aarch64"))]
mod aarch64;
#[cfg(all(feature = "simd", any(target_arch = "x86", target_arch = "x86_64")))]
mod x86;

/// Returns whether a SIMD backend is available on the current machine.
#[inline]
pub fn simd_available() -> bool {
    #[cfg(all(feature = "simd", any(target_arch = "x86", target_arch = "x86_64")))]
    {
        x86::is_available()
    }
    #[cfg(all(feature = "simd", target_arch = "aarch64"))]
    {
        aarch64::is_available()
    }
    #[cfg(not(all(
        feature = "simd",
        any(target_arch = "x86", target_arch = "x86_64", target_arch = "aarch64")
    )))]
    {
        false
    }
}

/// Returns the active vector width in bytes, or 0 when scalar fallback is used.
#[inline]
pub fn vector_width() -> usize {
    #[cfg(all(feature = "simd", any(target_arch = "x86", target_arch = "x86_64")))]
    {
        if x86::is_available() {
            x86::VECTOR_BYTES
        } else {
            0
        }
    }
    #[cfg(all(feature = "simd", target_arch = "aarch64"))]
    {
        if aarch64::is_available() {
            aarch64::VECTOR_BYTES
        } else {
            0
        }
    }
    #[cfg(not(all(
        feature = "simd",
        any(target_arch = "x86", target_arch = "x86_64", target_arch = "aarch64")
    )))]
    {
        0
    }
}

/// Compares two byte slices for equality using SIMD where possible.
#[inline]
pub fn sequence_equal(lhs: &[u8], rhs: &[u8]) -> bool {
    if lhs.len() != rhs.len() {
        return false;
    }

    #[cfg(all(feature = "simd", any(target_arch = "x86", target_arch = "x86_64")))]
    {
        if let Some(result) = x86::sequence_equal(lhs, rhs) {
            return result;
        }
    }

    #[cfg(all(feature = "simd", target_arch = "aarch64"))]
    {
        if let Some(result) = aarch64::sequence_equal(lhs, rhs) {
            return result;
        }
    }

    scalar::sequence_equal(lhs, rhs)
}

/// Copies bytes from `src` into `dst`.
///
/// Panics if the slices differ in length.
#[inline]
pub fn copy_nonoverlapping(dst: &mut [u8], src: &[u8]) {
    assert_eq!(
        dst.len(),
        src.len(),
        "copy_nonoverlapping requires equal-length slices",
    );

    #[cfg(all(feature = "simd", any(target_arch = "x86", target_arch = "x86_64")))]
    {
        if x86::copy_nonoverlapping(dst, src) {
            return;
        }
    }

    #[cfg(all(feature = "simd", target_arch = "aarch64"))]
    {
        if aarch64::copy_nonoverlapping(dst, src) {
            return;
        }
    }

    scalar::copy_nonoverlapping(dst, src);
}

/// Copies `len` bytes from `src` to `dst` with a non-overlapping contract.
///
/// # Safety
/// - `dst` must be valid for writes of `len` bytes.
/// - `src` must be valid for reads of `len` bytes.
/// - The source and destination ranges must not overlap.
#[inline]
pub unsafe fn copy_nonoverlapping_raw(dst: *mut u8, src: *const u8, len: usize) {
    if len == 0 {
        return;
    }

    // SAFETY: The caller guarantees pointer validity and non-overlap for `len` bytes.
    let dst_slice = unsafe { std::slice::from_raw_parts_mut(dst, len) };
    // SAFETY: The caller guarantees pointer validity for `len` bytes.
    let src_slice = unsafe { std::slice::from_raw_parts(src, len) };
    copy_nonoverlapping(dst_slice, src_slice);
}

/// Fills `dst` with `value`.
#[inline]
pub fn fill(dst: &mut [u8], value: u8) {
    #[cfg(all(feature = "simd", any(target_arch = "x86", target_arch = "x86_64")))]
    {
        if x86::fill(dst, value) {
            return;
        }
    }

    #[cfg(all(feature = "simd", target_arch = "aarch64"))]
    {
        if aarch64::fill(dst, value) {
            return;
        }
    }

    scalar::fill(dst, value);
}

/// Fills `len` bytes at `dst` with `value`.
///
/// # Safety
/// - `dst` must be valid for writes of `len` bytes.
#[inline]
pub unsafe fn fill_raw(dst: *mut u8, len: usize, value: u8) {
    if len == 0 {
        return;
    }

    // SAFETY: The caller guarantees pointer validity for `len` bytes.
    let dst_slice = unsafe { std::slice::from_raw_parts_mut(dst, len) };
    fill(dst_slice, value);
}

/// Clears `dst` to zero.
#[inline]
pub fn clear(dst: &mut [u8]) {
    fill(dst, 0);
}

/// Clears `len` bytes at `dst` to zero.
///
/// # Safety
/// - `dst` must be valid for writes of `len` bytes.
#[inline]
pub unsafe fn clear_raw(dst: *mut u8, len: usize) {
    // SAFETY: Same safety contract as `fill_raw`.
    unsafe { fill_raw(dst, len, 0) };
}

#[cfg(test)]
mod tests {
    use super::{
        clear, clear_raw, copy_nonoverlapping, copy_nonoverlapping_raw, fill, fill_raw,
        sequence_equal, simd_available, vector_width,
    };

    #[test]
    fn sequence_equal_reports_expected_results() {
        assert!(sequence_equal(b"", b""));
        assert!(sequence_equal(b"dotnet-rs", b"dotnet-rs"));
        assert!(!sequence_equal(b"dotnet-rs", b"dotnet-rx"));
        assert!(!sequence_equal(b"abc", b"abcd"));
    }

    #[test]
    fn copy_nonoverlapping_copies_all_bytes() {
        let src = *b"simd-bytes";
        let mut dst = [0u8; 10];
        copy_nonoverlapping(&mut dst, &src);
        assert_eq!(dst, src);
    }

    #[test]
    fn fill_and_clear_update_buffer_contents() {
        let mut buffer = [0u8; 64];
        fill(&mut buffer, 0xA5);
        assert!(buffer.iter().all(|value| *value == 0xA5));
        clear(&mut buffer);
        assert!(buffer.iter().all(|value| *value == 0));
    }

    #[test]
    fn raw_copy_fill_and_clear_update_buffer_contents() {
        let src = *b"0123456789abcdef";
        let mut dst = [0u8; 16];
        // SAFETY: Arrays are valid for 16 bytes and do not overlap.
        unsafe { copy_nonoverlapping_raw(dst.as_mut_ptr(), src.as_ptr(), src.len()) };
        assert_eq!(dst, src);

        // SAFETY: `dst` is valid for all writes below.
        unsafe { fill_raw(dst.as_mut_ptr(), dst.len(), 0x3C) };
        assert!(dst.iter().all(|value| *value == 0x3C));

        // SAFETY: `dst` is valid for all writes below.
        unsafe { clear_raw(dst.as_mut_ptr(), dst.len()) };
        assert!(dst.iter().all(|value| *value == 0));
    }

    #[test]
    fn vector_width_is_zero_when_simd_is_unavailable() {
        if simd_available() {
            assert_eq!(vector_width(), 16);
        } else {
            assert_eq!(vector_width(), 0);
        }
    }
}

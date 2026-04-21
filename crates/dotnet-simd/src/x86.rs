#[cfg(target_arch = "x86")]
use core::arch::x86::{
    __m128i, _mm_cmpeq_epi8, _mm_loadu_si128, _mm_movemask_epi8, _mm_set1_epi8, _mm_storeu_si128,
};
#[cfg(target_arch = "x86_64")]
use core::arch::x86_64::{
    __m128i, _mm_cmpeq_epi8, _mm_loadu_si128, _mm_movemask_epi8, _mm_set1_epi8, _mm_storeu_si128,
};

pub const VECTOR_BYTES: usize = 16;

#[inline]
pub fn is_available() -> bool {
    #[cfg(target_arch = "x86")]
    {
        std::arch::is_x86_feature_detected!("sse2")
    }
    #[cfg(target_arch = "x86_64")]
    {
        true
    }
}

#[inline]
pub fn sequence_equal(lhs: &[u8], rhs: &[u8]) -> Option<bool> {
    if !is_available() {
        return None;
    }

    // SAFETY: SSE2 support is guaranteed on x86_64 and checked at runtime on x86.
    Some(unsafe { sequence_equal_sse2(lhs, rhs) })
}

#[target_feature(enable = "sse2")]
unsafe fn sequence_equal_sse2(lhs: &[u8], rhs: &[u8]) -> bool {
    let mut offset = 0usize;
    while offset + VECTOR_BYTES <= lhs.len() {
        // SAFETY: The loop bounds guarantee at least 16 bytes remain for both slices.
        let left = unsafe { _mm_loadu_si128(lhs.as_ptr().add(offset).cast::<__m128i>()) };
        // SAFETY: The loop bounds guarantee at least 16 bytes remain for both slices.
        let right = unsafe { _mm_loadu_si128(rhs.as_ptr().add(offset).cast::<__m128i>()) };
        let mask = _mm_movemask_epi8(_mm_cmpeq_epi8(left, right));
        if mask != 0xFFFF_i32 {
            return false;
        }
        offset += VECTOR_BYTES;
    }

    lhs[offset..] == rhs[offset..]
}

#[inline]
pub fn copy_nonoverlapping(dst: &mut [u8], src: &[u8]) -> bool {
    if !is_available() {
        return false;
    }

    // SAFETY: SSE2 support is guaranteed on x86_64 and checked at runtime on x86.
    unsafe { copy_nonoverlapping_sse2(dst, src) };
    true
}

#[target_feature(enable = "sse2")]
unsafe fn copy_nonoverlapping_sse2(dst: &mut [u8], src: &[u8]) {
    let len = dst.len();
    let mut offset = 0usize;
    while offset + VECTOR_BYTES <= len {
        // SAFETY: The loop bounds guarantee at least 16 bytes remain.
        let value = unsafe { _mm_loadu_si128(src.as_ptr().add(offset).cast::<__m128i>()) };
        // SAFETY: The loop bounds guarantee at least 16 bytes remain.
        unsafe { _mm_storeu_si128(dst.as_mut_ptr().add(offset).cast::<__m128i>(), value) };
        offset += VECTOR_BYTES;
    }

    if offset < len {
        // SAFETY: Tail copy stays in-bounds and does not overlap by contract.
        unsafe {
            core::ptr::copy_nonoverlapping(
                src.as_ptr().add(offset),
                dst.as_mut_ptr().add(offset),
                len - offset,
            );
        }
    }
}

#[inline]
pub fn fill(dst: &mut [u8], value: u8) -> bool {
    if !is_available() {
        return false;
    }

    // SAFETY: SSE2 support is guaranteed on x86_64 and checked at runtime on x86.
    unsafe { fill_sse2(dst, value) };
    true
}

#[target_feature(enable = "sse2")]
unsafe fn fill_sse2(dst: &mut [u8], value: u8) {
    let len = dst.len();
    let pattern = _mm_set1_epi8(value as i8);
    let mut offset = 0usize;
    while offset + VECTOR_BYTES <= len {
        // SAFETY: The loop bounds guarantee at least 16 bytes remain.
        unsafe { _mm_storeu_si128(dst.as_mut_ptr().add(offset).cast::<__m128i>(), pattern) };
        offset += VECTOR_BYTES;
    }
    dst[offset..].fill(value);
}

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

    // SAFETY: F10.ArchIntrinsicPrecondition — `is_available` confirmed SSE2 above; the only caller, the crate-root `sequence_equal`, returns early unless the lengths are equal.
    Some(unsafe { sequence_equal_sse2(lhs, rhs) })
}

/// # Safety
///
/// SSE2 must be available, and `rhs` must be at least as long as `lhs`.
#[target_feature(enable = "sse2")]
unsafe fn sequence_equal_sse2(lhs: &[u8], rhs: &[u8]) -> bool {
    let mut offset = 0usize;
    while offset + VECTOR_BYTES <= lhs.len() {
        // SAFETY: F10.RawMemoryAccessValid — the loop condition keeps `offset + 16` within `lhs`, and this function's `# Safety` contract makes `rhs` at least as long.
        let left_ptr = unsafe { lhs.as_ptr().add(offset) }.cast::<__m128i>();
        // SAFETY: F10.RawMemoryAccessValid — `left_ptr` starts 16 or more readable bytes of `lhs`; the load is unaligned.
        let left = unsafe { _mm_loadu_si128(left_ptr) };
        // SAFETY: F10.RawMemoryAccessValid — the loop condition keeps `offset + 16` within `lhs`, and this function's `# Safety` contract makes `rhs` at least as long.
        let right_ptr = unsafe { rhs.as_ptr().add(offset) }.cast::<__m128i>();
        // SAFETY: F10.RawMemoryAccessValid — `right_ptr` starts 16 or more readable bytes of `rhs`; the load is unaligned.
        let right = unsafe { _mm_loadu_si128(right_ptr) };
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

    // SAFETY: F10.ArchIntrinsicPrecondition — `is_available` confirmed SSE2 above; the only caller, the crate-root `copy_nonoverlapping`, asserts equal lengths.
    unsafe { copy_nonoverlapping_sse2(dst, src) };
    true
}

/// # Safety
///
/// SSE2 must be available, and `src` must be at least as long as `dst`.
#[target_feature(enable = "sse2")]
unsafe fn copy_nonoverlapping_sse2(dst: &mut [u8], src: &[u8]) {
    let len = dst.len();
    let mut offset = 0usize;
    while offset + VECTOR_BYTES <= len {
        // SAFETY: F10.RawMemoryAccessValid — the loop condition keeps `offset + 16` within `dst`, and this function's `# Safety` contract makes `src` at least as long.
        let src_ptr = unsafe { src.as_ptr().add(offset) }.cast::<__m128i>();
        // SAFETY: F10.RawMemoryAccessValid — `src_ptr` starts 16 or more readable bytes of `src`; the load is unaligned.
        let value = unsafe { _mm_loadu_si128(src_ptr) };
        // SAFETY: F10.RawMemoryAccessValid — the loop condition keeps `offset + 16` within `dst`, and this function's `# Safety` contract makes `src` at least as long.
        let dst_ptr = unsafe { dst.as_mut_ptr().add(offset) }.cast::<__m128i>();
        // SAFETY: F10.RawMemoryAccessValid — `dst_ptr` starts 16 or more writable bytes of `dst`; the store is unaligned.
        unsafe { _mm_storeu_si128(dst_ptr, value) };
        offset += VECTOR_BYTES;
    }

    if offset < len {
        // SAFETY: F10.RawMemoryAccessValid — `offset < len`, so `offset` is within `dst` and, by this function's `# Safety` contract, within `src`.
        let src_tail = unsafe { src.as_ptr().add(offset) };
        // SAFETY: F10.RawMemoryAccessValid — `offset < len == dst.len()`, so `offset` is within `dst`.
        let dst_tail = unsafe { dst.as_mut_ptr().add(offset) };
        // SAFETY: F10.RawMemoryAccessValid — both tails have at least `len - offset` bytes; `dst` is a `&mut` borrow, so it cannot overlap `src`.
        unsafe { core::ptr::copy_nonoverlapping(src_tail, dst_tail, len - offset) };
    }
}

#[inline]
pub fn fill(dst: &mut [u8], value: u8) -> bool {
    if !is_available() {
        return false;
    }

    // SAFETY: F10.ArchIntrinsicPrecondition — `is_available` confirmed SSE2 above.
    unsafe { fill_sse2(dst, value) };
    true
}

/// # Safety
///
/// SSE2 must be available.
#[target_feature(enable = "sse2")]
unsafe fn fill_sse2(dst: &mut [u8], value: u8) {
    let len = dst.len();
    let pattern = _mm_set1_epi8(value as i8);
    let mut offset = 0usize;
    while offset + VECTOR_BYTES <= len {
        // SAFETY: F10.RawMemoryAccessValid — the loop condition keeps `offset + 16` within `dst`.
        let dst_ptr = unsafe { dst.as_mut_ptr().add(offset) }.cast::<__m128i>();
        // SAFETY: F10.RawMemoryAccessValid — `dst_ptr` starts 16 or more writable bytes of `dst`; the store is unaligned.
        unsafe { _mm_storeu_si128(dst_ptr, pattern) };
        offset += VECTOR_BYTES;
    }
    dst[offset..].fill(value);
}

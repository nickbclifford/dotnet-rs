use core::arch::aarch64::{vceqq_u8, vdupq_n_u8, vld1q_u8, vminvq_u8, vst1q_u8};

pub const VECTOR_BYTES: usize = 16;

#[inline]
pub fn is_available() -> bool {
    true
}

#[inline]
pub fn sequence_equal(lhs: &[u8], rhs: &[u8]) -> Option<bool> {
    // SAFETY: F10.ArchIntrinsicPrecondition — NEON is part of the baseline ISA on aarch64; the only caller, the crate-root `sequence_equal`, returns early unless the lengths are equal.
    Some(unsafe { sequence_equal_neon(lhs, rhs) })
}

/// # Safety
///
/// NEON must be available, and `rhs` must be at least as long as `lhs`.
#[target_feature(enable = "neon")]
unsafe fn sequence_equal_neon(lhs: &[u8], rhs: &[u8]) -> bool {
    let mut offset = 0usize;
    while offset + VECTOR_BYTES <= lhs.len() {
        // SAFETY: F10.RawMemoryAccessValid — the loop condition keeps `offset + 16` within `lhs`, and this function's `# Safety` contract makes `rhs` at least as long.
        let left_ptr = unsafe { lhs.as_ptr().add(offset) };
        // SAFETY: F10.RawMemoryAccessValid — `left_ptr` starts 16 or more readable bytes of `lhs`; the load is unaligned.
        let left = unsafe { vld1q_u8(left_ptr) };
        // SAFETY: F10.RawMemoryAccessValid — the loop condition keeps `offset + 16` within `lhs`, and this function's `# Safety` contract makes `rhs` at least as long.
        let right_ptr = unsafe { rhs.as_ptr().add(offset) };
        // SAFETY: F10.RawMemoryAccessValid — `right_ptr` starts 16 or more readable bytes of `rhs`; the load is unaligned.
        let right = unsafe { vld1q_u8(right_ptr) };
        let cmp = vceqq_u8(left, right);
        if vminvq_u8(cmp) != u8::MAX {
            return false;
        }
        offset += VECTOR_BYTES;
    }

    lhs[offset..] == rhs[offset..]
}

#[inline]
pub fn copy_nonoverlapping(dst: &mut [u8], src: &[u8]) -> bool {
    // SAFETY: F10.ArchIntrinsicPrecondition — NEON is part of the baseline ISA on aarch64; the only caller, the crate-root `copy_nonoverlapping`, asserts equal lengths.
    unsafe { copy_nonoverlapping_neon(dst, src) };
    true
}

/// # Safety
///
/// NEON must be available, and `src` must be at least as long as `dst`.
#[target_feature(enable = "neon")]
unsafe fn copy_nonoverlapping_neon(dst: &mut [u8], src: &[u8]) {
    let len = dst.len();
    let mut offset = 0usize;
    while offset + VECTOR_BYTES <= len {
        // SAFETY: F10.RawMemoryAccessValid — the loop condition keeps `offset + 16` within `dst`, and this function's `# Safety` contract makes `src` at least as long.
        let src_ptr = unsafe { src.as_ptr().add(offset) };
        // SAFETY: F10.RawMemoryAccessValid — `src_ptr` starts 16 or more readable bytes of `src`; the load is unaligned.
        let value = unsafe { vld1q_u8(src_ptr) };
        // SAFETY: F10.RawMemoryAccessValid — the loop condition keeps `offset + 16` within `dst`, and this function's `# Safety` contract makes `src` at least as long.
        let dst_ptr = unsafe { dst.as_mut_ptr().add(offset) };
        // SAFETY: F10.RawMemoryAccessValid — `dst_ptr` starts 16 or more writable bytes of `dst`; the store is unaligned.
        unsafe { vst1q_u8(dst_ptr, value) };
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
    // SAFETY: F10.ArchIntrinsicPrecondition — NEON is part of the baseline ISA on aarch64.
    unsafe { fill_neon(dst, value) };
    true
}

/// # Safety
///
/// NEON must be available.
#[target_feature(enable = "neon")]
unsafe fn fill_neon(dst: &mut [u8], value: u8) {
    let len = dst.len();
    let pattern = vdupq_n_u8(value);
    let mut offset = 0usize;
    while offset + VECTOR_BYTES <= len {
        // SAFETY: F10.RawMemoryAccessValid — the loop condition keeps `offset + 16` within `dst`.
        let dst_ptr = unsafe { dst.as_mut_ptr().add(offset) };
        // SAFETY: F10.RawMemoryAccessValid — `dst_ptr` starts 16 or more writable bytes of `dst`; the store is unaligned.
        unsafe { vst1q_u8(dst_ptr, pattern) };
        offset += VECTOR_BYTES;
    }
    dst[offset..].fill(value);
}

use core::arch::aarch64::{vceqq_u8, vdupq_n_u8, vld1q_u8, vminvq_u8, vst1q_u8};

pub const VECTOR_BYTES: usize = 16;

#[inline]
pub fn is_available() -> bool {
    true
}

#[inline]
pub fn sequence_equal(lhs: &[u8], rhs: &[u8]) -> Option<bool> {
    // SAFETY: NEON is part of the baseline ISA on aarch64.
    Some(unsafe { sequence_equal_neon(lhs, rhs) })
}

#[target_feature(enable = "neon")]
unsafe fn sequence_equal_neon(lhs: &[u8], rhs: &[u8]) -> bool {
    let mut offset = 0usize;
    while offset + VECTOR_BYTES <= lhs.len() {
        // SAFETY: The loop bounds guarantee at least 16 bytes remain for both slices.
        let left = unsafe { vld1q_u8(lhs.as_ptr().add(offset)) };
        // SAFETY: The loop bounds guarantee at least 16 bytes remain for both slices.
        let right = unsafe { vld1q_u8(rhs.as_ptr().add(offset)) };
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
    // SAFETY: NEON is part of the baseline ISA on aarch64.
    unsafe { copy_nonoverlapping_neon(dst, src) };
    true
}

#[target_feature(enable = "neon")]
unsafe fn copy_nonoverlapping_neon(dst: &mut [u8], src: &[u8]) {
    let len = dst.len();
    let mut offset = 0usize;
    while offset + VECTOR_BYTES <= len {
        // SAFETY: The loop bounds guarantee at least 16 bytes remain.
        let value = unsafe { vld1q_u8(src.as_ptr().add(offset)) };
        // SAFETY: The loop bounds guarantee at least 16 bytes remain.
        unsafe { vst1q_u8(dst.as_mut_ptr().add(offset), value) };
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
    // SAFETY: NEON is part of the baseline ISA on aarch64.
    unsafe { fill_neon(dst, value) };
    true
}

#[target_feature(enable = "neon")]
unsafe fn fill_neon(dst: &mut [u8], value: u8) {
    let len = dst.len();
    let pattern = vdupq_n_u8(value);
    let mut offset = 0usize;
    while offset + VECTOR_BYTES <= len {
        // SAFETY: The loop bounds guarantee at least 16 bytes remain.
        unsafe { vst1q_u8(dst.as_mut_ptr().add(offset), pattern) };
        offset += VECTOR_BYTES;
    }
    dst[offset..].fill(value);
}

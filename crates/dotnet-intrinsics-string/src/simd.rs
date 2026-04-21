#[inline]
pub fn probe_sequence_equal_u16(lhs: &[u16], rhs: &[u16]) -> Option<bool> {
    if lhs.len() != rhs.len() {
        return Some(false);
    }
    if lhs.len() < MIN_SIMD_PROBE_CHARS || !dotnet_simd::simd_available() {
        return None;
    }

    let byte_len = core::mem::size_of_val(lhs);
    // SAFETY: `u16` slices are contiguous; converting to an equal-size byte view is safe.
    let lhs_bytes = unsafe { core::slice::from_raw_parts(lhs.as_ptr().cast::<u8>(), byte_len) };
    // SAFETY: Same as `lhs_bytes`.
    let rhs_bytes = unsafe { core::slice::from_raw_parts(rhs.as_ptr().cast::<u8>(), byte_len) };
    Some(dotnet_simd::sequence_equal(lhs_bytes, rhs_bytes))
}

#[inline]
pub fn probe_index_of_char(chars: &[u16], _needle: u16) -> Option<Option<usize>> {
    if chars.len() < MIN_SIMD_PROBE_CHARS {
        return None;
    }

    #[cfg(all(feature = "simd", any(target_arch = "x86", target_arch = "x86_64")))]
    {
        let needle = _needle;
        if x86_sse2_available() {
            // SAFETY: SSE2 support is checked at runtime above.
            return Some(unsafe { index_of_char_sse2(chars, needle) });
        }
    }

    #[cfg(all(feature = "simd", target_arch = "aarch64"))]
    {
        let needle = _needle;
        // SAFETY: NEON is baseline on aarch64.
        return Some(unsafe { index_of_char_neon(chars, needle) });
    }

    None
}

#[inline]
pub fn probe_all_ascii_whitespace(chars: &[u16]) -> Option<bool> {
    if chars.len() < MIN_SIMD_PROBE_CHARS {
        return None;
    }

    #[cfg(all(feature = "simd", any(target_arch = "x86", target_arch = "x86_64")))]
    {
        if x86_sse2_available() {
            // SAFETY: SSE2 support is checked at runtime above.
            return unsafe { all_ascii_whitespace_sse2(chars) };
        }
    }

    #[cfg(all(feature = "simd", target_arch = "aarch64"))]
    {
        // SAFETY: NEON is baseline on aarch64.
        return unsafe { all_ascii_whitespace_neon(chars) };
    }

    None
}

const MIN_SIMD_PROBE_CHARS: usize = 32;

#[cfg(all(feature = "simd", target_arch = "x86"))]
#[inline]
fn x86_sse2_available() -> bool {
    std::arch::is_x86_feature_detected!("sse2")
}

#[cfg(all(feature = "simd", target_arch = "x86_64"))]
#[inline]
fn x86_sse2_available() -> bool {
    true
}

#[cfg(all(feature = "simd", any(target_arch = "x86", target_arch = "x86_64")))]
#[cfg(target_arch = "x86")]
use core::arch::x86::{
    __m128i, _mm_and_si128, _mm_cmpeq_epi16, _mm_cmpgt_epi16, _mm_loadu_si128, _mm_movemask_epi8,
    _mm_or_si128, _mm_set1_epi16, _mm_setzero_si128,
};
#[cfg(all(feature = "simd", any(target_arch = "x86", target_arch = "x86_64")))]
#[cfg(target_arch = "x86_64")]
use core::arch::x86_64::{
    __m128i, _mm_and_si128, _mm_cmpeq_epi16, _mm_cmpgt_epi16, _mm_loadu_si128, _mm_movemask_epi8,
    _mm_or_si128, _mm_set1_epi16, _mm_setzero_si128,
};

#[cfg(all(feature = "simd", any(target_arch = "x86", target_arch = "x86_64")))]
const X86_LANES_U16: usize = 8;

#[cfg(all(feature = "simd", any(target_arch = "x86", target_arch = "x86_64")))]
#[target_feature(enable = "sse2")]
unsafe fn index_of_char_sse2(chars: &[u16], needle: u16) -> Option<usize> {
    let mut offset = 0usize;
    let needle_pattern = _mm_set1_epi16(needle as i16);

    while offset + X86_LANES_U16 <= chars.len() {
        // SAFETY: Bounds guarantee at least 16 bytes from `offset`.
        let chunk = unsafe { _mm_loadu_si128(chars.as_ptr().add(offset).cast::<__m128i>()) };
        let cmp = _mm_cmpeq_epi16(chunk, needle_pattern);
        let mask = _mm_movemask_epi8(cmp);
        if mask != 0 {
            let lane = mask.trailing_zeros() as usize / 2;
            return Some(offset + lane);
        }
        offset += X86_LANES_U16;
    }

    chars[offset..]
        .iter()
        .position(|&c| c == needle)
        .map(|i| offset + i)
}

#[cfg(all(feature = "simd", any(target_arch = "x86", target_arch = "x86_64")))]
#[target_feature(enable = "sse2")]
unsafe fn all_ascii_whitespace_sse2(chars: &[u16]) -> Option<bool> {
    let mut offset = 0usize;
    let ascii_mask = _mm_set1_epi16(0xFF80u16 as i16);
    let zero = _mm_setzero_si128();
    let space = _mm_set1_epi16(0x20);
    let eight = _mm_set1_epi16(8);
    let fourteen = _mm_set1_epi16(14);

    while offset + X86_LANES_U16 <= chars.len() {
        // SAFETY: Bounds guarantee at least 16 bytes from `offset`.
        let chunk = unsafe { _mm_loadu_si128(chars.as_ptr().add(offset).cast::<__m128i>()) };
        let ascii_bits = _mm_and_si128(chunk, ascii_mask);
        if _mm_movemask_epi8(_mm_cmpeq_epi16(ascii_bits, zero)) != 0xFFFF_i32 {
            return None;
        }

        let is_space = _mm_cmpeq_epi16(chunk, space);
        let ge_9 = _mm_cmpgt_epi16(chunk, eight);
        let le_13 = _mm_cmpgt_epi16(fourteen, chunk);
        let in_control_whitespace = _mm_and_si128(ge_9, le_13);
        let is_whitespace = _mm_or_si128(is_space, in_control_whitespace);
        if _mm_movemask_epi8(is_whitespace) != 0xFFFF_i32 {
            return Some(false);
        }

        offset += X86_LANES_U16;
    }

    for &ch in &chars[offset..] {
        if ch > 0x7F {
            return None;
        }
        if !matches!(ch, 0x20 | 0x09..=0x0D) {
            return Some(false);
        }
    }

    Some(true)
}

#[cfg(all(feature = "simd", target_arch = "aarch64"))]
use core::arch::aarch64::{
    vandq_u16, vceqq_u16, vcgeq_u16, vcleq_u16, vdupq_n_u16, vld1q_u16, vminvq_u16, vorrq_u16,
    vst1q_u16,
};

#[cfg(all(feature = "simd", target_arch = "aarch64"))]
const AARCH64_LANES_U16: usize = 8;

#[cfg(all(feature = "simd", target_arch = "aarch64"))]
#[target_feature(enable = "neon")]
unsafe fn index_of_char_neon(chars: &[u16], needle: u16) -> Option<usize> {
    let mut offset = 0usize;
    let needle_pattern = vdupq_n_u16(needle);

    while offset + AARCH64_LANES_U16 <= chars.len() {
        // SAFETY: Bounds guarantee at least 8 u16 lanes from `offset`.
        let chunk = unsafe { vld1q_u16(chars.as_ptr().add(offset)) };
        let cmp = vceqq_u16(chunk, needle_pattern);

        let mut lane_matches = [0u16; AARCH64_LANES_U16];
        // SAFETY: `lane_matches` is valid for 8 lanes.
        unsafe { vst1q_u16(lane_matches.as_mut_ptr(), cmp) };
        if let Some(lane_idx) = lane_matches.iter().position(|&value| value == u16::MAX) {
            return Some(offset + lane_idx);
        }

        offset += AARCH64_LANES_U16;
    }

    chars[offset..]
        .iter()
        .position(|&c| c == needle)
        .map(|i| offset + i)
}

#[cfg(all(feature = "simd", target_arch = "aarch64"))]
#[target_feature(enable = "neon")]
unsafe fn all_ascii_whitespace_neon(chars: &[u16]) -> Option<bool> {
    let mut offset = 0usize;
    let ascii_mask = vdupq_n_u16(0xFF80);
    let space = vdupq_n_u16(0x20);
    let nine = vdupq_n_u16(9);
    let thirteen = vdupq_n_u16(13);

    while offset + AARCH64_LANES_U16 <= chars.len() {
        // SAFETY: Bounds guarantee at least 8 u16 lanes from `offset`.
        let chunk = unsafe { vld1q_u16(chars.as_ptr().add(offset)) };
        let ascii_bits = vandq_u16(chunk, ascii_mask);
        let mut lanes = [0u16; AARCH64_LANES_U16];
        // SAFETY: `lanes` is valid for 8 lanes.
        unsafe { vst1q_u16(lanes.as_mut_ptr(), ascii_bits) };
        if lanes.iter().any(|&lane| lane != 0) {
            return None;
        }

        let is_space = vceqq_u16(chunk, space);
        let ge_9 = vcgeq_u16(chunk, nine);
        let le_13 = vcleq_u16(chunk, thirteen);
        let in_control_whitespace = vandq_u16(ge_9, le_13);
        let is_whitespace = vorrq_u16(is_space, in_control_whitespace);
        if vminvq_u16(is_whitespace) != u16::MAX {
            return Some(false);
        }

        offset += AARCH64_LANES_U16;
    }

    for &ch in &chars[offset..] {
        if ch > 0x7F {
            return None;
        }
        if !matches!(ch, 0x20 | 0x09..=0x0D) {
            return Some(false);
        }
    }

    Some(true)
}

#[cfg(test)]
mod tests {
    use super::{probe_all_ascii_whitespace, probe_index_of_char, probe_sequence_equal_u16};

    fn scalar_index_of(chars: &[u16], needle: u16) -> Option<usize> {
        chars.iter().position(|&c| c == needle)
    }

    fn scalar_is_whitespace(chars: &[u16]) -> bool {
        chars.iter().all(|&ch| {
            char::from_u32(ch as u32)
                .map(|c| c.is_whitespace())
                .unwrap_or(false)
        })
    }

    #[test]
    fn probe_sequence_equal_matches_scalar_behavior() {
        let lhs = vec![b'a' as u16; 128];
        let lhs_clone = lhs.clone();
        let mut rhs = lhs.clone();
        rhs[77] = b'b' as u16;

        let equal = probe_sequence_equal_u16(&lhs, &lhs_clone).unwrap_or(lhs == lhs_clone);
        assert!(equal);

        let not_equal = probe_sequence_equal_u16(&lhs, &rhs).unwrap_or(lhs == rhs);
        assert!(!not_equal);
    }

    #[test]
    fn probe_index_of_matches_scalar_behavior() {
        let mut chars = vec![b'z' as u16; 256];
        chars[143] = b'a' as u16;

        let found = probe_index_of_char(&chars, b'a' as u16)
            .unwrap_or_else(|| scalar_index_of(&chars, b'a' as u16));
        assert_eq!(found, Some(143));

        let missing = probe_index_of_char(&chars, b'!' as u16)
            .unwrap_or_else(|| scalar_index_of(&chars, b'!' as u16));
        assert_eq!(missing, None);
    }

    #[test]
    fn probe_ascii_whitespace_matches_scalar_behavior() {
        let all_whitespace = vec![b' ' as u16; 128];
        let result = probe_all_ascii_whitespace(&all_whitespace)
            .unwrap_or_else(|| scalar_is_whitespace(&all_whitespace));
        assert!(result);

        let mut mixed = vec![b' ' as u16; 128];
        mixed[55] = b'X' as u16;
        let mixed_result =
            probe_all_ascii_whitespace(&mixed).unwrap_or_else(|| scalar_is_whitespace(&mixed));
        assert!(!mixed_result);
    }

    #[test]
    fn probe_ascii_whitespace_handles_unicode_fallback() {
        let unicode_ws = vec![0x2003u16; 64];
        let result = probe_all_ascii_whitespace(&unicode_ws)
            .unwrap_or_else(|| scalar_is_whitespace(&unicode_ws));
        assert!(result);
    }
}

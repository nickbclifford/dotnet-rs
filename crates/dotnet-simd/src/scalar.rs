#[inline]
pub fn sequence_equal(lhs: &[u8], rhs: &[u8]) -> bool {
    lhs == rhs
}

#[inline]
pub fn copy_nonoverlapping(dst: &mut [u8], src: &[u8]) {
    dst.copy_from_slice(src);
}

#[inline]
pub fn fill(dst: &mut [u8], value: u8) {
    dst.fill(value);
}

//! Branch prediction hint shims for stable Rust toolchains.
//!
//! `std::hint::{likely, unlikely}` is not available on the toolchain used by this
//! workspace, so we use a tiny stable shim: route the uncommon side through a
//! `#[cold]` function and return the original boolean unchanged.
//!
//! These helpers are intentionally used only on branches with measured skew from
//! benchmark instrumentation (Step 4b).

#[inline]
pub(crate) fn likely(b: bool) -> bool {
    if !b {
        cold_branch();
    }
    b
}

#[inline]
pub(crate) fn unlikely(b: bool) -> bool {
    if b {
        cold_branch();
    }
    b
}

#[cold]
#[inline(never)]
fn cold_branch() {}

//! Runtime-facing SIMD intrinsic handlers and capability probes.
//!
//! This crate provides `#[dotnet_intrinsic]` handlers for selected
//! `System.Runtime.Intrinsics` capability APIs and centralizes SIMD capability
//! probing logic used by VM dispatch.
//!
//! Hardware acceleration probes are currently conservative: both
//! [`vector128_is_hardware_accelerated`] and
//! [`vector256_is_hardware_accelerated`] return `false` until the broader
//! `Vector128`/`Vector256` intrinsic surface is implemented.
//!
//! ## Host Trait
//!
//! VM contexts integrating these handlers implement [`SimdIntrinsicHost<'gc>`].
//! This alias currently forwards directly to
//! `dotnet_vm_ops::ops::SimdIntrinsicHost<'gc>` and serves as this crate's
//! host-trait integration point for SIMD intrinsic dispatch.
//!
//! See `docs/BUILD_TIME_CODE_GENERATION.md` for how `#[dotnet_intrinsic]`
//! handlers are discovered and wired into generated intrinsic dispatch tables.
use dotnet_vm_ops::ops::SimdIntrinsicHost as VmSimdIntrinsicHost;

pub mod handlers;

/// Reports whether 128-bit vector intrinsics should be considered accelerated.
#[inline]
pub fn vector128_is_hardware_accelerated() -> bool {
    // We intentionally keep the runtime capability probes disabled until the
    // broader Vector128/Vector256 intrinsic surface is implemented.
    false
}

/// Reports whether 256-bit vector intrinsics should be considered accelerated.
#[inline]
pub fn vector256_is_hardware_accelerated() -> bool {
    false
}

dotnet_vm_ops::trait_alias! {
    /// Host contract for SIMD intrinsic handlers.
    pub trait SimdIntrinsicHost<'gc> = VmSimdIntrinsicHost<'gc>;
}

#[cfg(test)]
mod tests {
    use super::{vector128_is_hardware_accelerated, vector256_is_hardware_accelerated};

    #[test]
    fn vector128_probe_is_disabled_until_full_intrinsics_support_lands() {
        assert!(!vector128_is_hardware_accelerated());
    }

    #[test]
    fn vector256_probe_is_disabled_until_full_intrinsics_support_lands() {
        assert!(!vector256_is_hardware_accelerated());
    }

    #[test]
    fn vector256_probe_implies_vector128_probe() {
        let vector128 = vector128_is_hardware_accelerated();
        let vector256 = vector256_is_hardware_accelerated();
        assert!(!vector256 || vector128);
    }
}

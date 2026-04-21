//! Runtime-facing SIMD intrinsic capability probes and host contract.
//!
//! This crate hosts initial `.NET System.Runtime.Intrinsics` probes that map
//! VM capability checks onto intrinsic getter handlers.
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

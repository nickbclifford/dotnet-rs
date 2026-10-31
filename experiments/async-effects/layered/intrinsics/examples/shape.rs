//! Instantiate the same six future types on one concrete host for compile-cost comparison.
#[cfg(feature = "concrete")]
use effect_intrinsics::concrete as candidate;
#[cfg(all(feature = "gat", not(feature = "concrete")))]
use effect_intrinsics::gat as candidate;
use effect_intrinsics::{InvokeSpec, ReturnKind};
use effect_ops::*;
fn instantiate(fx: VmEffects, kind: u8) -> NativeFuture {
    match kind {
        0 => Box::pin(candidate::substring(fx, StringRoot(Stashed::Null), 0, None)),
        1 => Box::pin(candidate::wait_finalizers(fx)),
        2 => Box::pin(candidate::monitor_enter(fx, Stashed::Null)),
        3 => Box::pin(candidate::reflection_invoke(
            fx,
            InvokeSpec {
                method: 0,
                receiver: Stashed::Null,
                parameters: ArrayRoot(Stashed::Null),
                byref_indices: vec![],
                reference_byref_indices: vec![],
                return_kind: ReturnKind::Void,
            },
        )),
        4 => Box::pin(candidate::multicast(fx, vec![], vec![])),
        _ => Box::pin(candidate::ensure_static(fx, 0)),
    }
}
fn main() {
    std::hint::black_box(instantiate as fn(VmEffects, u8) -> NativeFuture);
}

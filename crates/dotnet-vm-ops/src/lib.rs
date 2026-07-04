//! # dotnet-vm-ops
//!
//! Core VES operation traits for the `dotnet-rs` Virtual Execution System (VES).
//!
//! Runtime execution data types are hosted in `dotnet-vm-data`.
//! Canonical imports should come from `dotnet_vm_data` directly.
//! Compatibility re-exports remain here for downstream crates.
//! This crate is intentionally separate from `dotnet-vm-data` to allow downstream
//! crates to depend on operation traits without pulling in any VM data structures.
pub mod intrinsic_args;
mod macros;
pub mod ops;
pub mod prepared_call;

use gc_arena::Collect;

pub const NULL_REF_MSG: &str = "Object reference not set to an instance of an object.";

/// A `Collect`-able unified encoding of cross-GC-safe-point VM resumption state.
/// Stored as `ThreadContext::continuation` so yield/resume state survives between
/// `mutate_root` calls without escaping `'gc`.
/// Replaces the ad-hoc combination of `suspended_handler_unwinds` and the
/// undocumented `back_up_ip + Yield` idiom.
#[derive(Collect)]
#[collect(no_drop, gc_lifetime = 'gc)]
pub enum VmContinuation<'gc> {
    /// No pending cross-safe-point state; normal execution.
    None,
    /// The current instruction yielded for a GC-safe-point retry.
    /// `back_up_ip` was called before yielding; IP is already restored. This marker is cleared
    /// when the executor re-enters the VM because the retry is encoded in the restored frame state.
    RetryInstruction,
    /// Nested exception-handler unwind states suspended during `leave`.
    /// Restored on the next exception-unwind completion.
    HandlerUnwinds(Vec<UnwindState<'gc>>),
}

pub use dotnet_macros::trait_alias;
pub use dotnet_vm_data::{
    self, CollectableMethodDescription, MethodInfo, MethodState, StepResult,
    exceptions::{
        ExceptionState, FilterState, HandlerAddress, ManagedException, ProtectedSection,
        SearchState, UnwindState, UnwindTarget, *,
    },
    stack::{BasePointer, EvaluationStack, FrameStack, MulticastState, PinnedLocals, StackFrame},
};
pub use ops::{
    ArgumentOps, DelegateIntrinsicHost, EvalStackOps, ExceptionContext, ExceptionOps, LoaderOps,
    LocalOps, MemoryOps, PInvokeContext, RawMemoryOps, ReflectionIntrinsicHost, ReflectionOps,
    ResolutionOps, SimdCapabilityOps, SimdIntrinsicHost, SpanIntrinsicHost, StackOps, StaticsOps,
    StringIntrinsicHost, ThreadOps, ThreadingIntrinsicHost, TypedStackOps, UnsafeIntrinsicHost,
    VariableOps, VesBaseOps, VesInternals,
};

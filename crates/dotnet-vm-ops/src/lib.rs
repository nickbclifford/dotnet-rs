//! # dotnet-vm-ops
//!
//! Core VES operation traits for the `dotnet-rs` Virtual Execution System (VES).
//!
//! Runtime execution data types are hosted in `dotnet-vm-data`.
//! Canonical imports should come from `dotnet_vm_data` directly.
//! Compatibility re-exports remain here for downstream crates.
//! P2.S4 decision: remain a separate crate from `dotnet-vm-data` for now, while
//! preserving a future compatibility-shim migration path if consolidation is justified.
mod macros;
pub mod ops;

// Keep trait-alias macro re-export for non-hot-path alias use-cases.
// High-traffic VM composition traits can use explicit trait definitions + explicit impls.
pub use dotnet_macros::trait_alias;
pub use dotnet_vm_data;
pub use dotnet_vm_data::{
    CollectableMethodDescription, MethodInfo, MethodState, StepResult,
    exceptions::{
        ExceptionState, FilterState, HandlerAddress, ManagedException, ProtectedSection,
        SearchState, UnwindState, UnwindTarget, *,
    },
    stack::{BasePointer, EvaluationStack, FrameStack, MulticastState, PinnedLocals, StackFrame},
};
pub use ops::{
    ArgumentOps, CallOps, DelegateIntrinsicHost, EvalStackOps, ExceptionContext, ExceptionOps,
    LoaderOps, LocalOps, MemoryOps, PInvokeContext, RawMemoryOps, ReflectionIntrinsicHost,
    ReflectionOps, ResolutionOps, SimdCapabilityOps, SimdIntrinsicHost, SpanIntrinsicHost,
    StackOps, StaticsOps, StringIntrinsicHost, ThreadOps, ThreadingIntrinsicHost, TypedStackOps,
    UnsafeIntrinsicHost, VariableOps, VesBaseOps, VesInternals,
};

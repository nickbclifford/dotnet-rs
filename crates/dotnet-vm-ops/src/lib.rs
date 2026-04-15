//! # dotnet-vm-ops
//!
//! Core VES operation traits for the `dotnet-rs` Virtual Execution System (VES).
//!
//! Runtime execution data types are hosted in `dotnet-vm-data` and re-exported
//! here so that intrinsics crates only need a single dependency on this crate
//! to access both the operation traits and the shared data types.
mod macros;
pub mod ops;

pub use dotnet_macros::trait_alias;
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
    ReflectionOps, ResolutionOps, SpanIntrinsicHost, StackOps, StaticsOps, StringIntrinsicHost,
    ThreadOps, ThreadingIntrinsicHost, TypedStackOps, UnsafeIntrinsicHost, VariableOps, VesBaseOps,
    VesInternals,
};

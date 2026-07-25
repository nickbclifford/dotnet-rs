//! # dotnet-vm-ops
//!
//! Core VES operation traits for the `dotnet-rs` Virtual Execution System (VES).
//!
//! Runtime execution data types are hosted in `dotnet-vm-data`.
//! Canonical imports should come from `dotnet_vm_data` directly.
//! This crate is intentionally separate from `dotnet-vm-data` to allow downstream
//! crates to depend on operation traits without depending on the VM implementation.
pub mod intrinsic_args;
mod macros;
pub mod ops;
pub mod prepared_call;

pub const NULL_REF_MSG: &str = "Object reference not set to an instance of an object.";

pub use dotnet_macros::trait_alias;
pub use ops::{
    ArgumentOps, DelegateIntrinsicHost, EvalStackOps, ExceptionContext, ExceptionOps, LoaderOps,
    LocalOps, MemoryOps, PInvokeContext, RawMemoryOps, ReflectionIntrinsicHost, ReflectionOps,
    ResolutionOps, SimdCapabilityOps, SimdIntrinsicHost, SpanIntrinsicHost, StackOps, StaticsOps,
    StringIntrinsicHost, ThreadOps, ThreadingIntrinsicHost, TypedStackOps, UnsafeIntrinsicHost,
    VariableOps, VesBaseOps, VesInternals,
};

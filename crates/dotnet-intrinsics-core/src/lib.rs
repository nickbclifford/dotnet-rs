//! Core intrinsic handlers shared across VM call paths.
//!
//! This crate contains foundational `#[dotnet_intrinsic]` handlers used by many
//! framework call sites, including `System.Array` operations, common numeric
//! and math helpers, and conservative `System.Runtime.Intrinsics` capability probes.
//!
//! ## Host Trait
//!
//! `dotnet-intrinsics-core` intentionally does **not** define an additional
//! `*IntrinsicHost` trait. Integration relies on the VM context implementing
//! `VesOps<'gc>` from `dotnet-vm` (with individual handlers constrained by the
//! specific `dotnet-vm-ops` traits they use, such as `TypedStackOps`,
//! `MemoryOps`, and `ExceptionOps`).
//!
//! See `docs/BUILD_TIME_CODE_GENERATION.md` for how `#[dotnet_intrinsic]`
//! handlers are discovered and wired into generated dispatch tables.
pub mod array_ops;
pub mod math;
pub mod simd;

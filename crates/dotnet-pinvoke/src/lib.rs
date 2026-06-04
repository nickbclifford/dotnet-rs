//! P/Invoke marshalling and native-call bridge for `dotnet-rs`.
//!
//! This crate resolves unmanaged libraries/symbols, marshals stack values into
//! native ABI arguments, invokes functions through `libffi`, and translates
//! outcomes back into VM-visible [`dotnet_vm_ops::StepResult`] values.
//!
//! Key public entry points:
//! - [`external_call`]: metadata-driven P/Invoke invocation path used by the VM.
//! - [`NativeLibraries`]: dynamic library loader/cache built on `libloading`,
//!   with optional sandbox policy enforcement.
//! - [`PInvokeSandbox`] and [`DefaultSandbox`]: allow/deny policy hooks for
//!   library and symbol resolution in runtime environments.
//! - `DenySandbox` (feature `fuzzing`): sandbox implementation that rejects
//!   all native calls so fuzz targets can exercise managed code paths without
//!   executing host-native functions.
//! - [`LAST_ERROR`]: per-call native OS error tracking slot updated by the
//!   invocation path for `GetLastError`/`errno`-style interop patterns.

mod call;
mod call_types;
mod loader;
mod marshal;
mod sandbox;

pub use call::{LAST_ERROR, external_call};
pub use loader::NativeLibraries;
#[cfg(feature = "fuzzing")]
pub use sandbox::DenySandbox;
pub use sandbox::{DefaultSandbox, PInvokeSandbox};

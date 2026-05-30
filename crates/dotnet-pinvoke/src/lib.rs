//! P/Invoke marshalling and native call bridge for dotnet-rs.

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

//! # dotnet-assemblies
//!
//! Assembly loading and metadata resolution for the dotnet-rs VM.
//! This crate handles finding, loading, and caching .NET assemblies from the file system.
pub mod ancestors;
#[cfg(test)]
mod drop_tests;
pub mod error;
pub mod loader;
pub mod resolution;
pub mod support;
pub mod validation;
#[cfg(test)]
mod validation_tests;
#[cfg(test)]
mod version_tests;

pub use ancestors::Ancestor;
pub use loader::{AssemblyLoader, SUPPORT_ASSEMBLY};
pub use resolution::find_dotnet_sdk_path;

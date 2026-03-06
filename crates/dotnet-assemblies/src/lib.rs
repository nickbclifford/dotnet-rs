//! # dotnet-assemblies
//!
//! Assembly loading and metadata resolution for the dotnet-rs VM.
//! This crate handles finding, loading, and caching .NET assemblies from the file system.
pub mod ancestors;
pub mod error;
pub mod loader;
pub mod resolution;
pub mod support;

pub use ancestors::Ancestor;
pub use loader::{AssemblyLoader, MetadataOwner, SUPPORT_ASSEMBLY};
pub use resolution::find_dotnet_sdk_path;

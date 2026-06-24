//! # dotnet-assemblies
//!
//! Assembly loading and metadata resolution for the dotnet-rs VM.
//! This crate handles finding, loading, and caching .NET assemblies from the file system.
pub mod ancestors;
#[cfg(test)]
mod drop_tests;
pub mod error;
pub mod host;
pub mod loader;
pub mod resolution;
pub mod support;
pub mod validation;
#[cfg(test)]
mod validation_tests;
#[cfg(test)]
mod version_tests;

pub use host::{
    AssemblyAssetInfo, DepsJson, DepsRuntimeTarget, FrameworkRef, HostError, LibraryInfo,
    RollForwardPolicy, RuntimeConfig, RuntimeOptions, TargetLibrary, derive_managed_probing_paths,
    derive_native_search_dirs, nuget_global_packages_dir, parse_deps_json, parse_runtimeconfig,
    resolve_framework_from_runtimeconfig, select_framework_version,
};
pub use loader::{AssemblyLoader, SUPPORT_ASSEMBLY, default_read_options};
pub use resolution::{find_dotnet_app_path, find_dotnet_sdk_path};

//! # Memory Management
//!
//! This module implements the VM's memory management system, including heap management,
//! object allocation, and low-level memory access.
//!
//! ## Submodules
//!
//! - **[`heap`]**: Manages object allocation and layout in the GC-managed heap.
//! - **[`access`]**: Provides safe and unsafe primitives for reading/writing values in memory.
//! - **[`ops`]**: Defines the [`ops::MemoryOps`] trait for unified memory operations.
pub mod access;
pub mod heap;
pub mod ops;
pub mod validation;

pub use access::{MemoryOwner, RawMemoryAccess};
pub use dotnet_utils::atomic::Atomic;
pub use heap::HeapManager;
pub use validation::{check_read_safety, has_ref_at};

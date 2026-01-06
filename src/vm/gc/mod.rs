//! Garbage collection subsystem for the dotnet-rs VM.
//!
//! This module contains all GC-related functionality:
//! - Arena-based memory management
//! - Cross-thread GC coordination (when multithreaded-gc feature is enabled)
//! - Runtime execution tracing for GC events

#[cfg(feature = "multithreaded-gc")]
pub mod arena;
pub mod coordinator;
pub mod tracer;

// Re-export main types
#[cfg(feature = "multithreaded-gc")]
pub use arena::THREAD_ARENA;
pub use coordinator::*;
pub use tracer::{TraceStats, Tracer};

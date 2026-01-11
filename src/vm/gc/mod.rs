//! Garbage collection subsystem for the dotnet-rs VM.
//!
//! This module contains all GC-related functionality:
//! - Arena-based memory management
//! - Cross-thread GC coordination (when multithreaded-gc feature is enabled)
//! - Runtime execution tracing for GC events
use gc_arena::Collect;

#[cfg(feature = "multithreaded-gc")]
pub mod arena;
pub mod coordinator;

#[cfg(feature = "multithreaded-gc")]
pub use arena::THREAD_ARENA;

pub use coordinator::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Collect)]
#[collect(require_static)]
pub enum GCHandleType {
    Weak = 0,
    WeakTrackResurrection = 1,
    Normal = 2,
    Pinned = 3,
}

impl From<i32> for GCHandleType {
    fn from(i: i32) -> Self {
        match i {
            0 => GCHandleType::Weak,
            1 => GCHandleType::WeakTrackResurrection,
            2 => GCHandleType::Normal,
            3 => GCHandleType::Pinned,
            _ => panic!("invalid GCHandleType: {}", i),
        }
    }
}

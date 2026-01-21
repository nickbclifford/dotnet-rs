//! Common types used across the VM subsystem.
//!
//! This module contains fundamental types that need to be shared
//! between different VM components without creating circular dependencies.
use gc_arena::{Collect, Mutation};

/// A handle to the GC mutation context.
pub type GCHandle<'gc> = &'gc Mutation<'gc>;

/// Type of GC handle, determines how the garbage collector treats the reference.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Collect)]
#[collect(require_static)]
pub enum GCHandleType {
    /// Weak reference that does not track resurrection
    Weak = 0,
    /// Weak reference that tracks resurrection (survives finalization)
    WeakTrackResurrection = 1,
    /// Normal (strong) GC handle
    Normal = 2,
    /// Pinned GC handle (prevents object from being moved by GC)
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

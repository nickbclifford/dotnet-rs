//! # dotnet-utils
//!
//! Shared utilities for the dotnet-rs project, including GC handles,
//! synchronization primitives, and memory alignment helpers.

use std::{
    fmt::{Debug, Formatter},
    mem::align_of,
};

pub mod atomic;
pub mod gc;
pub mod sync;

pub struct DebugStr(pub String);

impl Debug for DebugStr {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

pub fn is_ptr_aligned_to_field(ptr: *const u8, field_size: usize) -> bool {
    match field_size {
        1 => true, // u8 is always aligned
        2 => (ptr as usize).is_multiple_of(align_of::<u16>()),
        4 => (ptr as usize).is_multiple_of(align_of::<u32>()),
        8 => (ptr as usize).is_multiple_of(align_of::<u64>()),
        _ => false,
    }
}

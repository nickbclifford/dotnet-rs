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
pub mod newtypes;
pub mod sync;

pub use newtypes::{ArenaId, ArgumentIndex, ByteOffset, FieldIndex, LocalIndex, StackSlotIndex};

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
        _ => (ptr as usize).is_multiple_of(field_size),
    }
}

#[cfg(feature = "memory-validation")]
pub fn validate_alignment(ptr: *const u8, align: usize) {
    if !(ptr as usize).is_multiple_of(align) {
        panic!(
            "Alignment violation: pointer {:p} is not aligned to {}",
            ptr, align
        );
    }
}

#[cfg(not(feature = "memory-validation"))]
#[inline(always)]
pub fn validate_alignment(_ptr: *const u8, _align: usize) {}

pub trait BorrowScopeOps {
    fn enter_borrow_scope(&self);
    fn exit_borrow_scope(&self);
}

pub struct BorrowGuard {
    ctx: *const (dyn BorrowScopeOps + 'static),
}

impl BorrowGuard {
    pub fn new(ctx: &dyn BorrowScopeOps) -> Self {
        ctx.enter_borrow_scope();
        Self {
            ctx: unsafe {
                std::mem::transmute::<
                    *const dyn BorrowScopeOps,
                    *const (dyn BorrowScopeOps + 'static),
                >(ctx as *const dyn BorrowScopeOps)
            },
        }
    }
}

impl Drop for BorrowGuard {
    fn drop(&mut self) {
        unsafe { (*self.ctx).exit_borrow_scope() };
    }
}

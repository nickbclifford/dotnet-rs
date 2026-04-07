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

use std::marker::PhantomData;

/// Operations that track whether the current execution context is inside a
/// GC-critical scope where safepoint parking is not allowed.
pub trait BorrowScopeOps {
    /// Enter a GC-critical scope.
    fn enter_gc_scope(&self);
    /// Exit a GC-critical scope.
    fn exit_gc_scope(&self);
    /// Number of currently active GC-critical scopes in this context.
    fn active_gc_scope_depth(&self) -> usize;

    /// Returns a token proving the context is currently GC-safe (scope depth 0).
    fn gc_ready_token(&self) -> GcReadyToken<'_> {
        GcReadyToken::new(self)
    }
}

/// Proof that no GC-critical scope is active. Required for allocation.
pub struct GcReadyToken<'ctx> {
    owner: *const (),
    _marker: PhantomData<&'ctx mut ()>,
}

impl<'ctx> GcReadyToken<'ctx> {
    fn new<T: BorrowScopeOps + ?Sized>(ctx: &'ctx T) -> Self {
        let depth = ctx.active_gc_scope_depth();
        assert_eq!(
            depth, 0,
            "Cannot issue a GC-ready token while {} GC scope(s) are active",
            depth
        );
        Self {
            owner: std::ptr::from_ref(ctx).cast::<()>(),
            _marker: PhantomData,
        }
    }

    pub fn belongs_to(&self, ctx: &dyn BorrowScopeOps) -> bool {
        std::ptr::eq(self.owner, std::ptr::from_ref(ctx).cast::<()>())
    }
}

/// RAII guard for a GC-critical scope.
///
/// While this guard is alive, `check_gc_safe_point` should treat the current
/// context as not parkable.
pub struct GcScopeGuard<'ctx> {
    ctx: *const (dyn BorrowScopeOps + 'ctx),
    _marker: PhantomData<&'ctx ()>,
}

impl<'ctx> Drop for GcScopeGuard<'ctx> {
    fn drop(&mut self) {
        // SAFETY: `ctx` comes from `GcScopeGuard::enter` and remains valid for `'ctx`.
        // Drop runs at most once for this handle and balances `enter_gc_scope`.
        unsafe { (*self.ctx).exit_gc_scope() };
    }
}

impl<'ctx> GcScopeGuard<'ctx> {
    /// Enter a GC-critical scope.
    pub fn enter(ctx: &'ctx dyn BorrowScopeOps, token: GcReadyToken<'ctx>) -> Self {
        assert!(
            token.belongs_to(ctx),
            "GC-ready token must be issued by the same context that enters the GC scope",
        );
        ctx.enter_gc_scope();
        Self {
            ctx: ctx as *const (dyn BorrowScopeOps + 'ctx),
            _marker: PhantomData,
        }
    }

    /// Exit the GC-critical scope and return a new GC-ready token.
    pub fn exit(self) -> GcReadyToken<'ctx> {
        // SAFETY: `ctx` comes from `GcScopeGuard::enter` and is valid for `'ctx`.
        // `mem::forget(self)` prevents Drop from running, so this remains a single balanced exit.
        unsafe { (*self.ctx).exit_gc_scope() };
        let owner = self.ctx.cast::<()>();
        std::mem::forget(self);
        GcReadyToken {
            owner,
            _marker: PhantomData,
        }
    }
}

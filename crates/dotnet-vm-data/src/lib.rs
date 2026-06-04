//! # dotnet-vm-data
//!
//! Runtime data types shared across the VES implementation.
//!
//! This crate provides execution-step and call-frame data (`StepResult`, `MethodInfo`,
//! `MethodState`) plus stack and exception state models. It intentionally contains no
//! VM operation traits.
//! Operation traits live in `dotnet-vm-ops` to allow data-only consumers to avoid
//! pulling in the full operation trait surface.
//!
//! Canonical imports:
//! `dotnet_vm_data::{MethodInfo, MethodState, StepResult, CollectableMethodDescription}`.
use dotnet_types::{
    error::{ExecutionError, VmError},
    members::MethodDescription,
};
use dotnetdll::prelude::*;
use gc_arena::Collect;
use std::sync::Arc;

pub mod exceptions;
pub mod stack;

pub use exceptions::{
    ExceptionState, FilterState, HandlerAddress, ManagedException, ProtectedSection, SearchState,
    UnwindState, UnwindTarget,
};
pub use stack::{
    BasePointer, EvaluationStack, FrameStack, MulticastState, PinnedLocals, StackFrame,
};

/// Per-frame execution state for a single active method invocation.
///
/// This corresponds to the ECMA-335 I.12.3.2 method-state model. `ip` is the
/// current instruction pointer (index into `info_handle.instructions`).
/// `memory_pool` is the frame-local byte allocation pool used by `localloc`.
#[derive(Clone)]
pub struct MethodState {
    /// Current instruction pointer into `info_handle.instructions`.
    pub ip: usize,
    /// Parsed method metadata and instruction stream for this frame.
    pub info_handle: MethodInfo<'static>,
    /// Frame-local byte allocation pool used by `localloc`.
    pub memory_pool: Vec<u8>,
}

impl MethodState {
    pub fn new(info_handle: MethodInfo<'static>) -> Self {
        Self {
            ip: 0,
            info_handle,
            memory_pool: vec![],
        }
    }
}

unsafe impl<'gc> Collect<'gc> for MethodState {}

/// Fully parsed, cache-ready view of a resolved managed method.
///
/// `MethodInfo` bundles the decoded method signature, locals, exception metadata,
/// and instruction stream used by the dispatch loop. The `'a` lifetime is tied to
/// metadata-backed method structures allocated by `dotnetdll` and borrowed by the
/// runtime caches.
#[derive(Clone, Debug)]
pub struct MethodInfo<'a> {
    /// Decoded method signature (parameter list, generic context, and return type).
    pub signature: &'a ManagedMethod<MethodType>,
    /// Declared local variable slots from method metadata.
    pub locals: &'a [LocalVariable],
    /// Maximum evaluation stack depth declared for this method body.
    pub max_stack: usize,
    /// Parsed protected sections (`try`/handler regions) used during SEH dispatch.
    pub exceptions: Vec<Arc<exceptions::ProtectedSection>>,
    /// Decoded IL instruction stream for interpreter dispatch.
    pub instructions: &'a [Instruction],
    /// Method identity key used for cache lookup and intrinsic interception.
    pub source: MethodDescription,
    /// Whether the method is a static type initializer (`.cctor`).
    pub is_cctor: bool,
}

unsafe impl<'gc, 'a> Collect<'gc> for MethodInfo<'a> {}

/// GC-collectable wrapper around [`MethodDescription`] for storage in runtime state.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Collect)]
#[collect(require_static)]
pub struct CollectableMethodDescription(
    /// Method identity preserved across intrinsic/dispatch bookkeeping.
    pub MethodDescription,
);

#[must_use]
#[derive(Clone, Debug, PartialEq)]
pub enum StepResult {
    /// Instruction completed successfully. The dispatch loop advances IP by one.
    Continue,
    /// Branch to the given instruction index. The dispatch loop sets IP to this value.
    Jump(usize),
    /// A callee frame was pushed. The dispatch loop does not advance the current frame IP.
    FramePushed,
    /// The current frame returned. The dispatch loop pops the frame.
    Return,
    /// The current frame did not handle this exception. The dispatch loop propagates it.
    MethodThrew(ManagedException),
    /// An exception was thrown. The dispatch loop calls exception handling (`handle_exception`).
    Exception,
    /// Cooperative GC/thread yield point.
    Yield,
    /// Internal VM error surfaced to the dispatch loop.
    Error(VmError),
}

impl StepResult {
    pub fn internal_error(msg: impl Into<String>) -> Self {
        Self::Error(VmError::Execution(ExecutionError::InternalError(
            msg.into(),
        )))
    }

    pub fn not_implemented(msg: impl Into<String>) -> Self {
        Self::Error(VmError::Execution(ExecutionError::NotImplemented(
            msg.into(),
        )))
    }

    pub fn type_error(expected: impl Into<String>, actual: impl Into<String>) -> Self {
        Self::Error(VmError::Execution(ExecutionError::TypeMismatch {
            expected: expected.into(),
            actual: actual.into(),
        }))
    }
}

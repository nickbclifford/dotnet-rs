//! # dotnet-vm-data
//!
//! Runtime data types shared across the VES implementation.
//!
//! This crate provides execution-step and call-frame data (`StepResult`, `MethodInfo`,
//! `MethodState`) plus stack and exception state models. It intentionally contains no
//! VM operation traits.
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
pub use stack::{BasePointer, EvaluationStack, FrameStack, MulticastState, StackFrame};

// I.12.3.2
#[derive(Clone)]
pub struct MethodState {
    pub ip: usize,
    pub info_handle: MethodInfo<'static>,
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

#[derive(Clone, Debug)]
pub struct MethodInfo<'a> {
    pub signature: &'a ManagedMethod<MethodType>,
    pub locals: &'a [LocalVariable],
    pub exceptions: Vec<Arc<exceptions::ProtectedSection>>,
    pub instructions: &'a [Instruction],
    pub source: MethodDescription,
    pub is_cctor: bool,
}

unsafe impl<'gc, 'a> Collect<'gc> for MethodInfo<'a> {}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Collect)]
#[collect(require_static)]
pub struct CollectableMethodDescription(pub MethodDescription);

#[must_use]
#[derive(Clone, Debug, PartialEq)]
pub enum StepResult {
    Continue,                      // Advance IP
    Jump(usize),                   // Set IP to X
    FramePushed,                   // Do not advance IP (new frame active)
    Return,                        // Pop frame
    MethodThrew(ManagedException), // Exception unhandled in frame
    Exception,                     // Exception thrown, need to call handle_exception
    Yield,                         // GC/Thread yield
    Error(VmError),                // Internal VM error
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

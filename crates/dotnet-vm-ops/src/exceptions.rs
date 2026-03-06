//! Exception handling data types for the Virtual Execution System.
//!
//! This module contains the pure data structures for the exception handling state
//! machine. The actual exception handling logic (`parse`, `ExceptionHandlingSystem`)
//! lives in `dotnet-vm` and depends on the `ResolutionContext` and other VM internals.
use dotnet_types::generics::ConcreteType;
use dotnet_utils::DebugStr;
use dotnet_value::object::ObjectRef;
use gc_arena::Collect;
use std::{
    fmt::{self, Debug, Formatter},
    ops::Range,
};

/// Represents the location of an exception handler within the call stack.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Collect)]
#[collect(require_static)]
pub struct HandlerAddress {
    /// Index into the `CallStack::frames` vector.
    pub frame_index: usize,
    /// Index into the `MethodInfo::exceptions` vector for that frame.
    pub section_index: usize,
    /// Index into the `ProtectedSection::handlers` vector for that section.
    pub handler_index: usize,
}

#[derive(Clone, Copy, PartialEq, Collect, Debug)]
#[collect(no_drop)]
pub struct SearchState<'gc> {
    pub exception: ObjectRef<'gc>,
    pub cursor: HandlerAddress,
}

#[derive(Clone, Copy, PartialEq, Collect, Debug)]
#[collect(no_drop)]
pub struct FilterState<'gc> {
    pub exception: ObjectRef<'gc>,
    pub handler: HandlerAddress,
}

#[derive(Clone, Copy, PartialEq, Collect, Debug)]
#[collect(no_drop)]
pub struct UnwindState<'gc> {
    pub exception: Option<ObjectRef<'gc>>,
    pub target: UnwindTarget,
    pub cursor: HandlerAddress,
}

/// The destination of the current unwind operation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Collect)]
#[collect(require_static)]
pub enum UnwindTarget {
    /// Unwinding to a specific `catch` or `filter` handler.
    Handler(HandlerAddress),
    /// Unwinding because of a `leave` instruction to a specific IP.
    Instruction(usize),
}

/// The current state of the exception handling mechanism.
///
/// Exception handling in the CLI is a two-pass process:
/// 1. **Search Phase**: The runtime searches for a matching `catch` block or a `filter` that returns true.
/// 2. **Unwind Phase**: The runtime executes `finally` and `fault` blocks from the throw point
///    up to the found handler (or the target of a `leave` instruction).
#[derive(Clone, Copy, PartialEq, Collect, Debug)]
#[collect(no_drop)]
pub enum ExceptionState<'gc> {
    /// No exception is currently being processed.
    None,
    /// An exception has just been thrown. The next step is to begin the search phase.
    /// The boolean flag indicates whether the original stack trace should be preserved (for rethrow).
    Throwing(ObjectRef<'gc>, bool),
    /// Currently searching for a matching handler (catch or filter).
    Searching(SearchState<'gc>),
    /// A filter block is currently executing.
    Filtering(FilterState<'gc>),
    /// Currently in the unwind phase, executing `finally` and `fault` blocks.
    Unwinding(UnwindState<'gc>),
    /// A `finally`, `fault`, or `catch` handler is currently executing.
    ExecutingHandler(UnwindState<'gc>),
}

/// A human-readable representation of a managed exception, suitable for display outside the VM.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManagedException {
    pub type_name: String,
    pub message: Option<String>,
    pub stack_trace: Option<String>,
}

impl fmt::Display for ManagedException {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "Unhandled Exception: {}", self.type_name)?;
        if let Some(msg) = &self.message {
            write!(f, ": {}", msg)?;
        }
        if let Some(st) = &self.stack_trace {
            write!(f, "\nStack Trace:\n{}", st)?;
        }
        Ok(())
    }
}

/// A protected block of code (a `try` block) and its associated handlers.
#[derive(Clone)]
pub struct ProtectedSection {
    /// The range of instruction offsets protected by this section.
    pub instructions: Range<usize>,
    /// The exception handlers associated with this protected block.
    pub handlers: Vec<Handler>,
}

impl Debug for ProtectedSection {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_set()
            .entry(&DebugStr(format!("try {{ {:?} }}", self.instructions)))
            .entries(self.handlers.iter())
            .finish()
    }
}

/// An exception handler (catch, filter, finally, or fault).
#[derive(Clone)]
pub struct Handler {
    /// The range of instruction offsets for the handler's body.
    pub instructions: Range<usize>,
    /// The type of this handler.
    pub kind: HandlerKind,
}

impl Debug for Handler {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "{:?} {{ {:?} }}", self.kind, self.instructions)
    }
}

/// Specifies the behavior and trigger conditions of an exception handler.
#[derive(Clone)]
pub enum HandlerKind {
    /// Triggers only when the thrown exception is of the specified type (or a subtype).
    Catch(ConcreteType),
    /// Triggers when the filter clause at the specified offset returns true.
    Filter { clause_offset: usize },
    /// Always executes when exiting the protected block (whether normally or via exception).
    Finally,
    /// Executes only when exiting the protected block via an exception.
    Fault,
}

impl Debug for HandlerKind {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        use HandlerKind::*;
        match self {
            Catch(t) => write!(f, "catch({t:?})"),
            Filter { clause_offset } => write!(f, "filter({clause_offset}..)"),
            Finally => write!(f, "finally"),
            Fault => write!(f, "fault"),
        }
    }
}

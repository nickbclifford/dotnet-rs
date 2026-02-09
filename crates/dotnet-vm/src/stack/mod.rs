//! Stack management for the .NET virtual machine execution engine.
//!
//! This module provides the core stack infrastructure for executing .NET bytecode,
//! including the evaluation stack, call frame stack, and execution context management.
//!
//! # Architecture
//!
//! The stack system is organized into several key components:
//!
//! - **[`CallStack`]**: Top-level container for a thread's execution state, including:
//!   - Thread-local execution context ([`ThreadContext`])
//!   - Shared global state (assemblies, intrinsics, etc.)
//!   - Arena-local state (GC-managed heap objects)
//!   - Thread ID tracking
//!
//! - **[`VesContext`]**: The Virtual Execution System context, providing access to:
//!   - Evaluation stack operations (push/pop values)
//!   - Call frame management (method invocation/return)
//!   - Exception handling state
//!   - Memory and type operations via trait-based abstractions
//!
//! - **[`EvaluationStack`]**: Stack of values being computed during instruction execution.
//!   Stores [`StackValue`] instances (primitives, object references, managed pointers).
//!
//! - **[`FrameStack`]**: Stack of method call frames ([`StackFrame`]), tracking:
//!   - Current instruction pointer (IP)
//!   - Local variables and arguments
//!   - Method metadata and generic instantiation
//!   - Exception handling regions
//!
//! # Trait-Based Operations
//!
//! The [`ops`] module defines traits that decompose `VesContext` functionality:
//!
//! - [`StackOps`](ops::StackOps): Push/pop/dup operations on the evaluation stack
//! - [`CallOps`](ops::CallOps): Method invocation and frame management
//! - [`ExceptionOps`](ops::ExceptionOps): Exception throwing and handling
//! - [`ResolutionOps`](ops::ResolutionOps): Type and method resolution
//! - [`VesOps`](ops::VesOps): Unified trait combining all operations
//!
//! These traits enable instruction handlers and intrinsics to depend only on the
//! operations they need, rather than coupling to the full `VesContext` struct.
//!
//! # Garbage Collection
//!
//! Stack structures implement the `gc_arena::Collect` trait for tracing GC-managed
//! references. The [`CallStack`] type is the root of the GC arena for a thread's
//! execution context.
//!
//! # Example
//!
//! ```ignore
//! let mut call_stack = CallStack::new(shared, local);
//! let mut ctx = call_stack.ves_context();
//!
//! // Push a value onto the evaluation stack
//! ctx.push_i32(42);
//!
//! // Call a method (simplified)
//! ctx.invoke_method(method_ref, args)?;
//!
//! // Pop result
//! let result = ctx.pop_i32()?;
//! ```
use crate::{
    MethodState, StepResult,
    dispatch::ExecutionEngine,
    exceptions::ExceptionState,
    state::{ArenaLocalState, SharedGlobalState},
    sync::{Arc, MutexGuard, Ordering},
    tracer::Tracer,
};
use dotnet_utils::gc::GCHandle;
use dotnet_value::StackValue;
use gc_arena::{Arena, Collect, Rootable};
use std::cell::Cell;

pub mod context;
pub mod evaluation_stack;
pub mod frames;
pub mod ops;

pub use context::*;
pub use evaluation_stack::*;
pub use frames::*;

pub struct CallStack<'gc, 'm> {
    pub execution: ThreadContext<'gc, 'm>,
    pub shared: Arc<SharedGlobalState<'m>>,
    pub local: ArenaLocalState<'gc>,
    pub thread_id: Cell<u64>,
}

// SAFETY: `CallStack` correctly traces all GC-managed fields (`execution`, `local`, and
// `shared.statics`) in its `trace` implementation. The `thread_id` field is not GC-managed
// and does not need tracing. This implementation is safe because it delegates to the
// `Collect` implementations of its components, which are themselves safe.
unsafe impl<'gc, 'm: 'gc> Collect for CallStack<'gc, 'm> {
    fn trace(&self, cc: &gc_arena::Collection) {
        // NOTE: tracing_id is managed by execute_gc_command_for_current_thread,
        // NOT here. Setting/clearing it here would interfere with gc-arena's
        // deferred work list processing (StackValue traces happen AFTER this returns).
        self.execution.trace(cc);
        self.local.trace(cc);
        self.shared.statics.trace(cc);
    }
}

pub type GCArena = Arena<Rootable!['gc => ExecutionEngine<'gc, 'static>]>;

impl<'gc, 'm: 'gc> CallStack<'gc, 'm> {
    pub fn new(shared: Arc<SharedGlobalState<'m>>, local: ArenaLocalState<'gc>) -> Self {
        Self {
            execution: ThreadContext {
                evaluation_stack: EvaluationStack::new(),
                frame_stack: FrameStack::new(),
                exception_mode: ExceptionState::None,
                original_ip: 0,
                original_stack_height: 0,
            },
            shared,
            local,
            thread_id: Cell::new(0),
        }
    }

    pub fn ves_context(&mut self) -> VesContext<'_, 'gc, 'm> {
        VesContext {
            evaluation_stack: &mut self.execution.evaluation_stack,
            frame_stack: &mut self.execution.frame_stack,
            shared: &self.shared,
            local: &mut self.local,
            exception_mode: &mut self.execution.exception_mode,
            thread_id: &self.thread_id,
            original_ip: &mut self.execution.original_ip,
            original_stack_height: &mut self.execution.original_stack_height,
        }
    }

    pub(super) fn get_slot(&self, index: usize) -> StackValue<'gc> {
        self.execution.evaluation_stack.stack[index].clone()
    }

    pub fn handle_return(&mut self, gc: GCHandle<'gc>) -> StepResult {
        self.ves_context().handle_return(gc)
    }

    pub fn return_frame(&mut self, gc: GCHandle<'gc>) {
        let tracer_enabled = self.tracer_enabled();
        self.execution.frame_stack.return_frame(
            gc,
            &mut self.execution.evaluation_stack,
            &self.shared,
            &self.local.heap,
            tracer_enabled,
        );
    }

    pub fn unwind_frame(&mut self, gc: GCHandle<'gc>) {
        self.execution.frame_stack.unwind_frame(
            gc,
            &mut self.execution.evaluation_stack,
            &self.shared,
            &self.local.heap,
        );
    }

    pub fn current_frame(&self) -> &StackFrame<'gc, 'm> {
        self.execution.frame_stack.current_frame()
    }

    pub fn current_frame_mut(&mut self) -> &mut StackFrame<'gc, 'm> {
        self.execution.frame_stack.current_frame_mut()
    }

    pub fn state(&self) -> &MethodState<'m> {
        self.execution.frame_stack.state()
    }

    pub fn state_mut(&mut self) -> &mut MethodState<'m> {
        self.execution.frame_stack.state_mut()
    }

    pub fn branch(&mut self, target: usize) {
        crate::vm_trace_branch!(self, "BR", target, true);
        self.execution.frame_stack.branch(target);
    }

    pub fn conditional_branch(&mut self, condition: bool, target: usize) -> bool {
        crate::vm_trace_branch!(self, "BR_COND", target, condition);
        self.execution
            .frame_stack
            .conditional_branch(condition, target)
    }

    pub fn increment_ip(&mut self) {
        self.execution.frame_stack.increment_ip();
    }

    pub fn tracer_enabled(&self) -> bool {
        self.shared.tracer_enabled.load(Ordering::Relaxed)
    }

    pub fn indent(&self) -> usize {
        self.execution.frame_stack.len().saturating_sub(1)
    }

    pub fn tracer(&self) -> MutexGuard<'_, Tracer> {
        self.shared.tracer.lock()
    }
}

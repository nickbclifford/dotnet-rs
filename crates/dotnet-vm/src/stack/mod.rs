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
//! - [`StackOps`](StackOps): Push/pop/dup operations on the evaluation stack
//! - [`CallOps`](CallOps): Method invocation and frame management
//! - [`ExceptionOps`](ExceptionOps): Exception throwing and handling
//! - [`ResolutionOps`](ResolutionOps): Type and method resolution
//! - [`VesOps`](VesOps): Unified trait combining all operations
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
//! let mut ctx = call_stack.ves_context(gc);
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
    state::{ArenaLocalState, SharedGlobalState},
    sync::{Arc, Ordering},
};
use dotnet_tracer::Tracer;
use dotnet_utils::gc::GCHandle;
use dotnet_value::{StackValue, object::HeapStorage};
use gc_arena::{Arena, Collect, Gc, Mutation, Rootable, collect::Trace, metrics::Metrics};
use std::cell::Cell;

mod call_ops_impl;
pub mod context;
mod exception_ops_impl;
mod memory_ops_impl;
pub mod ops;
mod raw_memory_ops_impl;
mod reflection_ops_impl;
mod resolution_ops_impl;
mod stack_ops_impl;

pub use context::{BasePointer, PinnedLocals, ThreadContext, VesContext};
pub use dotnet_vm_ops::{EvaluationStack, ExceptionState, FrameStack, StackFrame};
pub use ops::{
    ArgumentOps, CallOps, EvalStackOps, ExceptionContext, ExceptionOps, IntrinsicDispatchOps,
    LoaderOps, LocalOps, MemoryOps, PInvokeContext, RawMemoryOps, ReflectionLookupOps,
    ReflectionOps, ResolutionOps, StackOps, StaticsOps, ThreadOps, TypedStackOps, VariableOps,
    VesBaseOps, VesInternals, VesOps, VmCallOps, VmExceptionContext, VmLoaderOps, VmPInvokeContext,
    VmRawMemoryOps, VmReflectionOps, VmResolutionOps, VmStackOps, VmStaticsOps,
};

pub struct CallStack<'gc> {
    pub execution: ThreadContext<'gc>,
    pub shared: Arc<SharedGlobalState>,
    pub local: ArenaLocalState<'gc>,
    pub thread_id: Cell<dotnet_utils::ArenaId>,
    #[cfg(feature = "multithreading")]
    pub arena: dotnet_utils::gc::ArenaHandle,
}

// SAFETY: `CallStack` correctly traces all GC-managed fields (`execution`, `local`, and
// `shared.statics`) in its `trace` implementation. The `thread_id` and `arena` fields are not GC-managed
// and do not need tracing. This implementation is safe because it delegates to the
// `Collect` implementations of its components, which are themselves safe.
unsafe impl<'gc> Collect<'gc> for CallStack<'gc> {
    fn trace<Tr: Trace<'gc>>(&self, cc: &mut Tr) {
        // NOTE: tracing_id is managed by execute_gc_command_for_current_thread,
        // NOT here. Setting/clearing it here would interfere with gc-arena's
        // deferred work list processing (StackValue traces happen AFTER this returns).
        self.execution.trace(cc);
        self.local.trace(cc);
        self.shared.statics.trace(cc);
    }
}

pub struct GCArenaRoot;
impl<'gc> Rootable<'gc> for GCArenaRoot {
    type Root = ExecutionEngine<'gc>;
}

pub struct GCArena {
    arena: Arena<GCArenaRoot>,
}

impl GCArena {
    pub fn new<F>(f: F) -> Self
    where
        F: for<'gc> FnOnce(&'gc Mutation<'gc>) -> <GCArenaRoot as Rootable<'gc>>::Root,
    {
        Self {
            arena: Arena::<GCArenaRoot>::new(f),
        }
    }

    pub fn mutate<R>(
        &self,
        f: impl for<'gc> FnOnce(&'gc Mutation<'gc>, &<GCArenaRoot as Rootable<'gc>>::Root) -> R,
    ) -> R {
        self.arena.mutate(f)
    }

    pub fn mutate_root<R>(
        &mut self,
        f: impl for<'gc> FnOnce(&'gc Mutation<'gc>, &mut <GCArenaRoot as Rootable<'gc>>::Root) -> R,
    ) -> R {
        self.arena.mutate_root(f)
    }

    pub fn finish_marking(&mut self) -> Option<gc_arena::arena::MarkedArena<'_, GCArenaRoot>> {
        self.arena.finish_marking()
    }

    pub fn finish_cycle(&mut self) {
        self.arena.finish_cycle()
    }

    pub fn collect_debt(&mut self) {
        self.arena.collect_debt()
    }

    pub fn metrics(&self) -> &Metrics {
        self.arena.metrics()
    }
}

impl<'gc> CallStack<'gc> {
    pub fn new(shared: Arc<SharedGlobalState>, local: ArenaLocalState<'gc>) -> Self {
        Self {
            execution: ThreadContext {
                evaluation_stack: EvaluationStack::new(),
                frame_stack: FrameStack::new(),
                exception_mode: ExceptionState::None,
                current_intrinsic: None,
                original_ip: 0,
                original_stack_height: dotnet_utils::StackSlotIndex(0),
                call_args_buffer: Vec::new(),
            },
            shared,
            local,
            thread_id: Cell::new(dotnet_utils::ArenaId::INVALID),
            #[cfg(feature = "multithreading")]
            arena: dotnet_utils::gc::ArenaHandle::new(dotnet_utils::ArenaId::INVALID),
        }
    }

    #[cfg(feature = "multithreading")]
    /// Returns the arena inner handle with the GC lifetime.
    ///
    /// # Safety
    ///
    /// The caller must ensure that the arena handle lives as long as the GC arena.
    /// In our case, `CallStack` is stored in the arena root, so it's safe.
    pub unsafe fn arena_inner_gc(&self) -> &'gc dotnet_utils::gc::ArenaHandleInner {
        unsafe { &*(self.arena.as_inner() as *const _) }
    }

    pub fn ves_context(&mut self, gc: GCHandle<'gc>) -> VesContext<'_, 'gc> {
        VesContext {
            gc,
            evaluation_stack: &mut self.execution.evaluation_stack,
            frame_stack: &mut self.execution.frame_stack,
            shared: &self.shared,
            local: &mut self.local,
            exception_mode: &mut self.execution.exception_mode,
            current_intrinsic: &mut self.execution.current_intrinsic,
            thread_id: &self.thread_id,
            original_ip: &mut self.execution.original_ip,
            original_stack_height: &mut self.execution.original_stack_height,
            call_args_buffer: &mut self.execution.call_args_buffer,
        }
    }

    pub(super) fn get_slot(&self, index: usize) -> StackValue<'gc> {
        self.execution.evaluation_stack.stack[index].clone()
    }

    pub fn handle_return(&mut self, gc: GCHandle<'gc>) -> StepResult {
        self.ves_context(gc).handle_return()
    }

    pub fn return_frame(&mut self, gc: GCHandle<'gc>) -> StepResult {
        self.ves_context(gc).return_frame()
    }

    pub fn unwind_frame(&mut self, gc: GCHandle<'gc>) {
        self.ves_context(gc).unwind_frame()
    }

    pub fn current_frame(&self) -> &StackFrame<'gc> {
        self.execution.frame_stack.current_frame()
    }

    pub fn current_frame_mut(&mut self) -> &mut StackFrame<'gc> {
        self.execution.frame_stack.current_frame_mut()
    }

    pub fn state(&self) -> &MethodState {
        self.execution.frame_stack.state()
    }

    pub fn state_mut(&mut self) -> &mut MethodState {
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

    pub fn tracer(&self) -> &Tracer {
        &self.shared.tracer
    }
}

impl<'gc: 'gc> CallStack<'gc> {
    // Tracer-integrated dump methods for comprehensive state capture
    pub fn trace_dump_stack(&self) {
        if !self.tracer_enabled() {
            return;
        }

        let contents: Vec<_> = self.execution.evaluation_stack.stack
            [..self.execution.evaluation_stack.stack.len()]
            .iter()
            .enumerate()
            .map(|(i, _)| format!("{:?}", self.get_slot(i)))
            .collect();

        let mut markers = Vec::new();
        for (i, frame) in self.execution.frame_stack.frames.iter().enumerate() {
            let base = &frame.base;
            markers.push((base.stack.as_usize(), format!("Stack base of frame #{}", i)));
            if base.locals != base.stack {
                markers.push((
                    base.locals.as_usize(),
                    format!("Locals base of frame #{}", i),
                ));
            }
            markers.push((
                base.arguments.as_usize(),
                format!("Arguments base of frame #{}", i),
            ));
        }

        self.tracer().dump_stack_state(&contents, &markers);
    }

    pub fn trace_dump_frames(&self) {
        if !self.tracer_enabled() {
            return;
        }

        let tracer = self.tracer();
        for (idx, frame) in self.execution.frame_stack.frames.iter().enumerate() {
            let method_name = format!("{:?}", frame.state.info_handle.source);
            tracer.dump_frame_state(
                idx,
                &method_name,
                frame.state.ip,
                frame.base.arguments.as_usize(),
                frame.base.locals.as_usize(),
                frame.base.stack.as_usize(),
                frame.stack_height.as_usize(),
            );
        }
    }

    pub fn trace_dump_heap(&self) {
        if !self.tracer_enabled() {
            return;
        }

        let objects: Vec<_> = self.local.heap.snapshot_objects();
        let tracer = self.tracer();
        tracer.dump_heap_snapshot_start(objects.len());

        for obj in objects {
            let Some(ptr) = obj.0 else {
                continue;
            };
            let raw_ptr = Gc::<_>::as_ptr(ptr) as *const _ as usize;
            let borrowed = ptr.borrow();
            match &borrowed.storage {
                HeapStorage::Obj(o) => {
                    let details = format!("{:?}", o);
                    tracer.dump_heap_object(raw_ptr, "Object", &details);
                }
                HeapStorage::Vec(v) => {
                    let details = format!("{:?}", v);
                    tracer.dump_heap_object(raw_ptr, "Vector", &details);
                }
                HeapStorage::Str(s) => {
                    let details = format!("{:?}", s);
                    tracer.dump_heap_object(raw_ptr, "String", &details);
                }
                HeapStorage::Boxed(b) => {
                    let details = format!("{:?}", b);
                    tracer.dump_heap_object(raw_ptr, "Boxed", &details);
                }
            }
        }

        tracer.dump_heap_snapshot_end();
    }

    pub fn trace_dump_statics(&self) {
        if !self.tracer_enabled() {
            return;
        }

        let s = &self.shared.statics;
        let debug_str = format!("{:#?}", s);
        self.tracer().dump_statics_snapshot(&debug_str);
    }

    pub fn trace_dump_gc_stats(&self) {
        if !self.tracer_enabled() {
            return;
        }

        self.tracer().dump_gc_stats(
            self.local.heap.finalization_queue.borrow().len(),
            self.local.heap.pending_finalization.borrow().len(),
            self.local.heap.pinned_objects.borrow().len(),
            self.local.heap.gchandles.borrow().len(),
            self.local.heap.live_object_count(),
        );
    }

    pub fn trace_dump_runtime_metrics(&self) {
        if !self.tracer_enabled() {
            return;
        }

        self.tracer().dump_runtime_metrics(&self.shared.metrics);
    }

    /// Captures a complete snapshot of all runtime state to the tracer
    pub fn trace_full_state(&self) {
        if !self.tracer_enabled() {
            return;
        }

        self.tracer().dump_full_state_header();
        self.trace_dump_frames();
        self.trace_dump_stack();
        self.trace_dump_heap();
        self.trace_dump_statics();
        self.trace_dump_gc_stats();
        self.trace_dump_runtime_metrics();
        self.tracer().flush();
    }
}

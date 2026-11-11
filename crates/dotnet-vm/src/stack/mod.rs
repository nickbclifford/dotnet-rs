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
//! - [`VmCallOps`](VmCallOps): Method invocation and frame management
//! - [`ExceptionOps`](ExceptionOps): Exception throwing and handling
//! - [`ResolutionOps`](ResolutionOps): Type and method resolution
//! - [`VesOps`](VesOps): Unified trait combining all operations
//!
//! These traits enable instruction handlers and intrinsics to depend only on the
//! operations they need, rather than coupling to the full `VesContext` struct.
//!
//! # Garbage Collection
//!
//! Stack structures implement `gc_arena::Collect`. The arena root is
//! [`ExecutionEngine`](crate::dispatch::ExecutionEngine) (see [`GCArenaRoot`]), which owns the
//! thread's [`CallStack`] and traces it.
//!
//! Instruction handlers reach this state through a [`VesContext`] borrowed from the
//! `CallStack`; see the [`ops`] traits for the operations it exposes.
use crate::{
    MethodState, StepResult,
    dispatch::ExecutionEngine,
    state::{ArenaLocalState, SharedGlobalState},
    sync::{Arc, Ordering},
};
use dotnet_tracer::Tracer;
use dotnet_types::generics::GenericLookup;
use dotnet_utils::gc::GCHandle;
use dotnet_value::{
    StackValue,
    object::{HeapStorage, ObjectRef},
};
use gc_arena::{Arena, Collect, Gc, Mutation, Rootable, collect::Trace, metrics::Metrics};
use std::cell::Cell;

mod call_ops_impl;
pub mod context;
mod context_ops;
mod exception_ops_impl;
mod managed_ptr_decode_cache;
mod memory_ops_impl;
pub(crate) mod native_effects;
pub mod ops;
mod raw_memory_ops_impl;
mod reflection_ops_impl;
mod resolution_ops_impl;
mod stack_ops_impl;
mod synchronized;

pub use context::{BasePointer, PinnedLocals, ThreadContext, VesContext};
pub use dotnet_vm_data::{EvaluationStack, ExceptionState, FrameStack, StackFrame};
pub use managed_ptr_decode_cache::{HeapManagedPtrDecodeCache, HeapManagedPtrDecodeCacheStats};
pub use ops::{
    ArgumentOps, CallArgumentBufferOps, EvalStackOps, ExceptionContext, ExceptionOps,
    IntrinsicDispatchOps, LoaderOps, LocalOps, MemoryOps, PInvokeContext, RawMemoryOps,
    ReflectionLookupOps, ReflectionOps, ResolutionOps, StackOps, StaticsOps, ThreadOps,
    TypedStackOps, VariableOps, VesBaseOps, VesInternals, VesOps, VmCallOps, VmExceptionContext,
    VmLoaderOps, VmPInvokeContext, VmRawMemoryOps, VmReflectionOps, VmResolutionOps, VmStackOps,
    VmStaticsOps,
};

pub(crate) fn receiver_object_generics(receiver: ObjectRef<'_>) -> GenericLookup {
    receiver.as_heap_storage(|storage| match storage {
        HeapStorage::Obj(instance) => instance.generics.clone(),
        HeapStorage::Boxed(instance) => instance.generics.clone(),
        HeapStorage::Vec(_) | HeapStorage::Str(_) => GenericLookup::default(),
    })
}

pub struct CallStack<'gc> {
    pub execution: ThreadContext<'gc>,
    pub(crate) parked_finalizer_caller: Option<ThreadContext<'gc>>,
    pub shared: Arc<SharedGlobalState>,
    pub local: ArenaLocalState<'gc>,
    pub(crate) heap_managed_ptr_decode_cache: HeapManagedPtrDecodeCache<'gc>,
    pub thread_id: Cell<dotnet_utils::ArenaId>,
    #[cfg(feature = "multithreading")]
    /// Original registry identity of this arena, set at Executor construction.
    pub(crate) arena_identity: Option<dotnet_utils::gc::ArenaIdentity>,
    #[cfg(feature = "multithreading")]
    pub arena: dotnet_utils::gc::ArenaHandle,
}

// SAFETY: F5.TracesEveryGcRef — `trace` visits every field that can hold a `Gc` handle of
// this arena: `execution` and `parked_finalizer_caller` (frames and evaluation stack), `heap_managed_ptr_decode_cache`
// (its cached handles stay live until `GCArena::finish_cycle` invalidates the cache),
// `local`, `shared.statics` and the shared reflection object table. The remaining fields hold no `Gc`
// handles: `thread_id`, `arena_identity` and `arena` are plain identifiers and registry
// handles, and the other `SharedGlobalState` fields hold `'static` metadata, caches and
// runtime services.
unsafe impl<'gc> Collect<'gc> for CallStack<'gc> {
    fn trace<Tr: Trace<'gc>>(&self, cc: &mut Tr) {
        // NOTE: tracing_id is managed by execute_gc_command_for_current_thread,
        // NOT here. Setting/clearing it here would interfere with gc-arena's
        // deferred work list processing (StackValue traces happen AFTER this returns).
        self.execution.trace(cc);
        self.parked_finalizer_caller.trace(cc);
        self.heap_managed_ptr_decode_cache.trace(cc);
        self.local.trace(cc);
        self.shared.statics.trace(cc);
        #[cfg(feature = "multithreading")]
        self.shared.reflection_registry.trace(cc);
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
        self.arena.finish_cycle();
        self.cycle_completed();
    }

    /// Advances a single-threaded arena's collection without ever sweeping an unfinalized cycle.
    ///
    /// The finalization queue holds finalizable objects weakly, so `finalize_check` must see every
    /// cycle at the fully marked point. Otherwise the sweep frees dead objects the queue still
    /// points to. gc-arena's `collect_debt` passes through that point on its own, so every cycle
    /// here stops when fully marked, runs `finalize_arena`, and only then starts sweeping.
    /// `cycle_debt` finishes a sweep without starting the next cycle's marking.
    #[cfg(not(feature = "multithreading"))]
    pub fn collect_single_threaded(&mut self, work: crate::gc::CollectionWork) {
        use gc_arena::arena::CollectionPhase;

        match work {
            crate::gc::CollectionWork::Debt => {
                if self.arena.collection_phase() == CollectionPhase::Sweeping {
                    self.arena.cycle_debt();
                    if self.arena.collection_phase() == CollectionPhase::Sleeping {
                        self.cycle_completed();
                    }
                } else if let Some(marked) = self.arena.mark_debt() {
                    crate::gc::finalize_arena(marked);
                    self.start_sweeping();
                }
            }
            crate::gc::CollectionWork::Full => {
                if self.arena.collection_phase() == CollectionPhase::Sweeping {
                    // Finish the cycle in progress; it was finalized before it began sweeping.
                    // The requested full collection still needs a fresh cycle.
                    self.finish_cycle();
                }
                if let Some(marked) = self.arena.finish_marking() {
                    crate::gc::finalize_arena(marked);
                    self.start_sweeping();
                }
                self.finish_cycle();
            }
        }
    }

    /// Marks what finalization resurrected, then moves the arena from fully marked to sweeping.
    #[cfg(not(feature = "multithreading"))]
    fn start_sweeping(&mut self) {
        // Resurrection can make the arena gray again. The arena is not sweeping yet, so
        // `finish_marking` marks the rest and returns the fully marked arena.
        if let Some(marked) = self.arena.finish_marking() {
            marked.start_sweeping();
        }
    }

    fn cycle_completed(&mut self) {
        // A completed cycle is the single collection-epoch boundary for every
        // per-CallStack decode cache. Entries remain traced during the cycle,
        // then must be discarded before an old serialized handle address can be
        // reused in the next mutator epoch.
        self.arena.mutate_root(|_, engine| {
            engine
                .stack
                .heap_managed_ptr_decode_cache
                .invalidate_after_collection();
        });
    }

    pub fn metrics(&self) -> &Metrics {
        self.arena.metrics()
    }
}

impl<'gc> CallStack<'gc> {
    pub fn new(shared: Arc<SharedGlobalState>, local: ArenaLocalState<'gc>) -> Self {
        Self {
            execution: ThreadContext::default(),
            parked_finalizer_caller: None,
            shared,
            local,
            heap_managed_ptr_decode_cache: HeapManagedPtrDecodeCache::default(),
            thread_id: Cell::new(dotnet_utils::ArenaId::INVALID),
            #[cfg(feature = "multithreading")]
            arena_identity: None,
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
        // SAFETY: F1.GcHandleRooted — The rooted CallStack retains its arena handle for
        // the GC lifetime; this shared reborrow narrows the interior mutable representation
        // to its immutable arena view.
        unsafe { &*(self.arena.as_inner() as *const _) }
    }

    pub fn ves_context(&mut self, gc: GCHandle<'gc>) -> VesContext<'_, 'gc> {
        VesContext {
            gc,
            evaluation_stack: &mut self.execution.evaluation_stack,
            frame_stack: &mut self.execution.frame_stack,
            parked_finalizer_caller: &self.parked_finalizer_caller,
            shared: &self.shared,
            local: &mut self.local,
            exception_mode: &mut self.execution.exception_mode,
            current_intrinsic: &mut self.execution.current_intrinsic,
            thread_id: &self.thread_id,
            original_ip: &mut self.execution.original_ip,
            original_stack_height: &mut self.execution.original_stack_height,
            continuation: &mut self.execution.continuation,
            call_state: &mut self.execution.call_state,
            heap_managed_ptr_decode_cache: &mut self.heap_managed_ptr_decode_cache,
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
            let raw_ptr = Gc::<_>::as_ptr(ptr).addr();
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

        let stats = self.shared.get_cache_stats();
        self.tracer()
            .dump_runtime_metrics(&self.shared.metrics, stats);
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

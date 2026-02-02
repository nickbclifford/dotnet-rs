use crate::vm_trace_instruction;
use dotnet_types::{
    generics::GenericLookup,
    members::{FieldDescription, MethodDescription},
    TypeDescription,
};
use dotnet_utils::gc::GCHandle;
use dotnetdll::prelude::*;
use std::sync::OnceLock;

pub mod arithmetic;
pub mod calls;
pub mod comparisons;
pub mod conversions;
pub mod exceptions;
pub mod flow;
pub mod memory;
pub mod objects;
pub mod reflection;
pub mod stack_ops;

use super::{
    exceptions::ExceptionState,
    intrinsics::*,
    layout::{type_layout, LayoutFactory},
    threading::ThreadManagerOps,
    CallStack, MethodInfo, ResolutionContext, StepResult,
};

pub type InstructionHandler =
    for<'gc, 'm> fn(GCHandle<'gc>, &mut CallStack<'gc, 'm>, &Instruction) -> StepResult;

pub struct InstructionEntry {
    pub name: &'static str,
    pub handler: InstructionHandler,
}

inventory::collect!(InstructionEntry);

pub struct InstructionRegistry;

type InstructionTable = [Option<InstructionHandler>; Instruction::VARIANT_COUNT];
static INSTRUCTION_TABLE: OnceLock<InstructionTable> = OnceLock::new();

impl InstructionRegistry {
    pub fn dispatch<'gc, 'm>(
        gc: GCHandle<'gc>,
        stack: &mut CallStack<'gc, 'm>,
        instr: &Instruction,
    ) -> Option<StepResult> {
        let table = INSTRUCTION_TABLE.get_or_init(|| {
            let mut t = [None; Instruction::VARIANT_COUNT];
            for entry in inventory::iter::<InstructionEntry> {
                if let Some(opcode) = Instruction::opcode_from_name(entry.name) {
                    t[opcode] = Some(entry.handler);
                }
            }
            t
        });
        let handler = table[instr.opcode()]?;
        Some(handler(gc, stack, instr))
    }
}

/// Check if a method is an intrinsic (with caching).
/// This is called on every method invocation, so caching is critical for performance.
pub fn is_intrinsic_cached(stack: &CallStack, method: MethodDescription) -> bool {
    // Check cache first
    if let Some(cached) = stack.shared.caches.intrinsic_cache.get(&method) {
        stack.shared.metrics.record_intrinsic_cache_hit();
        return *cached;
    }

    // Cache miss - perform check
    stack.shared.metrics.record_intrinsic_cache_miss();
    let result = is_intrinsic(
        method,
        stack.shared.loader,
        &stack.shared.caches.intrinsic_registry,
    );

    // Cache the result
    stack.shared.caches.intrinsic_cache.insert(method, result);
    result
}

/// Check if a field is an intrinsic (with caching).
pub fn is_intrinsic_field_cached(stack: &CallStack, field: FieldDescription) -> bool {
    // Check cache first
    if let Some(cached) = stack.shared.caches.intrinsic_field_cache.get(&field) {
        stack.shared.metrics.record_intrinsic_field_cache_hit();
        return *cached;
    }

    // Cache miss - perform check
    stack.shared.metrics.record_intrinsic_field_cache_miss();
    let result = is_intrinsic_field(
        field,
        stack.shared.loader,
        &stack.shared.caches.intrinsic_registry,
    );

    // Cache the result
    stack
        .shared
        .caches
        .intrinsic_field_cache
        .insert(field, result);
    result
}

impl<'gc, 'm: 'gc> CallStack<'gc, 'm> {
    /// Check for GC safe point and suspend if stop-the-world is requested.
    ///
    /// This method should be called:
    /// - Before long-running operations (large allocations, reflection, etc.)
    /// - Periodically during loops with many iterations
    /// - At method entry/exit points (already happens via executor loop)
    ///
    /// Note: The current implementation checks on every instruction via the executor loop.
    /// For better performance, future optimization could check only on backward branches
    /// (detected by IP decreasing), method entries, and explicit calls in long operations.
    #[inline]
    pub fn check_gc_safe_point(&self) {
        let thread_manager = &self.shared.thread_manager;
        if thread_manager.is_gc_stop_requested() {
            let managed_id = self.thread_id.get();
            if managed_id != 0 {
                #[cfg(feature = "multithreaded-gc")]
                thread_manager.safe_point(managed_id, &self.shared.gc_coordinator);
                #[cfg(not(feature = "multithreaded-gc"))]
                thread_manager.safe_point(managed_id, &Default::default());
            }
        }
    }

    fn find_generic_method(&self, source: &MethodSource) -> (MethodDescription, GenericLookup) {
        let ctx = self.current_context();

        let mut new_lookup = ctx.generics.clone();

        let method = match source {
            MethodSource::User(u) => *u,
            MethodSource::Generic(g) => {
                new_lookup.method_generics =
                    g.parameters.iter().map(|t| ctx.make_concrete(t)).collect();
                g.base
            }
        };

        if let UserMethod::Reference(r) = method {
            if let MethodReferenceParent::Type(t) = &ctx.resolution[r].parent {
                let parent = ctx.make_concrete(t);
                if let BaseType::Type {
                    source: TypeSource::Generic { parameters, .. },
                    ..
                } = parent.get()
                {
                    new_lookup.type_generics = parameters.clone();
                }
            }
        }

        (ctx.locate_method(method, ctx.generics), new_lookup)
    }

    /// Dispatches a method call through the appropriate mechanism.
    ///
    /// Call priority order:
    /// 1. Intrinsics (including DirectIntercept) - VM implementation always wins
    /// 2. P/Invoke - External native methods
    /// 3. Managed - CIL bytecode execution
    ///
    /// Note: DirectIntercept intrinsics are checked first, ensuring VM
    /// implementation is used even when BCL implementation exists.
    fn dispatch_method(
        &mut self,
        gc: GCHandle<'gc>,
        method: MethodDescription,
        lookup: GenericLookup,
    ) -> StepResult {
        // Check intrinsics first - this includes DirectIntercept methods
        // that MUST bypass any BCL implementation
        if is_intrinsic_cached(self, method) {
            intrinsic_call(gc, self, method, &lookup)
        } else if method.method.pinvoke.is_some() {
            self.external_call(method, gc);
            StepResult::Continue
        } else {
            if method.method.internal_call {
                panic!("intrinsic not found: {:?}", method);
            }
            self.call_frame(
                gc,
                MethodInfo::new(method, &lookup, self.shared.clone()),
                lookup,
            );
            StepResult::FramePushed
        }
    }

    /// Unified dispatch pipeline for method calls.
    ///
    /// This function provides a single, predictable entry point for all method
    /// dispatch operations.
    ///
    /// Pipeline stages:
    /// 1. Generic method resolution
    /// 2. Virtual dispatch (if needed) - integrates with VirtualOverride intrinsics
    /// 3. Final dispatch - intrinsics (DirectIntercept included) → P/Invoke → managed
    ///
    /// Parameters:
    /// - `source`: The method source from the instruction
    /// - `this_type`: Optional runtime type for virtual dispatch
    /// - `ctx`: Optional resolution context (for constrained calls)
    ///
    /// Returns: StepResult indicating the outcome of the dispatch
    fn unified_dispatch(
        &mut self,
        gc: GCHandle<'gc>,
        source: &MethodSource,
        this_type: Option<TypeDescription>,
        ctx: Option<&ResolutionContext<'gc, 'm>>,
    ) -> StepResult {
        // Stage 1: Resolve generic method from instruction operand
        let (base_method, lookup) = self.find_generic_method(source);

        // Stage 2: Virtual dispatch if needed
        let method = if let Some(runtime_type) = this_type {
            // Virtual call: resolve based on runtime type
            // resolve_virtual_method checks for VirtualOverride intrinsics
            self.resolve_virtual_method(base_method, runtime_type, ctx)
        } else {
            // Static call: use method as-is
            base_method
        };

        // Stage 3: Final dispatch through the standard pipeline
        // DirectIntercept intrinsics are prioritized here
        self.dispatch_method(gc, method, lookup)
    }

    pub fn step(&mut self, gc: GCHandle<'gc>) -> StepResult {
        if matches!(
            self.execution.exception_mode,
            ExceptionState::Throwing(_)
                | ExceptionState::Searching { .. }
                | ExceptionState::Unwinding { .. }
        ) {
            return self.handle_exception(gc);
        }

        let i = &self.state().info_handle.instructions[self.state().ip];
        let ip = self.state().ip;
        let i_res = self.state().info_handle.source.resolution();

        vm_trace_instruction!(self, ip, &i.show(i_res.definition()));

        let res = if let Some(res) = InstructionRegistry::dispatch(gc, self, i) {
            res
        } else {
            panic!("Unregistered instruction: {:?}", i);
        };

        match res {
            StepResult::Continue => {
                self.increment_ip();
                StepResult::Continue
            }
            StepResult::Jump(target) => {
                self.branch(target);
                StepResult::Continue
            }
            StepResult::Return => self.handle_return(gc),
            _ => res,
        }
    }
}

use crate::vm_trace_instruction;
use dotnet_types::{
    generics::GenericLookup,
    members::{FieldDescription, MethodDescription},
    TypeDescription,
};
use dotnet_utils::gc::GCHandle;
use dotnetdll::prelude::*;
use std::sync::OnceLock;

use super::{
    exceptions::ExceptionState, intrinsics::*, threading::ThreadManagerOps, CallStack, MethodInfo,
    ResolutionContext, StepResult,
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
pub fn is_intrinsic_cached(stack: &CallStack, method: MethodDescription) -> bool {
    if let Some(cached) = stack.shared.caches.intrinsic_cache.get(&method) {
        stack.shared.metrics.record_intrinsic_cache_hit();
        return *cached;
    }

    stack.shared.metrics.record_intrinsic_cache_miss();
    let result = is_intrinsic(
        method,
        stack.shared.loader,
        &stack.shared.caches.intrinsic_registry,
    );

    stack.shared.caches.intrinsic_cache.insert(method, result);
    result
}

/// Check if a field is an intrinsic (with caching).
pub fn is_intrinsic_field_cached(stack: &CallStack, field: FieldDescription) -> bool {
    if let Some(cached) = stack.shared.caches.intrinsic_field_cache.get(&field) {
        stack.shared.metrics.record_intrinsic_field_cache_hit();
        return *cached;
    }

    stack.shared.metrics.record_intrinsic_field_cache_miss();
    let result = is_intrinsic_field(
        field,
        stack.shared.loader,
        &stack.shared.caches.intrinsic_registry,
    );

    stack
        .shared
        .caches
        .intrinsic_field_cache
        .insert(field, result);
    result
}

impl<'gc, 'm: 'gc> CallStack<'gc, 'm> {
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
}

pub struct Interpreter<'a, 'gc, 'm: 'gc> {
    pub stack: &'a mut CallStack<'gc, 'm>,
    pub gc: GCHandle<'gc>,
}

impl<'a, 'gc, 'm: 'gc> Interpreter<'a, 'gc, 'm> {
    pub fn new(stack: &'a mut CallStack<'gc, 'm>, gc: GCHandle<'gc>) -> Self {
        Self { stack, gc }
    }

    pub fn step(&mut self) -> StepResult {
        if matches!(
            self.stack.execution.exception_mode,
            ExceptionState::Throwing(_)
                | ExceptionState::Searching { .. }
                | ExceptionState::Unwinding { .. }
        ) {
            return self.stack.handle_exception(self.gc);
        }

        let i = &self.stack.state().info_handle.instructions[self.stack.state().ip];
        let ip = self.stack.state().ip;
        let i_res = self.stack.state().info_handle.source.resolution();

        vm_trace_instruction!(self.stack, ip, &i.show(i_res.definition()));

        let res = if let Some(res) = InstructionRegistry::dispatch(self.gc, self.stack, i) {
            res
        } else {
            panic!("Unregistered instruction: {:?}", i);
        };

        match res {
            StepResult::Continue => {
                self.stack.increment_ip();
                StepResult::Continue
            }
            StepResult::Jump(target) => {
                self.stack.branch(target);
                StepResult::Continue
            }
            StepResult::Return => self.stack.handle_return(self.gc),
            _ => res,
        }
    }

    pub fn unified_dispatch(
        &mut self,
        source: &MethodSource,
        this_type: Option<TypeDescription>,
        ctx: Option<&ResolutionContext<'gc, 'm>>,
    ) -> StepResult {
        let (base_method, lookup) = self.find_generic_method(source);

        let method = if let Some(runtime_type) = this_type {
            self.stack
                .resolve_virtual_method(base_method, runtime_type, ctx)
        } else {
            base_method
        };

        self.dispatch_method(method, lookup)
    }

    pub fn dispatch_method(
        &mut self,
        method: MethodDescription,
        lookup: GenericLookup,
    ) -> StepResult {
        if is_intrinsic_cached(self.stack, method) {
            intrinsic_call(self.gc, self.stack, method, &lookup)
        } else if method.method.pinvoke.is_some() {
            self.stack.external_call(method, self.gc);
            StepResult::Continue
        } else {
            if method.method.internal_call {
                panic!("intrinsic not found: {:?}", method);
            }
            self.stack.call_frame(
                self.gc,
                MethodInfo::new(method, &lookup, self.stack.shared.clone()),
                lookup,
            );
            StepResult::FramePushed
        }
    }

    pub fn find_generic_method(&self, source: &MethodSource) -> (MethodDescription, GenericLookup) {
        let ctx = self.stack.current_context();

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

    #[inline]
    pub fn check_gc_safe_point(&self) {
        self.stack.check_gc_safe_point();
    }
}

use crate::vm_trace_instruction;
use dotnet_types::{TypeDescription, generics::GenericLookup, members::MethodDescription};
use dotnet_utils::gc::GCHandle;
use dotnetdll::prelude::*;
use gc_arena::Collect;

pub mod registry;

use super::{
    CallStack, MethodInfo, ResolutionContext, StepResult, exceptions::ExceptionState,
    threading::ThreadManagerOps,
};

pub struct InstructionRegistry;

impl InstructionRegistry {
    pub fn dispatch<'gc, 'm>(
        gc: GCHandle<'gc>,
        interp: &mut ExecutionEngine<'gc, 'm>,
        instr: &Instruction,
    ) -> Option<StepResult> {
        let handler = registry::get_handler(instr.opcode())?;
        let mut ctx = interp.ves_context();
        Some(handler(&mut ctx, gc, instr))
    }
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

pub struct ExecutionEngine<'gc, 'm: 'gc> {
    pub stack: CallStack<'gc, 'm>,
}

unsafe impl<'gc, 'm: 'gc> Collect for ExecutionEngine<'gc, 'm> {
    fn trace(&self, cc: &gc_arena::Collection) {
        self.stack.trace(cc);
    }
}

impl<'gc, 'm: 'gc> ExecutionEngine<'gc, 'm> {
    pub fn new(stack: CallStack<'gc, 'm>) -> Self {
        Self { stack }
    }

    pub fn ves_context(&mut self) -> crate::stack::VesContext<'_, 'gc, 'm> {
        self.stack.ves_context()
    }

    pub fn step(&mut self, gc: GCHandle<'gc>) -> StepResult {
        if matches!(
            self.stack.execution.exception_mode,
            ExceptionState::Throwing(_)
                | ExceptionState::Searching { .. }
                | ExceptionState::Unwinding { .. }
        ) {
            return self.ves_context().handle_exception(gc);
        }

        let i = &self.stack.state().info_handle.instructions[self.stack.state().ip];
        let ip = self.stack.state().ip;
        let i_res = self.stack.state().info_handle.source.resolution();

        vm_trace_instruction!(self.stack, ip, &i.show(i_res.definition()));

        let res = if let Some(res) = InstructionRegistry::dispatch(gc, self, i) {
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
            StepResult::Return => self.stack.handle_return(gc),
            _ => res,
        }
    }

    pub fn unified_dispatch(
        &mut self,
        gc: GCHandle<'gc>,
        source: &MethodSource,
        this_type: Option<TypeDescription>,
        ctx: Option<&ResolutionContext<'gc, 'm>>,
    ) -> StepResult {
        self.ves_context()
            .unified_dispatch(gc, source, this_type, ctx)
    }

    pub fn dispatch_method(
        &mut self,
        gc: GCHandle<'gc>,
        method: MethodDescription,
        lookup: GenericLookup,
    ) -> StepResult {
        let mut ctx = self.stack.ves_context();
        if ctx.is_intrinsic_cached(method) {
            crate::intrinsics::intrinsic_call(gc, &mut ctx, method, &lookup)
        } else if method.method.pinvoke.is_some() {
            ctx.external_call(method, gc);
            StepResult::Continue
        } else {
            if method.method.internal_call {
                panic!("intrinsic not found: {:?}", method);
            }
            ctx.call_frame(
                gc,
                MethodInfo::new(method, &lookup, ctx.shared.clone()),
                lookup,
            );
            StepResult::FramePushed
        }
    }
}

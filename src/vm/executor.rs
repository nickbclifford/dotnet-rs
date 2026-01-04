use crate::{
    types::members::MethodDescription,
    value::StackValue,
    vm::{stack::GCArena, MethodInfo, StepResult},
    vm_msg,
};

pub struct Executor {
    arena: &'static mut GCArena,
}

#[derive(Clone, Debug)]
pub enum ExecutorResult {
    Exited(u8),
    Threw, // TODO: well-typed exceptions
}

impl Executor {
    pub fn new(arena: &'static mut GCArena) -> Self {
        Self { arena }
    }

    pub fn entrypoint(&mut self, method: MethodDescription) {
        // TODO: initialize argv (entry point args are either string[] or nothing, II.15.4.1.2)
        self.arena.mutate_root(|gc, c| {
            c.entrypoint_frame(
                gc,
                MethodInfo::new(method, &Default::default(), c.runtime.loader),
                Default::default(),
                vec![],
            )
        });
    }

    // assumes args are already on stack
    pub fn run(&mut self) -> ExecutorResult {
        let result = loop {
            if let Some(marked) = self.arena.mark_all() {
                marked.finalize(|fc, c| c.finalize_check(fc));
            }

            let full_collect = self.arena.mutate(|_, c| {
                if c.gc.needs_full_collect.get() {
                    c.gc.needs_full_collect.set(false);
                    true
                } else {
                    false
                }
            });

            if full_collect {
                self.arena.mutate(|_, c| {
                    vm_msg!(c, "GC: Manual collection triggered");
                });
                let mut marked = None;
                while marked.is_none() {
                    marked = self.arena.mark_all();
                }
                if let Some(marked) = marked {
                    marked.finalize(|fc, c| c.finalize_check(fc));
                }
                self.arena.collect_all(); // Now it's safe to sweep resurrected objects are kept.
            }

            self.arena
                .mutate_root(|gc, c| c.process_pending_finalizers(gc));

            match self.arena.mutate_root(|gc, c| c.step(gc)) {
                StepResult::MethodReturned => {
                    let was_auto_invoked = self.arena.mutate_root(|gc, c| {
                        let frame = c.execution.frames.last().unwrap();
                        let val = frame.state.info_handle.is_cctor || frame.is_finalizer;
                        c.return_frame(gc);
                        val
                    });

                    if self.arena.mutate(|_, c| c.execution.frames.is_empty()) {
                        let exit_code = self.arena.mutate(|_, c| match c.bottom_of_stack() {
                            Some(StackValue::Int32(i)) => i as u8,
                            Some(v) => panic!("invalid value for entrypoint return: {:?}", v),
                            None => 0,
                        });
                        break ExecutorResult::Exited(exit_code);
                    } else if !was_auto_invoked {
                        // step the caller past the call instruction
                        self.arena.mutate_root(|_, c| c.increment_ip());
                    }
                }
                StepResult::MethodThrew => {
                    self.arena.mutate(|_, c| {
                        vm_msg!(c, "Exception thrown: {:?}", c.execution.exception_mode);
                    });
                    if self.arena.mutate(|_, c| c.execution.frames.is_empty()) {
                        break ExecutorResult::Threw;
                    }
                }
                StepResult::InstructionStepped => {}
            }
            // TODO(gc): poll arena for stats
        };

        self.arena.mutate(|_, c| c.runtime.tracer.flush());
        result
    }
}

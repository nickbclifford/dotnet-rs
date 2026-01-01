use crate::{
    value::{MethodDescription, StackValue},
    vm::{stack::GCArena, MethodInfo, StepResult},
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
                MethodInfo::new(method, &Default::default(), c.assemblies),
                Default::default(),
                vec![],
            )
        });
    }

    // assumes args are already on stack
    pub fn run(&mut self) -> ExecutorResult {
        loop {
            match self.arena.mutate_root(|gc, c| c.step(gc)) {
                StepResult::MethodReturned => {
                    let was_cctor = self.arena.mutate_root(|gc, c| {
                        let val = c.frames.last().unwrap().state.info_handle.is_cctor;
                        c.return_frame(gc);
                        val
                    });

                    if self.arena.mutate(|_, c| c.frames.is_empty()) {
                        let exit_code = self.arena.mutate(|_, c| match c.bottom_of_stack() {
                            Some(StackValue::Int32(i)) => i as u8,
                            Some(_) => panic!("invalid value for entrypoint return"),
                            None => 0,
                        });
                        return ExecutorResult::Exited(exit_code);
                    } else if !was_cctor {
                        // step the caller past the call instruction
                        self.arena.mutate_root(|_, c| c.increment_ip());
                    }
                }
                StepResult::MethodThrew => {
                    if self.arena.mutate(|_, c| c.frames.is_empty()) {
                        return ExecutorResult::Threw;
                    }
                }
                StepResult::InstructionStepped => {}
            }
            // TODO(gc): poll arena for stats
        }
    }
}

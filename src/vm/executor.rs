use dotnetdll::prelude::*;

use super::{MethodInfo, StepResult};
use crate::{
    value::{Context, StackValue},
    vm::stack::GCArena,
};

// TODO
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

    pub fn entrypoint(&mut self, method: &'static Method<'static>) {
        // TODO: initialize argv (entry point args are either string[] or nothing, II.15.4.1.2)
        self.arena.mutate_root(|gc, c| {
            let ctx = Context {
                generics: &Default::default(),
                assemblies: c.assemblies,
                resolution: c.assemblies.entrypoint,
            };

            c.entrypoint_frame(
                gc,
                MethodInfo::new(c.assemblies.entrypoint, method, ctx),
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

                    if self.arena.mutate(|gc, c| c.frames.len() == 0) {
                        let exit_code = self.arena.mutate(|gc, c| match c.bottom_of_stack() {
                            Some(StackValue::Int32(i)) => i as u8,
                            Some(_) => todo!("invalid value for entrypoint return"),
                            None => 0,
                        });
                        return ExecutorResult::Exited(exit_code);
                    } else if !was_cctor {
                        // step the caller past the call instruction
                        self.arena.mutate_root(|gc, c| c.increment_ip());
                    }
                }
                StepResult::MethodThrew => {
                    // TODO: where do we keep track of exceptions?
                    todo!()
                }
                StepResult::InstructionStepped => {}
            }
            // TODO(gc): poll arena for stats
        }
    }
}

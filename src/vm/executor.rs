use dotnetdll::prelude::{*, body::DataSection};

use crate::vm::gc::GCArena;

use super::{ExecutionResult, MethodInfo};

// TODO
pub struct Executor {
    instructions: &'static [Instruction],
    info: MethodInfo<'static>,
    arena: &'static mut GCArena
}

impl Executor {
    pub fn new(arena: &'static mut GCArena, method: &'static Method<'static>) -> Self {
        let body = match &method.body {
            Some(b) => b,
            None => todo!("no body in executing method"),
        };
        let mut exceptions: &[body::Exception] = &[];
        for sec in &body.data_sections {
            match sec {
                DataSection::Unrecognized { .. } => {}
                DataSection::ExceptionHandlers(e) => {
                    exceptions = e;
                }
            }
        }

        Self {
            instructions: &body.instructions,
            info: MethodInfo {
                signature: &method.signature,
                locals: &body.header.local_variables,
                exceptions,
            },
            arena
        }
    }

    // assumes args are already on stack
    pub fn run(&mut self) -> ExecutionResult {
        self.arena.mutate_root(|gc, c| c.new_frame(gc, self.info));

        loop {
            match self.arena.mutate_root(|gc, c| c.step(gc, self.instructions)) {
                Some(ExecutionResult::Returned) => {
                    // TODO: void returns
                    self.arena.mutate_root(|gc, c| c.return_frame(gc));
                }
                Some(ExecutionResult::Threw) => {
                    // TODO: where do we keep track of exceptions?
                }
                None => {}
            }
        }
    }
}

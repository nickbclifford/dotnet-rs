use crate::vm::gc::{GCArena, GCValueHandle};
use dotnetdll::prelude::{body::DataSection, *};

use super::{ExecutionResult, MethodInfo, MethodState};

// TODO
#[derive(Debug)]
pub struct Executor<'a> {
    instructions: &'a [Instruction],
    info: MethodInfo<'a>,
    arena: &'a GCArena<'a>
}

impl<'a> Executor<'a> {
    pub fn new(arena: &'a GCArena<'a>, method: &'a Method<'a>) -> Self {
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
            match self.arena.mutate_root(|gc, c| c.step_instruction(gc, self.instructions)) {
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

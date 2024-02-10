use dotnetdll::prelude::{*, body::DataSection};
use gc_arena::{Arena, Rootable};

use super::{MethodInfo, MethodState};

// TODO
#[derive(Debug)]
pub struct Executor<'a> {
    instructions: &'a [Instruction],
    info: MethodInfo<'a>,
}

type GCArena<'a> = Arena<Rootable!['gc => MethodState<'gc, 'a>]>;

impl<'a> Executor<'a> {
    pub fn new(method: &'a Method<'a>) -> Self {
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
        }
    }

    // TODO: arguments/returns between gc sessions?
    pub fn run(&self) {
        let mut arena = GCArena::new(|m| MethodState {
            ip: 0,
            // TODO: properly initialize
            stack: vec![],
            locals: vec![],
            arguments: vec![],
            info_handle: self.info,
            memory_pool: vec![],
            gc_handle: m
        });


        loop {
            // TODO
            // arena.mutate_root(|_, state| {
            //     if let Some(result) = state.execute(&self.instructions[state.ip]) {
            //         return result;
            //     }
            // });
        }
    }
}

use dotnetdll::prelude::*;
use dotnetdll::prelude::body::DataSection;
use crate::value::StackValue;
use super::{MethodInfo, ExecutionResult, MethodState};


// TODO
#[derive(Debug)]
pub struct Executor<'gc> {
    instructions: &'gc [Instruction],
    info: MethodInfo<'gc>
}

impl<'gc> Executor<'gc> {
    pub fn new(method: &Method<'gc>) -> Self {
        let body = match &method.body {
            Some(b) => b,
            None => todo!("no body in executing method")
        };
        let mut exceptions = &[];
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
                signature: &method.signature.clone(),
                locals: &body.header.local_variables,
                exceptions,
            }
        }
    }

    pub fn run(&self, args: Vec<StackValue<'gc>>) -> ExecutionResult<'gc> {
        // TODO: where do we initialize locals slots?
        let mut state = MethodState {
            ip: 0,
            stack: vec![],
            locals: vec![],
            arguments: args,
            info_handle: self.info,
            memory_pool: vec![],
        };

        loop {
            // TODO: auto-increment ip if it hasn't been changed by the instruction
            let step = state.execute(&self.instructions[state.ip]);
            if let Some(result) = step {
                return result
            }
        }
    }
}
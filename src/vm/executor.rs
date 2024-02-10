use super::{ExecutionResult, MethodInfo, MethodState};
use crate::value::StackValue;
use dotnetdll::prelude::{body::DataSection, *};

// TODO
#[derive(Debug)]
pub struct Executor<'a> {
    instructions: &'a [Instruction],
    info: MethodInfo<'a>,
}

impl<'gc, 'a: 'gc> Executor<'a> {
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
            if let Some(result) = state.execute(&self.instructions[state.ip]) {
                return result;
            }
        }
    }
}

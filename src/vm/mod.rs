use dotnetdll::prelude::body::DataSection;
use dotnetdll::prelude::*;
use gc_arena::{unsafe_empty_collect, Collect};

use crate::utils::ResolutionS;
pub use executor::*;
pub use stack::*;

mod executor;
mod stack;
mod instructions;
mod intrinsics;
mod pinvoke;

macro_rules! msg {
    ($src:expr, $($format:tt)*) => {
        $src.msg(format_args!($($format)*))
    }
}
pub(crate) use msg;

// I.12.3.2
#[derive(Clone)]
pub struct MethodState<'m> {
    ip: usize,
    info_handle: MethodInfo<'m>,
    memory_pool: Vec<u8>,
}
impl<'m> MethodState<'m> {
    pub fn new(info_handle: MethodInfo<'m>) -> Self {
        Self {
            ip: 0,
            info_handle,
            memory_pool: vec![],
        }
    }
}
unsafe_empty_collect!(MethodState<'_>);

#[derive(Copy, Clone, Debug)]
pub struct MethodInfo<'a> {
    signature: &'a ManagedMethod<MethodType>,
    locals: &'a [LocalVariable],
    exceptions: &'a [body::Exception],
    pub instructions: &'a [Instruction],
    pub source_resolution: ResolutionS,
    pub is_cctor: bool,
}
unsafe_empty_collect!(MethodInfo<'_>);
impl<'m> MethodInfo<'m> {
    pub fn new(source_resolution: ResolutionS, method: &'m Method<'m>) -> Self {
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

        let instructions = match &method.body {
            Some(b) => b.instructions.as_slice(),
            None => todo!("cannot call method with empty body"),
        };

        Self {
            is_cctor: method.runtime_special_name
                && method.name == ".cctor"
                && !method.signature.instance
                && method.signature.parameters.is_empty(),
            signature: &method.signature,
            locals: &body.header.local_variables,
            exceptions,
            instructions,
            source_resolution,
        }
    }
}

#[derive(Copy, Clone, Debug)]
pub enum StepResult {
    MethodReturned,
    MethodThrew,
    InstructionStepped,
}

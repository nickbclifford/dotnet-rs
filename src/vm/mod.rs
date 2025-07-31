use crate::{
    resolve::Assemblies,
    value::{Context, GenericLookup, MethodDescription},
};

use dotnetdll::prelude::*;
use gc_arena::{unsafe_empty_collect, Collect};
use std::rc::Rc;

mod exceptions;
mod executor;
mod instructions;
pub(crate) mod intrinsics;
mod pinvoke;
mod stack;

pub use executor::*;
pub use stack::*;

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

#[derive(Clone, Debug)]
pub struct MethodInfo<'a> {
    signature: &'a ManagedMethod<MethodType>,
    locals: &'a [LocalVariable],
    exceptions: Vec<Rc<exceptions::ProtectedSection>>,
    pub instructions: &'a [Instruction],
    pub source: MethodDescription,
    pub is_cctor: bool,
}
unsafe_empty_collect!(MethodInfo<'_>);
impl MethodInfo<'static> {
    pub fn new<'c>(
        method: MethodDescription,
        generics: &'c GenericLookup,
        assemblies: &'c Assemblies,
    ) -> Self {
        let body = match &method.method.body {
            Some(b) => b,
            None => todo!("no body in executing method"),
        };
        let mut exceptions: &[body::Exception] = &[];
        for sec in &body.data_sections {
            use body::DataSection::*;
            match sec {
                Unrecognized { .. } => {}
                ExceptionHandlers(e) => {
                    exceptions = e;
                }
            }
        }

        let instructions = match &method.method.body {
            Some(b) => b.instructions.as_slice(),
            None => todo!("cannot call method with empty body"),
        };

        let ctx = Context {
            generics,
            assemblies,
            resolution: method.resolution(),
        };

        Self {
            is_cctor: method.method.runtime_special_name
                && method.method.name == ".cctor"
                && !method.method.signature.instance
                && method.method.signature.parameters.is_empty(),
            signature: &method.method.signature,
            locals: &body.header.local_variables,
            exceptions: exceptions::parse(exceptions, ctx)
                .into_iter()
                .map(Rc::new)
                .collect(),
            instructions,
            source: method,
        }
    }
}

#[derive(Copy, Clone, Debug)]
pub enum StepResult {
    MethodReturned,
    MethodThrew,
    InstructionStepped,
}

use crate::types::{generics::GenericLookup, members::MethodDescription};
use dotnetdll::prelude::*;
use gc_arena::{unsafe_empty_collect, Collect};
use std::rc::Rc;

pub mod context;
mod exceptions;
mod executor;
pub mod gc;
mod instructions;
pub(crate) mod intrinsics;
#[macro_use]
mod macros;
pub mod metrics;
mod pinvoke;
mod stack;
pub mod state;
pub mod sync;
pub mod threading;
pub mod tracer;

pub use executor::*;
pub use stack::*;

use context::ResolutionContext;
use state::SharedGlobalState;
use sync::Arc;

// I.12.3.2
#[derive(Clone)]
pub struct MethodState<'m> {
    pub ip: usize,
    pub info_handle: MethodInfo<'m>,
    pub memory_pool: Vec<u8>,
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
    pub fn new(
        method: MethodDescription,
        generics: &GenericLookup,
        shared: Arc<SharedGlobalState>,
    ) -> Self {
        let loader = shared.loader;
        let body = match &method.method.body {
            Some(b) => b,
            None => panic!(
                "no body in executing method: {}.{}",
                method.parent.type_name(),
                method.method.name
            ),
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
            None => panic!("cannot call method with empty body"),
        };

        let ctx = ResolutionContext::for_method(method, loader, generics, shared);

        Self {
            is_cctor: method.method.runtime_special_name
                && method.method.name == ".cctor"
                && !method.method.signature.instance
                && method.method.signature.parameters.is_empty(),
            signature: &method.method.signature,
            locals: &body.header.local_variables,
            exceptions: exceptions::parse(exceptions, &ctx)
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
    YieldForGC,
}

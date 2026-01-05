use crate::{
    assemblies::AssemblyLoader,
    types::{generics::GenericLookup, members::MethodDescription},
};
use dotnetdll::prelude::*;
use gc_arena::{unsafe_empty_collect, Collect};
use std::rc::Rc;

pub mod context;
mod exceptions;
mod executor;
mod instructions;
pub(crate) mod intrinsics;
pub mod metrics;
#[macro_use]
mod macros;
#[cfg(feature = "multithreaded-gc")]
pub mod arena_storage;
pub mod gc_coordinator;
mod pinvoke;
mod stack;
pub mod sync;
pub mod threading;
pub mod tracer;

pub use executor::*;
pub use stack::*;

use context::ResolutionContext;

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
        loader: &'c AssemblyLoader,
    ) -> Self {
        let body = match &method.method.body {
            Some(b) => b,
            None => panic!("no body in executing method"),
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

        let ctx = ResolutionContext::for_method(method, loader, generics);

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Collect)]
#[collect(require_static)]
pub enum GCHandleType {
    Weak = 0,
    WeakTrackResurrection = 1,
    Normal = 2,
    Pinned = 3,
}

impl From<i32> for GCHandleType {
    fn from(i: i32) -> Self {
        match i {
            0 => GCHandleType::Weak,
            1 => GCHandleType::WeakTrackResurrection,
            2 => GCHandleType::Normal,
            3 => GCHandleType::Pinned,
            _ => panic!("invalid GCHandleType: {}", i),
        }
    }
}

use dotnet_types::{generics::GenericLookup, members::MethodDescription};
use dotnetdll::prelude::*;
use gc_arena::{Collect, unsafe_empty_collect};
use std::rc::Rc;

pub mod context;
pub mod dispatch;
mod exceptions;
mod executor;
pub mod gc;
mod instructions;
pub(crate) mod intrinsics;
pub mod layout;
#[macro_use]
mod macros;
pub mod memory;
pub mod metrics;
mod pinvoke;
pub mod resolution;
pub mod resolver;
mod stack;
pub mod state;
pub mod statics;
pub mod sync;
pub mod threading;
pub mod tracer;

pub use executor::*;
pub use stack::*;
pub use state::ReflectionRegistry;

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

        let ctx = ResolutionContext::for_method(method, loader, generics, shared.caches.clone());

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

#[derive(Copy, Clone, Debug, PartialEq)]
pub enum StepResult {
    Continue,    // Advance IP
    Jump(usize), // Set IP to X
    FramePushed, // Do not advance IP (new frame active)
    Return,      // Pop frame
    MethodThrew, // Exception unhandled in frame
    Yield,       // GC/Thread yield
}

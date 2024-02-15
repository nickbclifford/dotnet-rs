use dotnetdll::prelude::*;
use gc_arena::{Collect, unsafe_empty_collect};

pub use executor::*;
pub use gc::*;

mod executor;
mod gc;
mod instructions;

// I.12.3.2
#[derive(Clone, Collect)]
#[collect(no_drop)]
pub struct MethodState<'m> {
    ip: usize,
    info_handle: MethodInfo<'m>,
    memory_pool: Vec<u8>,
}

#[derive(Copy, Clone, Debug)]
pub struct MethodInfo<'a> {
    signature: &'a ManagedMethod,
    locals: &'a [LocalVariable],
    exceptions: &'a [body::Exception],
}
unsafe_empty_collect!(MethodInfo<'_>);

#[derive(Copy, Clone, Debug)]
pub enum ExecutionResult {
    Returned,
    Threw,
}

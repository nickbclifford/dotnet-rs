use dotnetdll::prelude::*;
use super::value::*;

// I.12.3.2

pub struct MethodState<'a> {
    ip: usize,
    stack: Vec<StackValue>,
    locals: Vec<StackValue>,
    arguments: Vec<StackValue>,
    info_handle: MethodInfo,
    memory_pool: (), // TODO
    return_state: &'a MethodState<'a>
    // skipping security descriptor
}

pub struct MethodInfo {
    signature: ManagedMethod,
    locals: Vec<LocalVariable>,
    exceptions: Vec<body::Exception>
}
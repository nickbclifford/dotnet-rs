use dotnetdll::prelude::*;
use super::value::*;

pub struct VM<'a> {
    instructions: &'a [Instruction],
    ip: usize,
    stack: Vec<Value>,
    locals: Vec<Value>
}

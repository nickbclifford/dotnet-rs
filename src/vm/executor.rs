use dotnetdll::prelude::*;

use crate::value::{ConcreteType, StackValue};
use crate::vm::gc::GCArena;

use super::{CallResult, MethodInfo};

// TODO
pub struct Executor {
    arena: &'static mut GCArena,
}

#[derive(Clone, Debug)]
pub enum ExecutorResult {
    Exited(u32),
    Threw, // TODO: well-typed exceptions
}

impl Executor {
    pub fn new(arena: &'static mut GCArena) -> Self {
        Self { arena }
    }

    pub fn entrypoint(&mut self, method: &'static Method<'static>) {
        // TODO: initialize argv (entry point args are either string[] or nothing, II.15.4.1.2)
        self.arena.mutate_root(|gc, c| {
            c.entrypoint_frame(gc, MethodInfo::new(method), Default::default(), vec![])
        });
    }

    pub fn call(&mut self, method: &'static Method<'static>, generic_inst: Vec<ConcreteType>) {
        self.arena.mutate_root(|gc, c| {
            c.call_frame(
                gc,
                MethodInfo::new(method),
                c.current_frame()
                    .generic_inst
                    .instantiate_method(generic_inst),
            )
        });
    }

    // assumes args are already on stack
    pub fn run(&mut self) -> ExecutorResult {
        loop {
            match self.arena.mutate_root(|gc, c| c.step(gc)) {
                Some(CallResult::Returned) => {
                    // TODO: void returns
                    self.arena.mutate_root(|gc, c| c.return_frame(gc));

                    let exit_code = self.arena.mutate(|gc, c| match c.bottom_of_stack() {
                        Some(StackValue::Int32(i)) => i as u32,
                        Some(_) => todo!("invalid value for entrypoint return"),
                        None => 0,
                    });
                    return ExecutorResult::Exited(exit_code);
                }
                Some(CallResult::Threw) => {
                    // TODO: where do we keep track of exceptions?
                    todo!()
                }
                None => {}
            }
            // TODO: check GC
        }
    }
}

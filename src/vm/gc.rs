use dotnetdll::prelude::Instruction;
use gc_arena::{Arena, Collect, DynamicRoot, DynamicRootSet, Gc, Mutation, Rootable, unsafe_empty_collect};
use gc_arena::lock::Lock;

use crate::value::StackValue;
use crate::vm::{ExecutionResult, MethodInfo, MethodState};

type StackSlot = Rootable![Gc<'_, Lock<StackValue<'_>>>];

#[derive(Collect)]
#[collect(require_static)]
pub struct GCValueHandle(DynamicRoot<StackSlot>);

#[derive(Collect)]
#[collect(no_drop)]
pub struct CallStack<'gc, 'm> {
    roots: DynamicRootSet<'gc>,
    stack: Vec<GCValueHandle>,
    frames: Vec<StackFrame<'m>>,
}

struct StackFrame<'m> {
    stack_height: usize,
    base: BasePointer,
    state: MethodState<'m>,
}
unsafe_empty_collect!(StackFrame<'_>);
struct BasePointer {
    arguments: usize,
    locals: usize,
    stack: usize,
}

// TODO: the 'm lifetime on CallStack is causing problems with the Rootable trait
pub type GCArena = Arena<Rootable!['gc => CallStack<'gc, 'static>]>;

impl<'gc, 'm: 'gc> CallStack<'gc, 'm> {
    pub fn new(gc_handle: &'gc Mutation<'gc>) -> Self {
        Self {
            roots: DynamicRootSet::new(gc_handle),
            stack: vec![],
            frames: vec![],
        }
    }

    pub fn new_frame(&mut self, gc_handle: &'gc Mutation<'gc>, method: MethodInfo<'m>) {
        // TODO: varargs?
        // since arguments are set up on the stack in order for a call and consumed for the caller
        // we can take advantage of the existing stack space and just move our base pointer back

        // before:
        // ─────────────┬────────────────┬─────────────────────────────────────┐
        //              │                │                                     │
        //    caller's  │   caller's     │   caller's       (args set up       │
        //    arguments │   locals       │   stack           here at the top)  │
        //              │                │                                     │
        // ─────────────┴────────────────┴─────────────────────────────────────┘
        //
        // after:
        // ─────────────┬────────────────┬──────────────┬──────────────────────┬───────────────────────┬──────
        //              │                │              │                      │                       │
        //    caller's  │   caller's     │   caller's   │   callee's           │   callee's  (to be    │  callee's  (starts
        //    arguments │   locals       │   stack      │   arguments          │   locals     inited)  │  stack      empty)
        //              │                │              │                      │                       │
        // ─────────────┴────────────────┴──────────────┴──────────────────────┴───────────────────────┴──────

        let num_args =
            if method.signature.instance { 1 } else { 0 } + method.signature.parameters.len();
        let argument_base = self.stack.len() - num_args;
        let locals_base = self.stack.len();
        for l in method.locals {
            // TODO: proper values
            let handle = self
                .roots
                .stash(gc_handle, Gc::new(gc_handle, Lock::new(StackValue::null())));
            self.stack.push(GCValueHandle(handle));
        }
        let stack_base = self.stack.len();

        self.current_frame_mut().stack_height -= num_args;
        self.frames.push(StackFrame {
            stack_height: 0,
            base: BasePointer {
                arguments: argument_base,
                locals: locals_base,
                stack: stack_base,
            },
            state: MethodState {
                ip: 0,
                info_handle: method,
                memory_pool: vec![],
            },
        });
    }

    pub fn return_frame(&mut self, gc_handle: &'gc Mutation<'gc>) {
        // since arguments are consumed by the caller and the return value is put on the top of the stack
        // for similar reasons as above, we can just "delete" the whole frame's slots (reclaimable)
        // and put the return value on the first slot (now open)
        let frame = self.frames.pop().unwrap();

        // TODO: void methods?
        let return_value = self.roots.fetch(&self.stack[frame.base.stack].0).get();

        for GCValueHandle(h) in &self.stack[frame.base.arguments..] {
            self.roots.fetch(h).set(gc_handle, StackValue::null());
        }

        // since we popped the returning frame off, this now refers to the caller frame
        self.push_stack(gc_handle, return_value);
    }

    pub fn current_frame(&self) -> &StackFrame<'m> {
        self.frames.last().unwrap()
    }
    pub fn current_frame_mut(&mut self) -> &mut StackFrame<'m> {
        self.frames.last_mut().unwrap()
    }

    pub fn top_of_stack(&self) -> usize {
        let f = self.current_frame();
        f.base.stack + f.stack_height
    }

    pub fn step_instruction(&mut self, gc_handle: &'gc Mutation<'gc>, body: &[Instruction]) -> Option<ExecutionResult> {
        let frame = self.current_frame_mut();
        let i = &body[frame.state.ip];
        frame.state.execute(gc_handle, self, i)
    }

    pub fn get_argument(&self, index: usize) -> StackValue<'gc> {
        let bp = &self.current_frame().base;
        let GCValueHandle(arg_handle) = &self.stack[bp.arguments + index];
        self.roots.fetch(arg_handle).get()
    }

    pub fn get_local(&self, index: usize) -> StackValue<'gc> {
        let bp = &self.current_frame().base;
        let GCValueHandle(local_handle) = &self.stack[bp.locals + index];
        self.roots.fetch(local_handle).get()
    }

    pub fn set_argument(&self, gc_handle: &'gc Mutation<'gc>, index: usize, value: StackValue<'gc>) {
        let bp = &self.current_frame().base;
        let GCValueHandle(arg_handle) = &self.stack[bp.arguments + index];
        self.roots.fetch(arg_handle).set(gc_handle, value)
    }

    pub fn set_local(&self, gc_handle: &'gc Mutation<'gc>, index: usize, value: StackValue<'gc>) {
        let bp = &self.current_frame().base;
        let GCValueHandle(local_handle) = &self.stack[bp.locals + index];
        self.roots.fetch(local_handle).set(gc_handle, value)
    }

    pub fn push_stack(&mut self, gc_handle: &'gc Mutation<'gc>, value: StackValue<'gc>) {
        match self.stack.get(self.top_of_stack()) {
            Some(GCValueHandle(h)) => {
                self.roots.fetch(h).set(gc_handle, value);
            }
            None => {
                let handle = self
                    .roots
                    .stash(gc_handle, Gc::new(gc_handle, Lock::new(value)));
                self.stack.push(GCValueHandle(handle));
            }
        };
        self.current_frame_mut().stack_height += 1;
    }

    pub fn pop_stack(&mut self) -> StackValue<'gc> {
        match self.stack.get(self.top_of_stack()) {
            Some(GCValueHandle(h)) => {
                let value = self.roots.fetch(h).take();
                self.current_frame_mut().stack_height -= 1;
                value
            }
            None => todo!("empty call stack"),
        }
    }

    // TODO: return values? arguments?
}

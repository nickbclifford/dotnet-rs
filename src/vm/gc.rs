use std::collections::HashMap;
use dotnetdll::prelude::LocalVariable;
use gc_arena::lock::Lock;
use gc_arena::{
    unsafe_empty_collect, Arena, Collect, Collection, DynamicRoot, DynamicRootSet, Gc, Mutation,
    Rootable,
};

use crate::utils::ResolutionS;
use crate::value::Context;
use crate::{
    resolve::Assemblies,
    value::{GenericLookup, StackValue},
    vm::{MethodInfo, MethodState},
};

type StackSlot = Rootable![Gc<'_, Lock<StackValue<'_>>>];

#[derive(Collect)]
#[collect(require_static)]
pub struct StackSlotHandle(DynamicRoot<StackSlot>);

pub struct CallStack<'gc, 'm> {
    roots: DynamicRootSet<'gc>,
    stack: Vec<StackSlotHandle>,
    pub(crate) frames: Vec<StackFrame<'m>>,
    pub(crate) assemblies: &'m Assemblies,
}
// this is sound, I think?
unsafe impl<'gc, 'm> Collect for CallStack<'gc, 'm> {
    fn trace(&self, cc: &Collection) {
        self.roots.trace(cc)
    }
}

pub struct StackFrame<'m> {
    pub stack_height: usize,
    pub base: BasePointer,
    pub state: MethodState<'m>,
    pub generic_inst: GenericLookup,
    pub source_resolution: ResolutionS,
}
unsafe_empty_collect!(StackFrame<'_>);
impl<'m> StackFrame<'m> {
    pub fn new(
        base_pointer: BasePointer,
        method: MethodInfo<'m>,
        generic_inst: GenericLookup,
    ) -> Self {
        Self {
            stack_height: 0,
            base: base_pointer,
            source_resolution: method.source_resolution,
            state: MethodState::new(method),
            generic_inst,
        }
    }
}

pub struct BasePointer {
    arguments: usize,
    locals: usize,
    stack: usize,
}

pub type GCArena = Arena<Rootable!['gc => CallStack<'gc, 'static>]>;

pub type GCHandle<'gc> = &'gc Mutation<'gc>;

impl<'gc, 'm: 'gc> CallStack<'gc, 'm> {
    pub fn new(gc: GCHandle<'gc>, assemblies: &'m Assemblies) -> Self {
        Self {
            roots: DynamicRootSet::new(gc),
            stack: vec![],
            frames: vec![],
            assemblies,
        }
    }

    // careful with this, it always allocates a new slot
    fn insert_value(&mut self, gc: GCHandle<'gc>, value: StackValue<'gc>) {
        let handle = self.roots.stash(gc, Gc::new(gc, Lock::new(value)));
        self.stack.push(StackSlotHandle(handle));
    }

    fn init_locals(&mut self, gc: GCHandle<'gc>, locals: &'m [LocalVariable]) {
        for l in locals {
            // TODO: proper values
            self.insert_value(gc, StackValue::null());
        }
    }

    pub fn entrypoint_frame(
        &mut self,
        gc: GCHandle<'gc>,
        method: MethodInfo<'m>,
        generic_inst: GenericLookup,
        args: Vec<StackValue<'gc>>,
    ) {
        let argument_base = self.stack.len();
        for a in args {
            self.insert_value(gc, a);
        }
        let locals_base = self.stack.len();
        self.init_locals(gc, method.locals);
        let stack_base = self.stack.len();

        self.frames.push(StackFrame::new(
            BasePointer {
                arguments: argument_base,
                locals: locals_base,
                stack: stack_base,
            },
            method,
            generic_inst,
        ));
    }

    pub fn call_frame(
        &mut self,
        gc: GCHandle<'gc>,
        method: MethodInfo<'m>,
        generic_inst: GenericLookup,
    ) {
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
        let Some(argument_base) = self.top_of_stack().checked_sub(num_args) else {
            panic!(
                "not enough values on stack! expected {} arguments, found {}",
                num_args,
                self.stack.len()
            )
        };
        let locals_base = self.top_of_stack();
        self.init_locals(gc, method.locals);
        let stack_base = self.top_of_stack() + method.locals.len();

        self.current_frame_mut().stack_height -= num_args;
        self.frames.push(StackFrame::new(
            BasePointer {
                arguments: argument_base,
                locals: locals_base,
                stack: stack_base,
            },
            method,
            generic_inst,
        ));
    }

    pub fn return_frame(&mut self, gc: GCHandle<'gc>) {

        // since arguments are consumed by the caller and the return value is put on the top of the stack
        // for similar reasons as above, we can just "delete" the whole frame's slots (reclaimable)
        // and put the return value on the first slot (now open)
        let frame = self.frames.pop().unwrap();

        for StackSlotHandle(h) in &self.stack[frame.base.arguments..] {
            self.roots.fetch(h).set(gc, StackValue::null());
        }

        if let Some(StackSlotHandle(handle)) = self.stack.get(frame.base.stack) {
            let return_value = self.roots.fetch(handle).get();
            // since we popped the returning frame off, this now refers to the caller frame
            self.push_stack(gc, return_value);
        }
    }

    pub fn current_frame(&self) -> &StackFrame<'m> {
        self.frames.last().unwrap()
    }
    pub fn current_frame_mut(&mut self) -> &mut StackFrame<'m> {
        self.frames.last_mut().unwrap()
    }

    pub fn current_context(&self) -> Context {
        let f = self.current_frame();
        Context {
            generics: &f.generic_inst,
            assemblies: self.assemblies,
            resolution: f.source_resolution,
        }
    }

    pub fn top_of_stack(&self) -> usize {
        let f = self.current_frame();
        f.base.stack + f.stack_height
    }

    pub fn get_argument(&self, index: usize) -> StackValue<'gc> {
        let bp = &self.current_frame().base;
        let StackSlotHandle(arg_handle) = &self.stack[bp.arguments + index];
        self.roots.fetch(arg_handle).get()
    }

    pub fn get_local(&self, index: usize) -> StackValue<'gc> {
        let bp = &self.current_frame().base;
        let StackSlotHandle(local_handle) = &self.stack[bp.locals + index];
        self.roots.fetch(local_handle).get()
    }

    pub fn set_argument(&self, gc: GCHandle<'gc>, index: usize, value: StackValue<'gc>) {
        let bp = &self.current_frame().base;
        let StackSlotHandle(arg_handle) = &self.stack[bp.arguments + index];
        self.roots.fetch(arg_handle).set(gc, value)
    }

    pub fn set_local(&self, gc: GCHandle<'gc>, index: usize, value: StackValue<'gc>) {
        let bp = &self.current_frame().base;
        let StackSlotHandle(local_handle) = &self.stack[bp.locals + index];
        self.roots.fetch(local_handle).set(gc, value)
    }

    pub fn push_stack(&mut self, gc: GCHandle<'gc>, value: StackValue<'gc>) {
        match self.stack.get(self.top_of_stack()) {
            Some(StackSlotHandle(h)) => {
                self.roots.fetch(h).set(gc, value);
            }
            None => {
                self.insert_value(gc, value);
            }
        };
        self.current_frame_mut().stack_height += 1;
    }

    pub fn pop_stack(&mut self) -> StackValue<'gc> {
        match self.stack.get(self.top_of_stack() - 1) {
            Some(StackSlotHandle(h)) => {
                let value = self.roots.fetch(h).take();
                self.current_frame_mut().stack_height -= 1;
                value
            }
            None => todo!("empty call stack"),
        }
    }

    pub fn bottom_of_stack(&self) -> Option<StackValue<'gc>> {
        match self.stack.first() {
            Some(StackSlotHandle(h)) => {
                let value = self.roots.fetch(h).get();
                Some(value)
            }
            None => None,
        }
    }

    pub fn dump_stack(&self) {
        let mut contents: Vec<_> = self
            .stack[..self.top_of_stack()].iter()
            .map(|h| format!("{:?}", self.roots.fetch(&h.0).get()))
            .collect();
        contents.reverse();

        let mut positions: HashMap<usize, Vec<String>> = HashMap::new();
        let mut insert = |i, value| {
            positions.entry(i).or_insert(vec![]).push(value);
        };
        for (i, frame) in self.frames.iter().enumerate() {
            let base = &frame.base;
            insert(base.stack, format!("stack base of frame #{}", i));
            insert(base.locals, format!("locals base of frame #{}", i));
            insert(base.arguments, format!("arguments base of frame #{}", i))
        }

        let longest = contents.iter().map(|s| s.len()).max().unwrap();
        println!("│ {} │", " ".repeat(longest));

        let last_idx = contents.len() - 1;

        if let Some(bases) = positions.get(&(last_idx + 1)) {
            for b in bases {
                println!("│ {:width$} │", b, width = longest);
            }
            println!("├─{}─┤", "─".repeat(longest));
        }

        for (i, entry) in contents.into_iter().enumerate() {
            println!("│ {:width$} │", entry, width = longest);
            if let Some(bases) = positions.get(&(last_idx - i)) {
                for b in bases {
                    println!("│ {:width$} │", b, width = longest);
                }
            }
            if i != last_idx {
                println!("├─{}─┤", "─".repeat(longest));
            }
        }

        println!("└─{}─┘", "─".repeat(longest))
    }
}

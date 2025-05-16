use crate::vm::exceptions::{Handler, ProtectedSection};
use crate::{
    resolve::Assemblies,
    utils::ResolutionS,
    value::{
        storage::StaticStorageManager, ConcreteType, Context, GenericLookup, HeapStorage,
        Object as ObjectInstance, ObjectPtr, ObjectRef, StackValue,
    },
    vm::{pinvoke::NativeLibraries, MethodInfo, MethodState},
};
use dotnetdll::prelude::*;
use gc_arena::{
    lock::RefLock, Arena, Collect, Collection, DynamicRoot, DynamicRootSet, Gc, Mutation, Rootable,
};
use std::collections::VecDeque;
use std::fmt::Debug;
use std::rc::Rc;
use std::{cell::RefCell, collections::HashMap, fs::OpenOptions, io::Write};

type StackSlot = Rootable![Gc<'_, RefLock<StackValue<'_>>>];

#[derive(Collect)]
#[collect(require_static)]
pub struct StackSlotHandle(DynamicRoot<StackSlot>);

pub struct CallStack<'gc, 'm> {
    roots: DynamicRootSet<'gc>,
    stack: Vec<StackSlotHandle>,
    pub frames: Vec<StackFrame<'gc, 'm>>,
    pub assemblies: &'m Assemblies,
    pub statics: RefCell<StaticStorageManager<'gc>>,
    pub pinvoke: NativeLibraries,
    pub _all_objs: Vec<usize>, // secretly ObjectHandles, not traced for GCing because these are for runtime debugging
}
// this is sound, I think?
unsafe impl<'gc, 'm> Collect for CallStack<'gc, 'm> {
    fn trace(&self, cc: &Collection) {
        self.roots.trace(cc);
        self.statics.borrow_mut().trace(cc);
    }
}

pub struct StackFrame<'gc, 'm> {
    pub stack_height: usize,
    pub base: BasePointer,
    pub state: MethodState<'m>,
    pub generic_inst: GenericLookup,
    pub source_resolution: ResolutionS,
    pub exception_value: Option<StackValue<'gc>>,
    pub protected_sections: VecDeque<(Rc<ProtectedSection>, Vec<Handler>)>,
}
unsafe impl<'gc, 'm> Collect for StackFrame<'gc, 'm> {
    fn trace(&self, cc: &Collection) {
        self.exception_value.trace(cc);
    }
}
impl<'gc, 'm> StackFrame<'gc, 'm> {
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
            exception_value: None,
            protected_sections: VecDeque::new(),
        }
    }
}

#[derive(Debug)]
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
            pinvoke: NativeLibraries::new(assemblies.get_root()),
            statics: RefCell::new(StaticStorageManager::new()),
            _all_objs: vec![],
        }
    }

    // careful with this, it always allocates a new slot
    fn insert_value(&mut self, gc: GCHandle<'gc>, value: StackValue<'gc>) {
        let handle = self.roots.stash(gc, Gc::new(gc, RefLock::new(value)));
        self.stack.push(StackSlotHandle(handle));
    }

    fn init_locals(&mut self, locals: &'m [LocalVariable]) -> Vec<StackValue<'gc>> {
        let mut values = vec![];

        for l in locals {
            use BaseType::*;
            use LocalVariable::*;
            match l {
                TypedReference => todo!("initialize typedref local"),
                Variable {
                    by_ref, var_type, ..
                } => {
                    if *by_ref {
                        // todo!("initialize byref local")
                        // maybe i don't need to care and it will always be referenced appropriately?
                    }
                    let ctx = self.current_context();

                    let v = match ctx.make_concrete(var_type).get() {
                        Type { source, .. } => {
                            let mut type_generics: &[ConcreteType] = &[];
                            let ut = match source {
                                TypeSource::User(u) => *u,
                                TypeSource::Generic { base, parameters } => {
                                    type_generics = parameters.as_slice();
                                    *base
                                }
                            };
                            let desc = ctx.locate_type(ut);

                            if desc.is_value_type(&ctx) {
                                let new_lookup = GenericLookup::new(type_generics.to_vec());
                                let new_ctx = Context::with_generics(ctx, &new_lookup);
                                let instance = ObjectInstance::new(desc, new_ctx);
                                StackValue::ValueType(Box::new(instance))
                            } else {
                                StackValue::null()
                            }
                        }
                        Boolean | Int8 | UInt8 | Char | Int16 | UInt16 | Int32 | UInt32 => {
                            StackValue::Int32(0)
                        }
                        Int64 | UInt64 => StackValue::Int64(0),
                        Float32 | Float64 => StackValue::NativeFloat(0.0),
                        IntPtr | UIntPtr | ValuePointer(_, _) | FunctionPointer(_) => {
                            StackValue::NativeInt(0)
                        }
                        Object | String | Vector(_, _) | Array(_, _) => StackValue::null(),
                    };
                    values.push(v);
                }
            }
        }
        values
    }

    fn get_slot(&self, handle: &StackSlotHandle) -> StackValue<'gc> {
        self.roots.fetch(&handle.0).borrow().clone()
    }

    fn set_slot(&self, gc: GCHandle<'gc>, handle: &StackSlotHandle, value: StackValue<'gc>) {
        *self.roots.fetch(&handle.0).borrow_mut(gc) = value;
    }

    fn set_slot_at(&mut self, gc: GCHandle<'gc>, index: usize, value: StackValue<'gc>) {
        match self.stack.get(index) {
            Some(h) => {
                self.set_slot(gc, h, value);
            }
            None => {
                for _ in self.top_of_stack()..index {
                    self.insert_value(gc, StackValue::null());
                }
                self.insert_value(gc, value);
            }
        };
    }
    
    fn mark_slot_available(&mut self, gc: GCHandle<'gc>, index: usize) {
        self.set_slot(gc, &self.stack[index], StackValue::null());
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
        for v in self.init_locals(method.locals) {
            self.insert_value(gc, v);
        }
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

    pub fn register_new_object(&mut self, instance: &ObjectRef<'gc>) {
        let ObjectRef(Some(ptr)) = instance else {
            unreachable!()
        };
        self._all_objs.push(Gc::as_ptr(*ptr) as usize);
    }

    pub fn constructor_frame(
        &mut self,
        gc: GCHandle<'gc>,
        instance: ObjectInstance<'gc>,
        method: MethodInfo<'m>,
        generic_inst: GenericLookup,
    ) {
        let desc = instance.description;

        // newobj is typically not used for value types, but still works to put them on the stack (III.4.21)
        let value = if desc.is_value_type(&self.current_context()) {
            StackValue::ValueType(Box::new(instance))
        } else {
            let in_heap = ObjectRef::new(gc, HeapStorage::Obj(instance));
            self.register_new_object(&in_heap);
            StackValue::ObjectRef(in_heap)
        };

        let mut args = vec![];
        for _ in 0..method.signature.parameters.len() {
            args.push(self.pop_stack());
        }

        // first pushing the NewObject 'return value', then the value of the 'this' parameter
        if desc.is_value_type(&self.current_context()) {
            self.push_stack(gc, value);

            self.push_stack(
                gc,
                StackValue::managed_ptr(self.top_of_stack_address() as *mut _, desc),
            );
        } else {
            self.push_stack(gc, value.clone());

            self.push_stack(gc, value);
        }

        for _ in 0..method.signature.parameters.len() {
            self.push_stack(gc, args.pop().unwrap());
        }

        self.call_frame(gc, method, generic_inst);
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
        let local_values = self.init_locals(method.locals);
        let mut local_index = 0;
        for v in local_values {
            self.set_slot_at(gc, locals_base + local_index, v);
            local_index += 1;
        }
        let stack_base = locals_base + local_index;

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

        for index in frame.base.arguments..frame.base.stack {
            self.mark_slot_available(gc, index);
        }

        let signature = frame.state.info_handle.signature;

        // only return value to caller if the method actually declares a return type
        match (&signature.return_type, self.stack.get(frame.base.stack)) {
            (ReturnType(_, Some(_)), Some(handle)) => {
                let return_value = self.get_slot(handle);
                // since we popped the returning frame off, this now refers to the caller frame
                self.push_stack(gc, return_value);
            }
            _ => {}
        }
    }

    pub fn current_frame(&self) -> &StackFrame<'gc, 'm> {
        self.frames.last().unwrap()
    }
    pub fn current_frame_mut(&mut self) -> &mut StackFrame<'gc, 'm> {
        self.frames.last_mut().unwrap()
    }

    pub fn increment_ip(&mut self) {
        self.current_frame_mut().state.ip += 1;
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

    fn get_handle_location(&self, handle: &StackSlotHandle) -> *const u8 {
        let slot_ref = self.roots.fetch(&handle.0).borrow();
        slot_ref.data_location()
    }

    fn get_arg_handle(&self, index: usize) -> &StackSlotHandle {
        let bp = &self.current_frame().base;
        &self.stack[bp.arguments + index]
    }

    pub fn get_argument(&self, index: usize) -> StackValue<'gc> {
        self.get_slot(self.get_arg_handle(index))
    }

    pub fn get_argument_address(&self, index: usize) -> *const u8 {
        self.get_handle_location(self.get_arg_handle(index))
    }

    pub fn set_argument(&self, gc: GCHandle<'gc>, index: usize, value: StackValue<'gc>) {
        self.set_slot(gc, self.get_arg_handle(index), value);
    }

    fn get_local_handle(&self, index: usize) -> &StackSlotHandle {
        let bp = &self.current_frame().base;
        &self.stack[bp.locals + index]
    }

    pub fn get_local(&self, index: usize) -> StackValue<'gc> {
        self.get_slot(self.get_local_handle(index))
    }

    pub fn get_local_address(&self, index: usize) -> *const u8 {
        self.get_handle_location(self.get_local_handle(index))
    }

    pub fn set_local(&self, gc: GCHandle<'gc>, index: usize, value: StackValue<'gc>) {
        self.set_slot(gc, self.get_local_handle(index), value);
    }

    pub fn push_stack(&mut self, gc: GCHandle<'gc>, value: StackValue<'gc>) {
        self.set_slot_at(gc, self.top_of_stack(), value);
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

    pub fn top_of_stack_address(&self) -> *const u8 {
        self.get_handle_location(&self.stack[self.top_of_stack() - 1])
    }

    pub fn bottom_of_stack(&self) -> Option<StackValue<'gc>> {
        match self.stack.first() {
            Some(h) => {
                let value = self.get_slot(h);
                Some(value)
            }
            None => None,
        }
    }

    pub fn msg(&self, fmt: std::fmt::Arguments) {
        println!("{}{}", "\t".repeat(self.frames.len() - 1), fmt);
    }
}

// this block is all for runtime debug methods
#[allow(dead_code)]
impl<'gc, 'm: 'gc> CallStack<'gc, 'm> {
    pub fn dump_stack(&self) {
        use std::fs::OpenOptions;
        use std::io::Write;

        let mut f = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open("stack.txt")
            .unwrap();

        macro_rules! dumpln {
            ($($fmt:tt)*) => {
                writeln!(f, $($fmt)*).unwrap();
            };
        }

        let mut contents: Vec<_> = self.stack[..self.top_of_stack()]
            .iter()
            .map(|h| format!("{:?}", self.get_slot(h)))
            .collect();
        contents.reverse();

        let mut positions: HashMap<usize, Vec<String>> = HashMap::new();
        let mut insert = |i, value| {
            positions.entry(i).or_insert(vec![]).push(value);
        };
        for (i, frame) in self.frames.iter().enumerate().rev() {
            let base = &frame.base;
            insert(base.stack, format!("stack base of frame #{}", i));
            if base.locals != base.stack {
                insert(base.locals, format!("locals base of frame #{}", i));
            }
            insert(base.arguments, format!("arguments base of frame #{}", i))
        }

        let longest = match contents.iter().map(|s| s.len()).max() {
            Some(longest) => {
                dumpln!("│ {} │", " ".repeat(longest));

                let last_idx = contents.len() - 1;

                if let Some(bases) = positions.get(&(last_idx + 1)) {
                    for b in bases {
                        dumpln!("├─ {:width$} │", b, width = longest);
                    }
                    dumpln!("├─{}─┤", "─".repeat(longest));
                }

                for (i, entry) in contents.into_iter().enumerate() {
                    dumpln!("│ {:width$} │", entry, width = longest);
                    if let Some(bases) = positions.get(&(last_idx - i)) {
                        for b in bases {
                            dumpln!("├─ {:width$}│", b, width = longest);
                        }
                    }
                    if i != last_idx {
                        dumpln!("├─{}─┤", "─".repeat(longest));
                    }
                }

                longest
            }
            None => 0,
        };

        dumpln!("└─{}─┘", "─".repeat(longest));

        f.flush().unwrap();
    }

    pub fn dump_statics(&self) {
        let mut f = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open("statics.txt")
            .unwrap();

        let s = self.statics.borrow();
        write!(f, "{:#?}", &s).unwrap();

        f.flush().unwrap();
    }

    pub fn dump_heap(&self) {
        let mut f = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open("heap.txt")
            .unwrap();

        for &obj in &self._all_objs {
            let ptr = obj as ObjectPtr;
            let gc = unsafe { Gc::from_ptr(ptr) };
            let handle = gc.borrow();
            match &*handle {
                HeapStorage::Obj(o) => {
                    writeln!(f, "{:#?} => {:#?}", ptr, o)
                }
                HeapStorage::Vec(v) => {
                    writeln!(f, "{:#?} => {:#?}", ptr, v)
                }
                _ => {
                    writeln!(f, "{:#?} => TODO other heap objects", ptr)
                }
            }
            .unwrap();
        }

        f.flush().unwrap();
    }

    pub fn debug_dump(&self) {
        self.dump_stack();
        self.dump_statics();
        self.dump_heap();
    }
}

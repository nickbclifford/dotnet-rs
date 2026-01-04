use crate::{
    assemblies::AssemblyLoader,
    types::{
        TypeDescription, generics::{ConcreteType, GenericLookup},
        members::{FieldDescription, MethodDescription},
    },
    utils::{ResolutionS, decompose_type_source},
    value::{
        StackValue,
        object::{HeapStorage, Object as ObjectInstance, ObjectPtr, ObjectRef},
        storage::StaticStorageManager,
    },
    vm::{
        GCHandleType, MethodInfo, MethodState, StepResult, context::ResolutionContext,
        exceptions::ExceptionState, intrinsics::reflection::RuntimeType,
        pinvoke::NativeLibraries,
    },
};
use dotnetdll::prelude::*;
use gc_arena::{Arena, Collect, Collection, Gc, Mutation, Rootable, lock::RefLock};
use std::{
    cell::{Cell, RefCell},
    collections::{HashMap, HashSet},
    fmt::Debug,
};

#[derive(Collect)]
#[collect(no_drop)]
pub struct StackSlotHandle<'gc>(Gc<'gc, RefLock<StackValue<'gc>>>);

#[derive(Collect)]
#[collect(no_drop)]
pub struct ExecutionStack<'gc, 'm> {
    pub stack: Vec<StackSlotHandle<'gc>>,
    pub frames: Vec<StackFrame<'gc, 'm>>,
    pub exception_mode: ExceptionState<'gc>,
    pub suspended_frames: Vec<StackFrame<'gc, 'm>>,
    pub suspended_stack: Vec<StackSlotHandle<'gc>>,
    pub original_ip: usize,
    pub original_stack_height: usize,
}

pub struct RuntimeEnvironment<'gc, 'm> {
    pub loader: &'m AssemblyLoader,
    pub statics: RefCell<StaticStorageManager<'gc>>,
    pub pinvoke: NativeLibraries,
    pub runtime_asms: HashMap<ResolutionS, ObjectRef<'gc>>,
    pub runtime_types: HashMap<RuntimeType, ObjectRef<'gc>>,
    pub runtime_types_list: Vec<RuntimeType>,
    pub runtime_methods: Vec<(MethodDescription, GenericLookup)>,
    pub runtime_method_objs: HashMap<(MethodDescription, GenericLookup), ObjectRef<'gc>>,
    pub runtime_fields: Vec<(FieldDescription, GenericLookup)>,
    pub runtime_field_objs: HashMap<(FieldDescription, GenericLookup), ObjectRef<'gc>>,
    pub method_tables: RefCell<HashMap<TypeDescription, Box<[u8]>>>,
    pub empty_generics: GenericLookup,
    pub tracer: crate::vm::tracer::Tracer,
}

unsafe impl<'gc, 'm: 'gc> Collect for RuntimeEnvironment<'gc, 'm> {
    fn trace(&self, cc: &Collection) {
        self.statics.borrow().trace(cc);
        self.empty_generics.trace(cc);

        for o in self.runtime_asms.values() {
            o.trace(cc);
        }
        for o in self.runtime_types.values() {
            o.trace(cc);
        }
        for o in self.runtime_method_objs.values() {
            o.trace(cc);
        }
        for o in self.runtime_field_objs.values() {
            o.trace(cc);
        }
    }
}

pub struct HeapManager<'gc> {
    pub finalization_queue: RefCell<Vec<ObjectRef<'gc>>>,
    pub pending_finalization: RefCell<Vec<ObjectRef<'gc>>>,
    pub pinned_objects: RefCell<HashSet<ObjectRef<'gc>>>,
    pub gchandles: RefCell<Vec<Option<(ObjectRef<'gc>, GCHandleType)>>>,
    pub processing_finalizer: bool,
    pub needs_full_collect: Cell<bool>,
    // secretly ObjectHandles, not traced for GCing because these are for runtime debugging
    _all_objs: RefCell<HashSet<ObjectRef<'gc>>>,
}

unsafe impl<'gc> Collect for HeapManager<'gc> {
    fn trace(&self, cc: &Collection) {
        // Normal and Pinned handles keep objects alive.
        // Weak handles DO NOT trace.
        for entry in self.gchandles.borrow().iter().flatten() {
            match entry.1 {
                GCHandleType::Normal | GCHandleType::Pinned => {
                    entry.0.trace(cc);
                }
                GCHandleType::Weak | GCHandleType::WeakTrackResurrection => {
                    // Weak handles don't trace
                }
            }
        }

        for obj in self.pinned_objects.borrow().iter() {
            obj.trace(cc);
        }
        self.pending_finalization.borrow().trace(cc);

        // - self.finalization_queue: Traced by finalize_check (resurrection).
        // - self._all_objs: Debugging handles (untraced).
    }
}

pub struct CallStack<'gc, 'm> {
    pub execution: ExecutionStack<'gc, 'm>,
    pub runtime: RuntimeEnvironment<'gc, 'm>,
    pub gc: HeapManager<'gc>,
}

unsafe impl<'gc, 'm: 'gc> Collect for CallStack<'gc, 'm> {
    fn trace(&self, cc: &Collection) {
        self.execution.trace(cc);
        self.runtime.trace(cc);
        self.gc.trace(cc);
    }
}

pub struct StackFrame<'gc, 'm> {
    pub stack_height: usize,
    pub base: BasePointer,
    pub state: MethodState<'m>,
    pub generic_inst: GenericLookup,
    pub source_resolution: ResolutionS,
    /// The exceptions currently being handled by catch blocks in this frame (required for rethrow).
    pub exception_stack: Vec<ObjectRef<'gc>>,
    pub pinned_locals: Vec<bool>,
    pub is_finalizer: bool,
}
unsafe impl<'gc, 'm> Collect for StackFrame<'gc, 'm> {
    fn trace(&self, cc: &Collection) {
        self.exception_stack.trace(cc);
        self.state.trace(cc);
        self.generic_inst.trace(cc);
    }
}
impl<'gc, 'm> StackFrame<'gc, 'm> {
    pub fn new(
        base_pointer: BasePointer,
        method: MethodInfo<'m>,
        generic_inst: GenericLookup,
        pinned_locals: Vec<bool>,
    ) -> Self {
        Self {
            stack_height: 0,
            base: base_pointer,
            source_resolution: method.source.resolution(),
            state: MethodState::new(method),
            generic_inst,
            exception_stack: Vec::new(),
            pinned_locals,
            is_finalizer: false,
        }
    }
}

#[derive(Debug)]
pub struct BasePointer {
    pub arguments: usize,
    pub locals: usize,
    pub stack: usize,
}

pub type GCArena = Arena<Rootable!['gc => CallStack<'gc, 'static>]>;

pub type GCHandle<'gc> = &'gc Mutation<'gc>;

impl<'gc, 'm: 'gc> CallStack<'gc, 'm> {
    pub fn new(_gc: GCHandle<'gc>, loader: &'m AssemblyLoader) -> Self {
        Self {
            execution: ExecutionStack {
                stack: vec![],
                frames: vec![],
                exception_mode: ExceptionState::None,
                suspended_frames: vec![],
                suspended_stack: vec![],
                original_ip: 0,
                original_stack_height: 0,
            },
            runtime: RuntimeEnvironment {
                loader,
                pinvoke: NativeLibraries::new(loader.get_root()),
                statics: RefCell::new(StaticStorageManager::new()),
                runtime_asms: HashMap::new(),
                runtime_types: HashMap::new(),
                runtime_types_list: vec![],
                runtime_methods: vec![],
                runtime_method_objs: HashMap::new(),
                runtime_fields: vec![],
                runtime_field_objs: HashMap::new(),
                method_tables: RefCell::new(HashMap::new()),
                empty_generics: GenericLookup::default(),
                tracer: crate::vm::tracer::Tracer::new(),
            },
            gc: HeapManager {
                _all_objs: RefCell::new(HashSet::new()),
                finalization_queue: RefCell::new(vec![]),
                pending_finalization: RefCell::new(vec![]),
                pinned_objects: RefCell::new(HashSet::new()),
                gchandles: RefCell::new(vec![]),
                processing_finalizer: false,
                needs_full_collect: Cell::new(false),
            },
        }
    }

    // careful with this, it always allocates a new slot
    fn insert_value(&mut self, gc: GCHandle<'gc>, value: StackValue<'gc>) {
        let handle = Gc::new(gc, RefLock::new(value));
        self.execution.stack.push(StackSlotHandle(handle));
    }

    fn init_locals(
        &mut self,
        method: MethodDescription,
        locals: &'m [LocalVariable],
        generics: &GenericLookup,
    ) -> (Vec<StackValue<'gc>>, Vec<bool>) {
        let mut values = vec![];
        let mut pinned_locals = vec![];

        for l in locals {
            use BaseType::*;
            use LocalVariable::*;
            match l {
                TypedReference => {
                    values.push(StackValue::null());
                    pinned_locals.push(false);
                }
                Variable {
                    by_ref,
                    var_type,
                    pinned,
                    ..
                } => {
                    pinned_locals.push(*pinned);
                    if *by_ref {
                        // todo!("initialize byref local")
                        // maybe i don't need to care and it will always be referenced appropriately?
                    }
                    let ctx = ResolutionContext {
                        generics,
                        loader: self.runtime.loader,
                        resolution: method.resolution(),
                        type_owner: Some(method.parent),
                        method_owner: Some(method),
                    };

                    let v = match ctx.make_concrete(var_type).get() {
                        Type { source, .. } => {
                            let (ut, type_generics) = decompose_type_source(source);
                            let desc = ctx.locate_type(ut);

                            if desc.is_value_type(&ctx) {
                                let new_lookup = GenericLookup {
                                    type_generics,
                                    ..generics.clone()
                                };
                                let new_ctx = ctx.with_generics(&new_lookup);
                                let instance = ObjectInstance::new(desc, &new_ctx);
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
        (values, pinned_locals)
    }

    fn get_slot(&self, handle: &StackSlotHandle<'gc>) -> StackValue<'gc> {
        handle.0.borrow().clone()
    }

    fn set_slot(&self, gc: GCHandle<'gc>, handle: &StackSlotHandle<'gc>, value: StackValue<'gc>) {
        *handle.0.borrow_mut(gc) = value;
    }

    fn set_slot_at(&mut self, gc: GCHandle<'gc>, index: usize, value: StackValue<'gc>) {
        match self.execution.stack.get(index) {
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

    pub fn entrypoint_frame(
        &mut self,
        gc: GCHandle<'gc>,
        method: MethodInfo<'m>,
        generic_inst: GenericLookup,
        args: Vec<StackValue<'gc>>,
    ) {
        let argument_base = self.execution.stack.len();
        for a in args {
            self.insert_value(gc, a);
        }
        let locals_base = self.execution.stack.len();
        let (local_values, pinned_locals) =
            self.init_locals(method.source, method.locals, &generic_inst);
        for v in local_values {
            self.insert_value(gc, v);
        }
        let stack_base = self.execution.stack.len();

        self.execution.frames.push(StackFrame::new(
            BasePointer {
                arguments: argument_base,
                locals: locals_base,
                stack: stack_base,
            },
            method,
            generic_inst,
            pinned_locals,
        ));
    }

    pub fn register_new_object(&self, instance: &ObjectRef<'gc>) {
        let ObjectRef(Some(ptr)) = instance else {
            return;
        };
        self.gc._all_objs.borrow_mut().insert(*instance);

        let ctx = self.current_context();

        if let HeapStorage::Obj(o) = &*ptr.borrow() {
            if o.description.has_finalizer(&ctx) {
                let mut queue = self.gc.finalization_queue.borrow_mut();
                queue.push(*instance);
            }
        }
    }

    pub fn process_pending_finalizers(&mut self, gc: GCHandle<'gc>) -> StepResult {
        if self.gc.processing_finalizer {
            return StepResult::MethodReturned;
        }

        let obj_ref = self.gc.pending_finalization.borrow_mut().pop();
        if let Some(obj_ref) = obj_ref {
            self.gc.processing_finalizer = true;
            let ptr = obj_ref.0.unwrap();
            let obj_type = match &*ptr.borrow() {
                HeapStorage::Obj(o) => o.description,
                _ => unreachable!(),
            };

            let object_type = self.runtime.loader.corlib_type("System.Object");
            let base_finalize = object_type
                .definition
                .methods
                .iter()
                .find(|m| {
                    m.name == "Finalize" && m.virtual_member && m.signature.parameters.is_empty()
                })
                .map(|m| MethodDescription {
                    parent: object_type,
                    method: m,
                })
                .expect("could not find System.Object::Finalize");

            // We need a context to resolve the virtual method, but if the stack is empty,
            // we can use a temporary one from the object's own resolution.
            let ctx = ResolutionContext {
                generics: &self.runtime.empty_generics,
                loader: self.runtime.loader,
                resolution: obj_type.resolution,
                type_owner: Some(obj_type),
                method_owner: None,
            };
            let target_method = self.resolve_virtual_method(base_finalize, obj_type, Some(&ctx));

            self.entrypoint_frame(
                gc,
                MethodInfo::new(
                    target_method,
                    &self.runtime.empty_generics,
                    self.runtime.loader,
                ),
                self.runtime.empty_generics.clone(),
                vec![StackValue::ObjectRef(obj_ref)],
            );
            self.execution.frames.last_mut().unwrap().is_finalizer = true;
            return StepResult::InstructionStepped;
        }
        StepResult::MethodReturned
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
                StackValue::managed_ptr(self.top_of_stack_address() as *mut _, desc, None, false),
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
        if self.tracer_enabled() {
            let method_desc = format!("{:?}", method.source);
            self.runtime
                .tracer
                .trace_method_entry(self.indent(), &method_desc, "");
        }

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

        let num_args = method.signature.instance as usize + method.signature.parameters.len();
        let Some(argument_base) = self.top_of_stack().checked_sub(num_args) else {
            panic!(
                "not enough values on stack! expected {} arguments, found {}",
                num_args,
                self.execution.stack.len()
            )
        };
        let locals_base = self.top_of_stack();
        let (local_values, pinned_locals) =
            self.init_locals(method.source, method.locals, &generic_inst);
        let mut local_index = 0;
        for v in local_values {
            self.set_slot_at(gc, locals_base + local_index, v);
            local_index += 1;
        }
        let stack_base = locals_base + local_index;

        if self.current_frame().stack_height < num_args {
            panic!(
                "stack overflow in constructor_frame: stack_height={}, num_args={}",
                self.current_frame().stack_height,
                num_args
            );
        }

        self.current_frame_mut().stack_height -= num_args;
        self.execution.frames.push(StackFrame::new(
            BasePointer {
                arguments: argument_base,
                locals: locals_base,
                stack: stack_base,
            },
            method,
            generic_inst,
            pinned_locals,
        ));
    }

    pub fn return_frame(&mut self, gc: GCHandle<'gc>) {
        // since arguments are consumed by the caller and the return value is put on the top of the stack
        // for similar reasons as above, we can just "delete" the whole frame's slots (reclaimable)
        // and put the return value on the first slot (now open)
        let frame = self.execution.frames.pop().unwrap();

        if self.tracer_enabled() {
            let method_name = format!("{:?}", frame.state.info_handle.source);
            self.runtime
                .tracer
                .trace_method_exit(self.indent(), &method_name);
        }

        if frame.is_finalizer {
            self.gc.processing_finalizer = false;
        }

        let signature = frame.state.info_handle.signature;

        // only return value to caller if the method actually declares a return type
        let return_value = if let ReturnType(_, Some(_)) = &signature.return_type {
            // The return value is at the top of the evaluation stack, not at the base
            let return_slot_index = frame.base.stack + frame.stack_height - 1;
            self.execution
                .stack
                .get(return_slot_index)
                .map(|handle| self.get_slot(handle))
        } else {
            None
        };

        self.execution.stack.truncate(frame.base.arguments);

        if let Some(return_value) = return_value {
            // since we popped the returning frame off, this now refers to the caller frame
            if !self.execution.frames.is_empty() {
                self.push_stack(gc, return_value);
            } else {
                self.insert_value(gc, return_value);
            }
        }
    }

    pub fn unwind_frame(&mut self, _gc: GCHandle<'gc>) {
        let frame = self
            .execution
            .frames
            .pop()
            .expect("unwind_frame called with empty stack");
        if frame.is_finalizer {
            self.gc.processing_finalizer = false;
        }
        self.execution.stack.truncate(frame.base.arguments);
    }

    pub fn current_frame(&self) -> &StackFrame<'gc, 'm> {
        self.execution.frames.last().unwrap()
    }
    pub fn current_frame_mut(&mut self) -> &mut StackFrame<'gc, 'm> {
        self.execution.frames.last_mut().unwrap()
    }

    pub fn increment_ip(&mut self) {
        self.current_frame_mut().state.ip += 1;
    }

    pub fn current_context(&self) -> ResolutionContext<'_> {
        if let Some(f) = self.execution.frames.last() {
            ResolutionContext {
                generics: &f.generic_inst,
                loader: self.runtime.loader,
                resolution: f.source_resolution,
                type_owner: Some(f.state.info_handle.source.parent),
                method_owner: Some(f.state.info_handle.source),
            }
        } else {
            ResolutionContext {
                generics: &self.runtime.empty_generics,
                loader: self.runtime.loader,
                resolution: self.runtime.loader.corlib_type("System.Object").resolution,
                type_owner: None,
                method_owner: None,
            }
        }
    }

    pub fn ctx_with_generics<'a>(&'a self, generics: &'a GenericLookup) -> ResolutionContext<'a> {
        self.current_context().with_generics(generics)
    }

    pub fn make_concrete<T: Clone + Into<MethodType>>(&self, t: &T) -> ConcreteType {
        self.current_context().make_concrete(t)
    }

    pub fn locate_type(&self, handle: UserType) -> TypeDescription {
        self.current_context().locate_type(handle)
    }

    pub fn locate_field(&self, handle: FieldSource) -> (FieldDescription, GenericLookup) {
        self.current_context().locate_field(handle)
    }

    pub fn is_a(&self, value: TypeDescription, ancestor: TypeDescription) -> bool {
        self.current_context().is_a(value, ancestor)
    }

    pub fn finalize_check(&self, fc: &gc_arena::Finalization<'gc>) {
        let mut queue = self.gc.finalization_queue.borrow_mut();
        let mut handles = self.gc.gchandles.borrow_mut();
        let mut resurrected = HashSet::new();

        let mut zero_out_handles = |for_type: GCHandleType, resurrected: &HashSet<usize>| {
            for entry in handles.iter_mut().flatten() {
                if entry.1 == for_type {
                    if let ObjectRef(Some(ptr)) = entry.0 {
                        if Gc::is_dead(fc, ptr) {
                            if for_type == GCHandleType::WeakTrackResurrection
                                && resurrected.contains(&(Gc::as_ptr(ptr) as usize))
                            {
                                continue;
                            }
                            entry.0 = ObjectRef(None);
                        }
                    }
                }
            }
        };

        if !queue.is_empty() {
            let mut to_finalize = Vec::new();
            let mut i = 0;
            while i < queue.len() {
                let obj = queue[i];
                let ptr = obj.0.expect("object in finalization queue is null");

                let is_suppressed = match &*ptr.borrow() {
                    HeapStorage::Obj(o) => o.finalizer_suppressed,
                    _ => false,
                };

                if is_suppressed {
                    queue.swap_remove(i);
                    continue;
                }

                if Gc::is_dead(fc, ptr) {
                    to_finalize.push(queue.swap_remove(i));
                } else {
                    i += 1;
                }
            }

            if !to_finalize.is_empty() {
                let mut pending = self.gc.pending_finalization.borrow_mut();
                for obj in to_finalize {
                    let ptr = obj.0.unwrap();
                    pending.push(obj);
                    if resurrected.insert(Gc::as_ptr(ptr) as usize) {
                        Gc::resurrect(fc, ptr);
                        ptr.borrow().resurrect(fc, &mut resurrected);
                    }
                }
            }
        }

        // 1. Zero out Weak handles for dead objects
        zero_out_handles(GCHandleType::Weak, &resurrected);

        // 2. Zero out WeakTrackResurrection handles
        zero_out_handles(GCHandleType::WeakTrackResurrection, &resurrected);

        // 3. Prune dead objects from the debugging harness
        self.gc._all_objs.borrow_mut().retain(|obj| match obj {
            ObjectRef(Some(ptr)) => {
                !Gc::is_dead(fc, *ptr) || resurrected.contains(&(Gc::as_ptr(*ptr) as usize))
            }
            ObjectRef(None) => false,
        });
    }

    pub fn top_of_stack(&self) -> usize {
        let f = self.current_frame();
        f.base.stack + f.stack_height
    }

    fn get_handle_location(&self, handle: &StackSlotHandle<'gc>) -> *const u8 {
        handle.0.borrow().data_location()
    }

    fn get_arg_handle(&self, index: usize) -> &StackSlotHandle<'gc> {
        let bp = &self.current_frame().base;
        let idx = bp.arguments + index;
        if idx >= self.execution.stack.len() {
            panic!(
                "get_arg_handle out of bounds: idx={} len={} bp.arguments={} stack.len={}",
                idx,
                self.execution.stack.len(),
                bp.arguments,
                self.execution.stack.len()
            );
        }
        &self.execution.stack[idx]
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

    fn get_local_handle_at(
        &self,
        frame: &StackFrame<'gc, 'm>,
        index: usize,
    ) -> &StackSlotHandle<'gc> {
        &self.execution.stack[frame.base.locals + index]
    }

    pub fn get_local(&self, index: usize) -> StackValue<'gc> {
        self.get_slot(self.get_local_handle_at(self.current_frame(), index))
    }

    pub fn get_local_address(&self, index: usize) -> *const u8 {
        self.get_handle_location(self.get_local_handle_at(self.current_frame(), index))
    }

    pub fn set_local(&self, gc: GCHandle<'gc>, index: usize, value: StackValue<'gc>) {
        let frame = self.current_frame();
        if index < frame.pinned_locals.len() && frame.pinned_locals[index] {
            if let StackValue::ObjectRef(obj) = value {
                if obj.0.is_some() {
                    self.gc.pinned_objects.borrow_mut().insert(obj);
                }
            }
        }
        self.set_slot(gc, self.get_local_handle_at(frame, index), value);
    }

    pub fn push_stack(&mut self, gc: GCHandle<'gc>, value: StackValue<'gc>) {
        if self.tracer_enabled() {
            self.runtime
                .tracer
                .trace_stack_op(self.indent(), "PUSH", &format!("{:?}", value));
        }
        self.set_slot_at(gc, self.top_of_stack(), value);
        self.current_frame_mut().stack_height += 1;
    }

    pub fn pop_stack(&mut self) -> StackValue<'gc> {
        let top = self.top_of_stack();
        if top == 0 {
            panic!("empty call stack");
        }
        let value = self.get_slot(&self.execution.stack[top - 1]);
        if self.tracer_enabled() {
            self.runtime
                .tracer
                .trace_stack_op(self.indent(), "POP", &format!("{:?}", value));
        }
        self.current_frame_mut().stack_height -= 1;
        value
    }

    pub fn top_of_stack_address(&self) -> *const u8 {
        self.get_handle_location(&self.execution.stack[self.top_of_stack() - 1])
    }

    pub fn bottom_of_stack(&self) -> Option<StackValue<'gc>> {
        match self.execution.stack.first() {
            Some(h) => {
                let value = self.get_slot(h);
                Some(value)
            }
            None => None,
        }
    }

    pub fn tracer_enabled(&self) -> bool {
        self.runtime.tracer.is_enabled()
    }

    pub fn indent(&self) -> usize {
        if self.execution.frames.is_empty() {
            0
        } else {
            (self.execution.frames.len() - 1) % 10
        }
    }

    pub fn msg(&self, fmt: std::fmt::Arguments) {
        self.runtime.tracer.msg(self.indent(), fmt);
    }

    pub fn throw_by_name(&mut self, gc: GCHandle<'gc>, name: &str) -> StepResult {
        let rt = self.runtime.loader.corlib_type(name);
        let rt_obj = ObjectInstance::new(rt, &self.current_context());
        let obj_ref = ObjectRef::new(gc, HeapStorage::Obj(rt_obj));
        self.register_new_object(&obj_ref);

        self.execution.exception_mode = ExceptionState::Throwing(obj_ref);
        self.handle_exception(gc)
    }

    pub fn resolve_virtual_method(
        &self,
        base_method: MethodDescription,
        this_type: TypeDescription,
        ctx: Option<&ResolutionContext>,
    ) -> MethodDescription {
        let default_ctx;
        let ctx = if let Some(ctx) = ctx {
            ctx
        } else {
            default_ctx = self.current_context();
            &default_ctx
        };

        for (parent, _) in ctx.get_ancestors(this_type) {
            if let Some(method) = self.runtime.loader.find_method_in_type(
                parent,
                &base_method.method.name,
                &base_method.method.signature,
                base_method.resolution(),
            ) {
                return method;
            }
        }
        panic!(
            "could not find virtual method implementation of {:?} in {:?}",
            base_method, this_type
        );
    }
}

// this block is all for runtime debug methods
#[allow(dead_code)]
impl<'gc, 'm: 'gc> CallStack<'gc, 'm> {
    // Tracer-integrated dump methods for comprehensive state capture
    pub fn trace_dump_stack(&self) {
        if !self.tracer_enabled() {
            return;
        }

        let contents: Vec<_> = self.execution.stack[..self.top_of_stack()]
            .iter()
            .map(|h| format!("{:?}", self.get_slot(h)))
            .collect();

        let mut markers = Vec::new();
        for (i, frame) in self.execution.frames.iter().enumerate() {
            let base = &frame.base;
            markers.push((base.stack, format!("Stack base of frame #{}", i)));
            if base.locals != base.stack {
                markers.push((base.locals, format!("Locals base of frame #{}", i)));
            }
            markers.push((base.arguments, format!("Arguments base of frame #{}", i)));
        }

        self.runtime.tracer.dump_stack_state(&contents, &markers);
    }

    pub fn trace_dump_frames(&self) {
        if !self.tracer_enabled() {
            return;
        }

        for (idx, frame) in self.execution.frames.iter().enumerate() {
            let method_name = format!("{:?}", frame.state.info_handle.source);
            self.runtime.tracer.dump_frame_state(
                idx,
                &method_name,
                frame.state.ip,
                frame.base.arguments,
                frame.base.locals,
                frame.base.stack,
                frame.stack_height,
            );
        }
    }

    pub fn trace_dump_heap(&self) {
        if !self.tracer_enabled() {
            return;
        }

        let objects: Vec<_> = self.gc._all_objs.borrow().iter().copied().collect();
        self.runtime.tracer.dump_heap_snapshot_start(objects.len());

        for obj in objects {
            let Some(ptr) = obj.0 else {
                continue;
            };
            let raw_ptr = Gc::as_ptr(ptr) as ObjectPtr as usize;
            match &*ptr.borrow() {
                HeapStorage::Obj(o) => {
                    let details = format!("{:?}", o);
                    self.runtime
                        .tracer
                        .dump_heap_object(raw_ptr, "Object", &details);
                }
                HeapStorage::Vec(v) => {
                    let details = format!("{:?}", v);
                    self.runtime
                        .tracer
                        .dump_heap_object(raw_ptr, "Vector", &details);
                }
                HeapStorage::Str(s) => {
                    let details = format!("{:?}", s);
                    self.runtime
                        .tracer
                        .dump_heap_object(raw_ptr, "String", &details);
                }
                HeapStorage::Boxed(b) => {
                    let details = format!("{:?}", b);
                    self.runtime
                        .tracer
                        .dump_heap_object(raw_ptr, "Boxed", &details);
                }
            }
        }

        self.runtime.tracer.dump_heap_snapshot_end();
    }

    pub fn trace_dump_statics(&self) {
        if !self.tracer_enabled() {
            return;
        }

        let s = self.runtime.statics.borrow();
        let debug_str = format!("{:#?}", &*s);
        self.runtime.tracer.dump_statics_snapshot(&debug_str);
    }

    pub fn trace_dump_gc_stats(&self) {
        if !self.tracer_enabled() {
            return;
        }

        self.runtime.tracer.dump_gc_stats(
            self.gc.finalization_queue.borrow().len(),
            self.gc.pending_finalization.borrow().len(),
            self.gc.pinned_objects.borrow().len(),
            self.gc.gchandles.borrow().len(),
            self.gc._all_objs.borrow().len(),
        );
    }

    /// Captures a complete snapshot of all runtime state to the tracer
    pub fn trace_full_state(&self) {
        if !self.tracer_enabled() {
            return;
        }

        self.runtime.tracer.dump_full_state_header();
        self.trace_dump_frames();
        self.trace_dump_stack();
        self.trace_dump_heap();
        self.trace_dump_statics();
        self.trace_dump_gc_stats();
        self.runtime.tracer.flush();
    }
}

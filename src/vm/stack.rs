use crate::{
    assemblies::AssemblyLoader,
    types::{
        generics::{ConcreteType, GenericLookup},
        members::{FieldDescription, MethodDescription},
        TypeDescription,
    },
    utils::{decompose_type_source, ResolutionS},
    value::{
        object::{HeapStorage, Object as ObjectInstance, ObjectRef},
        storage::StaticStorageManager,
        StackValue,
    },
    vm::{
        context::ResolutionContext,
        exceptions::ExceptionState,
        intrinsics::reflection::RuntimeType,
        pinvoke::NativeLibraries,
        tracer::Tracer,
        GCHandleType,
        MethodInfo,
        MethodState,
        StepResult,
    },
};
#[cfg(feature = "multithreaded-gc")]
use crate::{
    value::object::ObjectPtr,
    vm::gc_coordinator::GCCoordinator
};
use dotnetdll::prelude::*;
use gc_arena::{lock::RefLock, Arena, Collect, Collection, Gc, Mutation, Rootable};
use crate::vm::sync::{Mutex, RwLock};
use std::{
    cell::{Cell, Ref, RefCell, RefMut},
    collections::{HashMap, HashSet},
    fmt::Debug,
    ptr::NonNull,
    sync::Arc,
};

#[derive(Collect)]
#[collect(no_drop)]
pub struct StackSlotHandle<'gc>(Gc<'gc, RefLock<StackValue<'gc>>>);

/// Thread-local execution state for a single .NET thread.
/// Contains the evaluation stack, call frames, and exception state.
#[derive(Collect)]
#[collect(no_drop)]
pub struct ThreadContext<'gc, 'm> {
    pub stack: Vec<StackSlotHandle<'gc>>,
    pub frames: Vec<StackFrame<'gc, 'm>>,
    pub exception_mode: ExceptionState<'gc>,
    pub suspended_frames: Vec<StackFrame<'gc, 'm>>,
    pub suspended_stack: Vec<StackSlotHandle<'gc>>,
    pub original_ip: usize,
    pub original_stack_height: usize,
}

pub struct HeapManager<'gc> {
    pub finalization_queue: RefCell<Vec<ObjectRef<'gc>>>,
    pub pending_finalization: RefCell<Vec<ObjectRef<'gc>>>,
    pub pinned_objects: RefCell<HashSet<ObjectRef<'gc>>>,
    pub gchandles: RefCell<Vec<Option<(ObjectRef<'gc>, GCHandleType)>>>,
    pub processing_finalizer: Cell<bool>,
    pub needs_full_collect: Cell<bool>,
    /// Roots for objects in this arena that are referenced by other arenas.
    /// This is populated during coordinated GC marking phase.
    #[cfg(feature = "multithreaded-gc")]
    pub cross_arena_roots: RefCell<HashSet<ObjectPtr>>,
    // untraced handles to every heap object, used only by the tracer during debugging
    pub(super) _all_objs: RefCell<HashSet<ObjectRef<'gc>>>,
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
        #[cfg(feature = "multithreaded-gc")]
        for ptr in self.cross_arena_roots.borrow().iter() {
            unsafe {
                Gc::from_ptr(ptr.as_ptr()).trace(cc);
            }
        }
        self.pending_finalization.borrow().trace(cc);

        // - self.finalization_queue: Traced by finalize_check (resurrection).
        // - self._all_objs: Debugging handles (untraced).
    }
}

/// Thread-safe shared state that does not contain any GC-managed pointers.
/// This state is shared across all execution threads and arenas.
pub struct SharedGlobalState<'m> {
    pub loader: &'m AssemblyLoader,
    pub pinvoke: RwLock<NativeLibraries>,
    pub sync_blocks: crate::vm::sync::SyncBlockManager,
    #[cfg(feature = "multithreading")]
    pub thread_manager: Arc<crate::vm::threading::ThreadManager>,
    #[cfg(not(feature = "multithreading"))]
    pub thread_manager: crate::vm::threading::ThreadManager,
    pub metrics: crate::vm::metrics::RuntimeMetrics,
    pub tracer: Mutex<Tracer>,
    pub empty_generics: GenericLookup,
    #[cfg(feature = "multithreaded-gc")]
    pub gc_coordinator: Arc<GCCoordinator>,
    pub method_tables: RwLock<HashMap<TypeDescription, Box<[u8]>>>,
    pub statics: RwLock<StaticStorageManager>,
}

// SAFETY: SharedGlobalState contains only thread-safe components:
// - AssemblyLoader is immutable after construction
// - All mutable state is protected by RwLock/Mutex/Arc
// - No raw pointers or non-thread-safe types
unsafe impl<'m> Send for SharedGlobalState<'m> {}
unsafe impl<'m> Sync for SharedGlobalState<'m> {}

impl<'m> SharedGlobalState<'m> {
    pub fn new(loader: &'m AssemblyLoader) -> Self {
        Self {
            loader,
            pinvoke: RwLock::new(NativeLibraries::new(loader.get_root())),
            sync_blocks: crate::vm::sync::SyncBlockManager::new(),
            #[cfg(feature = "multithreading")]
            thread_manager: Arc::new(crate::vm::threading::ThreadManager::new()),
            #[cfg(not(feature = "multithreading"))]
            thread_manager: crate::vm::threading::ThreadManager::new(),
            metrics: crate::vm::metrics::RuntimeMetrics::new(),
            tracer: Mutex::new(Tracer::new()),
            empty_generics: GenericLookup::default(),
            #[cfg(feature = "multithreaded-gc")]
            gc_coordinator: Arc::new(GCCoordinator::new()),
            method_tables: RwLock::new(HashMap::new()),
            statics: RwLock::new(StaticStorageManager::new()),
        }
    }
}

/// GC-managed state local to a single thread's arena.
pub struct ArenaLocalState<'gc, 'm> {
    pub heap: HeapManager<'gc>,
    pub runtime_asms: RefCell<HashMap<ResolutionS, ObjectRef<'gc>>>,
    pub runtime_types: RefCell<HashMap<RuntimeType, ObjectRef<'gc>>>,
    pub runtime_types_list: RefCell<Vec<RuntimeType>>,
    pub runtime_methods: RefCell<Vec<(MethodDescription, GenericLookup)>>,
    pub runtime_method_objs: RefCell<HashMap<(MethodDescription, GenericLookup), ObjectRef<'gc>>>,
    pub runtime_fields: RefCell<Vec<(FieldDescription, GenericLookup)>>,
    pub runtime_field_objs: RefCell<HashMap<(FieldDescription, GenericLookup), ObjectRef<'gc>>>,
    pub _phantom: std::marker::PhantomData<&'m ()>,
}

unsafe impl<'gc, 'm> Collect for ArenaLocalState<'gc, 'm> {
    fn trace(&self, cc: &Collection) {
        self.heap.trace(cc);
        for o in self.runtime_asms.borrow().values() {
            o.trace(cc);
        }
        for o in self.runtime_types.borrow().values() {
            o.trace(cc);
        }
        for o in self.runtime_method_objs.borrow().values() {
            o.trace(cc);
        }
        for o in self.runtime_field_objs.borrow().values() {
            o.trace(cc);
        }
    }
}

impl<'gc, 'm> Default for ArenaLocalState<'gc, 'm> {
    fn default() -> Self {
        Self::new()
    }
}

impl<'gc, 'm> ArenaLocalState<'gc, 'm> {
    pub fn new() -> Self {
        Self {
            heap: HeapManager {
                _all_objs: RefCell::new(HashSet::new()),
                finalization_queue: RefCell::new(vec![]),
                pending_finalization: RefCell::new(vec![]),
                pinned_objects: RefCell::new(HashSet::new()),
                gchandles: RefCell::new(vec![]),
                processing_finalizer: Cell::new(false),
                needs_full_collect: Cell::new(false),
                #[cfg(feature = "multithreaded-gc")]
                cross_arena_roots: RefCell::new(HashSet::new()),
            },
            runtime_asms: RefCell::new(HashMap::new()),
            runtime_types: RefCell::new(HashMap::new()),
            runtime_types_list: RefCell::new(vec![]),
            runtime_methods: RefCell::new(vec![]),
            runtime_method_objs: RefCell::new(HashMap::new()),
            runtime_fields: RefCell::new(vec![]),
            runtime_field_objs: RefCell::new(HashMap::new()),
            _phantom: std::marker::PhantomData,
        }
    }
}

pub struct CallStack<'gc, 'm> {
    pub execution: ThreadContext<'gc, 'm>,
    pub shared: Arc<SharedGlobalState<'m>>,
    pub local: ArenaLocalState<'gc, 'm>,
    pub thread_id: Cell<u64>,
}

unsafe impl<'gc, 'm: 'gc> Collect for CallStack<'gc, 'm> {
    fn trace(&self, cc: &Collection) {
        self.execution.trace(cc);
        self.local.trace(cc);
        self.shared.statics.read().trace(cc);
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
    /// Create a new CallStack with separated global state (Phase 1 architecture).
    /// This is now the primary and only constructor for CallStack.
    pub fn new(
        _gc: GCHandle<'gc>,
        shared: Arc<SharedGlobalState<'m>>,
        local: ArenaLocalState<'gc, 'm>,
    ) -> Self {
        Self {
            execution: ThreadContext {
                stack: vec![],
                frames: vec![],
                exception_mode: ExceptionState::None,
                suspended_frames: vec![],
                suspended_stack: vec![],
                original_ip: 0,
                original_stack_height: 0,
            },
            shared,
            local,
            thread_id: Cell::new(0),
        }
    }

    // careful with this, it always allocates a new slot
    fn insert_value(&mut self, gc: GCHandle<'gc>, value: StackValue<'gc>) {
        #[cfg(feature = "multithreaded-gc")]
        crate::vm::gc_coordinator::record_allocation(value.size_bytes() + 16); // +16 for Gc/RefLock overhead
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
                        loader: self.shared.loader,
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

    pub(super) fn get_slot(&self, handle: &StackSlotHandle<'gc>) -> StackValue<'gc> {
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

        let heap = &self.local.heap;

        heap._all_objs.borrow_mut().insert(*instance);

        let ctx = self.current_context();

        if let HeapStorage::Obj(o) = &ptr.borrow().storage {
            if o.description.has_finalizer(&ctx) {
                let mut queue = heap.finalization_queue.borrow_mut();
                queue.push(*instance);
            }
        }
    }

    pub fn process_pending_finalizers(&mut self, gc: GCHandle<'gc>) -> StepResult {
        if self.local.heap.processing_finalizer.get() {
            return StepResult::MethodReturned;
        }

        let obj_ref = self.local.heap.pending_finalization.borrow_mut().pop();
        if let Some(obj_ref) = obj_ref {
            self.local.heap.processing_finalizer.set(true);
            let ptr = obj_ref.0.unwrap();
            let obj_type = match &ptr.borrow().storage {
                HeapStorage::Obj(o) => o.description,
                _ => unreachable!(),
            };

            // Trace finalization event
            if self.tracer_enabled() {
                let type_name = format!("{:?}", obj_type);
                let addr = gc_arena::Gc::as_ptr(ptr) as usize;
                self.shared
                    .tracer
                    .lock()
                    .trace_gc_finalization(self.indent(), &type_name, addr);
            }

            let object_type = self.shared.loader.corlib_type("System.Object");
            let base_finalize = object_type
                .definition()
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
                generics: &self.shared.empty_generics,
                loader: self.shared.loader,
                resolution: obj_type.resolution,
                type_owner: Some(obj_type),
                method_owner: None,
            };
            let target_method = self.resolve_virtual_method(base_finalize, obj_type, Some(&ctx));

            self.entrypoint_frame(
                gc,
                MethodInfo::new(
                    target_method,
                    &self.shared.empty_generics,
                    self.shared.loader,
                ),
                self.shared.empty_generics.clone(),
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
                StackValue::managed_ptr(
                    self.top_of_stack_address().as_ptr() as *mut _,
                    desc,
                    None,
                    false,
                ),
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
            self.shared
                .tracer
                .lock()
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
            self.shared
                .tracer
                .lock()
                .trace_method_exit(self.indent(), &method_name);
        }

        // If this was a static constructor (.cctor), mark the type as initialized
        if frame.state.info_handle.is_cctor {
            let type_desc = frame.state.info_handle.source.parent;
            self.shared.statics.write().mark_initialized(type_desc);
        }

        if frame.is_finalizer {
            self.local.heap.processing_finalizer.set(false);
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
            self.local.heap.processing_finalizer.set(false);
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
                loader: self.shared.loader,
                resolution: f.source_resolution,
                type_owner: Some(f.state.info_handle.source.parent),
                method_owner: Some(f.state.info_handle.source),
            }
        } else {
            ResolutionContext {
                generics: &self.shared.empty_generics,
                loader: self.shared.loader,
                resolution: self.shared.loader.corlib_type("System.Object").resolution,
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
        let heap = &self.local.heap;
        let mut queue = heap.finalization_queue.borrow_mut();
        let mut handles = heap.gchandles.borrow_mut();
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

                let is_suppressed = match &ptr.borrow().storage {
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
                let mut pending = heap.pending_finalization.borrow_mut();
                for obj in to_finalize {
                    let ptr = obj.0.unwrap();
                    pending.push(obj);
                    if resurrected.insert(Gc::as_ptr(ptr) as usize) {
                        // Trace resurrection event
                        if self.tracer_enabled() {
                            let obj_type_name = match &ptr.borrow().storage {
                                HeapStorage::Obj(o) => format!("{:?}", o.description),
                                HeapStorage::Vec(_) => "Vector".to_string(),
                                HeapStorage::Str(_) => "String".to_string(),
                                HeapStorage::Boxed(_) => "Boxed".to_string(),
                            };
                            let addr = Gc::as_ptr(ptr) as usize;
                            self.shared.tracer.lock().trace_gc_resurrection(
                                self.indent(),
                                &obj_type_name,
                                addr,
                            );
                        }
                        Gc::resurrect(fc, ptr);
                        ptr.borrow().storage.resurrect(fc, &mut resurrected);
                    }
                }
            }
        }

        // 1. Zero out Weak handles for dead objects
        zero_out_handles(GCHandleType::Weak, &resurrected);

        // 2. Zero out WeakTrackResurrection handles
        zero_out_handles(GCHandleType::WeakTrackResurrection, &resurrected);

        // 3. Prune dead objects from the debugging harness
        heap._all_objs.borrow_mut().retain(|obj| match obj.0 {
            Some(ptr) => !Gc::is_dead(fc, ptr) || resurrected.contains(&(Gc::as_ptr(ptr) as usize)),
            None => false,
        });
    }

    pub fn top_of_stack(&self) -> usize {
        let f = self.current_frame();
        f.base.stack + f.stack_height
    }

    fn get_handle_location(&self, handle: &StackSlotHandle<'gc>) -> NonNull<u8> {
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

    pub fn get_argument_address(&self, index: usize) -> NonNull<u8> {
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

    pub fn get_local_address(&self, index: usize) -> NonNull<u8> {
        self.get_handle_location(self.get_local_handle_at(self.current_frame(), index))
    }

    pub fn set_local(&self, gc: GCHandle<'gc>, index: usize, value: StackValue<'gc>) {
        let frame = self.current_frame();
        if index < frame.pinned_locals.len() && frame.pinned_locals[index] {
            if let StackValue::ObjectRef(obj) = value {
                if obj.0.is_some() {
                    self.local.heap.pinned_objects.borrow_mut().insert(obj);
                }
            }
        }
        self.set_slot(gc, self.get_local_handle_at(frame, index), value);
    }

    pub fn push_stack(&mut self, gc: GCHandle<'gc>, value: StackValue<'gc>) {
        if self.tracer_enabled() {
            self.shared.tracer.lock().trace_stack_op(
                self.indent(),
                "PUSH",
                &format!("{:?}", value),
            );
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
            self.shared
                .tracer
                .lock()
                .trace_stack_op(self.indent(), "POP", &format!("{:?}", value));
        }
        self.current_frame_mut().stack_height -= 1;
        value
    }

    pub fn top_of_stack_address(&self) -> NonNull<u8> {
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
        self.shared.tracer.lock().is_enabled()
    }

    pub fn indent(&self) -> usize {
        if self.execution.frames.is_empty() {
            0
        } else {
            (self.execution.frames.len() - 1) % 10
        }
    }

    pub fn msg(&self, fmt: std::fmt::Arguments) {
        self.shared.tracer.lock().msg(self.indent(), fmt);
    }

    pub fn throw_by_name(&mut self, gc: GCHandle<'gc>, name: &str) -> StepResult {
        let rt = self.shared.loader.corlib_type(name);
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
            if let Some(method) = self.shared.loader.find_method_in_type(
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

    // ========================================================================
    // Global State Delegation Methods
    // These methods provide a unified interface that delegates to global state.
    // ========================================================================

    /// Get the heap manager.
    #[inline]
    pub fn heap(&self) -> &HeapManager<'gc> {
        &self.local.heap
    }

    /// Get the assembly loader.
    #[inline]
    pub fn loader(&self) -> &'m AssemblyLoader {
        self.shared.loader
    }

    /// Get access to statics storage (read-only).
    /// Returns a guard that must be held for the duration of access.
    #[inline]
    pub fn statics_read(&self) -> parking_lot::RwLockReadGuard<'_, StaticStorageManager> {
        self.shared.statics.read()
    }

    /// Get access to statics storage (mutable).
    /// Returns a guard that must be held for the duration of access.
    #[inline]
    pub fn statics_write(&self) -> parking_lot::RwLockWriteGuard<'_, StaticStorageManager> {
        self.shared.statics.write()
    }

    /// Get access to the P/Invoke libraries manager (read-only).
    #[inline]
    pub fn pinvoke_read(&self) -> parking_lot::RwLockReadGuard<'_, NativeLibraries> {
        self.shared.pinvoke.read()
    }

    /// Get access to the P/Invoke libraries manager (mutable).
    #[inline]
    pub fn pinvoke_write(&self) -> parking_lot::RwLockWriteGuard<'_, NativeLibraries> {
        self.shared.pinvoke.write()
    }

    /// Get the tracer for debugging output.
    #[inline]
    pub fn tracer(&self) -> parking_lot::MutexGuard<'_, Tracer> {
        self.shared.tracer.lock()
    }

    /// Get the empty generics instance.
    #[inline]
    pub fn empty_generics(&self) -> &GenericLookup {
        &self.shared.empty_generics
    }

    // ========================================================================
    // Reflection Cache Methods
    // ========================================================================

    /// Get access to runtime_asms cache (read-only).
    #[inline]
    pub fn runtime_asms_read(&self) -> Ref<'_, HashMap<ResolutionS, ObjectRef<'gc>>> {
        self.local.runtime_asms.borrow()
    }

    /// Get access to runtime_asms cache (mutable).
    #[inline]
    pub fn runtime_asms_write(&self) -> RefMut<'_, HashMap<ResolutionS, ObjectRef<'gc>>> {
        self.local.runtime_asms.borrow_mut()
    }

    /// Get access to runtime_types cache (read-only).
    #[inline]
    pub fn runtime_types_read(&self) -> Ref<'_, HashMap<RuntimeType, ObjectRef<'gc>>> {
        self.local.runtime_types.borrow()
    }

    /// Get access to runtime_types cache (mutable).
    #[inline]
    pub fn runtime_types_write(&self) -> RefMut<'_, HashMap<RuntimeType, ObjectRef<'gc>>> {
        self.local.runtime_types.borrow_mut()
    }

    /// Get access to runtime_types_list (read-only).
    #[inline]
    pub fn runtime_types_list_read(&self) -> Ref<'_, Vec<RuntimeType>> {
        self.local.runtime_types_list.borrow()
    }

    /// Get access to runtime_types_list (mutable).
    #[inline]
    pub fn runtime_types_list_write(&self) -> RefMut<'_, Vec<RuntimeType>> {
        self.local.runtime_types_list.borrow_mut()
    }

    /// Get access to runtime_methods list (read-only).
    #[inline]
    pub fn runtime_methods_read(&self) -> Ref<'_, Vec<(MethodDescription, GenericLookup)>> {
        self.local.runtime_methods.borrow()
    }

    /// Get access to runtime_methods list (mutable).
    #[inline]
    pub fn runtime_methods_write(&self) -> RefMut<'_, Vec<(MethodDescription, GenericLookup)>> {
        self.local.runtime_methods.borrow_mut()
    }

    /// Get access to runtime_method_objs cache (read-only).
    #[inline]
    pub fn runtime_method_objs_read(
        &self,
    ) -> Ref<'_, HashMap<(MethodDescription, GenericLookup), ObjectRef<'gc>>> {
        self.local.runtime_method_objs.borrow()
    }

    /// Get access to runtime_method_objs cache (mutable).
    #[inline]
    pub fn runtime_method_objs_write(
        &self,
    ) -> RefMut<'_, HashMap<(MethodDescription, GenericLookup), ObjectRef<'gc>>> {
        self.local.runtime_method_objs.borrow_mut()
    }

    /// Get access to runtime_fields list (read-only).
    #[inline]
    pub fn runtime_fields_read(&self) -> Ref<'_, Vec<(FieldDescription, GenericLookup)>> {
        self.local.runtime_fields.borrow()
    }

    /// Get access to runtime_fields list (mutable).
    #[inline]
    pub fn runtime_fields_write(&self) -> RefMut<'_, Vec<(FieldDescription, GenericLookup)>> {
        self.local.runtime_fields.borrow_mut()
    }

    /// Get access to runtime_field_objs cache (read-only).
    #[inline]
    pub fn runtime_field_objs_read(
        &self,
    ) -> Ref<'_, HashMap<(FieldDescription, GenericLookup), ObjectRef<'gc>>> {
        self.local.runtime_field_objs.borrow()
    }

    /// Get access to runtime_field_objs cache (mutable).
    #[inline]
    pub fn runtime_field_objs_write(
        &self,
    ) -> RefMut<'_, HashMap<(FieldDescription, GenericLookup), ObjectRef<'gc>>> {
        self.local.runtime_field_objs.borrow_mut()
    }

    /// Get access to method_tables cache (read-only).
    #[inline]
    pub fn method_tables_read(
        &self,
    ) -> parking_lot::RwLockReadGuard<'_, HashMap<TypeDescription, Box<[u8]>>> {
        self.shared.method_tables.read()
    }

    /// Get access to method_tables cache (mutable).
    #[inline]
    pub fn method_tables_write(
        &self,
    ) -> parking_lot::RwLockWriteGuard<'_, HashMap<TypeDescription, Box<[u8]>>> {
        self.shared.method_tables.write()
    }
}

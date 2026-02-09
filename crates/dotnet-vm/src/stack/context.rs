use crate::{
    MethodInfo, MethodState, MethodType, ResolutionContext, StepResult,
    exceptions::{
        ExceptionState, HandlerAddress, HandlerKind, SearchState, UnwindState, UnwindTarget,
    },
    memory::ops::MemoryOps,
    resolution::{TypeResolutionExt, ValueResolution},
    resolver::ResolverService,
    state::{ArenaLocalState, SharedGlobalState, StaticStorageManager},
    sync::{Arc, MutexGuard},
    threading::ThreadManagerOps,
    tracer::Tracer,
};
use dotnet_assemblies::AssemblyLoader;
use dotnet_types::{
    TypeDescription,
    comparer::decompose_type_source,
    generics::{ConcreteType, GenericLookup},
    members::{FieldDescription, MethodDescription},
    resolution::ResolutionS,
    runtime::RuntimeType,
};
use dotnet_utils::{
    gc::{GCHandle, ThreadSafeLock},
    sync::Ordering,
};
use dotnet_value::{
    CLRString, StackValue,
    object::{
        CTSValue, HeapStorage, Object as ObjectInstance, ObjectHandle, ObjectInner, ObjectRef,
        ValueType, Vector,
    },
    pointer::ManagedPtr,
    storage::FieldStorage,
};
use dotnetdll::prelude::*;
use gc_arena::Collect;
use std::ptr::NonNull;

use super::{
    evaluation_stack::EvaluationStack,
    frames::FrameStack,
    ops::{
        CallOps, ExceptionOps, LoaderOps, PoolOps, RawMemoryOps, ReflectionOps, ResolutionOps,
        StackOps, StaticsOps, ThreadOps, VesOps,
    },
};

pub struct VesContext<'a, 'gc, 'm> {
    pub(crate) evaluation_stack: &'a mut EvaluationStack<'gc>,
    pub(crate) frame_stack: &'a mut FrameStack<'gc, 'm>,
    pub(crate) shared: &'a Arc<SharedGlobalState<'m>>,
    pub(crate) local: &'a mut ArenaLocalState<'gc>,
    pub(crate) exception_mode: &'a mut ExceptionState<'gc>,
    pub(crate) thread_id: &'a std::cell::Cell<u64>,
    pub(crate) original_ip: &'a mut usize,
    pub(crate) original_stack_height: &'a mut usize,
}

impl<'a, 'gc, 'm: 'gc> VesContext<'a, 'gc, 'm> {
    #[inline]
    fn on_push(&mut self) {
        if let Some(frame) = self.frame_stack.current_frame_opt_mut() {
            frame.stack_height += 1;
        }
    }

    #[inline]
    fn trace_push(&self, value: &StackValue<'gc>) {
        if self.tracer_enabled() {
            let val_str = format!("{:?}", value);
            self.tracer()
                .trace_stack_op(self.indent(), "PUSH", &val_str);
        }
    }

    #[inline]
    fn trace_pop(&self, value: &StackValue<'gc>) {
        if self.tracer_enabled() {
            let val_str = format!("{:?}", value);
            self.tracer().trace_stack_op(self.indent(), "POP", &val_str);
        }
    }

    #[inline]
    fn on_pop(&mut self) {
        if let Some(frame) = self.frame_stack.current_frame_opt_mut() {
            if frame.stack_height == 0 {
                panic!(
                    "Stack height underflow in frame {:?}",
                    frame.state.info_handle.source
                );
            }
            frame.stack_height -= 1;
        }
    }

    pub fn top_of_stack_address(&self) -> NonNull<u8> {
        self.evaluation_stack
            .get_slot_address(self.evaluation_stack.top_of_stack().saturating_sub(1))
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
                    by_ref: _,
                    var_type,
                    pinned,
                    ..
                } => {
                    pinned_locals.push(*pinned);
                    let ctx = ResolutionContext {
                        generics,
                        loader: self.shared.loader,
                        resolution: method.resolution(),
                        type_owner: Some(method.parent),
                        method_owner: Some(method),
                        caches: self.shared.caches.clone(),
                        shared: Some(self.shared.clone()),
                    };

                    let v = match ctx.make_concrete(var_type).get() {
                        Type { source, .. } => {
                            let (ut, type_generics) = decompose_type_source::<ConcreteType>(source);
                            let desc = ctx.locate_type(ut);

                            if desc.is_value_type(&ctx) {
                                let new_lookup = GenericLookup {
                                    type_generics: type_generics.into(),
                                    ..generics.clone()
                                };
                                let new_ctx = ctx.with_generics(&new_lookup);
                                let instance = new_ctx.new_object(desc);
                                StackValue::ValueType(instance)
                            } else {
                                StackValue::null()
                            }
                        }
                        _ => StackValue::null(),
                    };
                    values.push(v);
                }
            }
        }
        (values, pinned_locals)
    }

    pub fn handle_return(&mut self, gc: dotnet_utils::gc::GCHandle<'gc>) -> StepResult {
        let tracer_enabled = self.tracer_enabled();
        let shared = &self.shared;
        let heap = &self.local.heap;
        let res =
            self.frame_stack
                .handle_return(gc, self.evaluation_stack, shared, heap, tracer_enabled);

        if let Some(return_type) = self
            .frame_stack
            .current_frame_opt()
            .and_then(|f| f.awaiting_invoke_return.clone())
        {
            self.frame_stack.current_frame_mut().awaiting_invoke_return = None;

            let is_void = matches!(return_type, RuntimeType::Void);
            if is_void {
                self.push(gc, StackValue::null());
            } else {
                let val = self.pop(gc);
                let return_concrete = return_type.to_concrete(self.loader());
                let td = self.loader().find_concrete_type(return_concrete.clone());
                if td.is_value_type(&self.current_context()) {
                    let boxed = ObjectRef::new(
                        gc,
                        HeapStorage::Boxed(self.new_value_type(&return_concrete, val)),
                    );
                    self.register_new_object(&boxed);
                    self.push_obj(gc, boxed);
                } else {
                    // already a reference type (or null)
                    self.push(gc, val);
                }
            }
        }

        res
    }

    pub fn locate_type(&self, handle: UserType) -> TypeDescription {
        let f = self.current_frame();
        self.resolver().locate_type(f.source_resolution, handle)
    }

    pub fn new_instance_fields(&self, td: TypeDescription) -> FieldStorage {
        self.resolver()
            .new_instance_fields(td, &self.current_context())
    }

    pub fn new_static_fields(&self, td: TypeDescription) -> FieldStorage {
        self.resolver()
            .new_static_fields(td, &self.current_context())
    }
}

impl<'a, 'gc, 'm: 'gc> StackOps<'gc, 'm> for VesContext<'a, 'gc, 'm> {
    #[inline]
    fn push(&mut self, gc: GCHandle<'gc>, value: StackValue<'gc>) {
        self.trace_push(&value);
        self.evaluation_stack.push(gc, value);
        self.on_push();
    }

    #[inline]
    fn push_i32(&mut self, gc: GCHandle<'gc>, value: i32) {
        self.trace_push(&StackValue::Int32(value));
        self.evaluation_stack.push_i32(gc, value);
        self.on_push();
    }

    #[inline]
    fn push_i64(&mut self, gc: GCHandle<'gc>, value: i64) {
        self.trace_push(&StackValue::Int64(value));
        self.evaluation_stack.push_i64(gc, value);
        self.on_push();
    }

    #[inline]
    fn push_f64(&mut self, gc: GCHandle<'gc>, value: f64) {
        self.trace_push(&StackValue::NativeFloat(value));
        self.evaluation_stack.push_f64(gc, value);
        self.on_push();
    }

    #[inline]
    fn push_obj(&mut self, gc: GCHandle<'gc>, value: ObjectRef<'gc>) {
        self.trace_push(&StackValue::ObjectRef(value));
        self.evaluation_stack.push_obj(gc, value);
        self.on_push();
    }

    #[inline]
    fn push_ptr(&mut self, gc: GCHandle<'gc>, ptr: *mut u8, t: TypeDescription, is_pinned: bool) {
        self.trace_push(&StackValue::managed_ptr(ptr, t, is_pinned));
        self.evaluation_stack.push_ptr(gc, ptr, t, is_pinned);
        self.on_push();
    }

    #[inline]
    fn push_isize(&mut self, gc: GCHandle<'gc>, value: isize) {
        self.trace_push(&StackValue::NativeInt(value));
        self.evaluation_stack.push_isize(gc, value);
        self.on_push();
    }

    #[inline]
    fn push_value_type(&mut self, gc: GCHandle<'gc>, value: ObjectInstance<'gc>) {
        self.trace_push(&StackValue::ValueType(value.clone()));
        self.evaluation_stack.push_value_type(gc, value);
        self.on_push();
    }

    #[inline]
    fn push_managed_ptr(&mut self, gc: GCHandle<'gc>, value: ManagedPtr<'gc>) {
        self.trace_push(&StackValue::ManagedPtr(value));
        self.evaluation_stack.push_managed_ptr(gc, value);
        self.on_push();
    }

    #[inline]
    fn push_string(&mut self, gc: GCHandle<'gc>, value: CLRString) {
        let in_heap = ObjectRef::new(gc, HeapStorage::Str(value));
        self.register_new_object(&in_heap);
        self.push(gc, StackValue::ObjectRef(in_heap));
    }

    #[inline]
    fn pop(&mut self, gc: GCHandle<'gc>) -> StackValue<'gc> {
        self.on_pop();
        let val = self.evaluation_stack.pop(gc);
        self.trace_pop(&val);
        val
    }

    #[inline]
    fn pop_i32(&mut self, gc: GCHandle<'gc>) -> i32 {
        self.on_pop();
        let val = self.evaluation_stack.pop_i32(gc);
        self.trace_pop(&StackValue::Int32(val));
        val
    }

    #[inline]
    fn pop_i64(&mut self, gc: GCHandle<'gc>) -> i64 {
        self.on_pop();
        let val = self.evaluation_stack.pop_i64(gc);
        self.trace_pop(&StackValue::Int64(val));
        val
    }

    #[inline]
    fn pop_f64(&mut self, gc: GCHandle<'gc>) -> f64 {
        self.on_pop();
        let val = self.evaluation_stack.pop_f64(gc);
        self.trace_pop(&StackValue::NativeFloat(val));
        val
    }

    #[inline]
    fn pop_isize(&mut self, gc: GCHandle<'gc>) -> isize {
        self.on_pop();
        let val = self.evaluation_stack.pop_isize(gc);
        self.trace_pop(&StackValue::NativeInt(val));
        val
    }

    #[inline]
    fn pop_obj(&mut self, gc: GCHandle<'gc>) -> ObjectRef<'gc> {
        self.on_pop();
        let val = self.evaluation_stack.pop_obj(gc);
        self.trace_pop(&StackValue::ObjectRef(val));
        val
    }

    #[inline]
    fn pop_ptr(&mut self, gc: GCHandle<'gc>) -> *mut u8 {
        self.on_pop();
        let val = self.evaluation_stack.pop_ptr(gc);
        self.trace_pop(&StackValue::NativeInt(val as isize));
        val
    }

    #[inline]
    fn pop_value_type(&mut self, gc: GCHandle<'gc>) -> ObjectInstance<'gc> {
        self.on_pop();
        let val = self.evaluation_stack.pop_value_type(gc);
        self.trace_pop(&StackValue::ValueType(val.clone()));
        val
    }

    #[inline]
    fn pop_managed_ptr(&mut self, gc: GCHandle<'gc>) -> ManagedPtr<'gc> {
        let val = self.evaluation_stack.pop_managed_ptr(gc);
        self.trace_pop(&StackValue::ManagedPtr(val));
        self.on_pop();
        val
    }

    #[inline]
    fn pop_multiple(&mut self, gc: GCHandle<'gc>, count: usize) -> Vec<StackValue<'gc>> {
        let mut results = Vec::with_capacity(count);
        for _ in 0..count {
            results.push(self.pop(gc));
        }
        results.reverse();
        results
    }

    #[inline]
    fn peek_multiple(&self, count: usize) -> Vec<StackValue<'gc>> {
        self.evaluation_stack.peek_multiple(count)
    }

    #[inline]
    fn dup(&mut self, gc: GCHandle<'gc>) {
        let val = self.pop(gc);
        self.push(gc, val.clone());
        self.push(gc, val);
    }

    #[inline]
    fn peek(&self) -> Option<StackValue<'gc>> {
        self.evaluation_stack.stack.last().cloned()
    }

    #[inline]
    fn peek_stack(&self) -> StackValue<'gc> {
        self.evaluation_stack.peek_stack()
    }

    #[inline]
    fn peek_stack_at(&self, offset: usize) -> StackValue<'gc> {
        self.evaluation_stack.peek_stack_at(offset)
    }

    #[inline]
    fn get_local(&self, index: usize) -> StackValue<'gc> {
        let frame = self.frame_stack.current_frame();
        self.evaluation_stack.get_slot(frame.base.locals + index)
    }

    #[inline]
    fn set_local(&mut self, gc: GCHandle<'gc>, index: usize, value: StackValue<'gc>) {
        let bp = self.frame_stack.current_frame().base;
        self.evaluation_stack.set_slot(gc, bp.locals + index, value);
    }

    #[inline]
    fn get_argument(&self, index: usize) -> StackValue<'gc> {
        let frame = self.frame_stack.current_frame();
        self.evaluation_stack.get_slot(frame.base.arguments + index)
    }

    #[inline]
    fn set_argument(&mut self, gc: GCHandle<'gc>, index: usize, value: StackValue<'gc>) {
        let bp = self.frame_stack.current_frame().base;
        self.evaluation_stack
            .set_slot(gc, bp.arguments + index, value);
    }

    #[inline]
    fn get_local_address(&self, index: usize) -> std::ptr::NonNull<u8> {
        let frame = self.frame_stack.current_frame();
        self.evaluation_stack
            .get_slot_address(frame.base.locals + index)
    }

    #[inline]
    fn get_argument_address(&self, index: usize) -> std::ptr::NonNull<u8> {
        let frame = self.frame_stack.current_frame();
        self.evaluation_stack
            .get_slot_address(frame.base.arguments + index)
    }

    #[inline]
    fn current_frame(&self) -> &crate::stack::StackFrame<'gc, 'm> {
        self.frame_stack.current_frame()
    }

    #[inline]
    fn current_frame_mut(&mut self) -> &mut crate::stack::StackFrame<'gc, 'm> {
        self.frame_stack.current_frame_mut()
    }

    #[inline]
    fn get_local_info_for_managed_ptr(&self, index: usize) -> (std::ptr::NonNull<u8>, bool) {
        let frame = self.frame_stack.current_frame();
        let addr = self
            .evaluation_stack
            .get_slot_address(frame.base.locals + index);
        let is_pinned = frame.pinned_locals.get(index).copied().unwrap_or(false);
        (addr, is_pinned)
    }

    #[inline]
    fn get_slot(&self, index: usize) -> StackValue<'gc> {
        self.evaluation_stack.get_slot(index)
    }

    #[inline]
    fn get_slot_ref(&self, index: usize) -> &StackValue<'gc> {
        self.evaluation_stack.get_slot_ref(index)
    }

    #[inline]
    fn set_slot(&mut self, gc: GCHandle<'gc>, index: usize, value: StackValue<'gc>) {
        self.evaluation_stack.set_slot(gc, index, value)
    }

    #[inline]
    fn top_of_stack(&self) -> usize {
        self.evaluation_stack.top_of_stack()
    }

    #[inline]
    fn get_slot_address(&self, index: usize) -> std::ptr::NonNull<u8> {
        self.evaluation_stack.get_slot_address(index)
    }
}

impl<'a, 'gc, 'm: 'gc> MemoryOps<'gc> for VesContext<'a, 'gc, 'm> {
    #[inline]
    fn new_vector(&self, element: ConcreteType, size: usize) -> Vector<'gc> {
        self.resolver()
            .new_vector(element, size, &self.current_context())
    }

    #[inline]
    fn new_object(&self, td: TypeDescription) -> ObjectInstance<'gc> {
        self.resolver().new_object(td, &self.current_context())
    }

    #[inline]
    fn new_value_type(&self, t: &ConcreteType, data: StackValue<'gc>) -> ValueType<'gc> {
        self.resolver()
            .new_value_type(t, data, &self.current_context())
    }

    #[inline]
    fn new_cts_value(&self, t: &ConcreteType, data: StackValue<'gc>) -> CTSValue<'gc> {
        self.resolver()
            .new_cts_value(t, data, &self.current_context())
    }

    #[inline]
    fn read_cts_value(&self, t: &ConcreteType, data: &[u8], gc: GCHandle<'gc>) -> CTSValue<'gc> {
        self.resolver()
            .read_cts_value(t, data, gc, &self.current_context())
    }

    fn clone_object(&self, gc: GCHandle<'gc>, obj: ObjectRef<'gc>) -> ObjectRef<'gc> {
        let h = obj.0.expect("cannot clone null object");
        let inner = h.borrow();
        let cloned_storage = inner.storage.clone();

        let new_inner = ObjectInner {
            #[cfg(any(feature = "memory-validation", debug_assertions))]
            magic: inner.magic,
            owner_id: self.thread_id.get(),
            storage: cloned_storage,
        };

        let new_h = gc_arena::Gc::new(gc, ThreadSafeLock::new(new_inner));
        let new_ref = ObjectRef(Some(new_h));
        self.register_new_object(&new_ref);
        new_ref
    }

    #[inline]
    fn register_new_object(&self, instance: &ObjectRef<'gc>) {
        if let Some(ptr) = instance.0 {
            let addr = gc_arena::Gc::as_ptr(ptr) as usize;
            self.local
                .heap
                ._all_objs
                .borrow_mut()
                .insert(addr, *instance);

            // Add to finalization queue if it has a finalizer
            if let HeapStorage::Obj(o) = &ptr.borrow().storage
                && (o.description.static_initializer().is_some()
                    || o.description
                        .definition()
                        .methods
                        .iter()
                        .any(|m| m.name == "Finalize"))
            {
                self.local
                    .heap
                    .finalization_queue
                    .borrow_mut()
                    .push(*instance);
            }
        }
    }

    #[inline]
    fn heap(&self) -> &crate::memory::heap::HeapManager<'gc> {
        &self.local.heap
    }
}

impl<'a, 'gc, 'm: 'gc> ExceptionOps<'gc> for VesContext<'a, 'gc, 'm> {
    #[inline]
    fn throw_by_name(&mut self, gc: GCHandle<'gc>, name: &str) -> StepResult {
        let exception_type = self.shared.loader.corlib_type(name);
        let instance = self.new_object(exception_type);
        let obj_ref = ObjectRef::new(gc, HeapStorage::Obj(instance));
        self.register_new_object(&obj_ref);
        *self.exception_mode = ExceptionState::Throwing(obj_ref);
        StepResult::Exception
    }

    #[inline]
    fn throw(&mut self, gc: GCHandle<'gc>, exception: ObjectRef<'gc>) -> StepResult {
        *self.exception_mode = ExceptionState::Throwing(exception);
        self.handle_exception(gc)
    }

    #[inline]
    fn rethrow(&mut self, gc: GCHandle<'gc>) -> StepResult {
        let exception = self
            .current_frame()
            .exception_stack
            .last()
            .cloned()
            .expect("rethrow without active exception");
        *self.exception_mode = ExceptionState::Throwing(exception);
        self.handle_exception(gc)
    }

    #[inline]
    fn leave(&mut self, gc: GCHandle<'gc>, target_ip: usize) -> StepResult {
        let cursor = HandlerAddress {
            frame_index: self.frame_stack.len() - 1,
            section_index: 0,
            handler_index: 0,
        };

        *self.exception_mode = ExceptionState::Unwinding(UnwindState {
            exception: None,
            target: UnwindTarget::Instruction(target_ip),
            cursor,
        });
        self.handle_exception(gc)
    }

    #[inline]
    fn endfinally(&mut self, gc: GCHandle<'gc>) -> StepResult {
        match self.exception_mode {
            ExceptionState::ExecutingHandler(state) => {
                *self.exception_mode = ExceptionState::Unwinding(*state);
                self.handle_exception(gc)
            }
            _ => panic!(
                "endfinally called outside of handler, state: {:?}",
                self.exception_mode
            ),
        }
    }

    #[inline]
    fn endfilter(&mut self, _gc: GCHandle<'gc>, result: i32) -> StepResult {
        let (exception, handler) = match self.exception_mode {
            ExceptionState::Filtering(state) => (state.exception, state.handler),
            _ => panic!("EndFilter called but not in Filtering mode"),
        };

        // Restore state of the frame where filter ran
        let frame = &mut self.frame_stack.frames[handler.frame_index];
        frame.state.ip = *self.original_ip;
        frame.stack_height = *self.original_stack_height;
        frame.exception_stack.pop();

        // Restore suspended evaluation and frame stacks
        self.evaluation_stack.restore_suspended();
        self.frame_stack.restore_suspended();

        if result == 1 {
            // Result is 1: This handler is the one. Begin unwinding towards it.
            *self.exception_mode = ExceptionState::Unwinding(UnwindState {
                exception: Some(exception),
                target: UnwindTarget::Handler(handler),
                cursor: HandlerAddress {
                    frame_index: self.frame_stack.len() - 1,
                    section_index: 0,
                    handler_index: 0,
                },
            });
        } else {
            // Result is 0: Continue searching after this handler.
            *self.exception_mode = ExceptionState::Searching(SearchState {
                exception,
                cursor: HandlerAddress {
                    frame_index: handler.frame_index,
                    section_index: handler.section_index,
                    handler_index: handler.handler_index + 1,
                },
            });
        }

        StepResult::Exception
    }

    #[inline]
    fn ret(&mut self, gc: GCHandle<'gc>) -> StepResult {
        let frame_index = self.frame_stack.len() - 1;
        if let ExceptionState::ExecutingHandler(state) = *self.exception_mode
            && state.cursor.frame_index == frame_index
        {
            *self.exception_mode = ExceptionState::Unwinding(UnwindState {
                exception: state.exception,
                target: UnwindTarget::Instruction(usize::MAX),
                cursor: state.cursor,
            });
            return self.handle_exception(gc);
        }

        let ip = self.current_frame().state.ip;
        let mut has_finally_blocks = false;
        for section in &self.current_frame().state.info_handle.exceptions {
            if section.instructions.contains(&ip)
                && section
                    .handlers
                    .iter()
                    .any(|h| matches!(h.kind, HandlerKind::Finally | HandlerKind::Fault))
            {
                has_finally_blocks = true;
                break;
            }
        }

        if has_finally_blocks {
            *self.exception_mode = ExceptionState::Unwinding(UnwindState {
                exception: None,
                target: UnwindTarget::Instruction(usize::MAX),
                cursor: HandlerAddress {
                    frame_index,
                    section_index: 0,
                    handler_index: 0,
                },
            });
            return self.handle_exception(gc);
        }

        StepResult::Return
    }
}

impl<'a, 'gc, 'm: 'gc> ResolutionOps<'gc, 'm> for VesContext<'a, 'gc, 'm> {
    #[inline]
    fn stack_value_type(&self, val: &StackValue<'gc>) -> TypeDescription {
        self.resolver().stack_value_type(val)
    }

    #[inline]
    fn make_concrete(&self, t: &MethodType) -> ConcreteType {
        let f = self.current_frame();
        self.resolver()
            .make_concrete(f.source_resolution, &f.generic_inst, t)
    }

    #[inline]
    fn current_context(&self) -> ResolutionContext<'_, 'm> {
        if !self.frame_stack.is_empty() {
            let f = self.frame_stack.current_frame();
            ResolutionContext {
                generics: &f.generic_inst,
                loader: self.shared.loader,
                resolution: f.source_resolution,
                type_owner: Some(f.state.info_handle.source.parent),
                method_owner: Some(f.state.info_handle.source),
                caches: self.shared.caches.clone(),
                shared: Some(self.shared.clone()),
            }
        } else {
            ResolutionContext {
                generics: &self.shared.empty_generics,
                loader: self.shared.loader,
                resolution: self.shared.loader.corlib_type("System.Object").resolution,
                type_owner: None,
                method_owner: None,
                caches: self.shared.caches.clone(),
                shared: Some(self.shared.clone()),
            }
        }
    }

    #[inline]
    fn with_generics<'b>(&self, lookup: &'b GenericLookup) -> ResolutionContext<'b, 'm> {
        let frame = self.frame_stack.current_frame();
        ResolutionContext {
            loader: self.shared.loader,
            resolution: frame.source_resolution,
            generics: lookup,
            caches: self.shared.caches.clone(),
            type_owner: None,
            method_owner: None,
            shared: Some(self.shared.clone()),
        }
    }
}

impl<'a, 'gc, 'm: 'gc> PoolOps for VesContext<'a, 'gc, 'm> {
    /// # Safety
    ///
    /// The returned pointer is valid for the duration of the current method frame.
    /// It must not be stored in a way that outlives the frame.
    #[inline]
    fn localloc(&mut self, size: usize) -> *mut u8 {
        let s = self.state_mut();
        let loc = s.memory_pool.len();
        s.memory_pool.extend(vec![0; size]);
        s.memory_pool[loc..].as_mut_ptr()
    }
}

impl<'a, 'gc, 'm: 'gc> RawMemoryOps<'gc> for VesContext<'a, 'gc, 'm> {
    /// # Safety
    ///
    /// The caller must ensure that `ptr` is valid for writes and properly represents the memory location.
    /// If `owner` is provided, it must be the object that contains `ptr` to ensure GC safety.
    /// The `layout` must match the expected type of `value`.
    #[inline]
    unsafe fn write_unaligned(
        &mut self,
        ptr: *mut u8,
        owner: Option<ObjectRef<'gc>>,
        value: StackValue<'gc>,
        layout: &dotnet_value::layout::LayoutManager,
    ) -> Result<(), String> {
        let heap = &self.local.heap;
        let mut memory = crate::memory::RawMemoryAccess::new(heap);
        // SAFETY: The caller of RawMemoryOps::write_unaligned must ensure ptr is valid.
        // We delegate to RawMemoryAccess which performs owner-based validation.
        unsafe { memory.write_unaligned(ptr, owner, value, layout) }
    }

    /// # Safety
    ///
    /// The caller must ensure that `ptr` is valid for reads and points to a memory location.
    /// If `owner` is provided, it must be the object that contains `ptr` to ensure GC safety.
    /// The `layout` must match the expected type stored at `ptr`.
    #[inline]
    unsafe fn read_unaligned(
        &self,
        ptr: *const u8,
        owner: Option<ObjectRef<'gc>>,
        layout: &dotnet_value::layout::LayoutManager,
        type_desc: Option<TypeDescription>,
    ) -> Result<StackValue<'gc>, String> {
        let heap = &self.local.heap;
        let memory = crate::memory::RawMemoryAccess::new(heap);
        // SAFETY: The caller of RawMemoryOps::read_unaligned must ensure ptr is valid.
        // We delegate to RawMemoryAccess which performs owner-based validation.
        unsafe { memory.read_unaligned(ptr, owner, layout, type_desc) }
    }

    #[inline]
    fn check_gc_safe_point(&self) {
        let thread_manager = &self.shared.thread_manager;
        if thread_manager.is_gc_stop_requested() {
            let managed_id = self.thread_id.get();
            if managed_id != 0 {
                #[cfg(feature = "multithreaded-gc")]
                thread_manager.safe_point(managed_id, &self.shared.gc_coordinator);
                #[cfg(not(feature = "multithreaded-gc"))]
                thread_manager.safe_point(managed_id, &Default::default());
            }
        }
    }
}

impl<'a, 'gc, 'm: 'gc> LoaderOps<'m> for VesContext<'a, 'gc, 'm> {
    #[inline]
    fn loader(&self) -> &'m AssemblyLoader {
        self.shared.loader
    }

    #[inline]
    fn resolver(&self) -> ResolverService<'m> {
        ResolverService::new(self.shared.clone())
    }

    #[inline]
    fn shared(&self) -> &Arc<SharedGlobalState<'m>> {
        self.shared
    }
}

impl<'a, 'gc, 'm: 'gc> StaticsOps<'gc> for VesContext<'a, 'gc, 'm> {
    #[inline]
    fn statics(&self) -> &StaticStorageManager {
        &self.shared.statics
    }

    #[inline]
    fn initialize_static_storage(
        &mut self,
        gc: GCHandle<'gc>,
        description: TypeDescription,
        generics: GenericLookup,
    ) -> StepResult {
        self.check_gc_safe_point();

        let ctx = ResolutionContext {
            resolution: description.resolution,
            generics: &generics,
            loader: self.loader(),
            type_owner: Some(description),
            method_owner: None,
            caches: self.shared.caches.clone(),
            shared: Some(self.shared.clone()),
        };

        loop {
            let tid = self.thread_id.get();
            let init_result =
                self.shared
                    .statics
                    .init(description, &ctx, tid, Some(&self.shared.metrics));

            use crate::statics::StaticInitResult::*;
            match init_result {
                Execute(m) => {
                    crate::vm_trace!(
                        self,
                        "-- calling static constructor (will return to ip {}) --",
                        self.current_frame().state.ip
                    );
                    self.call_frame(
                        gc,
                        MethodInfo::new(m, &generics, self.shared.clone()),
                        generics.clone(),
                    );
                    return StepResult::FramePushed;
                }
                Initialized | Recursive => {
                    return StepResult::Continue;
                }
                Waiting => {
                    self.check_gc_safe_point();
                    std::thread::yield_now();
                }
                Failed => {
                    return self.throw_by_name(gc, "System.TypeInitializationException");
                }
            }
        }
    }
}

impl<'a, 'gc, 'm: 'gc> ThreadOps for VesContext<'a, 'gc, 'm> {
    #[inline]
    fn thread_id(&self) -> usize {
        self.thread_id.get() as usize
    }
}

impl<'a, 'gc, 'm: 'gc> ReflectionOps<'gc, 'm> for VesContext<'a, 'gc, 'm> {
    #[inline]
    fn pre_initialize_reflection(&mut self, gc: GCHandle<'gc>) {
        crate::intrinsics::reflection::common::pre_initialize_reflection(self, gc)
    }

    #[inline]
    fn get_runtime_method_index(
        &mut self,
        method: MethodDescription,
        lookup: GenericLookup,
    ) -> usize {
        crate::intrinsics::reflection::common::get_runtime_method_index(self, method, lookup)
            as usize
    }

    #[inline]
    fn get_runtime_type(&mut self, gc: GCHandle<'gc>, target: RuntimeType) -> ObjectRef<'gc> {
        crate::intrinsics::reflection::common::get_runtime_type(self, gc, target)
    }

    #[inline]
    fn get_runtime_method_obj(
        &mut self,
        gc: GCHandle<'gc>,
        method: MethodDescription,
        lookup: GenericLookup,
    ) -> ObjectRef<'gc> {
        crate::intrinsics::reflection::common::get_runtime_method_obj(self, gc, method, lookup)
    }

    #[inline]
    fn get_runtime_field_obj(
        &mut self,
        gc: GCHandle<'gc>,
        field: FieldDescription,
        lookup: GenericLookup,
    ) -> ObjectRef<'gc> {
        crate::intrinsics::reflection::common::get_runtime_field_obj(self, gc, field, lookup)
    }

    #[inline]
    fn make_runtime_type(
        &self,
        ctx: &ResolutionContext<'_, 'm>,
        source: &MethodType,
    ) -> RuntimeType {
        crate::intrinsics::reflection::common::make_runtime_type(ctx, source)
    }

    #[inline]
    fn get_heap_description(&self, object: ObjectHandle<'gc>) -> TypeDescription {
        self.resolver().get_heap_description(object)
    }

    #[inline]
    fn locate_field(&self, handle: FieldSource) -> (FieldDescription, GenericLookup) {
        let context = self.current_context();
        self.resolver()
            .locate_field(context.resolution, handle, context.generics)
    }

    #[inline]
    fn is_intrinsic_field_cached(&self, field: FieldDescription) -> bool {
        self.resolver().is_intrinsic_field_cached(field)
    }

    #[inline]
    fn is_intrinsic_cached(&self, method: MethodDescription) -> bool {
        self.resolver().is_intrinsic_cached(method)
    }

    #[inline]
    fn resolve_runtime_type(&self, obj: ObjectRef<'gc>) -> RuntimeType {
        crate::intrinsics::reflection::common::resolve_runtime_type(self, obj)
    }

    #[inline]
    fn resolve_runtime_method(&self, obj: ObjectRef<'gc>) -> (MethodDescription, GenericLookup) {
        crate::intrinsics::reflection::common::resolve_runtime_method(self, obj)
    }

    #[inline]
    fn resolve_runtime_field(&self, obj: ObjectRef<'gc>) -> (FieldDescription, GenericLookup) {
        crate::intrinsics::reflection::common::resolve_runtime_field(self, obj)
    }

    #[inline]
    fn lookup_method_by_index(&self, index: usize) -> (MethodDescription, GenericLookup) {
        #[cfg(feature = "multithreaded-gc")]
        return self
            .shared
            .shared_runtime_methods_rev
            .get(&index)
            .map(|e| e.clone())
            .expect("invalid method index in delegate");

        #[cfg(not(feature = "multithreaded-gc"))]
        self.reflection().methods_read()[index].clone()
    }

    #[inline]
    fn reflection(&self) -> crate::state::ReflectionRegistry<'_, 'gc> {
        crate::state::ReflectionRegistry::new(self.local)
    }
}

impl<'a, 'gc, 'm: 'gc> CallOps<'gc, 'm> for VesContext<'a, 'gc, 'm> {
    #[inline]
    fn constructor_frame(
        &mut self,
        gc: GCHandle<'gc>,
        instance: ObjectInstance<'gc>,
        method: crate::MethodInfo<'m>,
        generic_inst: GenericLookup,
    ) {
        let desc = instance.description;

        let value = if desc.is_value_type(&self.current_context()) {
            StackValue::ValueType(instance)
        } else {
            let in_heap = ObjectRef::new(gc, HeapStorage::Obj(instance));
            self.register_new_object(&in_heap);
            StackValue::ObjectRef(in_heap)
        };

        let num_params = method.signature.parameters.len();
        let args = self.pop_multiple(gc, num_params);

        if desc.is_value_type(&self.current_context()) {
            self.push(gc, value);
            let index = self.evaluation_stack.top_of_stack() - 1;
            let ptr = self.evaluation_stack.get_slot_address(index).as_ptr() as *mut _;
            self.push(
                gc,
                StackValue::managed_stack_ptr(index, 0, ptr, desc, false),
            );
        } else {
            self.push(gc, value.clone());
            self.push(gc, value);
        }

        for arg in args {
            self.push(gc, arg);
        }
        self.call_frame(gc, method, generic_inst);
    }

    #[inline]
    fn call_frame(
        &mut self,
        gc: GCHandle<'gc>,
        method: crate::MethodInfo<'m>,
        generic_inst: GenericLookup,
    ) {
        if self.tracer_enabled() {
            let method_desc = format!("{:?}", method.source);
            self.shared
                .tracer
                .lock()
                .trace_method_entry(self.indent(), &method_desc, "");
        }

        let num_args = method.signature.instance as usize + method.signature.parameters.len();
        let argument_base = self
            .evaluation_stack
            .top_of_stack()
            .checked_sub(num_args)
            .expect("not enough values on stack for call");

        let locals_base = self.evaluation_stack.top_of_stack();
        let (local_values, pinned_locals) =
            self.init_locals(method.source, method.locals, &generic_inst);

        for (i, v) in local_values.into_iter().enumerate() {
            self.evaluation_stack.set_slot_at(gc, locals_base + i, v);
        }

        let stack_base = locals_base + pinned_locals.len();

        if let Some(frame) = self.frame_stack.current_frame_opt_mut() {
            if frame.stack_height < num_args {
                panic!(
                    "Not enough values on stack for call: height={}, args={} in {:?}",
                    frame.stack_height, num_args, frame.state.info_handle.source
                );
            }
            frame.stack_height -= num_args;
        }

        self.frame_stack.push(StackFrame::new(
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

    #[inline]
    fn entrypoint_frame(
        &mut self,
        gc: GCHandle<'gc>,
        method: crate::MethodInfo<'m>,
        generic_inst: GenericLookup,
        args: Vec<StackValue<'gc>>,
    ) {
        let argument_base = self.evaluation_stack.top_of_stack();
        for a in args {
            self.push(gc, a);
        }
        let locals_base = self.evaluation_stack.top_of_stack();
        let (local_values, pinned_locals) =
            self.init_locals(method.source, method.locals, &generic_inst);
        for v in local_values {
            self.push(gc, v);
        }
        let stack_base = self.evaluation_stack.top_of_stack();

        self.frame_stack.push(StackFrame::new(
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

    #[inline]
    fn dispatch_method(
        &mut self,
        gc: GCHandle<'gc>,
        method: MethodDescription,
        lookup: GenericLookup,
    ) -> StepResult {
        if let Some(intrinsic) = self.shared.caches.intrinsic_registry.get(&method) {
            intrinsic(self, gc, method, &lookup)
        } else if method.method.pinvoke.is_some() {
            crate::pinvoke::external_call(self, method, gc);
            if matches!(*self.exception_mode, ExceptionState::Throwing(_)) {
                StepResult::Exception
            } else {
                StepResult::Continue
            }
        } else {
            if method.method.body.is_none() {
                if let Some(result) =
                    crate::intrinsics::delegates::try_delegate_dispatch(self, gc, method, &lookup)
                {
                    return result;
                }

                panic!(
                    "no body in executing method: {}.{}",
                    method.parent.type_name(),
                    method.method.name
                );
            }

            let info = MethodInfo::new(method, &lookup, self.shared.clone());
            self.call_frame(gc, info, lookup);
            StepResult::FramePushed
        }
    }

    #[inline]
    fn unified_dispatch(
        &mut self,
        gc: GCHandle<'gc>,
        source: &MethodSource,
        this_type: Option<TypeDescription>,
        ctx: Option<&ResolutionContext<'_, 'm>>,
    ) -> StepResult {
        let context = ctx.cloned().unwrap_or_else(|| self.current_context());
        let (resolved, lookup) = self.resolver().find_generic_method(source, &context);

        let final_method = if let Some(this_type) = this_type {
            self.resolver()
                .resolve_virtual_method(resolved, this_type, &lookup, &context)
        } else {
            resolved
        };

        self.dispatch_method(gc, final_method, lookup)
    }
}

impl<'a, 'gc, 'm: 'gc> VesOps<'gc, 'm> for VesContext<'a, 'gc, 'm> {
    #[inline]
    fn run(&mut self, gc: GCHandle<'gc>) -> StepResult {
        loop {
            let res = match self.exception_mode {
                ExceptionState::None
                | ExceptionState::ExecutingHandler(_)
                | ExceptionState::Filtering(_) => {
                    let mut last_res = StepResult::Continue;
                    for _ in 0..128 {
                        let ip = self.state().ip;
                        let i = &self.state().info_handle.instructions[ip];

                        last_res = if let Some(res) =
                            crate::dispatch::InstructionRegistry::dispatch(gc, self, i)
                        {
                            res
                        } else {
                            panic!("Unregistered instruction: {:?}", i);
                        };

                        match last_res {
                            StepResult::Continue => {
                                self.increment_ip();
                            }
                            StepResult::Jump(target) => {
                                self.branch(target);
                                last_res = StepResult::Continue;
                            }
                            StepResult::Yield => {
                                break;
                            }
                            StepResult::Return => {
                                last_res = self.handle_return(gc);
                                break;
                            }
                            _ => break,
                        }
                    }
                    last_res
                }
                _ => {
                    let res = self.handle_exception(gc);
                    match res {
                        StepResult::Jump(target) => {
                            self.branch(target);
                            StepResult::Continue
                        }
                        StepResult::Return => self.handle_return(gc),
                        _ => res,
                    }
                }
            };

            if res == StepResult::Exception {
                continue;
            }

            return res;
        }
    }

    #[inline]
    fn handle_exception(&mut self, gc: GCHandle<'gc>) -> StepResult {
        crate::exceptions::ExceptionHandlingSystem.handle_exception(self, gc)
    }

    #[inline]
    fn branch(&mut self, target: usize) {
        crate::vm_trace_branch!(self, "BR", target, true);
        self.frame_stack.branch(target);
    }

    #[inline]
    fn conditional_branch(&mut self, condition: bool, target: usize) -> bool {
        crate::vm_trace_branch!(self, "BR_COND", target, condition);
        self.frame_stack.conditional_branch(condition, target)
    }

    #[inline]
    fn increment_ip(&mut self) {
        self.frame_stack.increment_ip();
    }

    #[inline]
    fn state(&self) -> &crate::MethodState<'m> {
        self.frame_stack.state()
    }

    #[inline]
    fn state_mut(&mut self) -> &mut crate::MethodState<'m> {
        self.frame_stack.state_mut()
    }

    #[inline]
    fn exception_mode(&self) -> &ExceptionState<'gc> {
        self.exception_mode
    }

    #[inline]
    fn exception_mode_mut(&mut self) -> &mut ExceptionState<'gc> {
        self.exception_mode
    }

    #[inline]
    fn evaluation_stack(&self) -> &EvaluationStack<'gc> {
        self.evaluation_stack
    }

    #[inline]
    fn evaluation_stack_mut(&mut self) -> &mut EvaluationStack<'gc> {
        self.evaluation_stack
    }

    #[inline]
    fn frame_stack(&self) -> &FrameStack<'gc, 'm> {
        self.frame_stack
    }

    #[inline]
    fn frame_stack_mut(&mut self) -> &mut FrameStack<'gc, 'm> {
        self.frame_stack
    }

    #[inline]
    fn original_ip(&self) -> usize {
        *self.original_ip
    }

    #[inline]
    fn original_ip_mut(&mut self) -> &mut usize {
        self.original_ip
    }

    #[inline]
    fn original_stack_height(&self) -> usize {
        *self.original_stack_height
    }

    #[inline]
    fn original_stack_height_mut(&mut self) -> &mut usize {
        self.original_stack_height
    }

    #[inline]
    fn unwind_frame(&mut self, gc: GCHandle<'gc>) {
        self.frame_stack
            .unwind_frame(gc, self.evaluation_stack, self.shared, &self.local.heap);
    }

    #[inline]
    fn tracer_enabled(&self) -> bool {
        self.shared.tracer_enabled.load(Ordering::Relaxed)
    }

    #[inline]
    fn tracer(&self) -> MutexGuard<'_, Tracer> {
        self.shared.tracer.lock()
    }

    #[inline]
    fn indent(&self) -> usize {
        self.frame_stack.len()
    }

    #[inline]
    fn process_pending_finalizers(&mut self, gc: GCHandle<'gc>) -> StepResult {
        if self.local.heap.processing_finalizer.get() {
            return StepResult::Continue;
        }

        let instance = self.local.heap.pending_finalization.borrow_mut().pop();

        // Only run finalizers if we are at the bottom of the stack or in a safe state
        // For now, let's just run them one by one
        if let Some(instance) = instance {
            self.local.heap.processing_finalizer.set(true);
            let ctx = self.current_context();

            if self.tracer_enabled() {
                let ptr = instance.0.unwrap();
                let obj_type = match &ptr.borrow().storage {
                    HeapStorage::Obj(o) => o.description,
                    _ => unreachable!(),
                };
                let type_name = format!("{:?}", obj_type);
                let addr = gc_arena::Gc::as_ptr(ptr) as usize;
                self.shared
                    .tracer
                    .lock()
                    .trace_gc_finalization(self.indent(), &type_name, addr);
            }

            let finalizer: Option<MethodDescription> = instance.as_heap_storage(|storage| {
                if let HeapStorage::Obj(o) = storage {
                    let obj_type = o.description;
                    let object_type = self.shared.loader.corlib_type("System.Object");
                    let base_finalize = object_type
                        .definition()
                        .methods
                        .iter()
                        .find(|m| {
                            m.name == "Finalize"
                                && m.virtual_member
                                && m.signature.parameters.is_empty()
                        })
                        .expect("System.Object::Finalize not found");

                    let method_desc = MethodDescription {
                        parent: object_type,
                        method: base_finalize,
                        method_resolution: object_type.resolution,
                    };

                    Some(self.resolver().resolve_virtual_method(
                        method_desc,
                        obj_type,
                        &GenericLookup::default(),
                        &ctx,
                    ))
                } else {
                    None
                }
            });

            let Some(finalizer) = finalizer else {
                self.local.heap.processing_finalizer.set(false);
                return StepResult::Continue;
            };

            let method_info =
                MethodInfo::new(finalizer, &self.shared.empty_generics, self.shared.clone());

            // Push the object as 'this'
            self.push(gc, StackValue::ObjectRef(instance));

            self.call_frame(gc, method_info, self.shared.empty_generics.clone());
            self.current_frame_mut().is_finalizer = true;
            return StepResult::FramePushed;
        }

        StepResult::Continue
    }

    #[inline]
    fn back_up_ip(&mut self) {
        self.frame_stack.current_frame_mut().state.ip = *self.original_ip;
        self.evaluation_stack.truncate(*self.original_stack_height);
        if let Some(frame) = self.frame_stack.current_frame_opt_mut() {
            frame.stack_height = (*self.original_stack_height).saturating_sub(frame.base.stack);
        }
    }
}

#[derive(Collect)]
#[collect(no_drop)]
pub struct ThreadContext<'gc, 'm> {
    pub evaluation_stack: EvaluationStack<'gc>,
    pub frame_stack: FrameStack<'gc, 'm>,
    pub exception_mode: ExceptionState<'gc>,
    pub original_ip: usize,
    pub original_stack_height: usize,
}

#[derive(Collect)]
#[collect(no_drop)]
pub struct MulticastState<'gc> {
    pub targets: ObjectHandle<'gc>,
    pub next_index: usize,
    pub args: Vec<StackValue<'gc>>,
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
    pub multicast_state: Option<MulticastState<'gc>>,
    pub awaiting_invoke_return: Option<RuntimeType>,
}

// SAFETY: `StackFrame` correctly traces all GC-managed fields (`exception_stack`, `state`, `generic_inst`, and `multicast_state`)
// in its `trace` implementation. All other fields are either primitives or non-GC references
// and do not need tracing. This implementation is safe because it delegates to the `Collect`
// implementations of its components.
unsafe impl<'gc, 'm> Collect for StackFrame<'gc, 'm> {
    fn trace(&self, cc: &gc_arena::Collection) {
        self.exception_stack.trace(cc);
        self.state.trace(cc);
        self.generic_inst.trace(cc);
        self.multicast_state.trace(cc);
        self.awaiting_invoke_return.trace(cc);
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
            multicast_state: None,
            awaiting_invoke_return: None,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct BasePointer {
    pub arguments: usize,
    pub locals: usize,
    pub stack: usize,
}

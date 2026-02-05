use crate::{
    MethodInfo, MethodState, MethodType, ResolutionContext, StepResult,
    memory::heap::HeapManager,
    resolution::{TypeResolutionExt, ValueResolution},
    resolver::ResolverService,
    stack::ExceptionState,
    state::{ArenaLocalState, ReflectionRegistry, SharedGlobalState, StaticStorageManager},
    sync::{Arc, MutexGuard},
    threading::ThreadManagerOps,
    tracer::Tracer,
};
use dotnet_assemblies::{AssemblyLoader, decompose_type_source};
use dotnet_types::{
    TypeDescription,
    generics::{ConcreteType, GenericLookup},
    members::{FieldDescription, MethodDescription},
    resolution::ResolutionS,
};
use dotnet_value::{
    StackValue,
    layout::HasLayout,
    object::{CTSValue, HeapStorage, Object as ObjectInstance, ObjectRef, ValueType, Vector},
    pointer::ManagedPtr,
    storage::FieldStorage,
};
use dotnetdll::prelude::*;
use gc_arena::Collect;
use std::{ptr::NonNull, sync::atomic::Ordering};

use super::{evaluation_stack::EvaluationStack, frames::FrameStack};

pub struct VesContext<'a, 'gc, 'm> {
    pub evaluation_stack: &'a mut EvaluationStack<'gc>,
    pub frame_stack: &'a mut FrameStack<'gc, 'm>,
    pub shared: &'a Arc<SharedGlobalState<'m>>,
    pub local: &'a mut ArenaLocalState<'gc>,
    pub exception_mode: &'a mut ExceptionState<'gc>,
    pub thread_id: &'a std::cell::Cell<u64>,
    pub original_ip: &'a mut usize,
    pub original_stack_height: &'a mut usize,
}

impl<'a, 'gc, 'm: 'gc> VesContext<'a, 'gc, 'm> {
    pub fn resolver(&self) -> ResolverService<'m> {
        ResolverService::new(self.shared.clone())
    }

    pub fn check_gc_safe_point(&self) {
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

    #[inline]
    pub fn push(&mut self, gc: dotnet_utils::gc::GCHandle<'gc>, value: StackValue<'gc>) {
        self.trace_push(&value);
        self.evaluation_stack.push(gc, value);
        self.on_push();
    }

    #[inline]
    pub fn push_i32(&mut self, gc: dotnet_utils::gc::GCHandle<'gc>, value: i32) {
        self.trace_push(&StackValue::Int32(value));
        self.evaluation_stack.push_i32(gc, value);
        self.on_push();
    }

    #[inline]
    pub fn push_i64(&mut self, gc: dotnet_utils::gc::GCHandle<'gc>, value: i64) {
        self.trace_push(&StackValue::Int64(value));
        self.evaluation_stack.push_i64(gc, value);
        self.on_push();
    }

    #[inline]
    pub fn push_f64(&mut self, gc: dotnet_utils::gc::GCHandle<'gc>, value: f64) {
        self.trace_push(&StackValue::NativeFloat(value));
        self.evaluation_stack.push_f64(gc, value);
        self.on_push();
    }

    #[inline]
    pub fn push_obj(&mut self, gc: dotnet_utils::gc::GCHandle<'gc>, value: ObjectRef<'gc>) {
        self.trace_push(&StackValue::ObjectRef(value));
        self.evaluation_stack.push_obj(gc, value);
        self.on_push();
    }

    #[inline]
    pub fn push_ptr(
        &mut self,
        gc: dotnet_utils::gc::GCHandle<'gc>,
        ptr: *mut u8,
        t: TypeDescription,
        is_pinned: bool,
    ) {
        self.trace_push(&StackValue::managed_ptr(ptr, t, is_pinned));
        self.evaluation_stack.push_ptr(gc, ptr, t, is_pinned);
        self.on_push();
    }

    #[inline]
    pub fn pop(&mut self, gc: dotnet_utils::gc::GCHandle<'gc>) -> StackValue<'gc> {
        let val = self.evaluation_stack.pop(gc);
        self.trace_pop(&val);
        self.on_pop();
        val
    }

    #[inline]
    pub fn pop_i32(&mut self, gc: dotnet_utils::gc::GCHandle<'gc>) -> i32 {
        let val = self.evaluation_stack.pop_i32(gc);
        self.trace_pop(&StackValue::Int32(val));
        self.on_pop();
        val
    }

    #[inline]
    pub fn pop_i64(&mut self, gc: dotnet_utils::gc::GCHandle<'gc>) -> i64 {
        let val = self.evaluation_stack.pop_i64(gc);
        self.trace_pop(&StackValue::Int64(val));
        self.on_pop();
        val
    }

    #[inline]
    pub fn pop_f64(&mut self, gc: dotnet_utils::gc::GCHandle<'gc>) -> f64 {
        let val = self.evaluation_stack.pop_f64(gc);
        self.trace_pop(&StackValue::NativeFloat(val));
        self.on_pop();
        val
    }

    #[inline]
    pub fn pop_isize(&mut self, gc: dotnet_utils::gc::GCHandle<'gc>) -> isize {
        let val = self.evaluation_stack.pop_isize(gc);
        self.trace_pop(&StackValue::NativeInt(val));
        self.on_pop();
        val
    }

    #[inline]
    pub fn push_isize(&mut self, gc: dotnet_utils::gc::GCHandle<'gc>, value: isize) {
        self.trace_push(&StackValue::NativeInt(value));
        self.evaluation_stack.push_isize(gc, value);
        self.on_push();
    }

    #[inline]
    pub fn pop_obj(&mut self, gc: dotnet_utils::gc::GCHandle<'gc>) -> ObjectRef<'gc> {
        let val = self.evaluation_stack.pop_obj(gc);
        self.trace_pop(&StackValue::ObjectRef(val));
        self.on_pop();
        val
    }

    #[inline]
    pub fn pop_ptr(&mut self, gc: dotnet_utils::gc::GCHandle<'gc>) -> *mut u8 {
        let val = self.evaluation_stack.pop_ptr(gc);
        self.trace_pop(&StackValue::NativeInt(val as isize));
        self.on_pop();
        val
    }

    #[inline]
    pub fn pop_value_type(&mut self, gc: dotnet_utils::gc::GCHandle<'gc>) -> ObjectInstance<'gc> {
        let val = self.evaluation_stack.pop_value_type(gc);
        self.trace_pop(&StackValue::ValueType(val.clone()));
        self.on_pop();
        val
    }

    #[inline]
    pub fn push_value_type(
        &mut self,
        gc: dotnet_utils::gc::GCHandle<'gc>,
        value: ObjectInstance<'gc>,
    ) {
        self.trace_push(&StackValue::ValueType(value.clone()));
        self.evaluation_stack.push_value_type(gc, value);
        self.on_push();
    }

    #[inline]
    pub fn push_managed_ptr(
        &mut self,
        gc: dotnet_utils::gc::GCHandle<'gc>,
        value: ManagedPtr<'gc>,
    ) {
        self.trace_push(&StackValue::ManagedPtr(value));
        self.evaluation_stack.push_managed_ptr(gc, value);
        self.on_push();
    }

    #[inline]
    pub fn pop_managed_ptr(&mut self, gc: dotnet_utils::gc::GCHandle<'gc>) -> ManagedPtr<'gc> {
        let val = self.evaluation_stack.pop_managed_ptr(gc);
        self.trace_pop(&StackValue::ManagedPtr(val));
        self.on_pop();
        val
    }

    pub fn push_string(
        &mut self,
        gc: dotnet_utils::gc::GCHandle<'gc>,
        value: impl Into<dotnet_value::string::CLRString>,
    ) {
        let s: dotnet_value::string::CLRString = value.into();
        let in_heap = ObjectRef::new(gc, HeapStorage::Str(s));
        self.register_new_object(&in_heap);
        self.push(gc, StackValue::ObjectRef(in_heap));
    }

    pub fn peek_stack(&self) -> StackValue<'gc> {
        self.evaluation_stack.peek_stack()
    }

    pub fn peek_stack_at(&self, offset: usize) -> StackValue<'gc> {
        self.evaluation_stack.peek_stack_at(offset)
    }

    pub fn top_of_stack_address(&self) -> NonNull<u8> {
        self.evaluation_stack
            .get_slot_address(self.evaluation_stack.top_of_stack().saturating_sub(1))
    }

    pub fn branch(&mut self, target: usize) {
        self.frame_stack.current_frame_mut().state.ip = target;
    }

    pub fn conditional_branch(&mut self, condition: bool, target: usize) -> bool {
        if condition {
            self.branch(target);
            true
        } else {
            false
        }
    }

    pub fn increment_ip(&mut self) {
        self.frame_stack.current_frame_mut().state.ip += 1;
    }

    pub fn initialize_static_storage(
        &mut self,
        gc: dotnet_utils::gc::GCHandle<'gc>,
        description: TypeDescription,
        generics: GenericLookup,
    ) -> bool {
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
                    return true;
                }
                Initialized | Recursive => {
                    return false;
                }
                Waiting => {
                    #[cfg(feature = "multithreaded-gc")]
                    self.shared.statics.wait_for_init(
                        description,
                        &generics,
                        self.shared.thread_manager.as_ref(),
                        self.thread_id.get(),
                        &self.shared.gc_coordinator,
                    );
                    #[cfg(not(feature = "multithreaded-gc"))]
                    self.shared.statics.wait_for_init(
                        description,
                        &generics,
                        self.shared.thread_manager.as_ref(),
                        self.thread_id.get(),
                        &Default::default(),
                    );
                }
            }
        }
    }

    pub fn constructor_frame(
        &mut self,
        gc: dotnet_utils::gc::GCHandle<'gc>,
        instance: ObjectInstance<'gc>,
        method: MethodInfo<'m>,
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
            self.push(
                gc,
                StackValue::managed_ptr(
                    self.top_of_stack_address().as_ptr() as *mut _,
                    desc,
                    false,
                ),
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

    pub fn call_frame(
        &mut self,
        gc: dotnet_utils::gc::GCHandle<'gc>,
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
                            let (ut, type_generics) = decompose_type_source(source);
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
        self.frame_stack
            .handle_return(gc, self.evaluation_stack, shared, heap, tracer_enabled)
    }

    pub fn entrypoint_frame(
        &mut self,
        gc: dotnet_utils::gc::GCHandle<'gc>,
        method: MethodInfo<'m>,
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

    pub fn process_pending_finalizers(
        &mut self,
        gc: dotnet_utils::gc::GCHandle<'gc>,
    ) -> StepResult {
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

                    Some(
                        self.resolver()
                            .resolve_virtual_method(method_desc, obj_type, &ctx),
                    )
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

    pub fn throw_by_name(&mut self, gc: dotnet_utils::gc::GCHandle<'gc>, name: &str) -> StepResult {
        let ctx = self.current_context();
        let exception_type = self.shared.loader.corlib_type(name);
        let instance = ctx.new_object(exception_type);
        let obj_ref = ObjectRef::new(gc, HeapStorage::Obj(instance));
        self.register_new_object(&obj_ref);
        *self.exception_mode = ExceptionState::Throwing(obj_ref);
        StepResult::Exception
    }

    pub fn register_new_object(&self, instance: &ObjectRef<'gc>) {
        let ObjectRef(Some(ptr)) = instance else {
            return;
        };

        let heap = &self.local.heap;

        {
            let addr = gc_arena::Gc::as_ptr(*ptr) as usize;
            heap._all_objs.borrow_mut().insert(addr, *instance);
        }

        let ctx = self.current_context();
        let borrowed = ptr.borrow();

        if self.tracer_enabled() {
            let (type_name, size) = match &borrowed.storage {
                HeapStorage::Obj(o) => {
                    let name = o.description.type_name();
                    let size = self
                        .resolver()
                        .type_layout_cached(o.description.into(), &ctx)
                        .size();
                    (name, size)
                }
                HeapStorage::Vec(v) => ("System.Array".to_string(), v.size_bytes()),
                HeapStorage::Str(s) => ("System.String".to_string(), s.size_bytes()),
                HeapStorage::Boxed(b) => ("Boxed".to_string(), b.size_bytes()),
            };
            crate::vm_trace_gc_allocation!(self, &type_name, size);
        }

        if let HeapStorage::Obj(o) = &borrowed.storage
            && o.description.has_finalizer(&ctx)
        {
            let mut queue = heap.finalization_queue.borrow_mut();
            queue.push(*instance);
        }
    }

    pub fn current_context(&self) -> ResolutionContext<'_, 'm> {
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

    pub fn locate_type(&self, handle: UserType) -> TypeDescription {
        let f = self.current_frame();
        self.resolver().locate_type(f.source_resolution, handle)
    }

    pub fn locate_field(&self, handle: FieldSource) -> (FieldDescription, GenericLookup) {
        let f = self.current_frame();
        self.resolver()
            .locate_field(f.source_resolution, handle, &f.generic_inst)
    }

    pub fn get_local(&self, index: usize) -> StackValue<'gc> {
        let frame = self.frame_stack.current_frame();
        self.evaluation_stack.get_slot(frame.base.locals + index)
    }

    pub fn set_local(
        &mut self,
        gc: dotnet_utils::gc::GCHandle<'gc>,
        index: usize,
        value: StackValue<'gc>,
    ) {
        let frame = self.frame_stack.current_frame();
        self.evaluation_stack
            .set_slot(gc, frame.base.locals + index, value);
    }

    pub fn get_argument(&self, index: usize) -> StackValue<'gc> {
        let frame = self.frame_stack.current_frame();
        self.evaluation_stack.get_slot(frame.base.arguments + index)
    }

    pub fn set_argument(
        &mut self,
        gc: dotnet_utils::gc::GCHandle<'gc>,
        index: usize,
        value: StackValue<'gc>,
    ) {
        let frame = self.frame_stack.current_frame();
        self.evaluation_stack
            .set_slot(gc, frame.base.arguments + index, value);
    }

    pub fn get_local_address(&self, index: usize) -> NonNull<u8> {
        let frame = self.frame_stack.current_frame();
        self.evaluation_stack
            .get_slot_address(frame.base.locals + index)
    }

    pub fn get_argument_address(&self, index: usize) -> NonNull<u8> {
        let frame = self.frame_stack.current_frame();
        self.evaluation_stack
            .get_slot_address(frame.base.arguments + index)
    }

    pub fn get_local_info_for_managed_ptr(&self, index: usize) -> (NonNull<u8>, bool) {
        let frame = self.frame_stack.current_frame();
        let addr = self
            .evaluation_stack
            .get_slot_address(frame.base.locals + index);
        let is_pinned = frame.pinned_locals[index];
        (addr, is_pinned)
    }

    pub fn state(&self) -> &MethodState<'m> {
        &self.frame_stack.current_frame().state
    }

    pub fn state_mut(&mut self) -> &mut MethodState<'m> {
        &mut self.frame_stack.current_frame_mut().state
    }

    pub fn current_frame(&self) -> &StackFrame<'gc, 'm> {
        self.frame_stack.current_frame()
    }

    pub fn current_frame_mut(&mut self) -> &mut StackFrame<'gc, 'm> {
        self.frame_stack.current_frame_mut()
    }

    pub fn reflection(&self) -> ReflectionRegistry<'_, 'gc> {
        ReflectionRegistry::new(self.local)
    }

    pub fn get_heap_description(
        &self,
        object: dotnet_value::object::ObjectHandle<'gc>,
    ) -> TypeDescription {
        self.resolver().get_heap_description(object)
    }

    pub fn make_concrete<T: Clone + Into<MethodType>>(&self, t: &T) -> ConcreteType {
        let f = self.current_frame();
        self.resolver()
            .make_concrete(f.source_resolution, &f.generic_inst, t)
    }

    pub fn tracer_enabled(&self) -> bool {
        self.shared.tracer_enabled.load(Ordering::Relaxed)
    }

    pub fn tracer(&self) -> MutexGuard<'_, Tracer> {
        self.shared.tracer.lock()
    }

    #[inline]
    pub fn heap(&self) -> &HeapManager<'gc> {
        &self.local.heap
    }

    #[inline]
    pub fn back_up_ip(&mut self) {
        self.frame_stack.current_frame_mut().state.ip -= 1;
    }

    pub fn indent(&self) -> usize {
        self.frame_stack.len().saturating_sub(1)
    }

    pub fn loader(&self) -> &'m AssemblyLoader {
        self.shared.loader
    }

    pub fn is_intrinsic_cached(&self, method: MethodDescription) -> bool {
        self.resolver().is_intrinsic_cached(method)
    }

    pub fn dispatch_method(
        &mut self,
        gc: dotnet_utils::gc::GCHandle<'gc>,
        method: MethodDescription,
        lookup: GenericLookup,
    ) -> StepResult {
        if self.is_intrinsic_cached(method) {
            crate::intrinsics::intrinsic_call(gc, self, method, &lookup)
        } else if method.method.pinvoke.is_some() {
            self.external_call(method, gc);
            StepResult::Continue
        } else {
            if method.method.internal_call {
                panic!("intrinsic not found: {:?}", method);
            }
            let info = MethodInfo::new(method, &lookup, self.shared.clone());
            self.call_frame(gc, info, lookup);
            StepResult::FramePushed
        }
    }

    pub fn unified_dispatch(
        &mut self,
        gc: dotnet_utils::gc::GCHandle<'gc>,
        source: &MethodSource,
        this_type: Option<TypeDescription>,
        ctx: Option<&ResolutionContext<'_, 'm>>,
    ) -> StepResult {
        let default_ctx;
        let ctx = if let Some(ctx) = ctx {
            ctx
        } else {
            default_ctx = self.current_context();
            &default_ctx
        };

        let resolver = self.resolver();
        let (base_method, lookup) = resolver.find_generic_method(source, ctx);

        let method = if let Some(runtime_type) = this_type {
            resolver.resolve_virtual_method(base_method, runtime_type, ctx)
        } else {
            base_method
        };

        self.dispatch_method(gc, method, lookup)
    }

    pub fn pop_multiple(
        &mut self,
        gc: dotnet_utils::gc::GCHandle<'gc>,
        count: usize,
    ) -> Vec<StackValue<'gc>> {
        let res = self.evaluation_stack.pop_multiple(gc, count);
        if let Some(frame) = self.frame_stack.current_frame_opt_mut() {
            if frame.stack_height < count {
                panic!(
                    "Stack height underflow in pop_multiple: height={}, count={} in {:?}",
                    frame.stack_height, count, frame.state.info_handle.source
                );
            }
            frame.stack_height -= count;
        }
        res
    }

    pub fn stack_value_type(&self, val: &StackValue<'gc>) -> TypeDescription {
        self.resolver().stack_value_type(val)
    }

    pub fn new_instance_fields(&self, td: TypeDescription) -> FieldStorage {
        self.resolver()
            .new_instance_fields(td, &self.current_context())
    }

    pub fn new_static_fields(&self, td: TypeDescription) -> FieldStorage {
        self.resolver()
            .new_static_fields(td, &self.current_context())
    }

    pub fn new_value_type(&self, t: &ConcreteType, data: StackValue<'gc>) -> ValueType<'gc> {
        self.resolver()
            .new_value_type(t, data, &self.current_context())
    }

    pub fn value_type_description(&self, vt: &ValueType<'gc>) -> TypeDescription {
        self.resolver().value_type_description(vt)
    }

    pub fn new_cts_value(&self, t: &ConcreteType, data: StackValue<'gc>) -> CTSValue<'gc> {
        self.resolver()
            .new_cts_value(t, data, &self.current_context())
    }

    pub fn read_cts_value(
        &self,
        t: &ConcreteType,
        data: &[u8],
        gc: dotnet_utils::gc::GCHandle<'gc>,
    ) -> CTSValue<'gc> {
        self.resolver()
            .read_cts_value(t, data, gc, &self.current_context())
    }

    pub fn new_vector(&self, element: ConcreteType, size: usize) -> Vector<'gc> {
        self.resolver()
            .new_vector(element, size, &self.current_context())
    }

    pub fn new_object(&self, td: TypeDescription) -> ObjectInstance<'gc> {
        self.resolver().new_object(td, &self.current_context())
    }

    pub fn shared(&self) -> &Arc<SharedGlobalState<'m>> {
        self.shared
    }

    pub fn is_intrinsic_field_cached(&self, field: FieldDescription) -> bool {
        self.resolver().is_intrinsic_field_cached(field)
    }

    pub fn statics(&self) -> &StaticStorageManager {
        &self.shared.statics
    }

    pub fn with_generics<'b>(&self, lookup: &'b GenericLookup) -> ResolutionContext<'b, 'm> {
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

    pub fn thread_id(&self) -> usize {
        self.thread_id.get() as usize
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
    fn trace(&self, cc: &gc_arena::Collection) {
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

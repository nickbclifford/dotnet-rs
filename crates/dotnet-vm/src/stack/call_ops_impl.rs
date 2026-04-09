use crate::{
    BasePointer, ByteOffset, MethodInfo, ResolutionContext, StepResult,
    resolution::TypeResolutionExt,
    stack::{
        context::VesContext,
        ops::{
            BaseLoaderOps, BaseStaticsOps, CallOps, EvalStackOps, LoaderOps, RawMemoryOps,
            ResolutionOps,
        },
    },
};
use dotnet_types::{
    TypeDescription, error::TypeResolutionError, generics::GenericLookup,
    members::MethodDescription,
};
use dotnet_value::{
    StackValue,
    object::{HeapStorage, Object as ObjectInstance, ObjectRef},
    pointer::ManagedPtr,
};
use dotnetdll::prelude::{Instruction, MethodSource};

vm_cold_panic!(
    fn panic_not_enough_values_for_call(method: &MethodInfo<'static>, num_args: usize) =>
        "not enough values on stack for call: args={} in {:?}",
        num_args, method.source
);
vm_cold_panic!(
    fn panic_expected_boxed_value_type_in_unbox_canonicalization() =>
        "Expected boxed value type in unbox canonicalization"
);
vm_cold_panic!(
    fn panic_stack_height_underflow_for_call(
        frame_height: crate::StackSlotIndex,
        num_args: usize,
        source: &MethodDescription
    ) =>
        "Not enough values on stack for call: height={}, args={} in {:?}",
        frame_height, num_args, source
);
vm_cold_panic!(
    fn panic_no_body_in_executing_method(method: &MethodDescription) =>
        "no body in executing method: {}.{}",
        method.parent.type_name(),
        method.method().name
);
vm_cold_panic!(fn panic_tail_call_requires_current_frame() => "tail call requires a current frame");
vm_cold_panic!(fn panic_jmp_requires_current_frame() => "jmp requires a current frame");

impl<'a, 'gc> CallOps<'gc> for VesContext<'a, 'gc> {
    fn constructor_frame(
        &mut self,
        instance: ObjectInstance<'gc>,
        method: MethodInfo<'static>,
        generic_inst: GenericLookup,
    ) -> Result<(), TypeResolutionError> {
        let gc = self.gc;
        let desc = instance.description.clone();

        let value = if desc.is_value_type(&self.current_context())? {
            StackValue::ValueType(instance)
        } else {
            let in_heap = ObjectRef::new(gc, HeapStorage::Obj(instance));
            self.register_new_object(&in_heap);
            StackValue::ObjectRef(in_heap)
        };

        let num_params = method.signature.parameters.len();
        self.pop_call_args_into_buffer(num_params);
        let mut args = std::mem::take(self.call_args_buffer);

        if desc.is_value_type(&self.current_context())? {
            self.push(value);
            let index = self.evaluation_stack.top_of_stack() - 1;
            let ptr = self.evaluation_stack.get_slot_address(index).as_ptr() as *mut _;
            self.push(StackValue::managed_stack_ptr(
                index,
                ByteOffset(0),
                ptr,
                desc,
                false,
            ));
        } else {
            self.push(value.clone());
            self.push(value);
        }

        for arg in args.drain(..) {
            self.push(arg);
        }
        *self.call_args_buffer = args;
        self.call_frame(method, generic_inst)
    }

    #[inline]
    fn call_frame(
        &mut self,
        method: MethodInfo<'static>,
        generic_inst: GenericLookup,
    ) -> Result<(), TypeResolutionError> {
        let _ = self.check_gc_safe_point();
        let _gc = self.gc;
        if self.tracer_enabled() {
            let method_desc = format!("{:?}", method.source);
            self.shared
                .tracer
                .trace_method_entry(self.indent(), &method_desc, "");
        }

        let num_args = method.signature.instance as usize + method.signature.parameters.len();
        let Some(argument_base) = self.evaluation_stack.top_of_stack().checked_sub(num_args) else {
            panic_not_enough_values_for_call(&method, num_args);
        };

        let locals_base = self.evaluation_stack.top_of_stack();
        let local_slot_count = method.locals.len();
        self.evaluation_stack
            .reserve_slots(locals_base.as_usize() + local_slot_count + method.max_stack);
        let pinned_locals = self.init_locals(
            method.source.clone(),
            method.locals,
            &generic_inst,
            locals_base,
        )?;

        let stack_base = locals_base + pinned_locals.len();

        // Canonicalize 'this' for value type instance methods
        if method.signature.instance {
            let this_val = self.evaluation_stack.get_slot(argument_base);
            if let StackValue::ObjectRef(obj) = this_val {
                let td = method.source.parent.clone();
                if td.is_value_type(&self.current_context())? {
                    // Unbox this to a managed pointer. This is required when a virtual call
                    // on a boxed value type reaches a value type override.
                    let ptr = obj.as_heap_storage(|storage| match storage {
                        HeapStorage::Boxed(o) | HeapStorage::Obj(o) => unsafe {
                            o.instance_storage.raw_data_ptr()
                        },
                        _ => panic_expected_boxed_value_type_in_unbox_canonicalization(),
                    });
                    let managed_ptr = ManagedPtr::new(
                        std::ptr::NonNull::new(ptr),
                        td,
                        Some(obj),
                        false,
                        Some(ByteOffset(0)),
                    );
                    self.evaluation_stack
                        .set_slot_at(argument_base, StackValue::ManagedPtr(managed_ptr.into()));
                }
            }
        }

        if let Some(frame) = self.frame_stack.current_frame_opt_mut() {
            if frame.stack_height < crate::StackSlotIndex(num_args) {
                panic_stack_height_underflow_for_call(
                    frame.stack_height,
                    num_args,
                    &frame.state.info_handle.source,
                );
            }
            frame.stack_height -= num_args;
        }

        self.frame_stack.push_frame(
            BasePointer {
                arguments: argument_base,
                locals: locals_base,
                stack: stack_base,
            },
            method,
            generic_inst,
            pinned_locals,
        );
        Ok(())
    }

    #[inline]
    fn entrypoint_frame(
        &mut self,
        method: MethodInfo<'static>,
        generic_inst: GenericLookup,
        args: Vec<StackValue<'gc>>,
    ) -> Result<(), TypeResolutionError> {
        let _ = self.check_gc_safe_point();
        let _gc = self.gc;
        let argument_base = self.evaluation_stack.top_of_stack();
        let arg_count = args.len();
        let local_slot_count = method.locals.len();
        self.evaluation_stack.reserve_slots(
            argument_base.as_usize() + arg_count + local_slot_count + method.max_stack,
        );
        for a in args {
            self.push(a);
        }
        let locals_base = self.evaluation_stack.top_of_stack();
        let pinned_locals = self.init_locals(
            method.source.clone(),
            method.locals,
            &generic_inst,
            locals_base,
        )?;
        let stack_base = locals_base + pinned_locals.len();

        self.frame_stack.push_frame(
            BasePointer {
                arguments: argument_base,
                locals: locals_base,
                stack: stack_base,
            },
            method,
            generic_inst,
            pinned_locals,
        );
        Ok(())
    }

    fn dispatch_method(&mut self, method: MethodDescription, lookup: GenericLookup) -> StepResult {
        let _gc = self.gc;

        // ECMA-335 §II.10.5.3.3: Types without beforefieldinit must be initialized on
        // static method calls, instance calls for value types, and constructor calls.
        if !method.parent.before_field_init() {
            let is_static = !method.method().signature.instance;
            let is_value_type =
                vm_try!(method.parent.clone().is_value_type(&self.current_context()));
            let is_constructor = method.method().name == ".ctor";

            if is_static || is_value_type || is_constructor {
                let res = self.initialize_static_storage(method.parent.clone(), lookup.clone());
                if res != StepResult::Continue {
                    return res;
                }
            }
        }

        let intrinsic_metadata = crate::intrinsics::classify_intrinsic(
            method.clone(),
            self.loader(),
            Some(&self.shared.caches.intrinsic_registry),
        );
        // Instrumented benchmarks for current workloads report intrinsic_call_total=0.
        if vm_unlikely!(intrinsic_metadata.is_some()) {
            let metadata = intrinsic_metadata.unwrap();
            crate::intrinsics::dispatch_method_intrinsic(metadata.handler, self, method, &lookup)
        } else if method.method().pinvoke.is_some() {
            let shared = self.shared.clone();
            dotnet_pinvoke::external_call(self, method, &shared.pinvoke)
        } else {
            if method.method().body.is_none() {
                if let Some(result) = dotnet_intrinsics_delegates::try_delegate_dispatch(
                    self,
                    method.clone(),
                    &lookup,
                ) {
                    return result;
                }

                panic_no_body_in_executing_method(&method);
            }

            let info =
                match self
                    .shared
                    .caches
                    .get_method_info(method, &lookup, self.shared.clone())
                {
                    Ok(v) => v,
                    Err(e) => return StepResult::Error(e.into()),
                };
            vm_try!(self.call_frame(info, lookup));
            StepResult::FramePushed
        }
    }

    #[inline]
    fn unified_dispatch(
        &mut self,
        source: &MethodSource,
        this_type: Option<TypeDescription>,
        ctx: Option<&ResolutionContext<'_>>,
    ) -> StepResult {
        let context = ctx.cloned().unwrap_or_else(|| self.current_context());

        tracing::debug!(
            "unified_dispatch: source={:?}, this_type={:?}",
            source,
            this_type
        );

        let (resolved, lookup) = match self.resolver().find_generic_method(source, &context) {
            Ok(v) => v,
            Err(e) => return StepResult::Error(e.into()),
        };

        let final_method = if let Some(this_type) = this_type {
            match self
                .resolver()
                .resolve_virtual_method(resolved, this_type, &lookup, &context)
            {
                Ok(v) => v,
                Err(e) => return StepResult::Error(e.into()),
            }
        } else {
            resolved
        };

        self.dispatch_method(final_method, lookup)
    }

    #[inline]
    fn unified_dispatch_tail(
        &mut self,
        source: &MethodSource,
        this_type: Option<TypeDescription>,
        ctx: Option<&ResolutionContext<'_>>,
    ) -> StepResult {
        let context = ctx.cloned().unwrap_or_else(|| self.current_context());

        tracing::debug!(
            "unified_dispatch_tail: source={:?}, this_type={:?}",
            source,
            this_type
        );

        let (resolved, lookup) = match self.resolver().find_generic_method(source, &context) {
            Ok(v) => v,
            Err(e) => return StepResult::Error(e.into()),
        };

        let final_method = if let Some(this_type) = this_type {
            match self
                .resolver()
                .resolve_virtual_method(resolved, this_type, &lookup, &context)
            {
                Ok(v) => v,
                Err(e) => return StepResult::Error(e.into()),
            }
        } else {
            resolved
        };

        self.dispatch_method_tail(final_method, lookup)
    }

    #[inline]
    fn unified_dispatch_jmp(
        &mut self,
        source: &MethodSource,
        ctx: Option<&ResolutionContext<'_>>,
    ) -> StepResult {
        let context = ctx.cloned().unwrap_or_else(|| self.current_context());

        tracing::debug!("unified_dispatch_jmp: source={:?}", source);

        let (resolved, lookup) = match self.resolver().find_generic_method(source, &context) {
            Ok(v) => v,
            Err(e) => return StepResult::Error(e.into()),
        };

        // jmp is always a direct call (non-virtual)
        self.dispatch_method_jmp(resolved, lookup)
    }
}

impl<'a, 'gc> VesContext<'a, 'gc> {
    fn should_honor_tail_call(&self, arg_count: usize) -> bool {
        let frame = self.frame_stack.current_frame();

        // ECMA-335: tail. is ignored when exiting a synchronized method.
        if frame.state.info_handle.source.method().synchronized {
            return false;
        }

        // Avoid tail-call frame replacement for special frames that have deferred-return semantics
        // in `return_frame()` (cctor initialization, finalizer processing, multicast, etc.).
        if frame.state.info_handle.is_cctor
            || frame.is_finalizer
            || frame.multicast_state.is_some()
            || frame.awaiting_invoke_return.is_some()
        {
            return false;
        }

        // ECMA-335: tail. call must be immediately followed by `ret`.
        let ip = frame.state.ip;
        let instrs = frame.state.info_handle.instructions;
        if ip + 1 >= instrs.len() {
            return false;
        }
        if !matches!(instrs[ip + 1], Instruction::Return) {
            return false;
        }

        // ECMA-335: stack must be empty except for the call arguments.
        if frame.stack_height != crate::StackSlotIndex(arg_count) {
            return false;
        }

        // ECMA-335: cannot be used to transfer control out of try/filter/catch/finally blocks.
        for sec in frame.state.info_handle.exceptions.iter() {
            if sec.instructions.contains(&ip) {
                return false;
            }
            for handler in &sec.handlers {
                if handler.instructions.contains(&ip) {
                    return false;
                }
            }
        }

        true
    }

    fn dispatch_method_tail(
        &mut self,
        method: MethodDescription,
        lookup: GenericLookup,
    ) -> StepResult {
        // If we can't safely tail-call, fall back to the regular call path.
        if let Some(metadata) = crate::intrinsics::classify_intrinsic(
            method.clone(),
            self.loader(),
            Some(&self.shared.caches.intrinsic_registry),
        ) {
            return crate::intrinsics::dispatch_method_intrinsic(
                metadata.handler,
                self,
                method,
                &lookup,
            );
        }
        if method.method().pinvoke.is_some() {
            let shared = self.shared.clone();
            return dotnet_pinvoke::external_call(self, method, &shared.pinvoke);
        }

        if method.method().body.is_none() {
            // Delegate dispatch may emulate a call without a managed body; do not attempt to
            // tail-call optimize it.
            return self.dispatch_method(method, lookup);
        }

        let info = match self
            .shared
            .caches
            .get_method_info(method, &lookup, self.shared.clone())
        {
            Ok(v) => v,
            Err(e) => return StepResult::Error(e.into()),
        };

        let arg_count = info.signature.instance as usize + info.signature.parameters.len();
        if !self.should_honor_tail_call(arg_count) {
            vm_try!(self.call_frame(info, lookup));
            return StepResult::FramePushed;
        }

        // Preserve the call arguments.
        let (args_base, clear_from, old_top) = {
            let frame = self.frame_stack.current_frame();
            (
                frame.base.stack,
                frame.base.arguments,
                self.evaluation_stack.top_of_stack(),
            )
        };
        self.copy_slots_into_call_args_buffer(args_base, arg_count);
        let mut args = std::mem::take(self.call_args_buffer);

        // Pop/discard the current frame and clear its stack slots.
        let frame = self
            .frame_stack
            .pop()
            .unwrap_or_else(|| panic_tail_call_requires_current_frame());
        if self.tracer_enabled() {
            let method_name = format!("{:?}", frame.state.info_handle.source);
            self.shared
                .tracer
                .trace_method_exit(self.frame_stack.len(), &method_name);
        }
        self.frame_stack.recycle_frame(frame);

        for i in clear_from.as_usize()..old_top.as_usize() {
            self.evaluation_stack
                .set_slot(crate::StackSlotIndex(i), StackValue::null());
        }
        self.evaluation_stack.truncate(clear_from);

        // Re-push arguments onto the (now) caller stack, or directly onto the eval stack if this
        // was the last frame.
        if self.frame_stack.current_frame_opt_mut().is_some() {
            for a in args.drain(..) {
                self.push(a);
            }
        } else {
            for a in args.drain(..) {
                self.evaluation_stack.push(a);
            }
        }
        *self.call_args_buffer = args;

        vm_try!(self.call_frame(info, lookup));
        StepResult::FramePushed
    }

    fn dispatch_method_jmp(
        &mut self,
        method: MethodDescription,
        lookup: GenericLookup,
    ) -> StepResult {
        let target_sig = &method.method().signature;
        let (arg_count, args_base, clear_from) = {
            let frame = self.frame_stack.current_frame();

            // ECMA-335: evaluation stack shall be empty.
            if frame.stack_height != crate::StackSlotIndex(0) {
                return StepResult::Error(crate::error::VmError::Execution(
                    crate::error::ExecutionError::Aborted(
                        "jmp requires empty evaluation stack".to_string(),
                    ),
                ));
            }

            // ECMA-335: cannot be used to transfer control out of
            // try/filter/catch/fault/finally blocks.
            let ip = frame.state.ip;
            for sec in frame.state.info_handle.exceptions.iter() {
                if sec.instructions.contains(&ip) {
                    return StepResult::Error(crate::error::VmError::Execution(
                        crate::error::ExecutionError::Aborted(
                            "jmp out of try/catch/finally block".to_string(),
                        ),
                    ));
                }
                for handler in &sec.handlers {
                    if handler.instructions.contains(&ip) {
                        return StepResult::Error(crate::error::VmError::Execution(
                            crate::error::ExecutionError::Aborted(
                                "jmp out of exception handler".to_string(),
                            ),
                        ));
                    }
                }
            }

            // Signature matching check
            let current_sig = &frame.state.info_handle.signature;

            let loader = self.loader_arc();
            let comparer = dotnet_types::comparer::TypeComparer::new(loader.as_ref());
            let res_ctx = self.current_context();
            let res_s = res_ctx.resolution.clone();

            if !comparer.signatures_equal(
                res_s.clone(),
                current_sig,
                Some(res_ctx.generics), // Current generics
                res_s,
                target_sig,
                Some(&lookup), // Target generics
            ) {
                return StepResult::Error(crate::error::VmError::Execution(
                    crate::error::ExecutionError::Aborted("jmp signature mismatch".to_string()),
                ));
            }

            (
                target_sig.instance as usize + target_sig.parameters.len(),
                frame.base.arguments,
                frame.base.arguments,
            )
        };

        self.copy_slots_into_call_args_buffer(args_base, arg_count);
        let mut args = std::mem::take(self.call_args_buffer);

        // Discard the current frame and its locals/eval stack
        let old_top = self.evaluation_stack.top_of_stack();

        let popped_frame = self
            .frame_stack
            .pop()
            .unwrap_or_else(|| panic_jmp_requires_current_frame());
        if self.tracer_enabled() {
            let method_name = format!("{:?}", popped_frame.state.info_handle.source);
            self.shared
                .tracer
                .trace_method_exit(self.frame_stack.len(), &method_name);
        }
        self.frame_stack.recycle_frame(popped_frame);

        for i in clear_from.as_usize()..old_top.as_usize() {
            self.evaluation_stack
                .set_slot(crate::StackSlotIndex(i), StackValue::null());
        }
        self.evaluation_stack.truncate(clear_from);

        // Push arguments back onto the stack for the new call
        for a in args.drain(..) {
            self.push(a);
        }
        *self.call_args_buffer = args;

        // Push new frame
        let info = match self
            .shared
            .caches
            .get_method_info(method, &lookup, self.shared.clone())
        {
            Ok(v) => v,
            Err(e) => return StepResult::Error(e.into()),
        };

        vm_try!(self.call_frame(info, lookup));
        StepResult::FramePushed
    }
}

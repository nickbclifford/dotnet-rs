use crate::{
    MethodInfo, ResolutionContext, StepResult,
    resolution::TypeResolutionExt,
    stack::{
        context::{BasePointer, StackFrame, VesContext},
        ops::{CallOps, EvalStackOps, LoaderOps, RawMemoryOps, ResolutionOps},
    },
};
use dotnet_types::{
    TypeDescription, error::TypeResolutionError, generics::GenericLookup,
    members::MethodDescription,
};
use dotnet_value::{
    StackValue,
    object::{HeapStorage, Object as ObjectInstance, ObjectRef},
};
use dotnetdll::prelude::MethodSource;

impl<'a, 'gc, 'm: 'gc> CallOps<'gc, 'm> for VesContext<'a, 'gc, 'm> {
    #[inline]
    fn constructor_frame(
        &mut self,
        instance: ObjectInstance<'gc>,
        method: MethodInfo<'m>,
        generic_inst: GenericLookup,
    ) -> Result<(), TypeResolutionError> {
        let gc = self.gc;
        let desc = instance.description;

        let value = if desc.is_value_type(&self.current_context())? {
            StackValue::ValueType(instance)
        } else {
            let in_heap = ObjectRef::new(gc, HeapStorage::Obj(instance));
            self.register_new_object(&in_heap);
            StackValue::ObjectRef(in_heap)
        };

        let num_params = method.signature.parameters.len();
        let args = self.pop_multiple(num_params);

        if desc.is_value_type(&self.current_context())? {
            self.push(value);
            let index = self.evaluation_stack.top_of_stack() - 1;
            let ptr = self.evaluation_stack.get_slot_address(index).as_ptr() as *mut _;
            self.push(StackValue::managed_stack_ptr(
                index,
                crate::ByteOffset(0),
                ptr,
                desc,
                false,
            ));
        } else {
            self.push(value.clone());
            self.push(value);
        }

        for arg in args {
            self.push(arg);
        }
        self.call_frame(method, generic_inst)
    }

    #[inline]
    fn call_frame(
        &mut self,
        method: MethodInfo<'m>,
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
        let argument_base = self
            .evaluation_stack
            .top_of_stack()
            .checked_sub(num_args)
            .expect("not enough values on stack for call");

        let locals_base = self.evaluation_stack.top_of_stack();
        let (local_values, pinned_locals) =
            self.init_locals(method.source, method.locals, &generic_inst)?;

        for (i, v) in local_values.into_iter().enumerate() {
            self.evaluation_stack.set_slot_at(locals_base + i, v);
        }

        let stack_base = locals_base + pinned_locals.len();

        // Canonicalize 'this' for value type instance methods
        if method.signature.instance {
            let this_val = self.evaluation_stack.get_slot(argument_base);
            if let StackValue::ObjectRef(obj) = this_val {
                let td = method.source.parent;
                if td.is_value_type(&self.current_context())? {
                    // Unbox this to a managed pointer. This is required when a virtual call
                    // on a boxed value type reaches a value type override.
                    let ptr = obj.as_heap_storage(|storage| match storage {
                        HeapStorage::Boxed(o) | HeapStorage::Obj(o) => unsafe {
                            o.instance_storage.raw_data_ptr()
                        },
                        _ => panic!("Expected boxed value type in unbox canonicalization"),
                    });
                    let managed_ptr = dotnet_value::pointer::ManagedPtr::new(
                        std::ptr::NonNull::new(ptr),
                        td,
                        Some(obj),
                        false,
                        Some(dotnet_value::ByteOffset(0)),
                    );
                    self.evaluation_stack
                        .set_slot_at(argument_base, StackValue::ManagedPtr(managed_ptr));
                }
            }
        }

        if let Some(frame) = self.frame_stack.current_frame_opt_mut() {
            if frame.stack_height < crate::StackSlotIndex(num_args) {
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
        Ok(())
    }

    #[inline]
    fn entrypoint_frame(
        &mut self,
        method: MethodInfo<'m>,
        generic_inst: GenericLookup,
        args: Vec<StackValue<'gc>>,
    ) -> Result<(), TypeResolutionError> {
        let _ = self.check_gc_safe_point();
        let _gc = self.gc;
        let argument_base = self.evaluation_stack.top_of_stack();
        for a in args {
            self.push(a);
        }
        let locals_base = self.evaluation_stack.top_of_stack();
        let (local_values, pinned_locals) =
            self.init_locals(method.source, method.locals, &generic_inst)?;
        for v in local_values {
            self.push(v);
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
        Ok(())
    }

    #[inline]
    fn dispatch_method(&mut self, method: MethodDescription, lookup: GenericLookup) -> StepResult {
        let _gc = self.gc;
        if let Some(metadata) = crate::intrinsics::classify_intrinsic(
            method,
            self.loader(),
            Some(&self.shared.caches.intrinsic_registry),
        ) {
            (metadata.handler)(self, method, &lookup)
        } else if method.method.pinvoke.is_some() {
            crate::pinvoke::external_call(self, method)
        } else {
            if method.method.body.is_none() {
                if let Some(result) =
                    crate::intrinsics::delegates::try_delegate_dispatch(self, method, &lookup)
                {
                    return result;
                }

                panic!(
                    "no body in executing method: {}.{}",
                    method.parent.type_name(),
                    method.method.name
                );
            }

            let info = match self.shared.caches.get_method_info(method, &lookup, self.shared.clone()) {
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
        ctx: Option<&ResolutionContext<'_, 'm>>,
    ) -> StepResult {
        let context = ctx.cloned().unwrap_or_else(|| self.current_context());
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
}

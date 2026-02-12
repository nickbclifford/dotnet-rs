use super::{context::{BasePointer, VesContext, StackFrame}, ops::{CallOps, ResolutionOps, StackOps, LoaderOps}};
use crate::{StepResult, MethodInfo, ResolutionContext, resolution::TypeResolutionExt, stack::ExceptionState};
use dotnet_types::{TypeDescription, generics::GenericLookup, members::MethodDescription};
use dotnet_utils::gc::GCHandle;
use dotnet_value::{object::{HeapStorage, Object as ObjectInstance, ObjectRef}, StackValue};
use dotnetdll::prelude::MethodSource;

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

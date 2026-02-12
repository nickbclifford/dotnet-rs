use crate::stack::{
    context::VesContext,
    ops::{ArgumentOps, EvalStackOps, LocalOps, StackOps, TypedStackOps},
};
use dotnet_value::{
    CLRString, StackValue,
    object::{HeapStorage, ObjectRef},
};

impl<'a, 'gc, 'm: 'gc> EvalStackOps<'gc> for VesContext<'a, 'gc, 'm> {
    #[inline]
    fn push(&mut self, value: StackValue<'gc>) {
        #[cfg(feature = "multithreaded-gc")]
        self.gc.record_allocation(value.size_bytes());
        self.trace_push(&value);
        self.evaluation_stack.push(value);
        self.on_push();
    }

    #[inline]
    fn pop(&mut self) -> StackValue<'gc> {
        self.on_pop();
        let val = self.evaluation_stack.pop();
        self.trace_pop(&val);
        val
    }

    #[inline]
    fn pop_safe(&mut self) -> Result<StackValue<'gc>, crate::error::VmError> {
        self.on_pop_safe()?;
        let val = self.evaluation_stack.pop_safe()?;
        self.trace_pop(&val);
        Ok(val)
    }

    #[inline]
    fn pop_multiple(&mut self, count: usize) -> Vec<StackValue<'gc>> {
        let mut results = Vec::with_capacity(count);
        for _ in 0..count {
            results.push(self.pop());
        }
        results.reverse();
        results
    }

    #[inline]
    fn peek_multiple(&self, count: usize) -> Vec<StackValue<'gc>> {
        self.evaluation_stack.peek_multiple(count)
    }

    #[inline]
    fn dup(&mut self) {
        let val = self.pop();
        self.push(val.clone());
        self.push(val);
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
    fn top_of_stack(&self) -> usize {
        self.evaluation_stack.top_of_stack()
    }
}

impl<'a, 'gc, 'm: 'gc> TypedStackOps<'gc> for VesContext<'a, 'gc, 'm> {
    #[inline]
    fn push_string(&mut self, value: CLRString) {
        let gc = self.gc;
        let in_heap = ObjectRef::new(gc, HeapStorage::Str(value));
        self.register_new_object(&in_heap);
        self.push(StackValue::ObjectRef(in_heap));
    }
}

impl<'a, 'gc, 'm: 'gc> LocalOps<'gc> for VesContext<'a, 'gc, 'm> {
    #[inline]
    fn get_local(&self, index: usize) -> StackValue<'gc> {
        let frame = self.frame_stack.current_frame();
        self.evaluation_stack.get_slot(frame.base.locals + index)
    }

    #[inline]
    fn set_local(&mut self, index: usize, value: StackValue<'gc>) {
        #[cfg(feature = "multithreaded-gc")]
        if matches!(value, StackValue::ValueType(_)) {
            self.gc.record_allocation(value.size_bytes());
        }
        let bp = self.frame_stack.current_frame().base;
        self.evaluation_stack.set_slot(bp.locals + index, value);
    }

    #[inline]
    fn get_local_address(&self, index: usize) -> std::ptr::NonNull<u8> {
        let frame = self.frame_stack.current_frame();
        self.evaluation_stack
            .get_slot_address(frame.base.locals + index)
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
}

impl<'a, 'gc, 'm: 'gc> ArgumentOps<'gc> for VesContext<'a, 'gc, 'm> {
    #[inline]
    fn get_argument(&self, index: usize) -> StackValue<'gc> {
        let frame = self.frame_stack.current_frame();
        self.evaluation_stack.get_slot(frame.base.arguments + index)
    }

    #[inline]
    fn set_argument(&mut self, index: usize, value: StackValue<'gc>) {
        #[cfg(feature = "multithreaded-gc")]
        if matches!(value, StackValue::ValueType(_)) {
            self.gc.record_allocation(value.size_bytes());
        }
        let bp = self.frame_stack.current_frame().base;
        self.evaluation_stack.set_slot(bp.arguments + index, value);
    }

    #[inline]
    fn get_argument_address(&self, index: usize) -> std::ptr::NonNull<u8> {
        let frame = self.frame_stack.current_frame();
        self.evaluation_stack
            .get_slot_address(frame.base.arguments + index)
    }
}

impl<'a, 'gc, 'm: 'gc> StackOps<'gc, 'm> for VesContext<'a, 'gc, 'm> {
    #[inline]
    fn current_frame(&self) -> &crate::stack::StackFrame<'gc, 'm> {
        self.frame_stack.current_frame()
    }

    #[inline]
    fn current_frame_mut(&mut self) -> &mut crate::stack::StackFrame<'gc, 'm> {
        self.frame_stack.current_frame_mut()
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
    fn set_slot(&mut self, index: usize, value: StackValue<'gc>) {
        #[cfg(feature = "multithreaded-gc")]
        if matches!(value, StackValue::ValueType(_)) {
            self.gc.record_allocation(value.size_bytes());
        }
        self.evaluation_stack.set_slot(index, value)
    }

    #[inline]
    fn get_slot_address(&self, index: usize) -> std::ptr::NonNull<u8> {
        self.evaluation_stack.get_slot_address(index)
    }
}

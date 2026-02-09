use super::{context::VesContext, ops::StackOps};
use dotnet_types::TypeDescription;
use dotnet_utils::gc::GCHandle;
use dotnet_value::{
    CLRString, StackValue,
    object::{HeapStorage, Object as ObjectInstance, ObjectRef},
    pointer::ManagedPtr,
};

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
    fn pop_safe(&mut self, gc: GCHandle<'gc>) -> Result<StackValue<'gc>, crate::error::VmError> {
        self.on_pop_safe()?;
        let val = self.evaluation_stack.pop_safe(gc)?;
        self.trace_pop(&val);
        Ok(val)
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

use dotnet_types::TypeDescription;
use dotnet_utils::gc::GCHandle;
use dotnet_value::{
    StackValue,
    object::{Object as ObjectInstance, ObjectRef},
    pointer::ManagedPtr,
};
use gc_arena::Collect;
use std::ptr::NonNull;

#[derive(Collect, Default)]
#[collect(no_drop)]
pub struct EvaluationStack<'gc> {
    pub(crate) stack: Vec<StackValue<'gc>>,
    pub(crate) suspended_stack: Vec<StackValue<'gc>>,
}

impl<'gc> EvaluationStack<'gc> {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, _gc: GCHandle<'gc>, value: StackValue<'gc>) {
        #[cfg(feature = "multithreaded-gc")]
        crate::gc::coordinator::record_allocation(value.size_bytes());
        self.stack.push(value);
    }

    pub fn pop(&mut self, _gc: GCHandle<'gc>) -> StackValue<'gc> {
        self.stack.pop().expect("Evaluation stack underflow")
    }

    pub fn pop_i32(&mut self, gc: GCHandle<'gc>) -> i32 {
        self.pop(gc).as_i32()
    }

    pub fn pop_i64(&mut self, gc: GCHandle<'gc>) -> i64 {
        self.pop(gc).as_i64()
    }

    pub fn pop_f64(&mut self, gc: GCHandle<'gc>) -> f64 {
        self.pop(gc).as_f64()
    }

    pub fn pop_isize(&mut self, gc: GCHandle<'gc>) -> isize {
        self.pop(gc).as_isize()
    }

    pub fn pop_obj(&mut self, gc: GCHandle<'gc>) -> ObjectRef<'gc> {
        self.pop(gc).as_object_ref()
    }

    pub fn pop_ptr(&mut self, gc: GCHandle<'gc>) -> *mut u8 {
        self.pop(gc).as_ptr()
    }

    pub fn pop_managed_ptr(&mut self, gc: GCHandle<'gc>) -> ManagedPtr<'gc> {
        self.pop(gc).as_managed_ptr()
    }

    pub fn pop_value_type(&mut self, gc: GCHandle<'gc>) -> ObjectInstance<'gc> {
        self.pop(gc).as_value_type()
    }

    pub fn push_i32(&mut self, gc: GCHandle<'gc>, value: i32) {
        self.push(gc, StackValue::Int32(value));
    }

    pub fn push_i64(&mut self, gc: GCHandle<'gc>, value: i64) {
        self.push(gc, StackValue::Int64(value));
    }

    pub fn push_f64(&mut self, gc: GCHandle<'gc>, value: f64) {
        self.push(gc, StackValue::NativeFloat(value));
    }

    pub fn push_isize(&mut self, gc: GCHandle<'gc>, value: isize) {
        self.push(gc, StackValue::NativeInt(value));
    }

    pub fn push_obj(&mut self, gc: GCHandle<'gc>, value: ObjectRef<'gc>) {
        self.push(gc, StackValue::ObjectRef(value));
    }

    pub fn push_value_type(&mut self, gc: GCHandle<'gc>, value: ObjectInstance<'gc>) {
        self.push(gc, StackValue::ValueType(value));
    }

    pub fn push_managed_ptr(&mut self, gc: GCHandle<'gc>, value: ManagedPtr<'gc>) {
        self.push(gc, StackValue::ManagedPtr(value));
    }

    pub fn push_ptr(
        &mut self,
        gc: GCHandle<'gc>,
        ptr: *mut u8,
        t: TypeDescription,
        is_pinned: bool,
    ) {
        self.push(gc, StackValue::managed_ptr(ptr, t, is_pinned));
    }

    pub fn pop_multiple(&mut self, _gc: GCHandle<'gc>, count: usize) -> Vec<StackValue<'gc>> {
        let mut values = Vec::with_capacity(count);
        for _ in 0..count {
            values.push(self.stack.pop().expect("Evaluation stack underflow"));
        }
        values.reverse();
        values
    }

    pub fn peek_multiple(&self, count: usize) -> Vec<StackValue<'gc>> {
        let mut values = Vec::with_capacity(count);
        for i in 0..count {
            values.push(self.stack[self.stack.len() - 1 - i].clone());
        }
        values.reverse();
        values
    }

    pub fn peek_stack(&self) -> StackValue<'gc> {
        self.stack
            .last()
            .expect("Evaluation stack is empty")
            .clone()
    }

    pub fn peek_stack_at(&self, offset: usize) -> StackValue<'gc> {
        self.stack[self.stack.len() - 1 - offset].clone()
    }

    pub fn top_of_stack_address(&self) -> NonNull<u8> {
        self.stack
            .last()
            .expect("Evaluation stack is empty")
            .data_location()
    }

    pub fn top_of_stack(&self) -> usize {
        self.stack.len()
    }

    pub fn get_slot(&self, index: usize) -> StackValue<'gc> {
        self.stack[index].clone()
    }

    pub fn get_slot_address(&self, index: usize) -> NonNull<u8> {
        self.stack[index].data_location()
    }

    pub fn set_slot(&mut self, _gc: GCHandle<'gc>, index: usize, value: StackValue<'gc>) {
        #[cfg(feature = "multithreaded-gc")]
        if matches!(value, StackValue::ValueType(_)) {
            crate::gc::coordinator::record_allocation(value.size_bytes());
        }
        self.stack[index] = value;
    }

    pub fn truncate(&mut self, len: usize) {
        self.stack.truncate(len);
    }

    pub fn set_slot_at(&mut self, gc: GCHandle<'gc>, index: usize, value: StackValue<'gc>) {
        if index < self.stack.len() {
            self.set_slot(gc, index, value);
        } else {
            for _ in self.stack.len()..index {
                self.push(gc, StackValue::null());
            }
            self.push(gc, value);
        }
    }

    pub fn clear(&mut self) {
        self.stack.clear();
        self.suspended_stack.clear();
    }

    pub fn clear_suspended(&mut self) {
        self.suspended_stack.clear();
    }

    pub fn suspend_above(&mut self, index: usize) {
        self.suspended_stack = self.stack.split_off(index);
    }

    pub fn restore_suspended(&mut self) {
        self.stack.append(&mut self.suspended_stack);
    }
}

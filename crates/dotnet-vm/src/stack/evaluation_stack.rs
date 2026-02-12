use dotnet_types::TypeDescription;
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

    pub fn push(&mut self, value: StackValue<'gc>) {
        #[cfg(feature = "multithreaded-gc")]
        crate::gc::coordinator::record_allocation(value.size_bytes());
        let old_capacity = self.stack.capacity();
        self.stack.push(value);
        if self.stack.capacity() > old_capacity {
            self.update_stack_pointers();
        }
    }

    fn update_stack_pointers(&mut self) {
        let slot_locations: Vec<NonNull<u8>> =
            self.stack.iter().map(|v| v.data_location()).collect();
        let suspended_locations: Vec<NonNull<u8>> = self
            .suspended_stack
            .iter()
            .map(|v| v.data_location())
            .collect();

        let update_val = |val: &mut StackValue<'gc>| match val {
            StackValue::ManagedPtr(m) => {
                if let Some((idx, off)) = m.stack_slot_origin {
                    let slot_ptr = if idx < slot_locations.len() {
                        slot_locations[idx]
                    } else {
                        suspended_locations[idx - slot_locations.len()]
                    };
                    let new_ptr = unsafe { NonNull::new_unchecked(slot_ptr.as_ptr().add(off)) };
                    m.update_cached_ptr(new_ptr);
                }
            }
            StackValue::ValueType(obj) => {
                let layout = obj.instance_storage.layout().clone();
                let mut data = obj.instance_storage.get_mut();
                layout.visit_managed_ptrs(0, &mut |offset| {
                    if offset + 16 <= data.len() {
                        let slice = &mut data[offset..offset + 16];
                        let info = unsafe { ManagedPtr::read_stack_info(slice) };
                        if let Some((idx, _)) = info.stack_origin {
                            let slot_ptr = if idx < slot_locations.len() {
                                slot_locations[idx]
                            } else {
                                suspended_locations[idx - slot_locations.len()]
                            };
                            let new_ptr = unsafe {
                                NonNull::new_unchecked(slot_ptr.as_ptr().add(info.offset))
                            };
                            let word0 = 1 | ((idx & 0xFFFFFFFF) << 1) | (info.offset << 33);
                            let word1 = new_ptr.as_ptr() as usize;
                            slice[0..8].copy_from_slice(&word0.to_ne_bytes());
                            slice[8..16].copy_from_slice(&word1.to_ne_bytes());
                        }
                    }
                });
            }
            _ => {}
        };

        for val in self.stack.iter_mut() {
            update_val(val);
        }

        for val in self.suspended_stack.iter_mut() {
            update_val(val);
        }
    }

    pub fn pop(&mut self) -> StackValue<'gc> {
        self.stack.pop().expect("Evaluation stack underflow")
    }

    pub fn pop_safe(&mut self) -> Result<StackValue<'gc>, crate::error::VmError> {
        self.stack.pop().ok_or(crate::error::VmError::Execution(
            crate::error::ExecutionError::StackUnderflow,
        ))
    }

    pub fn pop_i32(&mut self) -> i32 {
        self.pop().as_i32()
    }

    pub fn pop_i64(&mut self) -> i64 {
        self.pop().as_i64()
    }

    pub fn pop_f64(&mut self) -> f64 {
        self.pop().as_f64()
    }

    pub fn pop_isize(&mut self) -> isize {
        self.pop().as_isize()
    }

    pub fn pop_obj(&mut self) -> ObjectRef<'gc> {
        self.pop().as_object_ref()
    }

    pub fn pop_ptr(&mut self) -> *mut u8 {
        self.pop().as_ptr()
    }

    pub fn pop_managed_ptr(&mut self) -> ManagedPtr<'gc> {
        self.pop().as_managed_ptr()
    }

    pub fn pop_value_type(&mut self) -> ObjectInstance<'gc> {
        self.pop().as_value_type()
    }

    pub fn push_i32(&mut self, value: i32) {
        self.push(StackValue::Int32(value));
    }

    pub fn push_i64(&mut self, value: i64) {
        self.push(StackValue::Int64(value));
    }

    pub fn push_f64(&mut self, value: f64) {
        self.push(StackValue::NativeFloat(value));
    }

    pub fn push_isize(&mut self, value: isize) {
        self.push(StackValue::NativeInt(value));
    }

    pub fn push_obj(&mut self, value: ObjectRef<'gc>) {
        self.push(StackValue::ObjectRef(value));
    }

    pub fn push_value_type(&mut self, value: ObjectInstance<'gc>) {
        self.push(StackValue::ValueType(value));
    }

    pub fn push_managed_ptr(&mut self, value: ManagedPtr<'gc>) {
        self.push(StackValue::ManagedPtr(value));
    }

    pub fn push_ptr(&mut self, ptr: *mut u8, t: TypeDescription, is_pinned: bool) {
        self.push(StackValue::managed_ptr(ptr, t, is_pinned));
    }

    pub fn pop_multiple(&mut self, count: usize) -> Vec<StackValue<'gc>> {
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

    pub fn get_slot_ref(&self, index: usize) -> &StackValue<'gc> {
        &self.stack[index]
    }

    pub fn get_slot_address(&self, index: usize) -> NonNull<u8> {
        self.stack[index].data_location()
    }

    pub fn set_slot(&mut self, index: usize, value: StackValue<'gc>) {
        #[cfg(feature = "multithreaded-gc")]
        if matches!(value, StackValue::ValueType(_)) {
            crate::gc::coordinator::record_allocation(value.size_bytes());
        }
        self.stack[index] = value;
    }

    pub fn truncate(&mut self, len: usize) {
        self.stack.truncate(len);
    }

    pub fn set_slot_at(&mut self, index: usize, value: StackValue<'gc>) {
        if index < self.stack.len() {
            self.set_slot(index, value);
        } else {
            for _ in self.stack.len()..index {
                self.push(StackValue::null());
            }
            self.push(value);
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
        self.update_stack_pointers();
    }

    pub fn restore_suspended(&mut self) {
        self.stack.append(&mut self.suspended_stack);
        self.update_stack_pointers();
    }
}

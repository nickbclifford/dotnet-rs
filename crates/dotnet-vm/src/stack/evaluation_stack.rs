use dotnet_types::TypeDescription;
use dotnet_value::{
    StackValue,
    object::{Object as ObjectInstance, ObjectRef},
    pointer::{ManagedPtr, PointerOrigin},
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
                if let PointerOrigin::Stack(idx) = m.origin() {
                    let off = m.byte_offset();
                    let slot_ptr = if idx.as_usize() < slot_locations.len() {
                        slot_locations[idx.as_usize()]
                    } else {
                        suspended_locations[idx.as_usize() - slot_locations.len()]
                    };
                    let new_ptr =
                        unsafe { NonNull::new_unchecked(slot_ptr.as_ptr().add(off.as_usize())) };
                    m.update_cached_ptr(new_ptr);
                }
            }
            StackValue::ValueType(obj) => {
                let layout = obj.instance_storage.layout().clone();
                obj.instance_storage.with_data_mut(|data| {
                    layout.visit_managed_ptrs(
                        dotnet_utils::ByteOffset(0),
                        &mut |offset: dotnet_utils::ByteOffset| {
                            if offset.as_usize() + 16 <= data.len() {
                                let offset_val = offset.as_usize();
                                let slice = &mut data[offset_val..offset_val + 16];
                                let info = unsafe { ManagedPtr::read_stack_info(slice) };
                                if let PointerOrigin::Stack(idx) = info.origin {
                                    let slot_ptr = if idx.as_usize() < slot_locations.len() {
                                        slot_locations[idx.as_usize()]
                                    } else {
                                        suspended_locations[idx.as_usize() - slot_locations.len()]
                                    };
                                    let new_ptr = unsafe {
                                        NonNull::new_unchecked(
                                            slot_ptr.as_ptr().add(info.offset.as_usize()),
                                        )
                                    };
                                    let word0: usize = 1
                                        | ((idx.as_usize() & 0x3FFFFFFF) << 3)
                                        | (info.offset.as_usize() << 33);
                                    let word1 = new_ptr.as_ptr() as usize;
                                    slice[0..8].copy_from_slice(&word0.to_ne_bytes());
                                    slice[8..16].copy_from_slice(&word1.to_ne_bytes());
                                }
                            }
                        },
                    );
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

    pub fn push_ptr(
        &mut self,
        ptr: *mut u8,
        t: TypeDescription,
        is_pinned: bool,
        owner: Option<ObjectRef<'gc>>,
        offset: Option<crate::ByteOffset>,
    ) {
        self.push(StackValue::managed_ptr_with_owner(
            ptr, t, owner, is_pinned, offset,
        ));
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

    pub fn top_of_stack(&self) -> crate::StackSlotIndex {
        crate::StackSlotIndex(self.stack.len())
    }

    #[inline]
    pub fn get_slot(&self, index: crate::StackSlotIndex) -> StackValue<'gc> {
        self.stack[index.as_usize()].clone()
    }

    #[inline]
    pub fn get_slot_ref(&self, index: crate::StackSlotIndex) -> &StackValue<'gc> {
        &self.stack[index.as_usize()]
    }

    #[inline]
    pub fn get_slot_address(&self, index: crate::StackSlotIndex) -> NonNull<u8> {
        self.stack[index.as_usize()].data_location()
    }

    #[inline]
    pub fn set_slot(&mut self, index: crate::StackSlotIndex, value: StackValue<'gc>) {
        self.stack[index.as_usize()] = value;
    }

    #[inline]
    pub fn truncate(&mut self, len: crate::StackSlotIndex) {
        self.stack.truncate(len.as_usize());
    }

    #[inline]
    pub fn set_slot_at(&mut self, index: crate::StackSlotIndex, value: StackValue<'gc>) {
        let idx = index.as_usize();
        if idx < self.stack.len() {
            self.set_slot(index, value);
        } else {
            for _ in self.stack.len()..idx {
                self.push(StackValue::null());
            }
            self.push(value);
        }
    }

    #[inline]
    pub fn clear(&mut self) {
        self.stack.clear();
        self.suspended_stack.clear();
    }

    #[inline]
    pub fn clear_suspended(&mut self) {
        self.suspended_stack.clear();
    }

    #[inline]
    pub fn suspend_above(&mut self, index: crate::StackSlotIndex) {
        self.suspended_stack = self.stack.split_off(index.as_usize());
        self.update_stack_pointers();
    }

    pub fn restore_suspended(&mut self) {
        self.stack.append(&mut self.suspended_stack);
        self.update_stack_pointers();
    }
}

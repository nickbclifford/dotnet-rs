use crate::{MethodInfo, MethodState};
use dotnet_types::{
    TypeDescription,
    error::{ExecutionError, VmError},
    generics::GenericLookup,
    resolution::ResolutionS,
    runtime::RuntimeType,
};
use dotnet_utils::{ByteOffset, StackSlotIndex};
use dotnet_value::{
    StackValue,
    object::{Object as ObjectInstance, ObjectHandle, ObjectRef},
    pointer::{ManagedPtr, PointerOrigin},
};
use gc_arena::Collect;
use std::ptr::NonNull;
#[cfg(feature = "bench-instrumentation")]
use std::time::Instant;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Collect)]
#[collect(require_static)]
pub struct BasePointer {
    pub arguments: StackSlotIndex,
    pub locals: StackSlotIndex,
    pub stack: StackSlotIndex,
}

impl Default for BasePointer {
    fn default() -> Self {
        Self {
            arguments: StackSlotIndex(0),
            locals: StackSlotIndex(0),
            stack: StackSlotIndex(0),
        }
    }
}

#[derive(Collect)]
#[collect(no_drop)]
pub struct MulticastState<'gc> {
    pub targets: ObjectHandle<'gc>,
    pub next_index: usize,
    pub args: Vec<StackValue<'gc>>,
}

#[derive(Collect)]
#[collect(no_drop)]
pub struct StackFrame<'gc> {
    pub stack_height: StackSlotIndex,
    pub base: BasePointer,
    pub state: MethodState,
    pub generic_inst: GenericLookup,
    pub source_resolution: ResolutionS,
    /// The exceptions currently being handled by catch blocks in this frame (required for rethrow).
    pub exception_stack: Vec<ObjectRef<'gc>>,
    pub pinned_locals: Vec<bool>,
    pub is_finalizer: bool,
    pub multicast_state: Option<MulticastState<'gc>>,
    pub awaiting_invoke_return: Option<RuntimeType>,
}

impl<'gc> StackFrame<'gc> {
    pub fn new(
        base_pointer: BasePointer,
        method: MethodInfo<'static>,
        generic_inst: GenericLookup,
        pinned_locals: Vec<bool>,
    ) -> Self {
        Self {
            stack_height: StackSlotIndex(0),
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

#[derive(Collect)]
#[collect(no_drop)]
pub struct EvaluationStack<'gc> {
    pub stack: Vec<StackValue<'gc>>,
    pub suspended_stack: Vec<StackValue<'gc>>,
}

impl<'gc> Default for EvaluationStack<'gc> {
    fn default() -> Self {
        Self::new()
    }
}

impl<'gc> EvaluationStack<'gc> {
    pub fn new() -> Self {
        Self {
            stack: vec![],
            suspended_stack: vec![],
        }
    }

    pub fn push(&mut self, value: StackValue<'gc>) {
        let old_capacity = self.stack.capacity();
        self.stack.push(value);
        if self.stack.capacity() > old_capacity {
            #[cfg(feature = "bench-instrumentation")]
            let fixup_start = Instant::now();
            self.update_stack_pointers();
            #[cfg(feature = "bench-instrumentation")]
            dotnet_metrics::record_active_eval_stack_reallocation(fixup_start.elapsed());
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
                    layout.visit_managed_ptrs(ByteOffset(0), &mut |offset: ByteOffset| {
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
                    });
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

    pub fn pop_safe(&mut self) -> Result<StackValue<'gc>, VmError> {
        self.stack
            .pop()
            .ok_or(VmError::Execution(ExecutionError::StackUnderflow))
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
        self.push(StackValue::ManagedPtr(value.into()));
    }

    pub fn push_ptr(
        &mut self,
        ptr: *mut u8,
        t: TypeDescription,
        is_pinned: bool,
        owner: Option<ObjectRef<'gc>>,
        offset: Option<ByteOffset>,
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

    pub fn top_of_stack(&self) -> StackSlotIndex {
        StackSlotIndex(self.stack.len())
    }

    pub fn get_slot(&self, index: StackSlotIndex) -> StackValue<'gc> {
        self.stack[index.as_usize()].clone()
    }

    pub fn get_slot_ref(&self, index: StackSlotIndex) -> &StackValue<'gc> {
        &self.stack[index.as_usize()]
    }

    pub fn get_slot_address(&self, index: StackSlotIndex) -> NonNull<u8> {
        self.stack[index.as_usize()].data_location()
    }

    pub fn set_slot(&mut self, index: StackSlotIndex, value: StackValue<'gc>) {
        self.stack[index.as_usize()] = value;
    }

    pub fn truncate(&mut self, len: StackSlotIndex) {
        self.stack.truncate(len.as_usize());
    }

    pub fn set_slot_at(&mut self, index: StackSlotIndex, value: StackValue<'gc>) {
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

    pub fn clear(&mut self) {
        self.stack.clear();
        self.suspended_stack.clear();
    }

    pub fn clear_suspended(&mut self) {
        self.suspended_stack.clear();
    }

    pub fn suspend_above(&mut self, index: StackSlotIndex) {
        self.suspended_stack = self.stack.split_off(index.as_usize());
        self.update_stack_pointers();
    }

    pub fn restore_suspended(&mut self) {
        self.stack.append(&mut self.suspended_stack);
        self.update_stack_pointers();
    }

    pub fn top_of_stack_address(&self) -> NonNull<u8> {
        self.stack
            .last()
            .expect("Evaluation stack is empty")
            .data_location()
    }
}

#[derive(Default, Collect)]
#[collect(no_drop)]
pub struct FrameStack<'gc> {
    pub frames: Vec<StackFrame<'gc>>,
    pub suspended_frames: Vec<StackFrame<'gc>>,
}

impl<'gc> FrameStack<'gc> {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, frame: StackFrame<'gc>) {
        self.frames.push(frame);
    }

    pub fn pop(&mut self) -> Option<StackFrame<'gc>> {
        self.frames.pop()
    }

    pub fn current_frame(&self) -> &StackFrame<'gc> {
        self.frames.last().expect("Frame stack underflow")
    }

    pub fn current_frame_opt_mut(&mut self) -> Option<&mut StackFrame<'gc>> {
        self.frames.last_mut()
    }

    pub fn current_frame_mut(&mut self) -> &mut StackFrame<'gc> {
        self.frames.last_mut().expect("Frame stack underflow")
    }

    pub fn state(&self) -> &MethodState {
        &self.current_frame().state
    }

    pub fn state_mut(&mut self) -> &mut MethodState {
        &mut self.current_frame_mut().state
    }

    pub fn branch(&mut self, target: usize) {
        self.state_mut().ip = target;
    }

    pub fn conditional_branch(&mut self, condition: bool, target: usize) -> bool {
        if condition {
            self.state_mut().ip = target;
            true
        } else {
            false
        }
    }

    pub fn increment_ip(&mut self) {
        self.state_mut().ip += 1;
    }

    pub fn is_empty(&self) -> bool {
        self.frames.is_empty()
    }

    pub fn len(&self) -> usize {
        self.frames.len()
    }

    pub fn clear(&mut self) {
        self.frames.clear();
        self.suspended_frames.clear();
    }

    pub fn clear_suspended(&mut self) {
        self.suspended_frames.clear();
    }

    pub fn suspend_above(&mut self, index: usize) {
        self.suspended_frames = self.frames.split_off(index + 1);
    }

    pub fn restore_suspended(&mut self) {
        self.frames.append(&mut self.suspended_frames);
    }
}

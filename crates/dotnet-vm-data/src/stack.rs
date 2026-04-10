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
use gc_arena::{Collect, collect::Trace};
use smallvec::SmallVec;
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

pub const STACKFRAME_EXCEPTION_INLINE_CAPACITY: usize = 2;
pub const STACKFRAME_PINNED_LOCALS_INLINE_CAPACITY: usize = 8;
pub const STACKFRAME_POOL_LIMIT: usize = 64;

pub type ExceptionStack<'gc> = SmallVec<[ObjectRef<'gc>; STACKFRAME_EXCEPTION_INLINE_CAPACITY]>;
pub type PinnedLocals = SmallVec<[bool; STACKFRAME_PINNED_LOCALS_INLINE_CAPACITY]>;

pub struct StackFrame<'gc> {
    pub stack_height: StackSlotIndex,
    pub base: BasePointer,
    pub state: MethodState,
    pub generic_inst: GenericLookup,
    pub source_resolution: ResolutionS,
    /// The exceptions currently being handled by catch blocks in this frame (required for rethrow).
    pub exception_stack: ExceptionStack<'gc>,
    pub pinned_locals: PinnedLocals,
    pub is_finalizer: bool,
    pub multicast_state: Option<MulticastState<'gc>>,
    pub awaiting_invoke_return: Option<RuntimeType>,
}

impl<'gc> StackFrame<'gc> {
    pub fn new(
        base_pointer: BasePointer,
        method: MethodInfo<'static>,
        generic_inst: GenericLookup,
        pinned_locals: PinnedLocals,
    ) -> Self {
        Self {
            stack_height: StackSlotIndex(0),
            base: base_pointer,
            source_resolution: method.source.resolution(),
            state: MethodState::new(method),
            generic_inst,
            exception_stack: SmallVec::new(),
            pinned_locals,
            is_finalizer: false,
            multicast_state: None,
            awaiting_invoke_return: None,
        }
    }

    pub fn reset_for_call(
        &mut self,
        base_pointer: BasePointer,
        method: MethodInfo<'static>,
        generic_inst: GenericLookup,
        pinned_locals: PinnedLocals,
    ) {
        self.stack_height = StackSlotIndex(0);
        self.base = base_pointer;
        self.source_resolution = method.source.resolution();
        self.state = MethodState::new(method);
        self.generic_inst = generic_inst;
        self.exception_stack.clear();
        self.pinned_locals = pinned_locals;
        self.is_finalizer = false;
        self.multicast_state = None;
        self.awaiting_invoke_return = None;
    }

    pub fn prepare_for_pool(&mut self) {
        self.stack_height = StackSlotIndex(0);
        self.base = BasePointer::default();
        self.exception_stack.clear();
        self.pinned_locals.clear();
        self.is_finalizer = false;
        self.multicast_state = None;
        self.awaiting_invoke_return = None;
    }
}

// SAFETY: `StackFrame` traces all GC-managed fields it can contain:
// `exception_stack` entries and optional `multicast_state`.
unsafe impl<'gc> Collect<'gc> for StackFrame<'gc> {
    fn trace<Tr: Trace<'gc>>(&self, cc: &mut Tr) {
        for exception in &self.exception_stack {
            exception.trace(cc);
        }
        self.multicast_state.trace(cc);
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
    #[cfg(feature = "segmented-eval-stack-prototype")]
    const SEGMENTED_STACK_GRANULARITY: usize = 64;

    pub fn new() -> Self {
        Self {
            stack: vec![],
            suspended_stack: vec![],
        }
    }

    #[inline]
    fn apply_reallocation_fixup(&mut self) {
        #[cfg(feature = "bench-instrumentation")]
        let fixup_start = Instant::now();
        self.update_stack_pointers();
        #[cfg(feature = "bench-instrumentation")]
        dotnet_metrics::record_active_eval_stack_reallocation(fixup_start.elapsed());
    }

    #[cfg(feature = "segmented-eval-stack-prototype")]
    #[inline]
    fn normalize_reserve_target(required_slots: usize) -> usize {
        if required_slots == 0 {
            return 0;
        }

        let rem = required_slots % Self::SEGMENTED_STACK_GRANULARITY;
        if rem == 0 {
            required_slots
        } else {
            required_slots.saturating_add(Self::SEGMENTED_STACK_GRANULARITY - rem)
        }
    }

    #[cfg(not(feature = "segmented-eval-stack-prototype"))]
    #[inline]
    fn normalize_reserve_target(required_slots: usize) -> usize {
        required_slots
    }

    /// Ensures the active evaluation stack has capacity for `total_slots` entries.
    /// If the underlying `Vec` reallocates, stack-origin managed pointers are fixed up.
    pub fn reserve_slots(&mut self, total_slots: usize) {
        if total_slots <= self.stack.capacity() {
            return;
        }

        let old_capacity = self.stack.capacity();
        let reserve_target = Self::normalize_reserve_target(total_slots).max(total_slots);
        let additional = reserve_target.saturating_sub(self.stack.len());
        self.stack.reserve(additional);

        if self.stack.capacity() > old_capacity {
            self.apply_reallocation_fixup();
        }
    }

    pub fn push(&mut self, value: StackValue<'gc>) {
        let old_capacity = self.stack.capacity();
        self.stack.push(value);
        if self.stack.capacity() > old_capacity {
            self.apply_reallocation_fixup();
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
        self.pop_multiple_into(count, &mut values);
        values
    }

    pub fn pop_multiple_into(&mut self, count: usize, out: &mut Vec<StackValue<'gc>>) {
        out.clear();
        out.reserve(count);
        for _ in 0..count {
            out.push(self.stack.pop().expect("Evaluation stack underflow"));
        }
        out.reverse();
    }

    pub fn copy_slots_into(
        &self,
        start: StackSlotIndex,
        count: usize,
        out: &mut Vec<StackValue<'gc>>,
    ) {
        out.clear();
        out.reserve(count);
        let start = start.as_usize();
        for i in 0..count {
            out.push(self.stack[start + i].clone());
        }
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
    pub pooled_frames: Vec<StackFrame<'gc>>,
}

impl<'gc> FrameStack<'gc> {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, frame: StackFrame<'gc>) {
        self.frames.push(frame);
    }

    pub fn push_frame(
        &mut self,
        base_pointer: BasePointer,
        method: MethodInfo<'static>,
        generic_inst: GenericLookup,
        pinned_locals: PinnedLocals,
    ) {
        if let Some(mut frame) = self.pooled_frames.pop() {
            frame.reset_for_call(base_pointer, method, generic_inst, pinned_locals);
            self.frames.push(frame);
            #[cfg(feature = "bench-instrumentation")]
            dotnet_metrics::record_active_frame_pool_hit();
        } else {
            self.frames.push(StackFrame::new(
                base_pointer,
                method,
                generic_inst,
                pinned_locals,
            ));
            #[cfg(feature = "bench-instrumentation")]
            dotnet_metrics::record_active_frame_pool_miss();
        }
    }

    pub fn pop(&mut self) -> Option<StackFrame<'gc>> {
        self.frames.pop()
    }

    pub fn recycle_frame(&mut self, mut frame: StackFrame<'gc>) {
        frame.prepare_for_pool();
        if self.pooled_frames.len() < STACKFRAME_POOL_LIMIT {
            self.pooled_frames.push(frame);
            #[cfg(feature = "bench-instrumentation")]
            dotnet_metrics::record_active_frame_pool_recycle();
        }
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
        self.pooled_frames.clear();
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

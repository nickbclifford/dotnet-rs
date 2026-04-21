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

pub struct EvaluationStack<'gc> {
    pub stack: Vec<StackValue<'gc>>,
    pub suspended_stack: Vec<StackValue<'gc>>,
    slot_locations_scratch: Vec<NonNull<u8>>,
    suspended_locations_scratch: Vec<NonNull<u8>>,
}

// SAFETY: `EvaluationStack` traces all GC-managed fields (`stack` and `suspended_stack`).
// Scratch pointer caches are raw addresses derived from stack values and contain no GC references.
unsafe impl<'gc> Collect<'gc> for EvaluationStack<'gc> {
    fn trace<Tr: Trace<'gc>>(&self, cc: &mut Tr) {
        self.stack.trace(cc);
        self.suspended_stack.trace(cc);
    }
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
            slot_locations_scratch: vec![],
            suspended_locations_scratch: vec![],
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
        self.slot_locations_scratch.clear();
        self.slot_locations_scratch
            .extend(self.stack.iter().map(|v| v.data_location()));
        self.suspended_locations_scratch.clear();
        self.suspended_locations_scratch
            .extend(self.suspended_stack.iter().map(|v| v.data_location()));

        let stack_len = self.slot_locations_scratch.len();
        let slot_locations = self.slot_locations_scratch.as_slice();
        let suspended_locations = self.suspended_locations_scratch.as_slice();

        let resolve_slot_ptr = |idx: StackSlotIndex| {
            let idx = idx.as_usize();
            if idx < stack_len {
                slot_locations[idx]
            } else {
                suspended_locations[idx - stack_len]
            }
        };

        let update_val = |val: &mut StackValue<'gc>| match val {
            StackValue::ManagedPtr(m) => {
                if let PointerOrigin::Stack(idx) = m.origin() {
                    let off = m.byte_offset();
                    let slot_ptr = resolve_slot_ptr(*idx);
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
                                let slot_ptr = resolve_slot_ptr(idx);
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
        let len = self.stack.len();
        let split_at = len.checked_sub(count).expect("Evaluation stack underflow");
        out.extend(self.stack.drain(split_at..));
    }

    pub fn copy_slots_into(
        &self,
        start: StackSlotIndex,
        count: usize,
        out: &mut Vec<StackValue<'gc>>,
    ) {
        out.clear();
        let start = start.as_usize();
        let end = start.checked_add(count).expect("Stack slot range overflow");
        let slice = self
            .stack
            .get(start..end)
            .expect("Stack slot range out of bounds");
        out.reserve(count);
        out.extend_from_slice(slice);
    }

    pub fn peek_multiple(&self, count: usize) -> Vec<StackValue<'gc>> {
        let len = self.stack.len();
        let start = len.checked_sub(count).expect("Evaluation stack underflow");
        self.stack[start..].to_vec()
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

#[cfg(test)]
mod tests {
    use super::*;

    fn int_stack(values: &[i32]) -> EvaluationStack<'static> {
        let mut stack = EvaluationStack::new();
        for value in values {
            stack.push_i32(*value);
        }
        stack
    }

    fn as_i32_values(values: &[StackValue<'static>]) -> Vec<i32> {
        values.iter().map(StackValue::as_i32).collect()
    }

    #[test]
    fn pop_multiple_into_keeps_original_order() {
        let mut stack = int_stack(&[1, 2, 3, 4, 5]);
        let mut out = vec![StackValue::Int32(99)];

        stack.pop_multiple_into(3, &mut out);

        assert_eq!(as_i32_values(&out), vec![3, 4, 5]);
        assert_eq!(stack.stack.len(), 2);
        assert_eq!(stack.peek_stack().as_i32(), 2);
    }

    #[test]
    fn copy_slots_into_copies_requested_slice() {
        let stack = int_stack(&[10, 20, 30, 40, 50]);
        let mut out = Vec::new();

        stack.copy_slots_into(StackSlotIndex(1), 3, &mut out);

        assert_eq!(as_i32_values(&out), vec![20, 30, 40]);
    }

    #[test]
    fn peek_multiple_returns_top_segment_in_stack_order() {
        let stack = int_stack(&[5, 6, 7, 8]);

        let values = stack.peek_multiple(2);

        assert_eq!(as_i32_values(&values), vec![7, 8]);
        assert_eq!(stack.top_of_stack().as_usize(), 4);
    }

    #[test]
    fn multi_value_ops_reuse_output_allocation() {
        let mut stack = int_stack(&[1, 2, 3, 4]);
        let mut out = Vec::with_capacity(8);
        out.push(StackValue::Int32(-1));
        let initial_capacity = out.capacity();

        stack.pop_multiple_into(2, &mut out);
        assert_eq!(out.capacity(), initial_capacity);

        stack.copy_slots_into(StackSlotIndex(0), 2, &mut out);
        assert_eq!(out.capacity(), initial_capacity);
    }

    #[test]
    fn stack_pointer_fixup_updates_cached_ptr_on_reallocation() {
        let mut stack = int_stack(&[11, 22]);
        let slot0_ptr = stack.get_slot_address(StackSlotIndex(0)).as_ptr();
        stack.push(StackValue::managed_stack_ptr(
            StackSlotIndex(0),
            ByteOffset(0),
            slot0_ptr,
            TypeDescription::NULL,
            false,
        ));

        stack.reserve_slots(512);

        let expected_ptr = stack.get_slot_address(StackSlotIndex(0)).as_ptr();
        let actual_ptr = stack.peek_stack().as_ptr();
        assert_eq!(actual_ptr, expected_ptr);
    }

    #[test]
    fn stack_pointer_fixup_tracks_suspended_slots() {
        let mut stack = int_stack(&[1, 2]);
        let slot1_ptr = stack.get_slot_address(StackSlotIndex(1)).as_ptr();
        stack.push(StackValue::managed_stack_ptr(
            StackSlotIndex(1),
            ByteOffset(0),
            slot1_ptr,
            TypeDescription::NULL,
            false,
        ));

        stack.suspend_above(StackSlotIndex(1));
        let expected_suspended_ptr = stack.suspended_stack[0].data_location().as_ptr();
        let actual_suspended_ptr = stack.suspended_stack[1].as_ptr();
        assert_eq!(actual_suspended_ptr, expected_suspended_ptr);

        stack.restore_suspended();
        let expected_restored_ptr = stack.get_slot_address(StackSlotIndex(1)).as_ptr();
        let actual_restored_ptr = stack.peek_stack().as_ptr();
        assert_eq!(actual_restored_ptr, expected_restored_ptr);
    }

    #[test]
    fn stack_pointer_fixup_reuses_scratch_buffers() {
        let mut stack = int_stack(&[3, 4, 5]);
        stack.update_stack_pointers();
        let initial_slot_capacity = stack.slot_locations_scratch.capacity();
        let initial_suspended_capacity = stack.suspended_locations_scratch.capacity();

        stack.update_stack_pointers();

        assert_eq!(
            stack.slot_locations_scratch.capacity(),
            initial_slot_capacity
        );
        assert_eq!(
            stack.suspended_locations_scratch.capacity(),
            initial_suspended_capacity
        );
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

use crate::{MethodInfo, MethodState, frame_homes::FrameHomes};
use dotnet_types::{
    TypeDescription,
    error::{ExecutionError, VmError},
    generics::GenericLookup,
    resolution::ResolutionS,
};
use dotnet_utils::{ByteOffset, StackSlotIndex};
use dotnet_value::pointer::FrameHomeSerial;
use dotnet_value::{
    StackValue,
    object::{Object as ObjectInstance, ObjectHandle, ObjectRef},
    pointer::{ManagedPtr, RetainedPtr},
};
use gc_arena::{Collect, collect::Trace};
use smallvec::SmallVec;
use std::ptr::NonNull;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Collect, Default)]
#[collect(require_static)]
pub struct BasePointer {
    pub arguments: StackSlotIndex,
    pub locals: StackSlotIndex,
    pub stack: StackSlotIndex,
}

/// Continuation for multicast invocations excluded from native effects transport.
/// Admitted invocations use the native activation stack instead.
#[derive(Collect)]
#[collect(no_drop)]
pub struct MulticastState<'gc> {
    pub targets: ObjectHandle<'gc>,
    pub next_index: usize,
    pub args: Vec<StackValue<'gc>>,
    pub retrying_initializer: bool,
}

pub const STACKFRAME_EXCEPTION_INLINE_CAPACITY: usize = 2;
pub const STACKFRAME_PINNED_LOCALS_INLINE_CAPACITY: usize = 8;
pub const STACKFRAME_POOL_LIMIT: usize = 64;

pub type ExceptionStack<'gc> = SmallVec<[ObjectRef<'gc>; STACKFRAME_EXCEPTION_INLINE_CAPACITY]>;
pub type PinnedLocals = SmallVec<[bool; STACKFRAME_PINNED_LOCALS_INLINE_CAPACITY]>;

/// Stable identity of a native activation within one thread.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Collect)]
#[collect(require_static)]
pub struct NativeId(pub std::num::NonZeroU64);

/// Address of the effect to receive a managed frame's return value.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Collect)]
#[collect(require_static)]
pub struct NativeReturnDestination {
    pub id: NativeId,
    pub generation: u64,
}

/// A timed byref monitor fallback or Thread.Join retry belongs to one invocation in this frame.
pub struct MonitorRetry {
    pub method: dotnet_types::members::MethodDescription,
    pub ip: usize,
    pub stack_top: StackSlotIndex,
    pub deadline: std::time::Instant,
}

/// A synchronization-only obligation owned by a legacy initializer frame.
///
/// # Safety
/// Implementations must contain no GC references: frames do not trace this interface.
/// Dropping an implementation must never execute managed code.
pub unsafe trait InitializerClaim: std::fmt::Debug {
    fn succeed(&mut self);
    fn fail(&mut self);
}

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
    pub monitor_retry: Option<Box<MonitorRetry>>,
    pub native_return: Option<NativeReturnDestination>,
    // INVARIANT: F6.InitializerClaimLifetime — frame disposal and pooling drop the synchronization-only obligation.
    pub initializer_claim: Option<Box<dyn InitializerClaim>>,
    /// Homes of the locals and arguments this frame's method addresses with `ldloca` and
    /// `ldarga`.
    pub homes: FrameHomes,
    pub constructor_result: Option<Box<crate::frame_homes::ConstructorResult>>,
    /// Monitor object of a synchronized method (ECMA-335 §II.23.1.11): set when the frame starts
    /// acquiring it. The frame holds it only once execution is past index 0; it is released when
    /// the frame returns or unwinds.
    pub sync_lock: Option<ObjectRef<'gc>>,
}

impl<'gc> StackFrame<'gc> {
    pub fn new(
        base_pointer: BasePointer,
        method: MethodInfo<'static>,
        generic_inst: GenericLookup,
        pinned_locals: PinnedLocals,
    ) -> Self {
        Self {
            stack_height: StackSlotIndex::new(0),
            base: base_pointer,
            source_resolution: method.source.resolution(),
            state: MethodState::new(method),
            generic_inst,
            exception_stack: SmallVec::new(),
            pinned_locals,
            is_finalizer: false,
            multicast_state: None,
            monitor_retry: None,
            native_return: None,
            initializer_claim: None,
            homes: FrameHomes::default(),
            constructor_result: None,
            sync_lock: None,
        }
    }

    pub fn reset_for_call(
        &mut self,
        base_pointer: BasePointer,
        method: MethodInfo<'static>,
        generic_inst: GenericLookup,
        pinned_locals: PinnedLocals,
    ) {
        self.stack_height = StackSlotIndex::new(0);
        self.base = base_pointer;
        self.source_resolution = method.source.resolution();
        self.state = MethodState::new(method);
        self.generic_inst = generic_inst;
        self.exception_stack.clear();
        self.pinned_locals = pinned_locals;
        self.is_finalizer = false;
        self.multicast_state = None;
        self.monitor_retry = None;
        self.native_return = None;
        self.initializer_claim = None;
        self.constructor_result = None;
        self.sync_lock = None;
        self.homes.release();
    }

    pub fn prepare_for_pool(&mut self) {
        self.state.allocations.release_all();
        self.homes.release();
        self.stack_height = StackSlotIndex::new(0);
        self.base = BasePointer::default();
        self.exception_stack.clear();
        self.pinned_locals.clear();
        self.is_finalizer = false;
        self.multicast_state = None;
        self.monitor_retry = None;
        self.native_return = None;
        self.initializer_claim = None;
        self.constructor_result = None;
        self.sync_lock = None;
    }
}

// SAFETY: F5.TracesEveryGcRef — `StackFrame` traces all GC-managed fields it can contain:
// `exception_stack` entries, optional `multicast_state`, the synchronized-method `sync_lock`, and the references serialized in its
// local `homes`. The native return destination
// contains only an ID and generation. Initializer claims contain synchronization state only;
// Retry state for byref monitor fallbacks and Thread.Join contains only static metadata and time.
// Constructor completion also carries only static metadata; its
// receiver bytes are traced through the caller's homes.
unsafe impl<'gc> Collect<'gc> for StackFrame<'gc> {
    fn trace<Tr: Trace<'gc>>(&self, cc: &mut Tr) {
        for exception in &self.exception_stack {
            exception.trace(cc);
        }
        self.multicast_state.trace(cc);
        self.sync_lock.trace(cc);
        self.homes.trace(cc);
    }
}

pub struct EvaluationStack<'gc> {
    pub stack: Vec<StackValue<'gc>>,
    pub suspended_stack: Vec<StackValue<'gc>>,
    suspended_segments: Vec<(usize, usize)>,
}

// SAFETY: F5.TracesEveryGcRef — `EvaluationStack` traces all GC-managed fields (`stack` and `suspended_stack`).
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
    pub fn new() -> Self {
        Self {
            stack: vec![],
            suspended_stack: vec![],
            suspended_segments: vec![],
        }
    }

    /// Ensures the active evaluation stack has capacity for `total_slots` entries.
    /// Frame-home addresses are independent of this vector's allocation.
    pub fn reserve_slots(&mut self, total_slots: usize) {
        if total_slots <= self.stack.capacity() {
            return;
        }

        let additional = total_slots.saturating_sub(self.stack.len());
        self.stack.reserve(additional);
    }

    pub fn push(&mut self, value: StackValue<'gc>) {
        self.stack.push(value);
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

    /// Pops a pointer-like value, retaining its owner for the lifetime of the result.
    ///
    /// # Panics
    /// Panics on stack underflow, a non-pointer operand, or an unavailable
    /// cross-arena owner.
    pub fn pop_retained_ptr(&mut self) -> RetainedPtr<'gc> {
        RetainedPtr::try_from_stack_value(self.pop()).expect("expected pointer on stack")
    }

    /// Pops a pointer-like value and observes its address without exposing provenance.
    /// The address does not retain an owner or authorize access.
    ///
    /// # Panics
    /// Panics on stack underflow or a non-pointer operand.
    pub fn pop_address(&mut self) -> usize {
        self.pop().as_ptr().addr()
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
        StackSlotIndex::new(self.stack.len())
    }

    pub fn get_slot(&self, index: StackSlotIndex) -> StackValue<'gc> {
        self.stack[index.as_usize()].clone()
    }

    pub fn get_slot_ref(&self, index: StackSlotIndex) -> &StackValue<'gc> {
        &self.stack[index.as_usize()]
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
        self.suspended_segments.clear();
    }

    pub fn clear_suspended(&mut self) {
        self.suspended_stack.clear();
        self.suspended_segments.clear();
    }

    pub fn suspend_above(&mut self, index: StackSlotIndex) {
        self.suspended_segments
            .push((index.as_usize(), self.suspended_stack.len()));
        self.suspended_stack
            .extend(self.stack.drain(index.as_usize()..));
    }

    pub fn suspension_depth(&self) -> usize {
        self.suspended_segments.len()
    }

    pub fn restore_suspended(&mut self) {
        let (base, start) = self
            .suspended_segments
            .pop()
            .expect("evaluation suspension underflow");
        assert!(self.stack.len() >= base, "evaluation suspension base lost");
        self.stack.truncate(base);
        self.stack.extend(self.suspended_stack.drain(start..));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nested_suspension_restores_each_operand_segment() {
        let mut stack = int_stack(&[10, 20, 30]);
        stack.suspend_above(StackSlotIndex::new(1));
        stack.push(StackValue::Int32(40));
        stack.suspend_above(StackSlotIndex::new(1));
        stack.push(StackValue::Int32(50));
        assert_eq!(stack.suspension_depth(), 2);
        stack.restore_suspended();
        assert_eq!(as_i32_values(&stack.stack), vec![10, 40]);
        stack.restore_suspended();
        assert_eq!(as_i32_values(&stack.stack), vec![10, 20, 30]);
        assert_eq!(stack.suspension_depth(), 0);
    }

    #[test]
    fn empty_suspension_has_its_own_depth_and_discards_cleanly() {
        let mut stack = int_stack(&[10]);
        stack.suspend_above(StackSlotIndex::new(0));
        stack.suspend_above(StackSlotIndex::new(0));
        assert_eq!(stack.suspension_depth(), 2);
        stack.restore_suspended();
        assert!(stack.stack.is_empty());
        assert_eq!(stack.suspended_stack.len(), 1);
        stack.clear_suspended();
        assert_eq!(stack.suspension_depth(), 0);
        assert!(stack.suspended_stack.is_empty());
    }

    fn owner_retention_transient_stack() -> (
        EvaluationStack<'static>,
        dotnet_utils::sync::Arc<dotnet_value::layout::FieldLayoutManager>,
    ) {
        use dotnet_utils::sync::Arc;
        use dotnet_value::{
            layout::{FieldLayoutManager, GcDesc},
            storage::FieldStorage,
        };
        use hashbrown::HashMap;

        let layout = Arc::new(FieldLayoutManager {
            fields: HashMap::new(),
            total_size: 1,
            alignment: 1,
            gc_desc: GcDesc::default(),
            has_ref_fields: false,
        });
        let probe = Arc::clone(&layout);
        let obj = ObjectInstance::new(
            TypeDescription::NULL,
            GenericLookup::default(),
            FieldStorage::new(layout, vec![0xA5]),
        );
        let base = obj.with_data(|data| NonNull::new(data.as_ptr().cast_mut()).unwrap());
        let mut stack = EvaluationStack::new();
        stack.push_managed_ptr(ManagedPtr::new_transient(
            Some(base),
            TypeDescription::NULL,
            obj,
            ByteOffset::new(0),
        ));
        (stack, probe)
    }

    #[test]
    fn owner_retention_pop_retained_ptr_keeps_transient_owner_alive() {
        let (mut stack, probe) = owner_retention_transient_stack();
        assert_eq!(dotnet_utils::sync::Arc::strong_count(&probe), 2);
        let pointer = stack.pop_retained_ptr();
        assert_eq!(dotnet_utils::sync::Arc::strong_count(&probe), 2);
        // SAFETY: F10.RawMemoryAccessValid — The retained transient owner contains
        // the initialized byte, and remains alive throughout this in-range read.
        assert_eq!(unsafe { pointer.as_ptr().read() }, 0xA5);
        drop(pointer);
        assert_eq!(dotnet_utils::sync::Arc::strong_count(&probe), 1);
    }

    #[test]
    fn owner_retention_pop_address_matches_stack_value_pointer() {
        let (mut stack, probe) = owner_retention_transient_stack();
        let expected = stack.stack.last().unwrap().as_ptr().addr();
        assert_eq!(stack.pop_address(), expected);
        assert_eq!(dotnet_utils::sync::Arc::strong_count(&probe), 1);
    }

    #[test]
    fn owner_retention_pointer_pops_reject_non_pointers() {
        let mut stack = EvaluationStack::new();
        stack.push_i32(1);
        assert!(
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                stack.pop_retained_ptr()
            }))
            .is_err()
        );
        stack.push_i32(1);
        assert!(
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| { stack.pop_address() }))
                .is_err()
        );
    }

    #[test]
    fn owner_retention_pop_ptr_retains_transient_owner() {
        let (mut stack, probe) = owner_retention_transient_stack();
        assert_eq!(dotnet_utils::sync::Arc::strong_count(&probe), 2);
        let _pointer = stack.pop_retained_ptr();
        assert_eq!(dotnet_utils::sync::Arc::strong_count(&probe), 2);
    }

    #[test]
    fn owner_retention_pop_ptr_access_after_pop() {
        let (mut stack, _probe) = owner_retention_transient_stack();
        let pointer = stack.pop_retained_ptr();
        // SAFETY: F10.RawMemoryAccessValid — The retained owner contains the
        // initialized byte and remains alive throughout this in-range read.
        let byte = unsafe { pointer.as_ptr().read() };
        assert_eq!(byte, 0xA5);
    }

    /// The P/Invoke non-ref marshalling shape (`dotnet-pinvoke` `call.rs`): a
    /// `peek_stack_at` clone derives a raw address, is dropped, and the address
    /// is then used while the original stack slot is still alive.
    #[test]
    fn owner_retention_peek_clone_drop_keeps_transient_address_valid() {
        use dotnet_utils::sync::Arc;
        let (stack, probe) = owner_retention_transient_stack();
        assert_eq!(Arc::strong_count(&probe), 2);
        let peeked = stack.peek_stack_at(0);
        // The clone shares the owner rather than forking the buffer.
        assert_eq!(Arc::strong_count(&probe), 2);
        let StackValue::ManagedPtr(p) = &peeked else {
            panic!("expected managed pointer");
        };
        // SAFETY: F10.RawMemoryAccessValid — in-range transient access, as in
        // the non-ref P/Invoke arm.
        let ptr = unsafe { p.with_data(0, |data| data.as_ptr().cast_mut()) };
        drop(peeked);
        assert_eq!(Arc::strong_count(&probe), 2);
        // SAFETY: F10.RawMemoryAccessValid — the original stack slot still
        // retains the shared Transient owner, so the byte is live.
        assert_eq!(unsafe { ptr.read() }, 0xA5);
        assert_eq!(
            ptr.addr(),
            stack.stack.last().unwrap().as_ptr().addr(),
            "peek clone and original slot must agree on the address"
        );
    }

    /// Ref write-back clones the origin; it must retain the same owner
    /// and stay valid after the argument slots are popped.
    #[test]
    fn owner_retention_write_back_origin_clone_retains_transient_owner() {
        use dotnet_utils::sync::Arc;
        let (mut stack, probe) = owner_retention_transient_stack();
        let peeked = stack.peek_stack_at(0);
        let StackValue::ManagedPtr(p) = &peeked else {
            panic!("expected managed pointer");
        };
        let origin = p.origin().clone();
        let offset = p.byte_offset();
        drop(peeked);
        drop(stack.pop_retained_ptr());
        assert_eq!(
            Arc::strong_count(&probe),
            2,
            "write-back origin is sole owner"
        );
        let dotnet_value::pointer::PointerOrigin::Transient(obj) = &origin else {
            panic!("origin must stay Transient");
        };
        assert_eq!(offset.as_usize(), 0);
        assert_eq!(obj.with_data(|data| data[0]), 0xA5);
    }

    /// A TypedRef slot clones with the stack value and keeps the owner.
    #[test]
    fn owner_retention_typed_ref_clone_retains_transient_owner() {
        use dotnet_utils::sync::Arc;
        use dotnet_value::TypedReferenceSlot;
        let (mut stack, probe) = owner_retention_transient_stack();
        let StackValue::ManagedPtr(p) = stack.pop() else {
            panic!("expected managed pointer");
        };
        let slot = TypedReferenceSlot::new(p, Arc::new(TypeDescription::NULL));
        stack.push(StackValue::TypedRef(slot));
        assert_eq!(Arc::strong_count(&probe), 2);
        let peeked = stack.peek_stack_at(0);
        assert_eq!(Arc::strong_count(&probe), 2);
        let StackValue::TypedRef(s) = &peeked else {
            panic!("expected typedref");
        };
        // SAFETY: F10.RawMemoryAccessValid — in-range transient access, as in
        // the typedref P/Invoke fallback arm.
        let ptr = unsafe { s.value().with_data(0, |data| data.as_ptr().cast_mut()) };
        drop(peeked);
        // SAFETY: F10.RawMemoryAccessValid — the original slot retains the owner.
        assert_eq!(unsafe { ptr.read() }, 0xA5);
    }

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
    fn copy_slots_into_copies_requested_slice() {
        let stack = int_stack(&[10, 20, 30, 40, 50]);
        let mut out = Vec::new();

        stack.copy_slots_into(StackSlotIndex::new(1), 3, &mut out);

        assert_eq!(as_i32_values(&out), vec![20, 30, 40]);
    }
}

#[derive(Default, Collect)]
#[collect(no_drop)]
pub struct FrameStack<'gc> {
    pub frames: Vec<StackFrame<'gc>>,
    pub suspended_frames: Vec<StackFrame<'gc>>,
    /// Enclosing filters retained while callees replace the current exception mode.
    pub active_filters: Vec<crate::FilterState<'gc>>,
    suspended_segments: Vec<(usize, usize)>,
    pub pooled_frames: Vec<StackFrame<'gc>>,
    /// The last home-block serial issued on this thread; zero before the first.
    last_home_serial: u64,
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

    /// Issues the serial of a new home block. Serials only increase, so no two blocks of this
    /// thread share one.
    pub fn issue_home_serial(&mut self) -> FrameHomeSerial {
        let serial = FrameHomeSerial::new(self.last_home_serial).next();
        self.last_home_serial = serial.get();
        serial
    }

    /// Transfer the thread's home serial high-water mark between execution segments.
    pub fn inherit_home_serial(&mut self, other: &Self) {
        self.last_home_serial = self.last_home_serial.max(other.last_home_serial);
    }

    /// The base and length of the home block of the live frame with `serial`. Searches from
    /// the current frame down, so a byref into the current or a nearby caller frame resolves
    /// in a few steps. Filter-suspended frames remain live; pooled frames do not.
    pub fn home_block(&self, serial: FrameHomeSerial) -> Option<(NonNull<u8>, usize)> {
        self.frames
            .iter()
            .rev()
            .chain(self.suspended_frames.iter().rev())
            .find_map(|frame| frame.homes.block_for(serial))
    }

    /// Recognizes serialized byref variables in active and suspended frames.
    pub fn is_byref_home(&self, serial: FrameHomeSerial, offset: usize) -> bool {
        self.frames
            .iter()
            .chain(self.suspended_frames.iter())
            .any(|frame| frame.homes.is_byref_home(serial, offset))
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
        self.active_filters.clear();
        self.suspended_segments.clear();
        self.pooled_frames.clear();
    }

    pub fn clear_suspended(&mut self) {
        self.suspended_frames.clear();
        self.active_filters.clear();
        self.suspended_segments.clear();
    }

    pub fn suspend_above(&mut self, index: usize) {
        self.suspended_segments
            .push((index + 1, self.suspended_frames.len()));
        self.suspended_frames.extend(self.frames.drain(index + 1..));
    }

    pub fn suspension_depth(&self) -> usize {
        self.suspended_segments.len()
    }

    pub fn restore_suspended(&mut self) {
        let (base, start) = self
            .suspended_segments
            .pop()
            .expect("frame suspension underflow");
        assert_eq!(self.frames.len(), base, "frame suspension base mismatch");
        self.frames.extend(self.suspended_frames.drain(start..));
    }
}

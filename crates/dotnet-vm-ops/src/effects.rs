//! Thread-local native effects and their synchronous, branded VM access.
use std::{
    cell::{Cell, RefCell},
    future::Future,
    pin::Pin,
    ptr,
    rc::Rc,
    task::{Context, Poll, Waker},
};

use dotnet_types::error::{ExecutionError, VmError};
use dotnet_utils::ByteOffset;
use dotnet_value::{
    ManagedPtr, StackValue,
    object::{HeapStorage, ObjectRef},
    pointer::{FrameHomeSerial, ManagedPtrInfo, PointerOrigin},
};
use gc_arena::{Collect, DynamicRoot, DynamicRootSet, Gc, Mutation, Rootable};

fn internal(message: &'static str) -> VmError {
    ExecutionError::InternalError(message.into()).into()
}

/// Failure of a native body, either a rooted guest exception or a host error.
#[derive(Debug)]
pub enum NativeFailure {
    Managed(RootedValue),
    Host(VmError),
}
impl From<VmError> for NativeFailure {
    fn from(value: VmError) -> Self {
        Self::Host(value)
    }
}
pub type NativeResult = Result<(), NativeFailure>;
pub type NativeFuture = Pin<Box<dyn Future<Output = NativeResult>>>;

#[derive(Collect)]
#[collect(no_drop)]
struct RootCell<'gc> {
    value: RootCellValue<'gc>,
}

#[derive(Collect)]
#[collect(no_drop)]
enum RootCellValue<'gc> {
    Value(StackValue<'gc>),
    Pointer {
        origin: PointerOrigin<'gc>,
        #[collect(require_static)]
        inner_type: dotnet_types::TypeDescription,
        offset: ByteOffset,
        pinned: bool,
    },
}

/// An activation-owned value. Reference roots are private and set-checked on fetch.
#[derive(Clone)]
pub struct RootedValue(RootedValueInner);
#[derive(Clone)]
enum RootedValueInner {
    Int32(i32),
    Int64(i64),
    NativeInt(isize),
    NativeFloat(f64),
    Null,
    Object(DynamicRoot<Rootable![RootCell<'_>]>),
}
impl std::fmt::Debug for RootedValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("RootedValue(..)")
    }
}
/// Result of a transparent call, including a distinct void result.
#[derive(Clone, Debug)]
pub enum NativeValue {
    Void,
    Value(RootedValue),
}

impl RootedValue {
    fn stash<'gc>(
        mc: &Mutation<'gc>,
        roots: DynamicRootSet<'gc>,
        value: StackValue<'gc>,
    ) -> Result<Self, VmError> {
        let value = match value {
            StackValue::Int32(v) => RootedValueInner::Int32(v),
            StackValue::Int64(v) => RootedValueInner::Int64(v),
            StackValue::NativeInt(v) => RootedValueInner::NativeInt(v),
            StackValue::NativeFloat(v) => RootedValueInner::NativeFloat(v),
            StackValue::ObjectRef(ObjectRef(None)) => RootedValueInner::Null,
            v @ StackValue::ObjectRef(ObjectRef(Some(_))) => RootedValueInner::Object(roots.stash(
                mc,
                Gc::new(
                    mc,
                    RootCell {
                        value: RootCellValue::Value(v),
                    },
                ),
            )),
            StackValue::ManagedPtr(pointer) => {
                // INVARIANT: F6.NativePointerDescriptor — only traced heap owners or
                // non-reused frame-home serials can survive across polls.
                let supported = match pointer.origin() {
                    PointerOrigin::Heap(ObjectRef(Some(_))) | PointerOrigin::FrameHome(_) => true,
                    #[cfg(feature = "multithreading")]
                    PointerOrigin::CrossArenaObjectRef(_) => true,
                    _ => false,
                };
                if !supported {
                    return Err(internal("unsupported native pointer origin"));
                }
                let value = RootCellValue::Pointer {
                    origin: pointer.origin().clone(),
                    inner_type: pointer.inner_type(),
                    offset: pointer.byte_offset(),
                    pinned: pointer.is_pinned(),
                };
                RootedValueInner::Object(roots.stash(mc, Gc::new(mc, RootCell { value })))
            }
            _ => return Err(internal("unsupported native operand")),
        };
        Ok(Self(value))
    }
    fn fetch<'gc>(
        &self,
        roots: DynamicRootSet<'gc>,
        backend: &dyn NativeBackend<'gc>,
    ) -> Result<StackValue<'gc>, VmError> {
        Ok(match &self.0 {
            RootedValueInner::Int32(v) => StackValue::Int32(*v),
            RootedValueInner::Int64(v) => StackValue::Int64(*v),
            RootedValueInner::NativeInt(v) => StackValue::NativeInt(*v),
            RootedValueInner::NativeFloat(v) => StackValue::NativeFloat(*v),
            RootedValueInner::Null => StackValue::ObjectRef(ObjectRef(None)),
            RootedValueInner::Object(v) => match &roots
                .try_fetch(v)
                .map_err(|_| internal("mismatched native root set"))?
                .value
            {
                RootCellValue::Value(value) => value.clone(),
                RootCellValue::Pointer {
                    origin,
                    inner_type,
                    offset,
                    pinned,
                } => {
                    // INVARIANT: F6.NativePointerDescriptor — the root keeps the heap
                    // origin live; frame-home addresses are resolved anew and retired homes fail.
                    let address = if let PointerOrigin::FrameHome(serial) = origin {
                        let (base, length) = backend
                            .frame_home_block(*serial)
                            .ok_or_else(|| internal("native frame home is no longer live"))?;
                        if offset.as_usize() > length {
                            return Err(internal("native frame home offset out of range"));
                        }
                        std::ptr::NonNull::new(base.as_ptr().wrapping_add(offset.as_usize()))
                    } else {
                        let pointer = ManagedPtr::from_info_full(
                            ManagedPtrInfo {
                                address: None,
                                origin: origin.clone(),
                                offset: *offset,
                            },
                            inner_type.clone(),
                            *pinned,
                        );
                        pointer.observe_address()
                    };
                    if address.is_none() {
                        return Err(internal("native pointer owner unavailable"));
                    }
                    StackValue::ManagedPtr(
                        ManagedPtr::from_info_full(
                            ManagedPtrInfo {
                                address,
                                origin: origin.clone(),
                                offset: *offset,
                            },
                            inner_type.clone(),
                            *pinned,
                        )
                        .into(),
                    )
                }
            },
        })
    }
}

/// Owned selector data interpreted by the selected native body.
#[derive(Clone, Copy, Debug)]
pub struct NativeInvocation {
    pub start: usize,
    pub length: usize,
    pub parameter_count: usize,
}

/// An initializer claim publishes failure when abandoned without invoking managed code.
pub trait NativeInitClaim {
    fn succeed(&mut self);
    fn fail(&mut self, exception: ObjectRef<'_>);
}

/// A wait registration owns its wait-graph edge until it is dropped.
pub trait NativeInitWait {
    fn ready(&self) -> bool;
    fn sleeper(&self) -> Box<dyn NativeInitSleeper>;
}

/// A condition wait that can run outside an arena mutation.
pub trait NativeInitSleeper {
    fn wait(&self, duration: std::time::Duration);
}

/// Result of probing the shared initializer state during a branded access.
pub enum NativeInitProbe<'gc> {
    Complete,
    Failed(Option<ObjectRef<'gc>>),
    Execute(NativeCall, Box<dyn NativeInitClaim>),
    Waiting(Box<dyn NativeInitWait>),
}

enum RootedInitProbe {
    Complete,
    Failed(Option<RootedValue>),
    Execute(NativeCall, Box<dyn NativeInitClaim>),
    Waiting(Box<dyn NativeInitWait>),
}

/// Adapter supplied for one poll. It owns no native future and must not dispatch recursively.
pub trait NativeBackend<'gc> {
    fn stop_requested(&self) -> bool;
    fn active_scope_depth(&self) -> usize;
    fn probe_initializer(
        &mut self,
        _description: dotnet_types::TypeDescription,
        _generics: &dotnet_types::generics::GenericLookup,
    ) -> Result<NativeInitProbe<'gc>, VmError> {
        Err(internal("native initialization unavailable"))
    }
    fn wrap_initializer_failure(
        &mut self,
        _description: &dotnet_types::TypeDescription,
        _inner: Option<ObjectRef<'gc>>,
    ) -> Result<ObjectRef<'gc>, VmError> {
        Err(internal("native initialization unavailable"))
    }
    fn operands(&mut self) -> &mut Vec<StackValue<'gc>>;
    fn push_new_string(&mut self, chars: Vec<u16>) -> Result<(), VmError>;
    fn frame_home_block(&self, _serial: FrameHomeSerial) -> Option<(std::ptr::NonNull<u8>, usize)> {
        None
    }
    fn multicast_target(
        &mut self,
        _receiver: StackValue<'gc>,
        _index: usize,
    ) -> Result<Option<NativeTarget<'gc>>, VmError> {
        Err(internal("native multicast unavailable"))
    }
    fn reflection_call(&mut self) -> Result<NativePreparedCall<'gc>, VmError> {
        Err(internal("native reflection unavailable"))
    }
    fn reflection_result(
        &mut self,
        _method: &dotnet_types::members::MethodDescription,
        _generics: &dotnet_types::generics::GenericLookup,
        _value: Option<StackValue<'gc>>,
    ) -> Result<StackValue<'gc>, VmError> {
        Err(internal("native reflection result unavailable"))
    }
    fn reflection_constructor_result(
        &mut self,
        _method: &dotnet_types::members::MethodDescription,
        _generics: &dotnet_types::generics::GenericLookup,
        _value: StackValue<'gc>,
    ) -> Result<StackValue<'gc>, VmError> {
        Err(internal("native reflection constructor result unavailable"))
    }
    /// Write a boolean through a freshly resolved managed pointer.
    ///
    /// # Safety
    /// Any in-range byte must be writable boolean storage, with synchronized access and no
    /// overlapping reference slot. The backend checks the complete range and live owner.
    unsafe fn write_monitor_flag(
        &mut self,
        _pointer: StackValue<'gc>,
        _value: bool,
    ) -> Result<(), VmError> {
        Err(internal("native monitor flag unavailable"))
    }
    fn push_output(&mut self, _value: StackValue<'gc>) -> Result<(), VmError> {
        Err(internal("native output unavailable"))
    }
}

/// A delegate target resolved for the duration of one branded access.
pub struct NativeTarget<'gc> {
    pub method: dotnet_types::members::MethodDescription,
    pub generics: dotnet_types::generics::GenericLookup,
    pub receiver: Option<StackValue<'gc>>,
}

/// Arguments produced during one synchronous reflection marshalling operation.
pub struct NativePreparedCall<'gc> {
    pub method: dotnet_types::members::MethodDescription,
    pub generics: dotnet_types::generics::GenericLookup,
    pub arguments: Vec<StackValue<'gc>>,
    pub constructor_result: Option<StackValue<'gc>>,
}

/// Branded synchronous access, never retained across a poll.
pub struct NativeRegion<'gc, 'a> {
    mc: &'a Mutation<'gc>,
    roots: DynamicRootSet<'gc>,
    backend: &'a mut dyn NativeBackend<'gc>,
}
impl<'gc, 'a> NativeRegion<'gc, 'a> {
    pub fn new(
        mc: &'a Mutation<'gc>,
        roots: DynamicRootSet<'gc>,
        backend: &'a mut dyn NativeBackend<'gc>,
    ) -> Self {
        Self { mc, roots, backend }
    }
    /// Return a value only while this region's brand is live.
    pub fn fetch(&self, value: &RootedValue) -> Result<StackValue<'gc>, VmError> {
        value.fetch(self.roots, self.backend)
    }
    /// Copy a traced value into this activation's root set.
    pub fn root(&self, value: StackValue<'gc>) -> Result<RootedValue, VmError> {
        RootedValue::stash(self.mc, self.roots, value)
    }
    fn prepare_multicast(&mut self) -> Result<(RootedValue, Vec<RootedValue>), VmError> {
        let values = self.backend.operands().clone();
        let (receiver, arguments) = values
            .split_first()
            .ok_or_else(|| internal("missing delegate receiver"))?;
        let receiver = self.root(receiver.clone())?;
        let arguments = arguments
            .iter()
            .cloned()
            .map(|value| self.root(value))
            .collect::<Result<Vec<_>, _>>()?;
        self.backend.operands().clear();
        Ok((receiver, arguments))
    }
    fn prepare_reflection(&mut self) -> Result<(NativeCall, Option<RootedValue>), VmError> {
        let prepared = self.backend.reflection_call()?;
        let constructor_result = prepared
            .constructor_result
            .map(|value| self.root(value))
            .transpose()?;
        // INVARIANT: F6.NativePointerDescriptor — marshalling values are converted to
        // traced descriptors in this region before the call can suspend.
        let arguments = prepared
            .arguments
            .into_iter()
            .map(|value| self.root(value))
            .collect::<Result<Vec<_>, _>>()?;
        self.backend.operands().clear();
        Ok((
            NativeCall {
                method: prepared.method,
                generics: prepared.generics,
                arguments,
            },
            constructor_result,
        ))
    }
    fn probe_initializer(
        &mut self,
        description: dotnet_types::TypeDescription,
        generics: &dotnet_types::generics::GenericLookup,
    ) -> Result<RootedInitProbe, VmError> {
        Ok(
            match self.backend.probe_initializer(description, generics)? {
                NativeInitProbe::Complete => RootedInitProbe::Complete,
                NativeInitProbe::Failed(exception) => RootedInitProbe::Failed(
                    exception
                        .map(|exception| self.root(StackValue::ObjectRef(exception)))
                        .transpose()?,
                ),
                NativeInitProbe::Execute(call, claim) => RootedInitProbe::Execute(call, claim),
                NativeInitProbe::Waiting(wait) => RootedInitProbe::Waiting(wait),
            },
        )
    }

    fn wrap_initializer_failure(
        &mut self,
        description: &dotnet_types::TypeDescription,
        inner: Option<&RootedValue>,
        claim: Option<&mut dyn NativeInitClaim>,
    ) -> Result<RootedValue, VmError> {
        let inner = match inner {
            Some(root) => match self.fetch(root)? {
                StackValue::ObjectRef(exception) => Some(exception),
                _ => return Err(internal("initializer failure is not an exception")),
            },
            None => None,
        };
        let exception = self.backend.wrap_initializer_failure(description, inner)?;
        let root = self.root(StackValue::ObjectRef(exception))?;
        if let Some(claim) = claim {
            claim.fail(exception);
        }
        Ok(root)
    }

    /// Transform a successful reflected return while its value is rooted.
    pub fn reflection_result(
        &mut self,
        method: &dotnet_types::members::MethodDescription,
        generics: &dotnet_types::generics::GenericLookup,
        value: NativeValue,
    ) -> Result<RootedValue, VmError> {
        let fetched = match &value {
            NativeValue::Void => None,
            NativeValue::Value(root) => Some(self.fetch(root)?),
        };
        let result = self.backend.reflection_result(method, generics, fetched)?;
        self.root(result)
    }
    /// Normalize a constructed value while its owner remains rooted.
    pub fn reflection_constructor_result(
        &mut self,
        method: &dotnet_types::members::MethodDescription,
        generics: &dotnet_types::generics::GenericLookup,
        value: &RootedValue,
    ) -> Result<RootedValue, VmError> {
        let fetched = self.fetch(value)?;
        let result = self
            .backend
            .reflection_constructor_result(method, generics, fetched)?;
        self.root(result)
    }
    fn multicast_call(
        &mut self,
        receiver: &RootedValue,
        arguments: &[RootedValue],
        index: usize,
    ) -> Result<Option<NativeCall>, VmError> {
        let receiver = self.fetch(receiver)?;
        let Some(target) = self.backend.multicast_target(receiver, index)? else {
            return Ok(None);
        };
        let mut call_arguments =
            Vec::with_capacity(arguments.len() + usize::from(target.receiver.is_some()));
        if let Some(receiver) = target.receiver {
            call_arguments.push(self.root(receiver)?);
        }
        call_arguments.extend(arguments.iter().cloned());
        Ok(Some(NativeCall {
            method: target.method,
            generics: target.generics,
            arguments: call_arguments,
        }))
    }
    fn push_value(&mut self, value: NativeValue) -> Result<(), VmError> {
        if let NativeValue::Value(value) = value {
            let value = self.fetch(&value)?;
            self.backend.push_output(value)?;
        }
        Ok(())
    }
    fn pop_object(&mut self) -> Result<RootedValue, VmError> {
        self.pop_reference("native object operand invalid")
    }
    fn pop_string(&mut self) -> Result<RootedValue, VmError> {
        self.pop_reference("native string operand invalid")
    }
    fn pop_reference(&mut self, invalid: &'static str) -> Result<RootedValue, VmError> {
        let value = self
            .backend
            .operands()
            .last()
            .ok_or_else(|| internal("native operand underflow"))?
            .clone();
        if !matches!(value, StackValue::ObjectRef(ObjectRef(Some(_)))) {
            return Err(internal(invalid));
        }
        let root = RootedValue::stash(self.mc, self.roots, value)?;
        self.backend.operands().pop();
        Ok(root)
    }
    fn prepare_monitor(
        &mut self,
        timed: bool,
        byref: bool,
    ) -> Result<(RootedValue, Option<RootedValue>, Option<i32>), VmError> {
        let operands = self.backend.operands().clone();
        let expected = 1 + usize::from(timed) + usize::from(byref);
        if operands.len() != expected
            || !matches!(operands[0], StackValue::ObjectRef(ObjectRef(Some(_))))
        {
            return Err(internal("invalid native monitor operands"));
        }
        let object = self.root(operands[0].clone())?;
        let timeout = if timed {
            match operands[1] {
                StackValue::Int32(value) => Some(value),
                _ => return Err(internal("invalid monitor timeout")),
            }
        } else {
            None
        };
        let flag = if byref {
            Some(self.root(operands[expected - 1].clone())?)
        } else {
            None
        };
        self.backend.operands().clear();
        Ok((object, flag, timeout))
    }
    /// # Safety
    /// Any in-range byte must be writable boolean storage, with synchronized access and no
    /// overlapping reference slot. The backend checks the complete range and live owner.
    unsafe fn write_monitor_flag(
        &mut self,
        pointer: &RootedValue,
        value: bool,
    ) -> Result<(), VmError> {
        let pointer = self.fetch(pointer)?;
        // SAFETY: F10.RawMemoryAccessValid — the caller supplies a writable bool; the backend checks the live range.
        unsafe { self.backend.write_monitor_flag(pointer, value) }
    }
    fn push_bool(&mut self, value: bool) -> Result<(), VmError> {
        self.backend
            .push_output(StackValue::Int32(i32::from(value)))
    }
    fn discard_scalars(&mut self, count: usize) -> Result<(), VmError> {
        let operands = self.backend.operands();
        let start = operands
            .len()
            .checked_sub(count)
            .ok_or_else(|| internal("native operand underflow"))?;
        if !operands[start..].iter().all(|v| {
            matches!(
                v,
                StackValue::Int32(_)
                    | StackValue::Int64(_)
                    | StackValue::NativeInt(_)
                    | StackValue::NativeFloat(_)
            )
        }) {
            return Err(internal("native operand is not scalar"));
        }
        operands.truncate(start);
        Ok(())
    }
}

/// A resolved managed call and its activation-owned arguments, in stack order.
pub struct NativeCall {
    pub method: dotnet_types::members::MethodDescription,
    pub generics: dotnet_types::generics::GenericLookup,
    pub arguments: Vec<RootedValue>,
}

enum Mailbox {
    Empty,
    Request(u64, NativeCall, bool),
    Reply(u64, Result<NativeValue, RootedValue>),
    MonitorRequest(u64, RootedValue, Option<std::time::Instant>),
    MonitorReply(u64, bool),
    FinalizerRequest(u64),
    FinalizerReply(u64),
    InitializerRequest(u64, Box<dyn NativeInitWait>),
    InitializerReply(u64),
}

/// Effects registered by the body for the executor to service.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NativeEffect {
    Yield,
    Call(u64),
    Monitor(u64),
    Finalizers(u64),
    Initializer(u64),
}
/// One executor turn's maximum number of VM access grants.
pub const VM_ACCESS_QUANTUM: usize = 32;

/// Shared control is thread-affine and never contains a branded reference.
pub struct NativeControl {
    adapter: Cell<*mut ()>,
    entered: Cell<bool>,
    accesses: Cell<usize>,
    grants: Cell<usize>,
    effect: Cell<Option<NativeEffect>>,
    generation: Cell<u64>,
    awaiting: Cell<Option<NativeEffect>>,
    mailbox: RefCell<Mailbox>,
    issued: Cell<bool>,
    disabled: Cell<bool>,
}
impl NativeControl {
    pub fn new() -> Rc<Self> {
        Rc::new(Self {
            adapter: Cell::new(ptr::null_mut()),
            entered: Cell::new(false),
            accesses: Cell::new(0),
            grants: Cell::new(0),
            effect: Cell::new(None),
            generation: Cell::new(0),
            awaiting: Cell::new(None),
            mailbox: RefCell::new(Mailbox::Empty),
            issued: Cell::new(false),
            disabled: Cell::new(false),
        })
    }
    pub fn effects(self: &Rc<Self>, invocation: NativeInvocation) -> Result<VmEffects, VmError> {
        if self.disabled.get() || self.issued.replace(true) {
            return Err(internal("native capability unavailable or already issued"));
        }
        Ok(VmEffects {
            control: self.clone(),
            invocation,
        })
    }
    pub fn take_effect(&self) -> Option<NativeEffect> {
        self.effect.take()
    }
    pub fn take_call(&self, generation: u64) -> Result<(NativeCall, bool), VmError> {
        let mut slot = self.mailbox.borrow_mut();
        match std::mem::replace(&mut *slot, Mailbox::Empty) {
            Mailbox::Request(actual, call, capture)
                if actual == generation && self.awaiting.get().is_none() =>
            {
                self.awaiting.set(Some(NativeEffect::Call(generation)));
                Ok((call, capture))
            }
            other => {
                *slot = other;
                Err(internal("invalid native call request"))
            }
        }
    }
    pub fn reply_call(&self, generation: u64, value: NativeValue) -> Result<(), VmError> {
        let mut slot = self.mailbox.borrow_mut();
        if self.awaiting.get() != Some(NativeEffect::Call(generation))
            || !matches!(*slot, Mailbox::Empty)
        {
            return Err(internal("invalid native call reply"));
        }
        self.awaiting.set(None);
        *slot = Mailbox::Reply(generation, Ok(value));
        Ok(())
    }
    pub fn reply_exception(&self, generation: u64, exception: RootedValue) -> Result<(), VmError> {
        let mut slot = self.mailbox.borrow_mut();
        if self.awaiting.get() != Some(NativeEffect::Call(generation))
            || !matches!(*slot, Mailbox::Empty)
        {
            return Err(internal("invalid native call reply"));
        }
        self.awaiting.set(None);
        *slot = Mailbox::Reply(generation, Err(exception));
        Ok(())
    }
    pub fn take_monitor(
        &self,
        generation: u64,
    ) -> Result<(RootedValue, Option<std::time::Instant>), VmError> {
        let mut slot = self.mailbox.borrow_mut();
        match std::mem::replace(&mut *slot, Mailbox::Empty) {
            Mailbox::MonitorRequest(actual, object, deadline)
                if actual == generation && self.awaiting.get().is_none() =>
            {
                self.awaiting.set(Some(NativeEffect::Monitor(generation)));
                Ok((object, deadline))
            }
            other => {
                *slot = other;
                Err(internal("invalid monitor request"))
            }
        }
    }
    pub fn reply_monitor(&self, generation: u64, acquired: bool) -> Result<(), VmError> {
        let mut slot = self.mailbox.borrow_mut();
        if self.awaiting.get() != Some(NativeEffect::Monitor(generation))
            || !matches!(*slot, Mailbox::Empty)
        {
            return Err(internal("invalid monitor reply"));
        }
        self.awaiting.set(None);
        *slot = Mailbox::MonitorReply(generation, acquired);
        Ok(())
    }
    pub fn take_finalizers(&self, generation: u64) -> Result<(), VmError> {
        let mut slot = self.mailbox.borrow_mut();
        match std::mem::replace(&mut *slot, Mailbox::Empty) {
            Mailbox::FinalizerRequest(actual)
                if actual == generation && self.awaiting.get().is_none() =>
            {
                self.awaiting
                    .set(Some(NativeEffect::Finalizers(generation)));
                Ok(())
            }
            other => {
                *slot = other;
                Err(internal("invalid finalizer wait request"))
            }
        }
    }
    pub fn reply_finalizers(&self, generation: u64) -> Result<(), VmError> {
        let mut slot = self.mailbox.borrow_mut();
        if self.awaiting.get() != Some(NativeEffect::Finalizers(generation))
            || !matches!(*slot, Mailbox::Empty)
        {
            return Err(internal("invalid finalizer wait reply"));
        }
        self.awaiting.set(None);
        *slot = Mailbox::FinalizerReply(generation);
        Ok(())
    }
    pub fn take_initializer(&self, generation: u64) -> Result<Box<dyn NativeInitWait>, VmError> {
        let mut slot = self.mailbox.borrow_mut();
        match std::mem::replace(&mut *slot, Mailbox::Empty) {
            Mailbox::InitializerRequest(actual, wait)
                if actual == generation && self.awaiting.get().is_none() =>
            {
                self.awaiting
                    .set(Some(NativeEffect::Initializer(generation)));
                Ok(wait)
            }
            other => {
                *slot = other;
                Err(internal("invalid initializer wait request"))
            }
        }
    }
    pub fn reply_initializer(&self, generation: u64) -> Result<(), VmError> {
        let mut slot = self.mailbox.borrow_mut();
        if self.awaiting.get() != Some(NativeEffect::Initializer(generation))
            || !matches!(*slot, Mailbox::Empty)
        {
            return Err(internal("invalid initializer wait reply"));
        }
        self.awaiting.set(None);
        *slot = Mailbox::InitializerReply(generation);
        Ok(())
    }
    /// Permanently revoke this activation's access before dropping its future.
    pub fn disable(&self) {
        self.disabled.set(true);
    }
    fn enter<R>(
        &self,
        f: impl for<'gc> FnOnce(&mut NativeRegion<'gc, 'gc>) -> R,
    ) -> Result<R, VmError> {
        let p = self.adapter.get();
        if p.is_null() || self.disabled.get() || self.entered.replace(true) {
            return Err(internal("native adapter unavailable or already entered"));
        }
        struct Exit<'a>(&'a Cell<bool>);
        impl Drop for Exit<'_> {
            fn drop(&mut self) {
                self.0.set(false);
            }
        }
        let _exit = Exit(&self.entered);
        // SAFETY: F6.NativePollAdapter — The erased pointer and reborrowed region have identical
        // types apart from lifetimes; the entered flag ensures unique access; the poll's install/reset
        // guard keeps the pointer valid; on panic the entry guard resets before the install guard.
        Ok(f(unsafe { &mut *p.cast::<NativeRegion<'_, '_>>() }))
    }
}

/// Thread-affine capability moved into the static native future.
pub struct VmEffects {
    control: Rc<NativeControl>,
    invocation: NativeInvocation,
}
impl VmEffects {
    pub fn invocation(&self) -> NativeInvocation {
        self.invocation
    }
    /// Call managed code transparently.
    pub async fn call(&mut self, call: NativeCall) -> Result<NativeValue, NativeFailure> {
        self.call_inner(call, false).await
    }

    /// Call managed code, capturing an escaping exception after its cleanup has run.
    /// A guest exception is returned as `NativeFailure::Managed` with an activation-owned
    /// root; host failures remain `NativeFailure::Host`.
    pub async fn try_call(&mut self, call: NativeCall) -> Result<NativeValue, NativeFailure> {
        self.call_inner(call, true).await
    }

    async fn call_inner(
        &mut self,
        call: NativeCall,
        capture: bool,
    ) -> Result<NativeValue, NativeFailure> {
        let mut call = Some(call);
        let mut generation = None;
        std::future::poll_fn(|_| {
            if let Some(expected) = generation {
                let mut slot = self.control.mailbox.borrow_mut();
                let value = match std::mem::replace(&mut *slot, Mailbox::Empty) {
                    Mailbox::Reply(actual, value) if actual == expected => value,
                    other => {
                        *slot = other;
                        return Poll::Ready(Err(internal("invalid native call reply").into()));
                    }
                };
                drop(slot);
                let value = match value {
                    Ok(value) => value,
                    Err(exception) if capture => {
                        match self
                            .control
                            .enter(|region| region.fetch(&exception).map(|_| ()))
                        {
                            Ok(Ok(())) => {
                                return Poll::Ready(Err(NativeFailure::Managed(exception)));
                            }
                            Ok(Err(error)) | Err(error) => return Poll::Ready(Err(error.into())),
                        }
                    }
                    Err(_) => {
                        return Poll::Ready(Err(
                            internal("unexpected native exception reply").into()
                        ));
                    }
                };
                if let NativeValue::Value(root) = &value {
                    match self.control.enter(|region| region.fetch(root).map(|_| ())) {
                        Ok(Ok(())) => {}
                        Ok(Err(error)) | Err(error) => return Poll::Ready(Err(error.into())),
                    }
                }
                return Poll::Ready(Ok(value));
            }
            let Some(next) = self.control.generation.get().checked_add(1) else {
                return Poll::Ready(Err(internal("native effect generation exhausted").into()));
            };
            let mut slot = self.control.mailbox.borrow_mut();
            if !matches!(*slot, Mailbox::Empty)
                || self.control.effect.get().is_some()
                || self.control.awaiting.get().is_some()
            {
                return Poll::Ready(Err(internal("native effect already registered").into()));
            }
            self.control.generation.set(next);
            *slot = Mailbox::Request(next, call.take().expect("call requested once"), capture);
            self.control.effect.set(Some(NativeEffect::Call(next)));
            generation = Some(next);
            Poll::Pending
        })
        .await
    }
    /// Ensure a type's static constructor has completed before continuing.
    pub async fn ensure_initialized(
        &mut self,
        description: dotnet_types::TypeDescription,
        generics: dotnet_types::generics::GenericLookup,
    ) -> NativeResult {
        loop {
            let probe = {
                let mut vm = self.vm().await?;
                vm.with(|region| region.probe_initializer(description.clone(), &generics))??
            };
            match probe {
                RootedInitProbe::Complete => return Ok(()),
                RootedInitProbe::Failed(Some(exception)) => {
                    return Err(NativeFailure::Managed(exception));
                }
                RootedInitProbe::Failed(None) => {
                    let exception = {
                        let mut vm = self.vm().await?;
                        vm.with(|region| {
                            region.wrap_initializer_failure(&description, None, None)
                        })??
                    };
                    return Err(NativeFailure::Managed(exception));
                }
                RootedInitProbe::Execute(call, mut claim) => match self.try_call(call).await {
                    Ok(NativeValue::Void) => {
                        claim.succeed();
                        return Ok(());
                    }
                    Ok(_) => return Err(internal("initializer returned a value").into()),
                    Err(NativeFailure::Host(error)) => return Err(error.into()),
                    Err(NativeFailure::Managed(inner)) => {
                        let exception = {
                            let mut vm = self.vm().await?;
                            vm.with(|region| {
                                region.wrap_initializer_failure(
                                    &description,
                                    Some(&inner),
                                    Some(claim.as_mut()),
                                )
                            })??
                        };
                        return Err(NativeFailure::Managed(exception));
                    }
                },
                RootedInitProbe::Waiting(wait) => self.wait_initializer(wait).await?,
            }
        }
    }

    async fn wait_initializer(&mut self, wait: Box<dyn NativeInitWait>) -> NativeResult {
        let mut wait = Some(wait);
        let mut generation = None;
        std::future::poll_fn(|_| {
            if let Some(expected) = generation {
                let mut slot = self.control.mailbox.borrow_mut();
                return match std::mem::replace(&mut *slot, Mailbox::Empty) {
                    Mailbox::InitializerReply(actual) if actual == expected => Poll::Ready(Ok(())),
                    other => {
                        *slot = other;
                        Poll::Ready(Err(internal("invalid initializer wait reply").into()))
                    }
                };
            }
            let Some(next) = self.control.generation.get().checked_add(1) else {
                return Poll::Ready(Err(internal("native effect generation exhausted").into()));
            };
            let mut slot = self.control.mailbox.borrow_mut();
            if !matches!(*slot, Mailbox::Empty)
                || self.control.effect.get().is_some()
                || self.control.awaiting.get().is_some()
            {
                return Poll::Ready(Err(internal("native effect already registered").into()));
            }
            self.control.generation.set(next);
            *slot =
                Mailbox::InitializerRequest(next, wait.take().expect("initializer requested once"));
            self.control
                .effect
                .set(Some(NativeEffect::Initializer(next)));
            generation = Some(next);
            Poll::Pending
        })
        .await
    }

    /// Wait until this arena's pending and running finalizers have completed.
    pub async fn wait_finalizers(&mut self) -> NativeResult {
        let mut generation = None;
        std::future::poll_fn(|_| {
            if let Some(expected) = generation {
                let mut slot = self.control.mailbox.borrow_mut();
                return match std::mem::replace(&mut *slot, Mailbox::Empty) {
                    Mailbox::FinalizerReply(actual) if actual == expected => Poll::Ready(Ok(())),
                    other => {
                        *slot = other;
                        Poll::Ready(Err(internal("invalid finalizer wait reply").into()))
                    }
                };
            }
            let Some(next) = self.control.generation.get().checked_add(1) else {
                return Poll::Ready(Err(internal("native effect generation exhausted").into()));
            };
            let mut slot = self.control.mailbox.borrow_mut();
            if !matches!(*slot, Mailbox::Empty)
                || self.control.effect.get().is_some()
                || self.control.awaiting.get().is_some()
            {
                return Poll::Ready(Err(internal("native effect already registered").into()));
            }
            self.control.generation.set(next);
            *slot = Mailbox::FinalizerRequest(next);
            self.control
                .effect
                .set(Some(NativeEffect::Finalizers(next)));
            generation = Some(next);
            Poll::Pending
        })
        .await
    }
    /// Wait for monitor ownership. A deadline is absolute and remains in force across stops.
    pub async fn monitor_enter(
        &mut self,
        object: RootedValue,
        deadline: Option<std::time::Instant>,
    ) -> Result<bool, NativeFailure> {
        let mut object = Some(object);
        let mut generation = None;
        std::future::poll_fn(|_| {
            if let Some(expected) = generation {
                let mut slot = self.control.mailbox.borrow_mut();
                return match std::mem::replace(&mut *slot, Mailbox::Empty) {
                    Mailbox::MonitorReply(actual, acquired) if actual == expected => {
                        Poll::Ready(Ok(acquired))
                    }
                    other => {
                        *slot = other;
                        Poll::Ready(Err(internal("invalid monitor reply").into()))
                    }
                };
            }
            let Some(next) = self.control.generation.get().checked_add(1) else {
                return Poll::Ready(Err(internal("native effect generation exhausted").into()));
            };
            let mut slot = self.control.mailbox.borrow_mut();
            if !matches!(*slot, Mailbox::Empty)
                || self.control.effect.get().is_some()
                || self.control.awaiting.get().is_some()
            {
                return Poll::Ready(Err(internal("native effect already registered").into()));
            }
            self.control.generation.set(next);
            *slot = Mailbox::MonitorRequest(
                next,
                object.take().expect("monitor requested once"),
                deadline,
            );
            self.control.effect.set(Some(NativeEffect::Monitor(next)));
            generation = Some(next);
            Poll::Pending
        })
        .await
    }
    /// Explicitly return to the executor once.
    pub async fn yield_now(&mut self) -> Result<(), NativeFailure> {
        let mut first = true;
        std::future::poll_fn(|_| {
            if first {
                first = false;
                if self
                    .control
                    .effect
                    .replace(Some(NativeEffect::Yield))
                    .is_some()
                {
                    return Poll::Ready(Err(internal("native effect already registered").into()));
                }
                Poll::Pending
            } else {
                Poll::Ready(Ok(()))
            }
        })
        .await
    }
    /// Grant synchronous access only after honoring stops and the fairness quantum.
    pub async fn vm(&mut self) -> Result<VmAccess<'_>, NativeFailure> {
        loop {
            let stop = self
                .control
                .enter(|region| region.backend.stop_requested())?;
            if !stop && self.control.grants.get() != 0 {
                break;
            }
            self.yield_now().await?;
        }
        let remaining = self.control.grants.get();
        if remaining == 0 {
            return Err(internal("native access resumed without a fresh turn").into());
        }
        self.control.grants.set(remaining - 1);
        self.control.accesses.set(self.control.accesses.get() + 1);
        Ok(VmAccess { fx: self })
    }
}
/// Exclusive access to the VM; holds no heap borrow.
///
/// ```compile_fail
/// use dotnet_vm_ops::effects::{RootedValue, VmEffects};
/// async fn escape(mut fx: VmEffects, root: RootedValue) {
///     let mut vm = fx.vm().await.unwrap();
///     let value = vm.with(|region| region.fetch(&root).unwrap()).unwrap();
///     drop(value);
/// }
/// ```
///
/// The callback cannot return the region itself, or a future holding it:
///
/// ```compile_fail
/// use dotnet_vm_ops::effects::VmEffects;
/// async fn escape(mut fx: VmEffects) {
///     let mut vm = fx.vm().await.unwrap();
///     let region = vm.with(|region| region).unwrap();
///     drop(region);
/// }
/// ```
///
/// ```compile_fail
/// use dotnet_vm_ops::effects::{RootedValue, VmEffects};
/// async fn escape(mut fx: VmEffects, root: RootedValue) {
///     let mut vm = fx.vm().await.unwrap();
///     let future = vm.with(|region| async move { region.fetch(&root) }).unwrap();
///     drop(future);
/// }
/// ```
///
/// An access or its view cannot remain live while another effect borrows the capability:
///
/// ```compile_fail
/// use dotnet_vm_ops::effects::VmEffects;
/// async fn escape(mut fx: VmEffects) {
///     let vm = fx.vm().await.unwrap();
///     fx.yield_now().await.unwrap();
///     drop(vm);
/// }
/// ```
///
/// ```compile_fail
/// use dotnet_vm_ops::effects::{RootedValue, VmEffects};
/// async fn escape(mut fx: VmEffects, root: RootedValue) {
///     let mut vm = fx.vm().await.unwrap();
///     let view = vm.string(&root);
///     fx.yield_now().await.unwrap();
///     drop(view);
/// }
/// ```
///
/// A branded stack value cannot be captured in the static native future:
///
/// ```compile_fail
/// use dotnet_value::StackValue;
/// use dotnet_vm_ops::effects::NativeFuture;
/// fn escape<'gc>(value: StackValue<'gc>) -> NativeFuture {
///     Box::pin(async move { drop(value); Ok(()) })
/// }
/// ```
pub struct VmAccess<'fx> {
    fx: &'fx mut VmEffects,
}
impl Drop for VmAccess<'_> {
    fn drop(&mut self) {
        self.fx
            .control
            .accesses
            .set(self.fx.control.accesses.get() - 1);
    }
}
impl<'fx> VmAccess<'fx> {
    pub fn with<R>(
        &mut self,
        f: impl for<'gc> FnOnce(&mut NativeRegion<'gc, 'gc>) -> R,
    ) -> Result<R, VmError> {
        self.fx.control.enter(f)
    }
    pub fn prepare_multicast(&mut self) -> Result<(RootedValue, Vec<RootedValue>), NativeFailure> {
        Ok(self.with(|region| region.prepare_multicast())??)
    }
    pub fn prepare_reflection(
        &mut self,
    ) -> Result<(NativeCall, Option<RootedValue>), NativeFailure> {
        Ok(self.with(|region| region.prepare_reflection())??)
    }
    pub fn multicast_call(
        &mut self,
        receiver: &RootedValue,
        arguments: &[RootedValue],
        index: usize,
    ) -> Result<Option<NativeCall>, NativeFailure> {
        Ok(self.with(|region| region.multicast_call(receiver, arguments, index))??)
    }
    pub fn push_value(&mut self, value: NativeValue) -> NativeResult {
        Ok(self.with(|region| region.push_value(value))??)
    }
    pub fn prepare_monitor(
        &mut self,
        timed: bool,
        byref: bool,
    ) -> Result<(RootedValue, Option<RootedValue>, Option<i32>), NativeFailure> {
        Ok(self.with(|region| region.prepare_monitor(timed, byref))??)
    }
    /// # Safety
    /// Any in-range byte must be writable boolean storage, with synchronized access and no
    /// overlapping reference slot. The backend checks the complete range and live owner.
    pub unsafe fn write_monitor_flag(
        &mut self,
        pointer: &RootedValue,
        value: bool,
    ) -> NativeResult {
        Ok(self.with(|region| {
            // SAFETY: F10.RawMemoryAccessValid — this method's caller supplies the bool storage contract.
            unsafe { region.write_monitor_flag(pointer, value) }
        })??)
    }
    pub fn push_bool(&mut self, value: bool) -> NativeResult {
        Ok(self.with(|region| region.push_bool(value))??)
    }
    pub fn pop_object(&mut self) -> Result<RootedValue, NativeFailure> {
        Ok(self.with(|region| region.pop_object())??)
    }
    pub fn pop_string(&mut self) -> Result<RootedValue, NativeFailure> {
        Ok(self.with(|region| region.pop_string())??)
    }
    pub fn discard_scalars(&mut self, count: usize) -> NativeResult {
        Ok(self.with(|region| region.discard_scalars(count))??)
    }
    pub fn push_new_string(&mut self, chars: Vec<u16>) -> NativeResult {
        Ok(self.with(|region| region.backend.push_new_string(chars))??)
    }
    pub fn string<'view>(&'view mut self, source: &'view RootedValue) -> VmString<'view, 'fx> {
        VmString {
            access: self,
            source,
        }
    }
}
/// A bounded proxy, never a heap slice or lock.
pub struct VmString<'view, 'fx> {
    access: &'view mut VmAccess<'fx>,
    source: &'view RootedValue,
}
impl VmString<'_, '_> {
    pub fn copy_chunk(
        &mut self,
        start: usize,
        length: usize,
        output: &mut Vec<u16>,
    ) -> NativeResult {
        self.access.with(|region| {
            let StackValue::ObjectRef(ObjectRef(Some(handle))) = region.fetch(self.source)? else {
                return Err(internal("native string root invalid"));
            };
            let inner = handle.borrow();
            let HeapStorage::Str(chars) = &inner.storage else {
                return Err(internal("native string root invalid"));
            };
            let offset = output.len();
            if offset > length {
                return Err(internal("native string output exceeds range"));
            }
            let end = offset
                .checked_add((length.saturating_sub(offset)).min(64 * 1024))
                .and_then(|n| start.checked_add(n))
                .ok_or_else(|| internal("native string range overflow"))?;
            let from = start
                .checked_add(offset)
                .ok_or_else(|| internal("native string range overflow"))?;
            let chunk = chars
                .get(from..end)
                .ok_or_else(|| internal("native string range invalid"))?;
            output.extend_from_slice(chunk);
            Ok(())
        })??;
        Ok(())
    }
}

/// Poll a future with a temporary adapter and validate its effect discipline.
pub fn poll_native(
    control: &NativeControl,
    future: Pin<&mut dyn Future<Output = NativeResult>>,
    region: &mut NativeRegion<'_, '_>,
) -> Result<Poll<NativeResult>, VmError> {
    poll_native_impl(control, future, region, true)
}

/// Continue an inline effect without replenishing the turn's access grants.
pub fn poll_native_continuation(
    control: &NativeControl,
    future: Pin<&mut dyn Future<Output = NativeResult>>,
    region: &mut NativeRegion<'_, '_>,
) -> Result<Poll<NativeResult>, VmError> {
    poll_native_impl(control, future, region, false)
}

fn poll_native_impl(
    control: &NativeControl,
    future: Pin<&mut dyn Future<Output = NativeResult>>,
    region: &mut NativeRegion<'_, '_>,
    new_turn: bool,
) -> Result<Poll<NativeResult>, VmError> {
    if control.disabled.get()
        || !control.adapter.get().is_null()
        || control.effect.get().is_some()
        || control.accesses.get() != 0
        || control.entered.get()
        || region.backend.active_scope_depth() != 0
    {
        control.disabled.set(true);
        return Err(internal("invalid native poll entry"));
    }
    if new_turn {
        control.grants.set(VM_ACCESS_QUANTUM);
    }
    control.adapter.set(ptr::from_mut(region).cast());
    struct Reset<'a> {
        control: &'a NativeControl,
        completed: bool,
    }
    impl Drop for Reset<'_> {
        fn drop(&mut self) {
            self.control.adapter.set(ptr::null_mut());
            if !self.completed {
                self.control.disabled.set(true);
            }
        }
    }
    let mut reset = Reset {
        control,
        completed: false,
    };
    let poll = future.poll(&mut Context::from_waker(Waker::noop()));
    reset.completed = true;
    drop(reset);
    if control.disabled.get()
        || control.accesses.get() != 0
        || control.entered.get()
        || region.backend.active_scope_depth() != 0
        || poll.is_pending() != control.effect.get().is_some()
        || (poll.is_ready()
            && (control.awaiting.get().is_some()
                || !matches!(*control.mailbox.borrow(), Mailbox::Empty)))
    {
        control.disabled.set(true);
        return Err(internal("invalid native poll result"));
    }
    Ok(poll)
}

/// Read-only evaluation-stack access while choosing an intrinsic path.
pub struct NativeOperandView<'a, 'gc> {
    stack: &'a dyn crate::EvalStackOps<'gc>,
}
impl<'a, 'gc> NativeOperandView<'a, 'gc> {
    pub fn new(stack: &'a dyn crate::EvalStackOps<'gc>) -> Self {
        Self { stack }
    }

    pub fn peek_at(&self, offset: usize) -> StackValue<'gc> {
        self.stack.peek_stack_at(offset)
    }
}

/// Selector outcome before any operands are consumed.
pub enum NativeSelection<T> {
    Async(NativeInvocation),
    Synchronous(NativeSyncHandler<T>),
    Reject(VmError),
}
pub type NativeSyncHandler<T> = fn(
    &mut T,
    dotnet_types::members::MethodDescription,
    &dotnet_types::generics::GenericLookup,
) -> dotnet_vm_data::StepResult;
pub type NativeSelector<'gc, T> = fn(
    &T,
    NativeOperandView<'_, 'gc>,
    &dotnet_types::members::MethodDescription,
    &dotnet_types::generics::GenericLookup,
) -> NativeSelection<T>;

/// Connects a generated dispatch arm to its selector without constraining the body.
pub trait NativeSelectorProvider<'gc, T> {
    fn selector() -> Option<NativeSelector<'gc, T>>;
    fn fallback() -> NativeSyncHandler<T>;
}

/// How the resolved intrinsic is being invoked. Only inline calls can suspend.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NativeDispatchMode {
    InlineCall,
    Other,
}

/// Host entry hook. Implementations without a driver reject native calls before commitment.
pub trait NativeIntrinsicHost<'gc> {
    fn start_native(
        &mut self,
        _method: dotnet_types::members::MethodDescription,
        _mode: NativeDispatchMode,
        _generics: &dotnet_types::generics::GenericLookup,
        _selector: Option<NativeSelector<'gc, Self>>,
        _fallback: NativeSyncHandler<Self>,
        _entry: fn(VmEffects) -> NativeFuture,
    ) -> dotnet_vm_data::StepResult
    where
        Self: Sized,
    {
        dotnet_vm_data::StepResult::internal_error("native effects driver unavailable")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dotnet_utils::{ArenaId, gc::ThreadSafeLock};
    use dotnet_value::{CLRString, object::ObjectInner};
    use gc_arena::{Arena, GcWeak};

    struct Mock<'gc> {
        operands: Vec<StackValue<'gc>>,
        output: Vec<Vec<u16>>,
        stop: bool,
    }
    impl<'gc> NativeBackend<'gc> for Mock<'gc> {
        fn stop_requested(&self) -> bool {
            self.stop
        }
        fn active_scope_depth(&self) -> usize {
            0
        }
        fn operands(&mut self) -> &mut Vec<StackValue<'gc>> {
            &mut self.operands
        }
        fn push_new_string(&mut self, chars: Vec<u16>) -> Result<(), VmError> {
            self.output.push(chars);
            Ok(())
        }
    }
    fn invocation() -> NativeInvocation {
        NativeInvocation {
            start: 0,
            length: 0,
            parameter_count: 0,
        }
    }
    fn mock<'gc>() -> Mock<'gc> {
        Mock {
            operands: vec![],
            output: vec![],
            stop: false,
        }
    }
    fn arena() -> Arena<Rootable![DynamicRootSet<'_>]> {
        Arena::new(|mc| DynamicRootSet::new(mc))
    }

    #[test]
    fn condition_replies_require_matching_effect_and_generation() {
        let arena = arena();
        arena.mutate(|mc, roots| {
            let control = NativeControl::new();
            let mut fx = control.effects(invocation()).unwrap();
            let mut future = Box::pin(async move {
                fx.wait_finalizers().await?;
                fx.wait_finalizers().await
            });
            let mut backend = mock();
            let mut region = NativeRegion::new(mc, *roots, &mut backend);
            assert!(matches!(
                poll_native(&control, future.as_mut(), &mut region),
                Ok(Poll::Pending)
            ));
            assert_eq!(control.take_effect(), Some(NativeEffect::Finalizers(1)));
            assert!(control.take_finalizers(2).is_err());
            control.take_finalizers(1).unwrap();
            assert!(control.reply_finalizers(2).is_err());
            assert!(control.reply_monitor(1, true).is_err());
            control.reply_finalizers(1).unwrap();
            assert!(control.reply_finalizers(1).is_err());
            assert!(matches!(
                poll_native(&control, future.as_mut(), &mut region),
                Ok(Poll::Pending)
            ));
            assert_eq!(control.take_effect(), Some(NativeEffect::Finalizers(2)));
            control.take_finalizers(2).unwrap();
            assert!(control.reply_finalizers(1).is_err());
            control.reply_finalizers(2).unwrap();
            assert!(matches!(
                poll_native(&control, future.as_mut(), &mut region),
                Ok(Poll::Ready(Ok(())))
            ));
        });
    }

    #[test]
    fn initializer_replies_require_matching_effect_and_generation() {
        struct Wait(Rc<Cell<bool>>);
        impl Drop for Wait {
            fn drop(&mut self) {
                self.0.set(true);
            }
        }
        impl NativeInitWait for Wait {
            fn ready(&self) -> bool {
                false
            }
            fn sleeper(&self) -> Box<dyn NativeInitSleeper> {
                Box::new(Sleeper)
            }
        }
        struct Sleeper;
        impl NativeInitSleeper for Sleeper {
            fn wait(&self, _: std::time::Duration) {}
        }
        let arena = arena();
        arena.mutate(|mc, roots| {
            let control = NativeControl::new();
            let mut fx = control.effects(invocation()).unwrap();
            let dropped = Rc::new(Cell::new(false));
            let witness = dropped.clone();
            let mut future =
                Box::pin(async move { fx.wait_initializer(Box::new(Wait(witness))).await });
            let mut backend = mock();
            let mut region = NativeRegion::new(mc, *roots, &mut backend);
            assert!(matches!(
                poll_native(&control, future.as_mut(), &mut region),
                Ok(Poll::Pending)
            ));
            assert_eq!(control.take_effect(), Some(NativeEffect::Initializer(1)));
            assert!(control.take_initializer(2).is_err());
            let wait = control.take_initializer(1).unwrap();
            assert!(control.reply_finalizers(1).is_err());
            assert!(control.reply_initializer(2).is_err());
            assert!(!dropped.get());
            drop(wait);
            assert!(dropped.get());
            control.reply_initializer(1).unwrap();
            assert!(control.reply_initializer(1).is_err());
            assert!(matches!(
                poll_native(&control, future.as_mut(), &mut region),
                Ok(Poll::Ready(Ok(())))
            ));
        });
    }

    #[test]
    fn nested_entry_rejected() {
        let arena = arena();
        arena.mutate(|mc, roots| {
            let control = NativeControl::new();
            let nested = control.clone();
            let mut fx = control.effects(invocation()).unwrap();
            let mut future = Box::pin(async move {
                let mut vm = fx.vm().await?;
                vm.with(|_| assert!(nested.enter(|_| ()).is_err()))?;
                Ok(())
            });
            let mut backend = mock();
            let mut region = NativeRegion::new(mc, *roots, &mut backend);
            assert!(matches!(
                poll_native(&control, future.as_mut(), &mut region),
                Ok(Poll::Ready(Ok(())))
            ));
        });
    }

    #[test]
    fn panic_resets_entry_and_installation() {
        let arena = arena();
        arena.mutate(|mc, roots| {
            let control = NativeControl::new();
            let mut fx = control.effects(invocation()).unwrap();
            let mut future = Box::pin(async move {
                let mut vm = fx.vm().await?;
                vm.with(|_| panic!("poll panic"))?;
                Ok(())
            });
            let mut backend = mock();
            let mut region = NativeRegion::new(mc, *roots, &mut backend);
            assert!(
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| poll_native(
                    &control,
                    future.as_mut(),
                    &mut region
                )))
                .is_err()
            );
            assert!(control.adapter.get().is_null());
            assert!(control.disabled.get());
            assert!(!control.entered.get());
            assert_eq!(control.accesses.get(), 0);
            assert!(control.enter(|_| ()).is_err());
        });
    }

    #[test]
    fn grant_budget_yields_before_grant_33() {
        let arena = arena();
        arena.mutate(|mc, roots| {
            let control = NativeControl::new();
            let mut fx = control.effects(invocation()).unwrap();
            let count = Rc::new(Cell::new(0));
            let c = count.clone();
            let mut future = Box::pin(async move {
                for _ in 0..33 {
                    let _vm = fx.vm().await?;
                    c.set(c.get() + 1);
                }
                Ok(())
            });
            let mut backend = mock();
            let mut region = NativeRegion::new(mc, *roots, &mut backend);
            assert!(matches!(
                poll_native(&control, future.as_mut(), &mut region),
                Ok(Poll::Pending)
            ));
            assert_eq!(count.get(), 32);
            assert_eq!(control.take_effect(), Some(NativeEffect::Yield));
            assert!(matches!(
                poll_native(&control, future.as_mut(), &mut region),
                Ok(Poll::Ready(Ok(())))
            ));
            assert_eq!(count.get(), 33);
        });
    }

    #[test]
    fn stop_and_explicit_yield_return_to_executor() {
        let arena = arena();
        arena.mutate(|mc, roots| {
            let control = NativeControl::new();
            let mut fx = control.effects(invocation()).unwrap();
            let granted = Rc::new(Cell::new(false));
            let body_granted = granted.clone();
            let mut future = Box::pin(async move {
                drop(fx.vm().await?);
                body_granted.set(true);
                fx.yield_now().await?;
                Ok(())
            });
            let mut backend = mock();
            backend.stop = true;
            let mut region = NativeRegion::new(mc, *roots, &mut backend);
            assert!(matches!(
                poll_native(&control, future.as_mut(), &mut region),
                Ok(Poll::Pending)
            ));
            assert_eq!(control.take_effect(), Some(NativeEffect::Yield));
            assert!(!granted.get());
            // A stop can still be pending when the executor next polls the body.
            assert!(matches!(
                poll_native(&control, future.as_mut(), &mut region),
                Ok(Poll::Pending)
            ));
            assert_eq!(control.take_effect(), Some(NativeEffect::Yield));
            assert!(!granted.get());
            backend.stop = false;
            let mut region = NativeRegion::new(mc, *roots, &mut backend);
            assert!(matches!(
                poll_native(&control, future.as_mut(), &mut region),
                Ok(Poll::Pending)
            ));
            assert_eq!(control.take_effect(), Some(NativeEffect::Yield));
            assert!(granted.get());
            assert!(matches!(
                poll_native(&control, future.as_mut(), &mut region),
                Ok(Poll::Ready(Ok(())))
            ));
        });
    }

    #[test]
    fn pending_without_effect_is_rejected() {
        let arena = arena();
        arena.mutate(|mc, roots| {
            let control = NativeControl::new();
            let mut future = Box::pin(std::future::pending::<NativeResult>());
            let mut backend = mock();
            let mut region = NativeRegion::new(mc, *roots, &mut backend);
            assert!(poll_native(&control, future.as_mut(), &mut region).is_err());
            assert!(control.disabled.get());
        });
    }

    #[test]
    fn ready_with_registered_effect_is_rejected() {
        let arena = arena();
        arena.mutate(|mc, roots| {
            let control = NativeControl::new();
            let mut fx = control.effects(invocation()).unwrap();
            let mut future = Box::pin(async move {
                let mut request = Box::pin(fx.yield_now());
                std::future::poll_fn(|cx| {
                    assert!(request.as_mut().poll(cx).is_pending());
                    Poll::Ready(Ok(()))
                })
                .await
            });
            let mut backend = mock();
            let mut region = NativeRegion::new(mc, *roots, &mut backend);
            assert!(poll_native(&control, future.as_mut(), &mut region).is_err());
            assert!(control.disabled.get());
        });
    }

    #[test]
    fn forgotten_access_is_rejected() {
        let arena = arena();
        arena.mutate(|mc, roots| {
            let control = NativeControl::new();
            let mut fx = control.effects(invocation()).unwrap();
            let mut future = Box::pin(async move {
                std::mem::forget(fx.vm().await?);
                Ok(())
            });
            let mut backend = mock();
            let mut region = NativeRegion::new(mc, *roots, &mut backend);
            assert!(poll_native(&control, future.as_mut(), &mut region).is_err());
        });
    }

    #[test]
    fn substring_operands_and_root_survive_collection() {
        let mut arena = arena();
        let control = NativeControl::new();
        let mut fx = control.effects(invocation()).unwrap();
        let mut future = Box::pin(async move {
            let source = {
                let mut vm = fx.vm().await?;
                vm.discard_scalars(2)?;
                vm.pop_string()?
            };
            let mut output = Vec::new();
            {
                let mut vm = fx.vm().await?;
                vm.string(&source).copy_chunk(1, 65538, &mut output)?;
            }
            assert_eq!(output.len(), 65536);
            fx.yield_now().await?;
            let mut vm = fx.vm().await?;
            vm.string(&source).copy_chunk(1, 65538, &mut output)?;
            vm.push_new_string(output)?;
            Ok(())
        });
        arena.mutate(|mc, roots| {
            let source = Gc::new(
                mc,
                ThreadSafeLock::new(ObjectInner::new(
                    HeapStorage::Str(CLRString::new(vec![0x61; 65540])),
                    ArenaId::new(1),
                )),
            );
            let mut backend = mock();
            backend.operands = vec![
                StackValue::ObjectRef(ObjectRef(Some(source))),
                StackValue::Int32(1),
                StackValue::Int32(65538),
            ];
            let mut region = NativeRegion::new(mc, *roots, &mut backend);
            assert!(matches!(
                poll_native(&control, future.as_mut(), &mut region),
                Ok(Poll::Pending)
            ));
            assert!(backend.operands.is_empty());
        });
        assert_eq!(control.take_effect(), Some(NativeEffect::Yield));
        arena.finish_cycle();
        arena.mutate(|mc, roots| {
            let mut backend = mock();
            let mut region = NativeRegion::new(mc, *roots, &mut backend);
            assert!(matches!(
                poll_native(&control, future.as_mut(), &mut region),
                Ok(Poll::Ready(Ok(())))
            ));
            assert_eq!(backend.output, vec![vec![0x61; 65538]]);
        });
    }

    #[cfg(feature = "multithreading")]
    #[test]
    fn raw_gc_trace_bypasses_foreign_object_recording() {
        use dotnet_utils::{
            gc::{
                register_arena, set_currently_tracing, take_found_cross_arena_refs,
                unregister_arena,
            },
            sync::{Arc, AtomicBool},
        };
        use gc_arena::{GcWeak, collect::Trace};
        struct Counter(usize);
        impl<'gc> Trace<'gc> for Counter {
            fn trace_gc(&mut self, _: Gc<'gc, ()>) {
                self.0 += 1;
            }
            fn trace_gc_weak(&mut self, _: GcWeak<'gc, ()>) {}
        }
        let owner = ArenaId::new(789445);
        register_arena(owner, Arc::new(AtomicBool::new(true)));
        let arena = Arena::<Rootable![()]>::new(|_| ());
        arena.mutate(|mc, _| {
            let object = Gc::new(
                mc,
                ThreadSafeLock::new(ObjectInner::new(
                    HeapStorage::Str(CLRString::from("trace")),
                    owner,
                )),
            );
            let cell = RootCell {
                value: RootCellValue::Value(StackValue::ObjectRef(ObjectRef(Some(object)))),
            };
            let mut counter = Counter(0);
            // INVARIANT: F1.StwParked — This single-threaded tracing probe owns the object and
            // registration exclusively throughout both trace calls.
            set_currently_tracing(Some(ArenaId::new(789446)));
            object.trace(&mut counter);
            assert_eq!(counter.0, 1);
            assert!(take_found_cross_arena_refs().is_empty());
            cell.trace(&mut counter);
            assert_eq!(counter.0, 1);
            assert_eq!(take_found_cross_arena_refs().len(), 1);
            set_currently_tracing(None);
        });
        unregister_arena(owner);
    }

    #[derive(Collect)]
    #[collect(no_drop)]
    struct TestRoots<'gc> {
        roots: DynamicRootSet<'gc>,
        weak: GcWeak<'gc, ThreadSafeLock<ObjectInner<'gc>>>,
    }
    #[test]
    fn rooted_frame_home_descriptor_preserves_aliases_and_rejects_retired_home() {
        struct HomeBackend<'gc> {
            mock: Mock<'gc>,
            serial: FrameHomeSerial,
            block: Option<(std::ptr::NonNull<u8>, usize)>,
        }
        impl<'gc> NativeBackend<'gc> for HomeBackend<'gc> {
            fn stop_requested(&self) -> bool {
                false
            }
            fn active_scope_depth(&self) -> usize {
                0
            }
            fn operands(&mut self) -> &mut Vec<StackValue<'gc>> {
                &mut self.mock.operands
            }
            fn push_new_string(&mut self, _: Vec<u16>) -> Result<(), VmError> {
                unreachable!()
            }
            fn frame_home_block(
                &self,
                serial: FrameHomeSerial,
            ) -> Option<(std::ptr::NonNull<u8>, usize)> {
                (serial == self.serial).then_some(self.block).flatten()
            }
        }
        let arena = Arena::<Rootable![()]>::new(|_| ());
        arena.mutate(|mc, _| {
            let mut bytes = [0u8; 8];
            let serial = FrameHomeSerial::new(7);
            let base = std::ptr::NonNull::from(&mut bytes[0]);
            let address = std::ptr::NonNull::from(&mut bytes[2]);
            let pointer = ManagedPtr::new_frame_home(
                address,
                dotnet_types::TypeDescription::NULL,
                serial,
                ByteOffset::new(2),
                false,
            );
            let roots = DynamicRootSet::new(mc);
            let value =
                RootedValue::stash(mc, roots, StackValue::ManagedPtr(pointer.into())).unwrap();
            let mut backend = HomeBackend {
                mock: mock(),
                serial,
                block: Some((base, bytes.len())),
            };
            let first = value.fetch(roots, &backend).unwrap();
            let second = value.clone().fetch(roots, &backend).unwrap();
            let (StackValue::ManagedPtr(first), StackValue::ManagedPtr(second)) = (first, second)
            else {
                panic!("expected byrefs")
            };
            assert_eq!(first.origin(), &PointerOrigin::FrameHome(serial));
            assert_eq!(first.byte_offset(), ByteOffset::new(2));
            assert_eq!(first.observe_address(), second.observe_address());
            backend.block = Some((base, 1));
            assert!(value.fetch(roots, &backend).is_err());
            backend.block = None;
            assert!(value.fetch(roots, &backend).is_err());
            let excluded = ManagedPtr::new_static(
                Some(address),
                dotnet_types::TypeDescription::NULL,
                dotnet_types::TypeDescription::NULL,
                Default::default(),
                false,
                ByteOffset::new(2),
            );
            assert!(
                RootedValue::stash(mc, roots, StackValue::ManagedPtr(excluded.into())).is_err()
            );
            let unmanaged = ManagedPtr::new(
                Some(address),
                dotnet_types::TypeDescription::NULL,
                None,
                false,
                None,
            );
            assert!(
                RootedValue::stash(mc, roots, StackValue::ManagedPtr(unmanaged.into())).is_err()
            );
            let layout = dotnet_value::layout::FieldLayoutManager {
                fields: Default::default(),
                total_size: 0,
                alignment: 1,
                gc_desc: Default::default(),
                has_ref_fields: false,
            };
            let transient = dotnet_value::object::Object::new(
                dotnet_types::TypeDescription::NULL,
                Default::default(),
                dotnet_value::storage::FieldStorage::new(std::sync::Arc::new(layout), Vec::new()),
            );
            let transient = ManagedPtr::new_transient(
                Some(address),
                dotnet_types::TypeDescription::NULL,
                transient,
                ByteOffset::new(0),
            );
            assert!(
                RootedValue::stash(mc, roots, StackValue::ManagedPtr(transient.into())).is_err()
            );
        });
    }

    #[test]
    fn rooted_heap_pointer_descriptor_keeps_owner_alive() {
        let mut saved = None;
        let mut arena = Arena::<Rootable![TestRoots<'_>]>::new(|mc| {
            let roots = DynamicRootSet::new(mc);
            let obj = Gc::new(
                mc,
                ThreadSafeLock::new(ObjectInner::new(
                    HeapStorage::Str(CLRString::from("owner")),
                    ArenaId::new(1),
                )),
            );
            let pointer = ManagedPtr::new(
                None,
                dotnet_types::TypeDescription::NULL,
                Some(ObjectRef(Some(obj))),
                false,
                Some(ByteOffset::new(0)),
            );
            saved = Some(
                RootedValue::stash(mc, roots, StackValue::ManagedPtr(pointer.into())).unwrap(),
            );
            TestRoots {
                roots,
                weak: Gc::downgrade(obj),
            }
        });
        let value = saved.take().unwrap();
        arena.finish_cycle();
        arena.mutate(|mc, roots| {
            assert!(roots.weak.upgrade(mc).is_some());
            let StackValue::ManagedPtr(pointer) = value.fetch(roots.roots, &mock()).unwrap() else {
                panic!("expected byref")
            };
            assert!(matches!(
                pointer.origin(),
                PointerOrigin::Heap(ObjectRef(Some(_)))
            ));
            assert_eq!(pointer.byte_offset(), ByteOffset::new(0));
            assert!(value.fetch(DynamicRootSet::new(mc), &mock()).is_err());
        });
        drop(value);
        arena.finish_cycle();
        arena.mutate(|mc, roots| assert!(roots.weak.upgrade(mc).is_none()));
    }

    #[test]
    fn rooted_clone_retains_then_releases_and_mismatch_is_error() {
        let mut saved = None;
        let mut arena = Arena::<Rootable![TestRoots<'_>]>::new(|mc| {
            let roots = DynamicRootSet::new(mc);
            let obj = Gc::new(
                mc,
                ThreadSafeLock::new(ObjectInner::new(
                    HeapStorage::Str(CLRString::from("rooted")),
                    ArenaId::new(1),
                )),
            );
            saved = Some(
                RootedValue::stash(mc, roots, StackValue::ObjectRef(ObjectRef(Some(obj)))).unwrap(),
            );
            TestRoots {
                roots,
                weak: Gc::downgrade(obj),
            }
        });
        let original = saved.take().unwrap();
        let clone = original.clone();
        drop(original);
        arena.finish_cycle();
        arena.mutate(|mc, root| {
            assert!(root.weak.upgrade(mc).is_some());
            let backend = mock();
            assert!(matches!(
                clone.fetch(root.roots, &backend),
                Ok(StackValue::ObjectRef(ObjectRef(Some(_))))
            ));
            assert!(clone.fetch(DynamicRootSet::new(mc), &backend).is_err());
        });
        drop(clone);
        arena.finish_cycle();
        arena.mutate(|mc, root| assert!(root.weak.upgrade(mc).is_none()));
    }
}

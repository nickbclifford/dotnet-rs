use super::{
    VesContext,
    context::{InstructionCompletion, NativeActivation, NativeFutureOwner, NativeOutput},
    ops::*,
};
use crate::StepResult;
use crate::sync::SyncBlockOps;
use crate::threading::ThreadManagerOps;
use dotnet_intrinsics_threading::MonitorHost;
use dotnet_runtime_memory::ops::BaseMemoryOps;
use dotnet_runtime_resolver::resolution::TypeResolutionExt;
use dotnet_types::{
    TypeDescription, WellKnown, error::VmError, generics::GenericLookup, members::MethodDescription,
};
use dotnet_value::{
    CLRString, StackValue,
    object::{HeapStorage, ObjectRef},
};
use dotnet_vm_ops::SupportSlotOps;
use dotnet_vm_ops::effects::{
    NativeBackend, NativeCall, NativeControl, NativeDispatchMode, NativeEffect, NativeFailure,
    NativeFuture, NativeInitClaim, NativeInitProbe, NativeIntrinsicHost, NativeInvocation,
    NativeOperandView, NativePrepareError, NativeRegion, NativeSelection, NativeSelector,
    NativeSyncHandler, NativeValue, VmEffects, poll_native, poll_native_continuation,
};
use dotnetdll::prelude::{BaseType, MethodType, ParameterType};
use gc_arena::DynamicRootSet;
use std::task::Poll;

enum NativeEntry {
    Intrinsic(fn(VmEffects) -> NativeFuture),
    Instruction(StaticFieldAction),
    /// Initialize a type, then resume the interrupted dispatch or instruction.
    Initializer(TypeDescription, GenericLookup, InitializerResume),
}

/// How the interrupted caller continues once its type initializer has completed.
#[derive(Clone)]
pub(crate) enum InitializerResume {
    /// Re-execute the current instruction (or multicast step) with its operands intact.
    Retry,
    /// Dispatch the method again: initialization was triggered below the instruction, after
    /// delegate, reflection or expression dispatch had already rewritten the operand stack.
    Dispatch(
        MethodDescription,
        GenericLookup,
        super::call_ops_impl::ResolvedMethodDispatchMode,
    ),
}

fn reflection_prepare_error<'gc>(
    context: &VesContext<'_, 'gc>,
    step: StepResult,
) -> NativePrepareError<'gc> {
    match step {
        StepResult::Error(error) => NativePrepareError::Host(error),
        StepResult::Exception => match context.exception_mode() {
            dotnet_vm_data::ExceptionState::Throwing(exception, _) => {
                NativePrepareError::Managed(*exception)
            }
            _ => NativePrepareError::Host(
                dotnet_types::error::ExecutionError::InternalError(
                    "reflection validation threw without an exception".into(),
                )
                .into(),
            ),
        },
        _ => NativePrepareError::Host(
            dotnet_types::error::ExecutionError::InternalError(
                "unexpected reflection validation result".into(),
            )
            .into(),
        ),
    }
}

pub(crate) struct PendingCall {
    pub(super) generation: u64,
    pub(super) capture: bool,
    call: NativeCall,
    staged: bool,
    redirected: Option<(MethodDescription, GenericLookup)>,
}

#[cfg(test)]
#[derive(Clone, Copy)]
pub(crate) enum WaitTestPoint {
    Probed,
    Registered,
    #[cfg(feature = "multithreading")]
    FinalizersRegistered,
}

#[cfg(test)]
type WaitTestHook = Box<dyn FnMut(WaitTestPoint)>;

#[cfg(test)]
thread_local! {
    pub(crate) static WAIT_TEST_HOOK: std::cell::RefCell<Option<WaitTestHook>> = std::cell::RefCell::new(None);
    pub(crate) static MONITOR_FLAG_WRITES: std::cell::RefCell<Option<Vec<bool>>> =
        const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
pub(crate) fn wait_test_hook(point: WaitTestPoint) {
    WAIT_TEST_HOOK.with(|hook| {
        if let Some(callback) = hook.borrow_mut().as_mut() {
            callback(point);
        }
    });
}

pub(crate) enum PendingWait {
    Monitor(PendingMonitor),
    MonitorWait(PendingMonitorWait),
    Event(PendingEvent),
    Finalizers(u64),
    Sleep(u64, Option<std::time::Instant>),
    Initializer(u64, Box<dyn dotnet_vm_ops::effects::NativeInitWait>),
}

pub(crate) struct PendingEvent {
    pub(crate) wait: crate::threading::events::EventWait,
    pub(crate) epoch: u64,
    generation: u64,
}

pub(crate) struct PendingMonitor {
    pub(crate) block: crate::sync::Arc<crate::sync::SyncBlock>,
    pub(crate) epoch: Option<u64>,
    pub(crate) deadline: Option<std::time::Instant>,
    generation: u64,
    // INVARIANT: F1.GcHandleRooted — the activation's traced root set retains this object while its monitor is pending.
    _object: dotnet_vm_ops::effects::RootedValue,
}

pub(crate) struct PendingMonitorWait {
    pub(crate) wait: crate::sync::MonitorWait,
    pub(crate) epoch: u64,
    generation: u64,
    _object: dotnet_vm_ops::effects::RootedValue,
}

struct NativeInitializerClaim {
    manager: crate::sync::Arc<crate::statics::StaticStorageManager>,
    description: dotnet_types::TypeDescription,
    generics: GenericLookup,
    claim: crate::statics::InitializerClaim,
}

/// `Thread.Start` marks a thread started before it spawns the worker and publishes its
/// completion, so a concurrent `Join` can arrive between the two. Resolve the completion
/// again until it is published rather than capturing the absent value once. The key is an
/// inert registry key, as in the worker registry itself; it is never dereferenced.
struct ThreadCompletion {
    shared: std::sync::Arc<crate::state::SharedGlobalState>,
    thread: dotnet_value::object::ObjectPtr,
    completion: std::cell::OnceCell<crate::threading::workers::WorkerCompletion>,
}

impl ThreadCompletion {
    fn completion(&self) -> Option<&crate::threading::workers::WorkerCompletion> {
        if let Some(completion) = self.completion.get() {
            return Some(completion);
        }
        let completion = self.shared.workers.managed_completion(self.thread)?;
        Some(self.completion.get_or_init(|| completion))
    }
}

impl dotnet_vm_ops::effects::NativeThreadCompletion for ThreadCompletion {
    fn finished(&self) -> bool {
        self.completion()
            .is_some_and(|completion| completion.is_finished())
    }
    fn outcome(&self) -> Result<(), String> {
        match self
            .completion()
            .and_then(|completion| completion.take_outcome())
        {
            None | Some(Ok(crate::ExecutorResult::Exited(_))) => Ok(()),
            Some(Ok(crate::ExecutorResult::Threw(_))) => {
                unreachable!("worker exceptions are process fatal")
            }
            Some(Ok(crate::ExecutorResult::Error(error))) => Err(format!(
                "Managed ThreadStart failed: Internal VM Error: {error}"
            )),
            Some(Err(_)) => Err("Managed ThreadStart panicked.".into()),
        }
    }
    fn sleeper(&self) -> Box<dyn dotnet_vm_ops::effects::NativeInitSleeper> {
        Box::new(ThreadCompletionSleeper(self.completion().cloned()))
    }
}
struct ThreadCompletionSleeper(Option<crate::threading::workers::WorkerCompletion>);
impl dotnet_vm_ops::effects::NativeInitSleeper for ThreadCompletionSleeper {
    fn wait(&self, duration: std::time::Duration) {
        if let Some(completion) = &self.0 {
            completion.wait(duration);
        } else {
            // Not yet published: poll briefly so the next readiness check resolves it.
            std::thread::sleep(duration.min(std::time::Duration::from_millis(1)));
        }
    }
}

impl NativeInitClaim for NativeInitializerClaim {
    fn succeed(&mut self) {
        dotnet_vm_data::stack::InitializerClaim::succeed(&mut self.claim);
    }

    fn fail(&mut self, exception: ObjectRef<'_>) {
        self.manager.mark_failed_with_exception(
            self.description.clone(),
            &self.generics,
            exception,
        );
        dotnet_vm_data::stack::InitializerClaim::fail(&mut self.claim);
    }
}

#[cfg(feature = "multithreading")]
struct InitializerWait {
    _edge: crate::statics::StaticWaitRegistration,
    storage: crate::sync::Arc<crate::statics::StaticStorage>,
}

#[cfg(feature = "multithreading")]
impl dotnet_vm_ops::effects::NativeInitWait for InitializerWait {
    fn ready(&self) -> bool {
        !self.storage.init_state().is_initializing()
    }
    fn sleeper(&self) -> Box<dyn dotnet_vm_ops::effects::NativeInitSleeper> {
        Box::new(InitializerSleeper(self.storage.clone()))
    }
}

#[cfg(feature = "multithreading")]
struct InitializerSleeper(crate::sync::Arc<crate::statics::StaticStorage>);

#[cfg(feature = "multithreading")]
impl dotnet_vm_ops::effects::NativeInitSleeper for InitializerSleeper {
    fn wait(&self, duration: std::time::Duration) {
        let mut lock = self.0.init_mutex.lock();
        if self.0.init_state().is_initializing() {
            let _ = self.0.init_cond.wait_for(lock.raw_mut(), duration);
        }
    }
}

struct Adapter<'a, 'b, 'gc> {
    context: &'a mut VesContext<'b, 'gc>,
    operands: &'a mut Vec<StackValue<'gc>>,
    outputs: &'a mut Vec<StackValue<'gc>>,
}

fn event_method(
    loader: &dotnet_assemblies::AssemblyLoader,
    type_name: &str,
    method_name: &str,
    parameters: usize,
) -> Result<MethodDescription, VmError> {
    use dotnetdll::prelude::MethodMemberIndex;
    let ty = loader.corlib_type(type_name)?;
    let methods = &ty.definition().methods;
    let method = methods
        .iter()
        .enumerate()
        .find_map(|(index, method)| {
            if method.name != method_name {
                return None;
            }
            let description = MethodDescription::new(
                ty.clone(),
                GenericLookup::default(),
                ty.resolution.clone(),
                MethodMemberIndex::Method(index),
            );
            (description.signature().parameters.len() == parameters).then_some(description)
        })
        .or_else(|| {
            ty.definition()
                .properties
                .iter()
                .enumerate()
                .find_map(|(index, property)| {
                    let getter = property
                        .getter
                        .as_ref()
                        .filter(|method| method.name == method_name)
                        .map(|_| MethodMemberIndex::PropertyGetter(index));
                    let setter = property
                        .setter
                        .as_ref()
                        .filter(|method| method.name == method_name)
                        .map(|_| MethodMemberIndex::PropertySetter(index));
                    let description = MethodDescription::new(
                        ty.clone(),
                        GenericLookup::default(),
                        ty.resolution.clone(),
                        getter.or(setter)?,
                    );
                    (description.signature().parameters.len() == parameters).then_some(description)
                })
        });
    method.ok_or_else(|| {
        dotnet_types::error::ExecutionError::InternalError("CoreLib event method not found".into())
            .into()
    })
}

impl<'gc> NativeBackend<'gc> for Adapter<'_, '_, 'gc> {
    fn event_methods(&mut self) -> Result<dotnet_vm_ops::effects::NativeEventMethods, VmError> {
        let loader = &self.context.shared.loader;
        Ok(dotnet_vm_ops::effects::NativeEventMethods {
            constructor: event_method(
                loader,
                "Microsoft.Win32.SafeHandles.SafeWaitHandle",
                ".ctor",
                2,
            )?,
            setter: event_method(
                loader,
                "System.Threading.WaitHandle",
                "set_SafeWaitHandle",
                1,
            )?,
            getter: event_method(
                loader,
                "System.Threading.WaitHandle",
                "get_SafeWaitHandle",
                0,
            )?,
            is_closed: event_method(
                loader,
                "System.Runtime.InteropServices.SafeHandle",
                "get_IsClosed",
                0,
            )?,
            get_handle: event_method(
                loader,
                "System.Runtime.InteropServices.SafeHandle",
                "DangerousGetHandle",
                0,
            )?,
        })
    }
    fn event_create(
        &mut self,
        manual: bool,
        signaled: bool,
    ) -> Result<(ObjectRef<'gc>, isize), VmError> {
        let ty = self
            .context
            .shared
            .loader
            .corlib_type("Microsoft.Win32.SafeHandles.SafeWaitHandle")?;
        let object = self.context.new_object(ty)?;
        let object = self.context.alloc_obj_ref(self.context.gc, object);
        let id = self
            .context
            .shared
            .events
            .create(manual, signaled)
            .ok_or_else(|| {
                dotnet_types::error::ExecutionError::InternalError("event IDs exhausted".into())
            })?;
        Ok((object, id))
    }
    fn event_update(&mut self, id: isize, set: bool) -> Result<(), VmError> {
        let event = self.context.shared.events.get(id).ok_or_else(|| {
            dotnet_types::error::ExecutionError::InternalError("unsupported event handle".into())
        })?;
        if set {
            event.set();
        } else {
            event.reset();
        }
        Ok(())
    }

    fn stop_requested(&self) -> bool {
        self.context.shared.thread_manager.is_gc_stop_requested()
            || self.context.local.heap.needs_full_collect.get()
            || self.context.gc.needs_collection()
    }
    fn request_full_collection(&mut self) -> Result<(), VmError> {
        self.context.local.heap.needs_full_collect.set(true);
        Ok(())
    }
    fn thread_join(
        &mut self,
        thread: ObjectRef<'gc>,
    ) -> Result<dotnet_vm_ops::effects::NativeThreadJoin, VmError> {
        let Some(ptr) = thread.as_ptr() else {
            return Err(dotnet_types::error::ExecutionError::InternalError(
                "null thread receiver".into(),
            )
            .into());
        };
        let completion = self.context.shared.workers.managed_completion(ptr);
        let current = self.context.local.managed_threads.current() == Some(thread);
        #[cfg(feature = "multithreading")]
        let started = current
            || self
                .context
                .shared
                .workers
                .managed_thread(ptr)
                .is_some_and(|record| record.started);
        #[cfg(not(feature = "multithreading"))]
        let started = current;
        if completion.is_none() && !started {
            return Ok(dotnet_vm_ops::effects::NativeThreadJoin::Unstarted);
        }
        let resolved = std::cell::OnceCell::new();
        if let Some(completion) = completion {
            let _ = resolved.set(completion);
        }
        Ok(dotnet_vm_ops::effects::NativeThreadJoin::Waiting(Box::new(
            ThreadCompletion {
                shared: std::sync::Arc::clone(self.context.shared),
                thread: ptr,
                completion: resolved,
            },
        )))
    }
    fn active_scope_depth(&self) -> usize {
        self.context.local.active_borrows.get()
    }
    fn class_constructor_target(
        &mut self,
    ) -> Result<(dotnet_types::TypeDescription, GenericLookup), VmError> {
        let arg = self.operands.last().ok_or_else(|| {
            dotnet_types::error::ExecutionError::InternalError(
                "class constructor operand missing".into(),
            )
        })?;
        let StackValue::ValueType(handle) = arg else {
            return Err(dotnet_types::error::ExecutionError::TypeMismatch {
                expected: "RuntimeTypeHandle",
                actual: format!("{arg:?}").into(),
            }
            .into());
        };
        let target = self
            .context
            .loader()
            .rth_value_field(&handle.instance_storage, handle.description.clone())
            .expect("validated RuntimeTypeHandle support slot")
            .read();
        let ty = dotnet_intrinsics_reflection::common::resolve_runtime_type(self.context, target)?;
        let loader = self.context.loader();
        let description = loader.find_concrete_type(ty.to_concrete(loader.as_ref()))?;
        let generics = match ty {
            dotnet_types::runtime::RuntimeType::Generic(_, args) => GenericLookup::new(
                args.iter()
                    .map(|arg| arg.to_concrete(loader.as_ref()))
                    .collect(),
            ),
            _ => GenericLookup::default(),
        };
        self.operands.pop();
        Ok((description, generics))
    }
    fn probe_initializer(
        &mut self,
        description: dotnet_types::TypeDescription,
        generics: &GenericLookup,
    ) -> Result<NativeInitProbe<'gc>, VmError> {
        let arity = description.definition().generic_parameters.len();
        let generics =
            GenericLookup::new(generics.type_generics.iter().take(arity).cloned().collect());
        let manager = self.context.shared.statics.clone();
        let thread_id = self.context.thread_id.get();
        let result = manager.init(
            description.clone(),
            &self.context.with_generics(&generics),
            thread_id,
        )?;
        Ok(match result {
            crate::statics::StaticInitResult::Initialized
            | crate::statics::StaticInitResult::Recursive => NativeInitProbe::Complete,
            crate::statics::StaticInitResult::Failed => NativeInitProbe::Failed(
                manager.failed_exception(description, &generics, self.context.gc),
            ),
            crate::statics::StaticInitResult::Execute(method, claim) => {
                let claim = NativeInitializerClaim {
                    manager,
                    description,
                    generics: generics.clone(),
                    claim,
                };
                NativeInitProbe::Execute(
                    NativeCall {
                        method,
                        generics,
                        arguments: Vec::new(),
                    },
                    Box::new(claim),
                )
            }
            crate::statics::StaticInitResult::Waiting(registration) => {
                #[cfg(feature = "multithreading")]
                {
                    let storage = manager.get(description, &generics);
                    let edge = registration;
                    NativeInitProbe::Waiting(Box::new(InitializerWait {
                        _edge: edge,
                        storage,
                    }))
                }
                #[cfg(not(feature = "multithreading"))]
                {
                    drop(registration);
                    unreachable!(
                        "single-threaded static initialization cannot wait for another thread"
                    )
                }
            }
        })
    }
    fn wrap_initializer_failure(
        &mut self,
        description: &dotnet_types::TypeDescription,
        inner: Option<ObjectRef<'gc>>,
    ) -> Result<ObjectRef<'gc>, VmError> {
        let previous = *self.context.exception_mode;
        let result = self
            .context
            .throw_type_initialization_exception(description, inner);
        let current = *self.context.exception_mode;
        *self.context.exception_mode = previous;
        match (result, current) {
            (StepResult::Exception, dotnet_vm_data::ExceptionState::Throwing(exception, _)) => {
                Ok(exception)
            }
            (StepResult::Error(error), _) => Err(error),
            _ => Err(dotnet_types::error::ExecutionError::InternalError(
                "failed to create initializer exception".into(),
            )
            .into()),
        }
    }
    fn exception(
        &mut self,
        name: &str,
        message: &str,
        parameter: Option<&str>,
    ) -> Result<ObjectRef<'gc>, VmError> {
        let gc = self.context.gc;
        let exception_type = self.context.shared.loader.corlib_type(name)?;
        let instance = self.context.new_object(exception_type)?;
        let object = self.context.alloc_obj_ref(gc, instance);
        let base = self
            .context
            .shared
            .loader
            .corlib_wkt(WellKnown::Exception)?;
        if !message.is_empty() {
            let text = StackValue::string(gc, CLRString::from(message)).as_object_ref();
            self.context.register_new_object(&text);
            object.as_object_mut(gc, |obj| {
                if let Some(field) = obj.instance_storage.field::<ObjectRef>(base, "_message") {
                    field.write(text);
                }
            });
        }
        if let Some(parameter) = parameter {
            let argument_type = self
                .context
                .shared
                .loader
                .corlib_type("System.ArgumentException")?;
            let text = StackValue::string(gc, CLRString::from(parameter)).as_object_ref();
            self.context.register_new_object(&text);
            object.as_object_mut(gc, |obj| {
                if obj
                    .instance_storage
                    .has_field(argument_type.clone(), "_paramName")
                {
                    let mut field = obj
                        .instance_storage
                        .get_field_mut_local(argument_type, "_paramName");
                    text.write(&mut field);
                }
            });
        }
        Ok(object)
    }
    fn monitor_owned(&self, object: ObjectRef<'gc>) -> Result<bool, VmError> {
        Ok(self
            .context
            .monitor_get_sync_block_for_object(object)
            .is_some_and(|block| {
                self.context
                    .monitor_is_owned(&block, self.context.thread_id())
            }))
    }
    fn processing_finalizer(&self) -> Result<bool, VmError> {
        Ok(self.context.local.heap.processing_finalizer.get())
    }
    unsafe fn read_monitor_flag(&mut self, pointer: StackValue<'gc>) -> Result<bool, VmError> {
        use dotnet_vm_ops::ops::RawMemoryOps;
        let StackValue::ManagedPtr(pointer) = pointer else {
            return Err(dotnet_types::error::ExecutionError::InternalError(
                "invalid monitor flag pointer".into(),
            )
            .into());
        };
        let mut value = [0u8; 1];
        // SAFETY: F10.RawMemoryAccessValid — the caller supplies synchronized bool storage;
        // read_bytes resolves static storage afresh and checks the complete range for each admitted origin.
        unsafe {
            self.context
                .read_bytes(pointer.origin().clone(), pointer.byte_offset(), &mut value)?
        };
        Ok(value[0] != 0)
    }
    fn operands(&mut self) -> &mut Vec<StackValue<'gc>> {
        self.operands
    }
    fn push_new_string(&mut self, chars: Vec<u16>) -> Result<(), VmError> {
        let value = ObjectRef::new(self.context.gc, HeapStorage::Str(CLRString::new(chars)));
        self.context.register_new_object(&value);
        self.outputs.push(StackValue::ObjectRef(value));
        Ok(())
    }
    unsafe fn write_monitor_flag(
        &mut self,
        pointer: StackValue<'gc>,
        value: bool,
    ) -> Result<(), VmError> {
        use dotnet_vm_ops::ops::RawMemoryOps;
        let StackValue::ManagedPtr(pointer) = pointer else {
            return Err(dotnet_types::error::ExecutionError::InternalError(
                "invalid monitor flag pointer".into(),
            )
            .into());
        };
        // SAFETY: F10.RawMemoryAccessValid — the caller supplies synchronized bool storage;
        // write_bytes resolves static storage afresh and checks the complete range for each admitted origin.
        unsafe {
            self.context.write_bytes(
                pointer.origin().clone(),
                pointer.byte_offset(),
                &[u8::from(value)],
            )?
        };
        #[cfg(test)]
        MONITOR_FLAG_WRITES.with(|writes| {
            if let Some(writes) = writes.borrow_mut().as_mut() {
                writes.push(value);
            }
        });
        Ok(())
    }
    unsafe fn bulk_copy_chunk(
        &mut self,
        source: StackValue<'gc>,
        destination: StackValue<'gc>,
        total: usize,
        done: usize,
    ) -> Result<usize, VmError> {
        use dotnet_value::pointer::RetainedPtr;
        use dotnet_vm_ops::effects::BULK_CHUNK_SIZE;
        use dotnet_vm_ops::ops::RawMemoryOps;
        let source = RetainedPtr::try_from_stack_value(source)?;
        let destination = RetainedPtr::try_from_stack_value(destination)?;
        let src = self.context.retained_range(&source, total)?;
        let dst = self.context.retained_range(&destination, total)?;
        let chunk = BULK_CHUNK_SIZE.min(total.saturating_sub(done));
        if chunk == 0 || src == dst {
            return Ok(chunk);
        }
        let overlap = src.addr() < dst.addr().saturating_add(total)
            && dst.addr() < src.addr().saturating_add(total);
        // A forward-overlapping move must run from the end, so `done` counts from the tail.
        let backward = overlap && dst.addr() > src.addr();
        let start = if backward { total - done - chunk } else { done };
        #[expect(
            clippy::multiple_unsafe_ops_per_block,
            reason = "offsetting and moving the same chunk share one range proof"
        )]
        // SAFETY: F10.RawMemoryAccessValid — both full ranges were just resolved and checked;
        // `start + chunk <= total`. Overlap is handled by `ptr::copy`, and chunk order
        // preserves memmove semantics across suspensions because the direction is a pure
        // function of the unchanged addresses.
        unsafe {
            if overlap {
                std::ptr::copy(src.add(start), dst.add(start), chunk);
            } else {
                dotnet_simd::copy_nonoverlapping_raw(dst.add(start), src.add(start), chunk);
            }
        }
        Ok(chunk)
    }
    unsafe fn bulk_fill_chunk(
        &mut self,
        destination: StackValue<'gc>,
        total: usize,
        done: usize,
        value: u8,
    ) -> Result<usize, VmError> {
        use dotnet_value::pointer::RetainedPtr;
        use dotnet_vm_ops::effects::BULK_CHUNK_SIZE;
        use dotnet_vm_ops::ops::RawMemoryOps;
        let destination = RetainedPtr::try_from_stack_value(destination)?;
        let dst = self.context.retained_range(&destination, total)?;
        let chunk = BULK_CHUNK_SIZE.min(total.saturating_sub(done));
        #[expect(
            clippy::multiple_unsafe_ops_per_block,
            reason = "offsetting and filling the same chunk share one range proof"
        )]
        // SAFETY: F10.RawMemoryAccessValid — the full range was just resolved and checked;
        // `done + chunk <= total`.
        unsafe {
            dotnet_simd::fill_raw(dst.add(done), chunk, value)
        };
        Ok(chunk)
    }
    fn frame_home_block(
        &self,
        serial: dotnet_value::pointer::FrameHomeSerial,
    ) -> Option<(std::ptr::NonNull<u8>, usize)> {
        self.context.frame_stack.home_block(serial).or_else(|| {
            self.context
                .parked_finalizer_caller
                .as_ref()?
                .frame_stack
                .home_block(serial)
        })
    }
    fn multicast_target(
        &mut self,
        receiver: StackValue<'gc>,
        index: usize,
    ) -> Result<Option<dotnet_vm_ops::effects::NativeTarget<'gc>>, VmError> {
        let StackValue::ObjectRef(delegate @ ObjectRef(Some(_))) = receiver else {
            return Err(dotnet_types::error::ExecutionError::InternalError(
                "invalid multicast receiver".into(),
            )
            .into());
        };
        let targets =
            dotnet_intrinsics_delegates::helpers::get_multicast_targets_ref(self.context, delegate)
                .ok_or_else(|| {
                    dotnet_types::error::ExecutionError::InternalError(
                        "missing multicast list".into(),
                    )
                })?;
        let target = targets.as_vector(|vector| {
            vector
                .object_ref_elements(self.context.gc.mutation())
                .nth(index)
        });
        let Some(target) = target else {
            return Ok(None);
        };
        let (receiver, index) =
            dotnet_intrinsics_delegates::helpers::get_delegate_info(self.context, target);
        let (method, generics) =
            dotnet_intrinsics_delegates::DelegateInvokeHost::delegate_lookup_method_by_index(
                self.context,
                index,
            );
        let receiver = (method.signature().instance || receiver.0.is_some())
            .then_some(StackValue::ObjectRef(receiver));
        Ok(Some(dotnet_vm_ops::effects::NativeTarget {
            method,
            generics,
            receiver,
        }))
    }
    fn reflection_call(
        &mut self,
    ) -> Result<dotnet_vm_ops::effects::NativeReflectionPlan<'gc>, NativePrepareError<'gc>> {
        let (mut method, mut generics, receiver, parameters) = match self.operands.as_slice() {
            [
                StackValue::ObjectRef(target),
                StackValue::ObjectRef(parameters),
            ] => {
                let (target, parameters) = (*target, *parameters);
                match dotnet_intrinsics_reflection::types::activator_target(
                    self.context,
                    target,
                    parameters,
                )
                .map_err(|step| reflection_prepare_error(self.context, step))?
                {
                    dotnet_intrinsics_reflection::types::ActivatorTarget::Constructor(
                        method,
                        generics,
                    ) => (method, generics, None, parameters),
                    dotnet_intrinsics_reflection::types::ActivatorTarget::Default(value) => {
                        return Ok(dotnet_vm_ops::effects::NativeReflectionPlan::Value(
                            StackValue::ObjectRef(value),
                        ));
                    }
                }
            }
            [
                StackValue::ObjectRef(obj),
                receiver,
                _,
                _,
                StackValue::ObjectRef(parameters),
                _,
            ] => {
                let (method, generics) =
                    dotnet_intrinsics_reflection::common::resolve_runtime_method(
                        self.context,
                        *obj,
                    )?;
                (method, generics, Some(receiver.clone()), *parameters)
            }
            [
                StackValue::ObjectRef(obj),
                _,
                _,
                StackValue::ObjectRef(parameters),
                _,
            ] => {
                let (method, generics) =
                    dotnet_intrinsics_reflection::common::resolve_runtime_method(
                        self.context,
                        *obj,
                    )?;
                (method, generics, None, *parameters)
            }
            _ => {
                return Err(dotnet_types::error::ExecutionError::InternalError(
                    "invalid reflection operands".into(),
                )
                .into());
            }
        };
        let is_constructor = method.method().name == ".ctor";
        if let Some(receiver) = &receiver {
            dotnet_intrinsics_reflection::methods::validate_invoke_target(
                self.context,
                &method,
                &generics,
                receiver,
            )
            .map_err(|step| reflection_prepare_error(self.context, step))?;
        }
        dotnet_intrinsics_reflection::methods::validate_invoke_return(
            self.context,
            &method,
            &generics,
        )
        .map_err(|step| reflection_prepare_error(self.context, step))?;
        let mut constructor_result = None;
        let receiver = if is_constructor && receiver.is_none() {
            let instance = dotnet_intrinsics_reflection::ResolutionContextHost::reflection_new_object_with_lookup(self.context, method.parent.clone(), &generics)?;
            let is_value_type = method
                .parent
                .is_value_type(&self.context.current_context())?;
            let storage = if is_value_type {
                HeapStorage::Boxed(Box::new(instance))
            } else {
                HeapStorage::Obj(Box::new(instance))
            };
            let owner = ObjectRef::new(self.context.gc, storage);
            self.context.register_new_object(&owner);
            constructor_result = Some(StackValue::ObjectRef(owner));
            if is_value_type {
                // INVARIANT: F6.NativePointerDescriptor — this address is used only to form
                // an owner-backed descriptor; dispatch reconstructs it from the rooted box.
                let data = owner.as_heap_storage(|storage| match storage {
                    HeapStorage::Boxed(value) => value
                        .instance_storage
                        .with_data(|data| data.as_ptr() as *mut u8),
                    _ => unreachable!("new value constructor owner is boxed"),
                });
                Some(StackValue::managed_ptr_with_owner(
                    data,
                    method.parent.clone(),
                    Some(owner),
                    false,
                    None,
                ))
            } else {
                Some(StackValue::ObjectRef(owner))
            }
        } else {
            if is_constructor {
                constructor_result = receiver.clone();
            }
            receiver
        };
        if method.signature().instance
            && method.method().virtual_member
            && let Some(StackValue::ObjectRef(this_obj)) = &receiver
            && this_obj.0.is_some()
        {
            let this_type = self.context.get_heap_description(*this_obj)?;
            method = self.context.resolver().resolve_virtual_method(
                method,
                this_type,
                &generics,
                &self.context.current_context(),
            )?;
            let arity = method.parent.definition().generic_parameters.len();
            if arity == 0 {
                generics.set_type_generics(Vec::new().into());
            } else if method.parent_generics.type_generics.len() == arity {
                generics.set_type_generics(method.parent_generics.type_generics.clone());
            } else {
                generics.set_type_generics(
                    generics
                        .type_generics
                        .iter()
                        .take(arity)
                        .cloned()
                        .collect::<Vec<_>>()
                        .into(),
                );
            }
        }
        let gc = self.context.gc;
        let mut arguments = dotnet_intrinsics_reflection::methods::unmarshal_invoke_params(
            self.context,
            &gc,
            &method,
            &generics,
            parameters,
        )
        .map_err(|step| reflection_prepare_error(self.context, step))?;
        if method.signature().instance {
            arguments.insert(
                0,
                receiver.ok_or_else(|| {
                    dotnet_types::error::ExecutionError::InternalError(
                        "missing reflection receiver".into(),
                    )
                })?,
            );
        }
        Ok(dotnet_vm_ops::effects::NativeReflectionPlan::Call(
            dotnet_vm_ops::effects::NativePreparedCall {
                method,
                generics,
                arguments,
                constructor_result,
            },
        ))
    }
    fn reflection_result(
        &mut self,
        method: &MethodDescription,
        generics: &GenericLookup,
        value: Option<StackValue<'gc>>,
    ) -> Result<StackValue<'gc>, VmError> {
        let Some(ParameterType::Value(ty)) = &method.signature().return_type.1 else {
            return Ok(StackValue::null());
        };
        let concrete = generics.make_concrete(
            method.resolution(),
            ty.clone(),
            self.context.loader().as_ref(),
        )?;
        let description = self.context.loader().find_concrete_type(concrete.clone())?;
        let value = value.ok_or_else(|| {
            dotnet_types::error::ExecutionError::InternalError(
                "missing reflected return value".into(),
            )
        })?;
        if description.is_value_type(&self.context.current_context())? {
            let boxed = self.context.box_value(&concrete, value)?;
            Ok(StackValue::ObjectRef(boxed))
        } else {
            Ok(value)
        }
    }
    fn reflection_constructor_result(
        &mut self,
        method: &MethodDescription,
        generics: &GenericLookup,
        value: StackValue<'gc>,
    ) -> Result<StackValue<'gc>, VmError> {
        if method.parent.type_name() != "System.Nullable`1" {
            return Ok(value);
        }
        let runtime_args = generics
            .type_generics
            .iter()
            .map(|arg| {
                dotnet_types::runtime::runtime_type_from_concrete(
                    self.context.loader().as_ref(),
                    arg,
                )
                .ok_or(dotnet_types::error::TypeResolutionError::InvalidHandle)
            })
            .collect::<Result<Vec<_>, _>>()?;
        let concrete =
            dotnet_types::runtime::RuntimeType::Generic(method.parent.clone(), runtime_args)
                .to_concrete(self.context.loader().as_ref());
        let StackValue::ObjectRef(owner @ ObjectRef(Some(_))) = value else {
            return Err(dotnet_types::error::ExecutionError::InternalError(
                "invalid constructed value".into(),
            )
            .into());
        };
        let instance = owner
            .as_heap_storage(|storage| match storage {
                HeapStorage::Boxed(instance) => Some((**instance).clone()),
                _ => None,
            })
            .ok_or_else(|| {
                dotnet_types::error::ExecutionError::InternalError("invalid constructed box".into())
            })?;
        let boxed = self
            .context
            .box_value(&concrete, StackValue::ValueType(instance))?;
        Ok(StackValue::ObjectRef(boxed))
    }
    fn push_output(&mut self, value: StackValue<'gc>) -> Result<(), VmError> {
        self.outputs.push(value);
        Ok(())
    }
}

fn multicast_value_shape(
    ctx: &VesContext<'_, '_>,
    method: &MethodDescription,
    generics: &GenericLookup,
    ty: &MethodType,
) -> NativeOutput {
    let Ok(concrete) =
        generics.make_concrete(method.resolution(), ty.clone(), ctx.loader().as_ref())
    else {
        return NativeOutput::Unsupported;
    };
    if matches!(concrete.get(), BaseType::Type { .. })
        && let Ok(description) = ctx.loader().find_concrete_type(concrete.clone())
        && let Some(underlying) = description.is_enum()
    {
        let underlying: MethodType = underlying.clone().into();
        return multicast_value_shape(ctx, method, generics, &underlying);
    }
    let Ok(concrete) = ctx.current_context().normalize_type(concrete) else {
        return NativeOutput::Unsupported;
    };
    match concrete.get() {
        BaseType::Boolean
        | BaseType::Char
        | BaseType::Int8
        | BaseType::UInt8
        | BaseType::Int16
        | BaseType::UInt16
        | BaseType::Int32
        | BaseType::UInt32 => NativeOutput::Int32,
        BaseType::Int64 | BaseType::UInt64 => NativeOutput::Int64,
        BaseType::IntPtr | BaseType::UIntPtr => NativeOutput::NativeInt,
        BaseType::Float32 | BaseType::Float64 => NativeOutput::NativeFloat,
        _ if concrete.is_class(ctx.loader().as_ref()) => NativeOutput::Object,
        BaseType::Type { .. }
            if ctx
                .current_context()
                .resolver()
                .type_layout_cached_with_lookup(concrete.clone(), method.resolution(), generics)
                .is_ok_and(|layout| !layout.has_managed_ptrs()) =>
        {
            NativeOutput::ValueType
        }
        _ => NativeOutput::Unsupported,
    }
}

fn multicast_output(
    ctx: &VesContext<'_, '_>,
    method: &MethodDescription,
    generics: &GenericLookup,
) -> NativeOutput {
    match &method.signature().return_type.1 {
        None => NativeOutput::Void,
        Some(ParameterType::Value(ty)) => multicast_value_shape(ctx, method, generics, ty),
        _ => NativeOutput::Unsupported,
    }
}

fn multicast_argument_supported(shape: NativeOutput, value: &StackValue<'_>) -> bool {
    matches!(
        (shape, value),
        (NativeOutput::Int32, StackValue::Int32(_))
            | (NativeOutput::Int64, StackValue::Int64(_))
            | (NativeOutput::NativeInt, StackValue::NativeInt(_))
            | (NativeOutput::NativeFloat, StackValue::NativeFloat(_))
            | (NativeOutput::Object, StackValue::ObjectRef(_))
            | (NativeOutput::ValueType, StackValue::ValueType(_))
    )
}

async fn multicast_body(mut fx: VmEffects) -> dotnet_vm_ops::effects::NativeResult {
    let (receiver, arguments) = {
        let mut vm = fx.vm().await?;
        vm.prepare_multicast()?
    };
    let mut last = NativeValue::Void;
    let mut index = 0;
    loop {
        let call = {
            let mut vm = fx.vm().await?;
            vm.multicast_call(&receiver, &arguments, index)?
        };
        let Some(call) = call else {
            break;
        };
        last = fx.call(call).await?;
        index += 1;
    }
    let mut vm = fx.vm().await?;
    vm.push_value(last)
}

fn multicast_entry(fx: VmEffects) -> NativeFuture {
    Box::pin(multicast_body(fx))
}

impl NativeActivation<'_> {
    pub(crate) fn cleanup(mut self) {
        drop(self.wait.take());
        drop(self);
    }
}

impl<'gc> NativeIntrinsicHost<'gc> for VesContext<'_, 'gc> {
    fn start_native(
        &mut self,
        method: MethodDescription,
        mode: NativeDispatchMode,
        generics: &GenericLookup,
        selector: Option<NativeSelector<'gc, Self>>,
        fallback: Option<NativeSyncHandler<Self>>,
        entry: fn(VmEffects) -> NativeFuture,
    ) -> StepResult {
        if mode == NativeDispatchMode::Other {
            return match fallback {
                Some(handler) => handler(self, method, generics),
                None => dotnet_vm_data::StepResult::not_implemented(format!(
                    "async intrinsic {method:?} reached through non-inline dispatch"
                )),
            };
        }
        self.start_native_inner(
            method,
            generics,
            selector,
            fallback,
            NativeEntry::Intrinsic(entry),
            mode,
        )
    }
}

impl<'gc> VesContext<'_, 'gc> {
    pub(crate) fn start_static_field_effect(&mut self, action: StaticFieldAction) -> StepResult {
        let state = self
            .shared
            .statics
            .get_init_state(action.field.parent.clone(), &action.generics);
        // The acquire read observes published bytes; only the initializing owner may
        // access incomplete storage without going through the wait/cycle protocol.
        if state == crate::statics::InitState::Initialized
            || state == crate::statics::InitState::Initializing(self.thread_id.get().as_u64())
        {
            return crate::instructions::objects::fields::complete_static_field(self, action);
        }
        let method = self.state().info_handle.source.clone();
        let generics = action.generics.clone();
        self.start_native_inner(
            method,
            &generics,
            None,
            None,
            NativeEntry::Instruction(action),
            NativeDispatchMode::InlineCall,
        )
    }

    /// Ensure `description` is initialized before the caller proceeds.
    ///
    /// Returns `Continue` when initialization has completed (or is recursive on this thread).
    /// Otherwise starts an `ensure_initialized` activation: the initializer runs under a
    /// capturing boundary, so outer filters see the `TypeInitializationException`, never the
    /// original exception, matching the CLR. On success the caller resumes as `resume` says.
    pub(crate) fn initialize_type_then(
        &mut self,
        description: TypeDescription,
        generics: &GenericLookup,
        resume: InitializerResume,
    ) -> StepResult {
        // Static storage is keyed per type instantiation: trim call-site generics to the
        // declaring type's arity so keys stay stable.
        let arity = description.definition().generic_parameters.len();
        let generics =
            GenericLookup::new(generics.type_generics.iter().take(arity).cloned().collect());
        let state = self
            .shared
            .statics
            .get_init_state(description.clone(), &generics);
        if state == crate::statics::InitState::Initialized
            || state == crate::statics::InitState::Initializing(self.thread_id.get().as_u64())
        {
            return StepResult::Continue;
        }
        let depth = self.frame_stack.len();
        let resume = if self.frame_stack.current_frame().multicast_state.is_some() {
            // A multicast step rebuilds its operands from retained arguments on retry.
            InitializerResume::Retry
        } else {
            resume
        };
        if let InitializerResume::Dispatch(method, lookup, _) = &resume
            && self.has_staged_native_call(depth)
        {
            self.retain_nested_native_target(depth, method, lookup);
        }
        let method = self.state().info_handle.source.clone();
        self.start_native_inner(
            method,
            &generics.clone(),
            None,
            None,
            NativeEntry::Initializer(description, generics, resume),
            NativeDispatchMode::InlineCall,
        )
    }

    fn start_native_inner(
        &mut self,
        method: MethodDescription,
        generics: &GenericLookup,
        selector: Option<NativeSelector<'gc, Self>>,
        fallback: Option<NativeSyncHandler<Self>>,
        entry: NativeEntry,
        mode: NativeDispatchMode,
    ) -> StepResult {
        let signature = method.signature();
        let count = match &entry {
            NativeEntry::Instruction(action) => {
                usize::from(matches!(action.kind, StaticFieldKind::Store { .. }))
            }
            NativeEntry::Initializer(..) => 0,
            NativeEntry::Intrinsic(_) => {
                signature.parameters.len()
                    + usize::from(
                        signature.instance
                            && !matches!(
                                mode,
                                NativeDispatchMode::Constructor(
                                    dotnet_vm_ops::effects::NativeConstructorKind::String
                                        | dotnet_vm_ops::effects::NativeConstructorKind::Object
                                )
                            ),
                    )
            }
        };
        if self.frame_stack.is_empty()
            || self.frame_stack.current_frame().stack_height.as_usize() < count
        {
            return StepResult::internal_error("native call operand underflow");
        }
        let parent_destination = match self.call_state.native.last() {
            Some(parent) if parent.caller_depth == self.frame_stack.len() => {
                let Some(pending) = parent.pending.as_ref().filter(|pending| pending.staged) else {
                    return StepResult::internal_error("native activation already live at caller");
                };
                Some(dotnet_vm_data::NativeReturnDestination {
                    id: parent.id,
                    generation: pending.generation,
                })
            }
            Some(parent) if parent.caller_depth > self.frame_stack.len() => {
                return StepResult::internal_error("native caller changed");
            }
            _ => None,
        };
        let invocation = match selector {
            Some(selector) => match selector(self, NativeOperandView::new(self), &method, generics)
            {
                NativeSelection::Async(invocation) => invocation,
                NativeSelection::Fallback => {
                    return match fallback {
                        Some(handler) => handler(self, method, generics),
                        None => StepResult::internal_error(
                            "native selector requested a fallback but none is registered",
                        ),
                    };
                }
                NativeSelection::Reject(error) => return StepResult::Error(error),
                NativeSelection::InvalidProgram => {
                    return self.throw_by_name_with_message(
                        "System.InvalidProgramException",
                        "Common Language Runtime detected an invalid program.",
                    );
                }
            },
            None => NativeInvocation {
                start: 0,
                length: 0,
                parameter_count: signature.parameters.len(),
            },
        };
        let output = if mode
            == NativeDispatchMode::Constructor(
                dotnet_vm_ops::effects::NativeConstructorKind::String,
            ) {
            NativeOutput::String
        } else if mode
            == NativeDispatchMode::Constructor(
                dotnet_vm_ops::effects::NativeConstructorKind::Object,
            )
        {
            NativeOutput::Object
        } else if matches!(
            entry,
            NativeEntry::Instruction(_) | NativeEntry::Initializer(..)
        ) {
            NativeOutput::Void
        } else {
            let shape = multicast_output(self, &method, generics);
            // Delegate transport accepts null; ordinary string intrinsics retain their stricter check.
            if matches!(shape, NativeOutput::Object)
                && matches!(
                    &signature.return_type.1,
                    Some(ParameterType::Value(MethodType::Base(base))) if matches!(base.as_ref(), BaseType::String)
                )
                && !dotnet_intrinsics_delegates::helpers::is_delegate_type(
                    self,
                    method.parent.clone(),
                )
            {
                NativeOutput::String
            } else {
                shape
            }
        };
        if matches!(output, NativeOutput::Unsupported) {
            return StepResult::internal_error("unsupported native return shape");
        }
        // Raw addresses can occur as byref operands in unsafe IL. Resolve their declared
        // target types before committing operands, then canonicalize them for native calls.
        // `count` includes an instance receiver below the parameters; parameter `index` is at
        // depth `parameters - index - 1` and at operand `index + receiver`.
        let mut raw_byrefs = Vec::new();
        for (index, parameter) in signature.parameters.iter().enumerate() {
            if matches!(entry, NativeEntry::Intrinsic(_))
                && let ParameterType::Ref(ty) = &parameter.1
                && matches!(
                    self.peek_stack_at(signature.parameters.len() - index - 1),
                    StackValue::NativeInt(_) | StackValue::UnmanagedPtr(_)
                )
            {
                let concrete = dotnet_vm_ops::vm_try!(generics.make_concrete(
                    method.resolution(),
                    ty.clone(),
                    self.loader().as_ref()
                ));
                let ty = dotnet_vm_ops::vm_try!(self.loader().find_concrete_type(concrete));
                raw_byrefs.push((index + count - signature.parameters.len(), ty));
            }
        }
        let control = NativeControl::new();
        let fx = match control.effects(invocation) {
            Ok(fx) => fx,
            Err(error) => return StepResult::Error(error),
        };
        let Some(next_id) = self.call_state.last_native_id.checked_add(1) else {
            return StepResult::internal_error("native activation identity exhausted");
        };
        let roots = DynamicRootSet::new(self.gc.mutation());
        let mut operands = self.pop_multiple(count);
        for (index, ty) in raw_byrefs {
            let address = match operands[index] {
                StackValue::NativeInt(value) => {
                    std::ptr::NonNull::new(std::ptr::with_exposed_provenance_mut(value as usize))
                }
                StackValue::UnmanagedPtr(pointer) => Some(pointer.0),
                _ => unreachable!(),
            };
            operands[index] = StackValue::ManagedPtr(
                dotnet_value::pointer::ManagedPtr::new(address, ty, None, false, None).into(),
            );
        }
        if matches!(&entry, NativeEntry::Intrinsic(_)) {
            for value in &mut operands {
                if let StackValue::ManagedPtr(pointer) = value
                    && matches!(
                        pointer.origin(),
                        dotnet_value::pointer::PointerOrigin::Static(_)
                    )
                {
                    // INVARIANT: F6.NativePointerDescriptor — an access grant can wait for a
                    // GC stop before the body roots its operands. Keep static metadata and
                    // offset, not the observed address, in the activation during that wait.
                    *pointer = dotnet_value::pointer::ManagedPtr::from_info_full(
                        dotnet_value::pointer::ManagedPtrInfo {
                            address: None,
                            origin: pointer.origin().clone(),
                            offset: pointer.byte_offset(),
                        },
                        pointer.inner_type(),
                        pointer.is_pinned(),
                    )
                    .into();
                }
            }
        }
        let (future, instruction, resume) = match entry {
            NativeEntry::Initializer(description, generics, resume) => {
                let future: NativeFuture = Box::pin(async move {
                    let mut fx = fx;
                    fx.ensure_initialized(description, generics).await
                });
                (future, None, Some(resume))
            }
            NativeEntry::Instruction(action) => {
                let description = action.field.parent.clone();
                let generics = action.generics.clone();
                let future: NativeFuture = Box::pin(async move {
                    let mut fx = fx;
                    fx.ensure_initialized(description, generics).await
                });
                let operand = operands.pop();
                (
                    future,
                    Some(InstructionCompletion { action, operand }),
                    None,
                )
            }
            NativeEntry::Intrinsic(entry) => (entry(fx), None, None),
        };
        self.call_state.last_native_id = next_id;
        self.call_state.native.push(NativeActivation {
            ambient_exception: None,
            owner: NativeFutureOwner {
                control,
                future: Some(future),
            },
            pending: None,
            wait: None,
            roots,
            id: dotnet_vm_data::NativeId(
                std::num::NonZeroU64::new(next_id).expect("native identity is nonzero"),
            ),
            parent_destination,
            method: crate::CollectableMethodDescription(method.clone()),
            operands,
            outputs: Vec::new(),
            instruction,
            resume,
            caller_depth: self.frame_stack.len(),
            caller_ip: self.state().ip,
            stack_height: self.top_of_stack(),
            output,
            constructor_result: None,
            constructor: matches!(mode, NativeDispatchMode::Constructor(_)),
        });
        if matches!(mode, NativeDispatchMode::Constructor(_)) {
            return StepResult::Yield;
        }
        if parent_destination.is_some() {
            let child_id = dotnet_vm_data::NativeId(
                std::num::NonZeroU64::new(next_id).expect("native identity is nonzero"),
            );
            let depth = self
                .call_state
                .native_entry_depth
                .get_or_insert_with(Default::default)
                .clone();
            if depth.get() >= 32 {
                return StepResult::NativeSuspended(child_id);
            }
            depth.set(depth.get() + 1);
            struct EntryDepth(std::rc::Rc<std::cell::Cell<usize>>);
            impl Drop for EntryDepth {
                fn drop(&mut self) {
                    self.0.set(self.0.get() - 1);
                }
            }
            let _entry = EntryDepth(depth);
            match self.advance_native_effect_once(true) {
                StepResult::Continue => StepResult::NativeSuspended(child_id),
                other => other,
            }
        } else {
            self.advance_native_effect()
        }
    }
}

impl<'gc> VesContext<'_, 'gc> {
    pub(crate) fn redirected_native_return(
        &self,
        destination: dotnet_vm_data::NativeReturnDestination,
    ) -> Option<(bool, crate::StackSlotIndex)> {
        let work = self.call_state.native.last()?;
        let pending = work.pending.as_ref()?;
        if work.id != destination.id
            || pending.generation != destination.generation
            || pending.redirected.is_none()
        {
            return None;
        }
        Some((
            pending.call.method.signature().return_type.1.is_some(),
            work.stack_height,
        ))
    }

    pub(crate) fn has_staged_native_call(&self, depth: usize) -> bool {
        self.call_state.native.last().is_some_and(|work| {
            work.caller_depth == depth
                && work.pending.as_ref().is_some_and(|pending| pending.staged)
        })
    }

    pub(crate) fn retain_nested_native_target(
        &mut self,
        caller_depth: usize,
        method: &MethodDescription,
        generics: &GenericLookup,
    ) {
        if let Some(work) = self.call_state.native.last_mut()
            && work.caller_depth == caller_depth
            && let Some(pending) = work.pending.as_mut().filter(|pending| pending.staged)
            && pending.call.method != *method
        {
            pending.redirected = Some((method.clone(), generics.clone()));
        }
    }

    /// Whether a type-initializer activation is live for the frame at `depth`.
    pub(crate) fn has_initializer_gate(&self, depth: usize) -> bool {
        self.call_state
            .native
            .last()
            .is_some_and(|work| work.caller_depth == depth && work.resume.is_some())
    }

    pub(crate) fn try_start_multicast(
        &mut self,
        method: MethodDescription,
        generics: &GenericLookup,
    ) -> Option<StepResult> {
        let signature = method.signature();
        let count = signature.parameters.len();
        if method.method().name != "Invoke"
            || !signature.instance
            || self.frame_stack.is_empty()
            || self.frame_stack.current_frame().stack_height.as_usize() < count + 1
            || dotnet_intrinsics_delegates::helpers::cached_delegate_dispatch_kind(self, &method)
                != dotnet_intrinsics_delegates::DelegateDispatchKind::Invoke
        {
            return None;
        }
        let StackValue::ObjectRef(receiver @ ObjectRef(Some(_))) = self.peek_stack_at(count) else {
            return None;
        };
        let targets =
            dotnet_intrinsics_delegates::helpers::get_multicast_targets_ref(self, receiver)?;
        let multicast = targets.as_vector(|vector| {
            vector.layout.length > 1
                || (vector.layout.length == 1
                    && vector.object_ref_elements(self.gc.mutation()).next() != Some(receiver))
        });
        if !multicast {
            return None;
        }
        for (index, parameter) in signature.parameters.iter().enumerate() {
            if matches!(parameter.1, ParameterType::Ref(_))
                && matches!(self.peek_stack_at(count - index - 1), StackValue::ManagedPtr(pointer)
                    if matches!(pointer.origin(), dotnet_value::pointer::PointerOrigin::Transient(_)))
            {
                return Some(self.throw_by_name_with_message(
                    "System.InvalidProgramException",
                    "Common Language Runtime detected an invalid program.",
                ));
            }
        }
        if matches!(
            multicast_output(self, &method, generics),
            NativeOutput::Unsupported
        ) {
            tracing::debug!(
                multicast_path = "fallback",
                reason = "return",
                "multicast selection"
            );
            return None;
        }
        for (index, parameter) in signature.parameters.iter().enumerate() {
            let value = self.peek_stack_at(count - index - 1);
            let supported = match &parameter.1 {
                ParameterType::Value(ty) => multicast_argument_supported(
                    multicast_value_shape(self, &method, generics, ty),
                    &value,
                ),
                ParameterType::Ref(ty) => match value {
                    value @ (StackValue::ManagedPtr(_)
                    | StackValue::NativeInt(_)
                    | StackValue::UnmanagedPtr(_))
                        if generics
                            .make_concrete(method.resolution(), ty.clone(), self.loader().as_ref())
                            .ok()
                            .and_then(|concrete| {
                                self.current_context()
                                    .resolver()
                                    .type_layout_cached_with_lookup(
                                        concrete,
                                        method.resolution(),
                                        generics,
                                    )
                                    .ok()
                            })
                            .is_some_and(|layout| !layout.has_managed_ptrs()) =>
                    {
                        match value {
                            StackValue::NativeInt(_) | StackValue::UnmanagedPtr(_) => true,
                            StackValue::ManagedPtr(pointer) => match pointer.origin() {
                                dotnet_value::pointer::PointerOrigin::Heap(ObjectRef(Some(_)))
                                | dotnet_value::pointer::PointerOrigin::FrameHome(_)
                                | dotnet_value::pointer::PointerOrigin::Static(_)
                                | dotnet_value::pointer::PointerOrigin::Unmanaged => true,
                                #[cfg(feature = "multithreading")]
                                dotnet_value::pointer::PointerOrigin::CrossArenaObjectRef(_) => {
                                    true
                                }
                                _ => false,
                            },
                            _ => unreachable!(),
                        }
                    }
                    _ => false,
                },
                ParameterType::TypedReference => false,
            };
            if !supported {
                tracing::debug!(
                    multicast_path = "fallback",
                    reason = "argument",
                    "multicast selection"
                );
                return None;
            }
        }
        tracing::debug!(multicast_path = "async", "multicast selection");
        Some(self.start_native(
            method,
            NativeDispatchMode::InlineCall,
            generics,
            None,
            None,
            multicast_entry,
        ))
    }

    pub(crate) fn advance_native_effect(&mut self) -> StepResult {
        for completed in 0..32 {
            let result = self.advance_native_effect_once(completed == 0);
            if result != StepResult::Continue {
                if matches!(result, StepResult::Error(_)) {
                    self.call_state
                        .cleanup_native(self.frame_stack, self.evaluation_stack);
                }
                return result;
            }
        }
        StepResult::Yield
    }

    fn advance_monitor(
        &mut self,
        mut work: NativeActivation<'gc>,
        mut wait: PendingMonitor,
    ) -> StepResult {
        if self.shared.thread_manager.is_gc_stop_requested()
            || self.local.heap.needs_full_collect.get()
            || self.gc.needs_collection()
        {
            work.wait = Some(PendingWait::Monitor(wait));
            self.call_state.native.push(work);
            return StepResult::Yield;
        }
        let thread_id = self.thread_id();
        let mut result = wait.block.try_enter(thread_id).then_some(true);
        if result.is_none()
            && wait
                .deadline
                .is_some_and(|d| std::time::Instant::now() >= d)
        {
            result = Some(false);
        }
        if result.is_none() {
            #[cfg(test)]
            wait_test_hook(WaitTestPoint::Probed);
            // Capture the release epoch before rechecking; a concurrent release either
            // makes the probe succeed or changes the epoch observed by the sleeper.
            wait.epoch = Some(wait.block.release_epoch());
            #[cfg(test)]
            wait_test_hook(WaitTestPoint::Registered);
            result = wait.block.try_enter(thread_id).then_some(true);
            if result.is_none()
                && wait
                    .deadline
                    .is_some_and(|d| std::time::Instant::now() >= d)
            {
                result = Some(false);
            }
        }
        if let Some(acquired) = result {
            if let Err(error) = work.owner.control.reply_monitor(wait.generation, acquired) {
                return StepResult::Error(error);
            }
            self.call_state.native.push(work);
            StepResult::Continue
        } else {
            work.wait = Some(PendingWait::Monitor(wait));
            self.call_state.native.push(work);
            StepResult::Yield
        }
    }

    fn advance_monitor_wait(
        &mut self,
        mut work: NativeActivation<'gc>,
        mut pending: PendingMonitorWait,
    ) -> StepResult {
        let stopped = self.shared.thread_manager.is_gc_stop_requested()
            || self.local.heap.needs_full_collect.get()
            || self.gc.needs_collection();
        if !stopped {
            if let Some(pulsed) = pending.wait.poll(self.thread_id()) {
                if let Err(error) = work.owner.control.reply_monitor(pending.generation, pulsed) {
                    return StepResult::Error(error);
                }
                self.call_state.native.push(work);
                return StepResult::Continue;
            }
            #[cfg(test)]
            wait_test_hook(WaitTestPoint::Probed);
            pending.epoch = pending.wait.block.release_epoch();
            #[cfg(test)]
            wait_test_hook(WaitTestPoint::Registered);
            if let Some(pulsed) = pending.wait.poll(self.thread_id()) {
                if let Err(error) = work.owner.control.reply_monitor(pending.generation, pulsed) {
                    return StepResult::Error(error);
                }
                self.call_state.native.push(work);
                return StepResult::Continue;
            }
        }
        work.wait = Some(PendingWait::MonitorWait(pending));
        self.call_state.native.push(work);
        StepResult::Yield
    }

    // INVARIANT: F6.EventWaitRegistration — notifications only trigger another locked probe; the owned registration is retired on completion or cancellation.
    fn advance_event_wait(
        &mut self,
        mut work: NativeActivation<'gc>,
        mut pending: PendingEvent,
    ) -> StepResult {
        let stopped = self.shared.thread_manager.is_gc_stop_requested()
            || self.local.heap.needs_full_collect.get()
            || self.gc.needs_collection();
        if !stopped {
            let mut result = pending.wait.poll();
            if result.is_none() {
                #[cfg(test)]
                wait_test_hook(WaitTestPoint::Probed);
                pending.epoch = pending.wait.event.epoch();
                #[cfg(test)]
                wait_test_hook(WaitTestPoint::Registered);
                result = pending.wait.poll();
            }
            if let Some(signaled) = result {
                if let Err(error) = work.owner.control.reply_event(pending.generation, signaled) {
                    return StepResult::Error(error);
                }
                self.call_state.native.push(work);
                return StepResult::Continue;
            }
            #[cfg(not(feature = "multithreading"))]
            if pending.wait.deadline.is_none() {
                return StepResult::internal_error(
                    "event wait would deadlock in single-threaded mode",
                );
            }
        }
        work.wait = Some(PendingWait::Event(pending));
        self.call_state.native.push(work);
        StepResult::Yield
    }

    fn advance_finalizers(
        &mut self,
        mut work: NativeActivation<'gc>,
        generation: u64,
    ) -> StepResult {
        let ready = || {
            !self.shared.thread_manager.is_gc_stop_requested()
                && !self.local.heap.needs_full_collect.get()
                && !self.gc.needs_collection()
                && !self.shared.has_pending_finalizers()
                && !self.local.heap.processing_finalizer.get()
                && self.local.heap.pending_finalization.borrow().is_empty()
        };
        if !ready() {
            work.wait = Some(PendingWait::Finalizers(generation));
            #[cfg(all(test, feature = "multithreading"))]
            wait_test_hook(WaitTestPoint::FinalizersRegistered);
            // Recheck after registration without moving a branded queue outside the mutation.
            if !ready() {
                self.call_state.native.push(work);
                return StepResult::Yield;
            }
            drop(work.wait.take());
        }
        if let Err(error) = work.owner.control.reply_finalizers(generation) {
            return StepResult::Error(error);
        }
        self.call_state.native.push(work);
        StepResult::Continue
    }

    fn advance_sleep(
        &mut self,
        mut work: NativeActivation<'gc>,
        generation: u64,
        deadline: Option<std::time::Instant>,
    ) -> StepResult {
        let stopped = self.shared.thread_manager.is_gc_stop_requested()
            || self.local.heap.needs_full_collect.get()
            || self.gc.needs_collection();
        if !stopped && deadline.is_some_and(|d| std::time::Instant::now() >= d) {
            if let Err(error) = work.owner.control.reply_sleep(generation) {
                return StepResult::Error(error);
            }
            self.call_state.native.push(work);
            StepResult::Continue
        } else {
            work.wait = Some(PendingWait::Sleep(generation, deadline));
            self.call_state.native.push(work);
            StepResult::Yield
        }
    }

    fn advance_initializer(
        &mut self,
        mut work: NativeActivation<'gc>,
        generation: u64,
        wait: Box<dyn dotnet_vm_ops::effects::NativeInitWait>,
    ) -> StepResult {
        let stopped = self.shared.thread_manager.is_gc_stop_requested()
            || self.local.heap.needs_full_collect.get()
            || self.gc.needs_collection();
        if !stopped && !wait.ready() {
            work.wait = Some(PendingWait::Initializer(generation, wait));
            self.call_state.native.push(work);
            return StepResult::Yield;
        }
        // Removing the edge before resuming forces the next probe to repeat the cycle check.
        drop(wait);
        if let Err(error) = work.owner.control.reply_initializer(generation) {
            return StepResult::Error(error);
        }
        self.call_state.native.push(work);
        if stopped {
            StepResult::Yield
        } else {
            StepResult::Continue
        }
    }

    fn advance_native_effect_once(&mut self, new_turn: bool) -> StepResult {
        let Some(mut work) = self.call_state.native.pop() else {
            return StepResult::internal_error("missing native activation");
        };
        if self.frame_stack.len() != work.caller_depth
            || self.frame_stack.is_empty()
            || self.state().ip != work.caller_ip
            || (work.pending.is_none() && self.top_of_stack() != work.stack_height)
        {
            // The caller mismatch can leave a child frame live. Keep its activation
            // on the stack so terminal cleanup retires that frame before the future.
            self.call_state.native.push(work);
            return StepResult::internal_error("native caller changed");
        }
        if let Some(wait) = work.wait.take() {
            return match wait {
                PendingWait::Monitor(wait) => self.advance_monitor(work, wait),
                PendingWait::MonitorWait(wait) => self.advance_monitor_wait(work, wait),
                PendingWait::Event(wait) => self.advance_event_wait(work, wait),
                PendingWait::Finalizers(generation) => self.advance_finalizers(work, generation),
                PendingWait::Sleep(generation, deadline) => {
                    self.advance_sleep(work, generation, deadline)
                }
                PendingWait::Initializer(generation, wait) => {
                    self.advance_initializer(work, generation, wait)
                }
            };
        }
        if work.pending.is_some() {
            if self.shared.thread_manager.is_gc_stop_requested()
                || self.local.heap.needs_full_collect.get()
                || self.gc.needs_collection()
            {
                self.call_state.native.push(work);
                return StepResult::Yield;
            }
            return self.dispatch_pending_native_call(work);
        }
        let mut future = work
            .owner
            .future
            .take()
            .expect("native activation must own its future");
        let gc = self.gc;
        let poll = {
            let mut adapter = Adapter {
                context: self,
                operands: &mut work.operands,
                outputs: &mut work.outputs,
            };
            let mut region = NativeRegion::new(gc.mutation(), work.roots, &mut adapter);
            if new_turn {
                poll_native(&work.owner.control, future.as_mut(), &mut region)
            } else {
                poll_native_continuation(&work.owner.control, future.as_mut(), &mut region)
            }
        };
        work.owner.future = Some(future);
        match poll {
            Ok(Poll::Pending) => match work.owner.control.take_effect() {
                Some(NativeEffect::Yield) => {
                    self.call_state.native.push(work);
                    StepResult::Yield
                }
                Some(NativeEffect::Finalizers(generation)) if work.operands.is_empty() => {
                    if let Err(error) = work.owner.control.take_finalizers(generation) {
                        return StepResult::Error(error);
                    }
                    self.advance_finalizers(work, generation)
                }
                Some(NativeEffect::Sleep(generation)) if work.operands.is_empty() => {
                    let deadline = match work.owner.control.take_sleep(generation) {
                        Ok(deadline) => deadline,
                        Err(error) => return StepResult::Error(error),
                    };
                    self.advance_sleep(work, generation, deadline)
                }
                Some(NativeEffect::Initializer(generation)) if work.operands.is_empty() => {
                    let wait = match work.owner.control.take_initializer(generation) {
                        Ok(wait) => wait,
                        Err(error) => return StepResult::Error(error),
                    };
                    self.advance_initializer(work, generation, wait)
                }
                Some(NativeEffect::Event(generation)) if work.operands.is_empty() => {
                    let (id, deadline) = match work.owner.control.take_event(generation) {
                        Ok(request) => request,
                        Err(error) => return StepResult::Error(error),
                    };
                    let Some(event) = self.shared.events.get(id) else {
                        return StepResult::internal_error("unsupported wait handle");
                    };
                    let wait = event.begin(deadline);
                    let epoch = event.epoch();
                    self.advance_event_wait(
                        work,
                        PendingEvent {
                            wait,
                            epoch,
                            generation,
                        },
                    )
                }
                Some(NativeEffect::Monitor(generation)) if work.operands.is_empty() => {
                    let (object, deadline, is_wait) =
                        match work.owner.control.take_monitor(generation) {
                            Ok(request) => request,
                            Err(error) => return StepResult::Error(error),
                        };
                    let value = {
                        let mut adapter = Adapter {
                            context: self,
                            operands: &mut work.operands,
                            outputs: &mut work.outputs,
                        };
                        let region = NativeRegion::new(gc.mutation(), work.roots, &mut adapter);
                        match region.fetch(&object) {
                            Ok(value) => value,
                            Err(error) => return StepResult::Error(error),
                        }
                    };
                    let StackValue::ObjectRef(object_ref @ ObjectRef(Some(_))) = value else {
                        return StepResult::internal_error("invalid monitor object");
                    };
                    let block = self.monitor_get_or_create_sync_block_for_object(object_ref, gc);
                    if is_wait {
                        #[cfg(not(feature = "multithreading"))]
                        if deadline.is_none() {
                            return StepResult::internal_error(
                                "Monitor.Wait would deadlock in single-threaded mode",
                            );
                        }
                        let Some(wait) =
                            crate::sync::MonitorWait::begin(block, self.thread_id(), deadline)
                        else {
                            return StepResult::internal_error(
                                "monitor ownership changed before wait",
                            );
                        };
                        let epoch = wait.block.release_epoch();
                        return self.advance_monitor_wait(
                            work,
                            PendingMonitorWait {
                                wait,
                                epoch,
                                generation,
                                _object: object,
                            },
                        );
                    }
                    self.advance_monitor(
                        work,
                        PendingMonitor {
                            block,
                            epoch: None,
                            deadline,
                            generation,
                            _object: object,
                        },
                    )
                }
                Some(NativeEffect::Call(generation)) if work.operands.is_empty() => {
                    let (call, capture) = match work.owner.control.take_call(generation) {
                        Ok(call) => call,
                        Err(error) => return StepResult::Error(error),
                    };
                    // A captured exception must resume the interrupted filter or handler.
                    work.ambient_exception = capture.then_some(*self.exception_mode);
                    work.pending = Some(PendingCall {
                        generation,
                        capture,
                        call,
                        staged: false,
                        redirected: None,
                    });
                    if self.shared.thread_manager.is_gc_stop_requested()
                        || self.local.heap.needs_full_collect.get()
                        || self.gc.needs_collection()
                    {
                        self.call_state.native.push(work);
                        StepResult::Yield
                    } else {
                        self.dispatch_pending_native_call(work)
                    }
                }
                _ => StepResult::internal_error("invalid native effect or unconsumed operands"),
            },
            Ok(Poll::Ready(Ok(()))) => {
                let valid = work.operands.is_empty()
                    && match work.output {
                        NativeOutput::Void => work.outputs.is_empty(),
                        NativeOutput::String => {
                            matches!(work.outputs.as_slice(), [StackValue::ObjectRef(ObjectRef(Some(obj)))] if matches!(obj.borrow().storage, HeapStorage::Str(_)))
                        }
                        NativeOutput::Int32 => {
                            matches!(work.outputs.as_slice(), [StackValue::Int32(_)])
                        }
                        NativeOutput::Int64 => {
                            matches!(work.outputs.as_slice(), [StackValue::Int64(_)])
                        }
                        NativeOutput::NativeInt => {
                            matches!(work.outputs.as_slice(), [StackValue::NativeInt(_)])
                        }
                        NativeOutput::NativeFloat => {
                            matches!(work.outputs.as_slice(), [StackValue::NativeFloat(_)])
                        }
                        NativeOutput::Object => {
                            matches!(work.outputs.as_slice(), [StackValue::ObjectRef(_)])
                        }
                        NativeOutput::ValueType => {
                            matches!(work.outputs.as_slice(), [StackValue::ValueType(_)])
                        }
                        NativeOutput::Unsupported => false,
                    };
                if !valid {
                    work.cleanup();
                    return StepResult::internal_error("native result does not match signature");
                }
                if let Some(resume) = work.resume.take() {
                    let caller_ip = work.caller_ip;
                    let height = work.stack_height;
                    let nested = work.parent_destination.is_some();
                    work.cleanup();
                    return match resume {
                        // A nested activation leaves its parent's staged call to re-dispatch.
                        InitializerResume::Retry => StepResult::Yield,
                        InitializerResume::Dispatch(..) if nested => StepResult::Yield,
                        InitializerResume::Dispatch(method, lookup, mode) => {
                            let saved_ip = *self.original_ip;
                            let saved_height = *self.original_stack_height;
                            *self.original_ip = caller_ip;
                            *self.original_stack_height = height;
                            let result = self.dispatch_method_with_mode(method, lookup, mode);
                            *self.original_ip = saved_ip;
                            *self.original_stack_height = saved_height;
                            if result == StepResult::Continue {
                                StepResult::Jump(caller_ip + 1)
                            } else {
                                result
                            }
                        }
                    };
                }
                if let Some(instruction) = work.instruction.take() {
                    if let Some(value) = instruction.operand {
                        self.push(value);
                    }
                    let result = crate::instructions::objects::fields::complete_static_field(
                        self,
                        instruction.action,
                    );
                    let next = work.caller_ip + 1;
                    work.cleanup();
                    return if result == StepResult::Continue {
                        StepResult::Jump(next)
                    } else {
                        result
                    };
                }
                if let Some(destination) = work.parent_destination {
                    // INVARIANT: F6.NativeRootTransfer — fetch from the child's set, copy
                    // into the parent's set, then release the child's root.
                    let output = work.outputs.pop();
                    let rooted = {
                        let mut adapter = Adapter {
                            context: self,
                            operands: &mut work.operands,
                            outputs: &mut work.outputs,
                        };
                        let region = NativeRegion::new(gc.mutation(), work.roots, &mut adapter);
                        match output.map(|value| region.root(value)).transpose() {
                            Ok(value) => value,
                            Err(error) => return StepResult::Error(error),
                        }
                    };
                    let value = {
                        let mut adapter = Adapter {
                            context: self,
                            operands: &mut work.operands,
                            outputs: &mut work.outputs,
                        };
                        let region = NativeRegion::new(gc.mutation(), work.roots, &mut adapter);
                        match rooted.as_ref().map(|value| region.fetch(value)).transpose() {
                            Ok(value) => value,
                            Err(error) => return StepResult::Error(error),
                        }
                    };
                    let result = self.deliver_native_return(destination, value);
                    drop(rooted);
                    work.cleanup();
                    return match result {
                        Ok(()) => StepResult::Yield,
                        Err(error) => StepResult::Error(error),
                    };
                }
                for value in work.outputs.drain(..) {
                    self.push(value);
                }
                if let Some(result) = work.constructor_result.take()
                    && let Err(error) = self.complete_constructor_receiver(result)
                {
                    work.cleanup();
                    return StepResult::Error(error.into());
                }
                let next = work.caller_ip + 1;
                work.cleanup();
                StepResult::Jump(next)
            }
            Ok(Poll::Ready(Err(NativeFailure::Host(error)))) | Err(error) => {
                work.cleanup();
                StepResult::Error(error)
            }
            Ok(Poll::Ready(Err(NativeFailure::Managed(exception)))) => {
                let fetched = {
                    let mut adapter = Adapter {
                        context: self,
                        operands: &mut work.operands,
                        outputs: &mut work.outputs,
                    };
                    NativeRegion::new(gc.mutation(), work.roots, &mut adapter).fetch(&exception)
                };
                let result = match fetched {
                    Ok(StackValue::ObjectRef(exception @ ObjectRef(Some(_)))) => {
                        self.throw(exception)
                    }
                    Ok(_) => StepResult::internal_error("invalid native exception"),
                    Err(error) => StepResult::Error(error),
                };
                drop(exception);
                work.cleanup();
                result
            }
        }
    }
    fn dispatch_pending_native_call(&mut self, mut work: NativeActivation<'gc>) -> StepResult {
        let pending = work.pending.as_mut().expect("pending native call");
        let signature = pending.call.method.signature();
        let expected = signature.parameters.len() + usize::from(signature.instance);
        if pending.redirected.is_none() && pending.call.arguments.len() != expected {
            return StepResult::internal_error("native call argument count mismatch");
        }
        if !pending.staged {
            let gc = self.gc;
            // INVARIANT: F6.NativeRootTransfer — source roots stay live until their
            // fetched values enter the traced evaluation stack in this mutation.
            let values = {
                let mut adapter = Adapter {
                    context: self,
                    operands: &mut work.operands,
                    outputs: &mut work.outputs,
                };
                let region = NativeRegion::new(gc.mutation(), work.roots, &mut adapter);
                pending
                    .call
                    .arguments
                    .iter()
                    .map(|argument| region.fetch(argument))
                    .collect::<Result<Vec<_>, _>>()
            };
            let values = match values {
                Ok(values) => values,
                Err(error) => return StepResult::Error(error),
            };
            for value in values {
                self.push(value);
            }
            pending.staged = true;
        }
        let depth = self.frame_stack.len();
        let (method, generics) = pending
            .redirected
            .clone()
            .unwrap_or_else(|| (pending.call.method.clone(), pending.call.generics.clone()));
        let has_return = method.signature().return_type.1.is_some();
        let generation = pending.generation;
        let id = work.id;
        // Keep the activation rooted while dispatch may allocate or enter another native body.
        self.call_state.native.push(work);
        let saved_ip = *self.original_ip;
        let saved_height = *self.original_stack_height;
        let ip_before = self.state().ip;
        *self.original_ip = ip_before;
        *self.original_stack_height = self.top_of_stack();
        let result = self.dispatch_method_inline(method.clone(), generics);
        *self.original_ip = saved_ip;
        *self.original_stack_height = saved_height;
        match result {
            StepResult::FramePushed if self.frame_stack.len() == depth + 1 => {
                let child = self.frame_stack.current_frame_mut();
                let source = &child.state.info_handle.source;
                let requested = source.parent == method.parent
                    && source.method_resolution == method.method_resolution
                    && source.method_member_index == method.method_member_index;
                // An initializer for another target returns before dispatch is retried.
                if !child.state.info_handle.is_cctor || requested {
                    child.native_return =
                        Some(dotnet_vm_data::NativeReturnDestination { id, generation });
                }
                StepResult::NativeSuspended(id)
            }
            StepResult::Continue => {
                let work = self.call_state.native.last().expect("pending native call");
                let has_return = work
                    .pending
                    .as_ref()
                    .and_then(|pending| pending.redirected.as_ref())
                    .map_or(has_return, |(target, _)| {
                        target.signature().return_type.1.is_some()
                    });
                let expected_top = work.stack_height + usize::from(has_return);
                if self.top_of_stack() != expected_top {
                    return StepResult::internal_error("native call result shape mismatch");
                }
                let destination = dotnet_vm_data::NativeReturnDestination { id, generation };
                let final_return = if let Some((final_return, base)) =
                    self.redirected_native_return(destination)
                {
                    if self.top_of_stack() != base + usize::from(final_return) {
                        return StepResult::internal_error("native call result shape mismatch");
                    }
                    final_return
                } else {
                    has_return
                };
                let value = if final_return { Some(self.pop()) } else { None };
                match self.deliver_native_return(
                    dotnet_vm_data::NativeReturnDestination { id, generation },
                    value,
                ) {
                    Ok(()) => StepResult::Continue,
                    Err(error) => StepResult::Error(error),
                }
            }
            StepResult::NativeSuspended(_) => StepResult::Yield,
            StepResult::Yield
                if self.frame_stack.len() == depth
                    && self
                        .call_state
                        .native
                        .last()
                        .is_some_and(|work| work.id == id && work.pending.is_some()) =>
            {
                let work = self
                    .call_state
                    .native
                    .last_mut()
                    .expect("pending native call");
                if work
                    .pending
                    .as_ref()
                    .is_some_and(|pending| pending.redirected.is_some())
                {
                    return StepResult::Yield;
                }
                let base = work.stack_height;
                work.pending.as_mut().expect("pending native call").staged = false;
                self.evaluation_stack.truncate(base);
                let frame = self.frame_stack.current_frame_mut();
                frame.stack_height = base.saturating_sub_idx(frame.base.stack);
                StepResult::Yield
            }
            other => other,
        }
    }

    pub(crate) fn deliver_native_exception(
        &mut self,
        destination: dotnet_vm_data::NativeReturnDestination,
        exception: ObjectRef<'gc>,
    ) -> Result<(), VmError> {
        let mut work = self
            .call_state
            .native
            .pop()
            .expect("missing native capture destination");
        if work.id != destination.id
            || !work.pending.as_ref().is_some_and(|pending| {
                pending.capture && pending.generation == destination.generation
            })
        {
            work.cleanup();
            return Err(dotnet_types::error::ExecutionError::InternalError(
                "stale native capture destination".into(),
            )
            .into());
        }
        self.evaluation_stack.truncate(work.stack_height);
        let frame = self.frame_stack.current_frame_mut();
        frame.stack_height = work.stack_height.saturating_sub_idx(frame.base.stack);
        let gc = self.gc;
        // INVARIANT: F6.NativeRootTransfer — the exception remains traced by the
        // unwind state until it is copied into the waiting activation's root set.
        let root = {
            let mut adapter = Adapter {
                context: self,
                operands: &mut work.operands,
                outputs: &mut work.outputs,
            };
            NativeRegion::new(gc.mutation(), work.roots, &mut adapter)
                .root(StackValue::ObjectRef(exception))?
        };
        work.owner
            .control
            .reply_exception(destination.generation, root)?;
        work.pending = None;
        *self.exception_mode = work
            .ambient_exception
            .take()
            .unwrap_or(dotnet_vm_data::ExceptionState::None);
        self.call_state.native.push(work);
        Ok(())
    }

    pub(crate) fn deliver_native_return(
        &mut self,
        destination: dotnet_vm_data::NativeReturnDestination,
        value: Option<StackValue<'gc>>,
    ) -> Result<(), VmError> {
        let Some(mut work) = self.call_state.native.pop() else {
            return Err(dotnet_types::error::ExecutionError::InternalError(
                "missing native return destination".into(),
            )
            .into());
        };
        if work.id != destination.id
            || work.pending.as_ref().map(|p| p.generation) != Some(destination.generation)
        {
            return Err(dotnet_types::error::ExecutionError::InternalError(
                "stale native return destination".into(),
            )
            .into());
        }
        let gc = self.gc;
        // INVARIANT: F6.NativeRootTransfer — the frame result is copied into the waiting
        // activation's set in the same mutation, before a collection can intervene.
        let value = {
            let mut adapter = Adapter {
                context: self,
                operands: &mut work.operands,
                outputs: &mut work.outputs,
            };
            let region = NativeRegion::new(gc.mutation(), work.roots, &mut adapter);
            value.map(|value| region.root(value)).transpose()?
        };
        work.owner.control.reply_call(
            destination.generation,
            match value {
                Some(value) => NativeValue::Value(value),
                None => NativeValue::Void,
            },
        )?;
        work.pending = None;
        if matches!(*self.exception_mode, dotnet_vm_data::ExceptionState::None)
            && let Some(ambient) = work.ambient_exception.take()
        {
            *self.exception_mode = ambient;
        }
        work.ambient_exception = None;
        self.call_state.native.push(work);
        Ok(())
    }
}

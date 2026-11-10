use crate::{
    StepResult,
    dispatch::ExecutionEngine,
    error::{ExecutionError, VmError},
    gc::coordinator::*,
    stack::{CallStack, GCArena, ops::VmCallOps},
    state::{ArenaLocalState, SharedGlobalState},
    threading::ThreadManagerOps,
};
use dotnet_metrics::{CacheStats, RuntimeMetricsSnapshot};
use dotnet_tracer::Tracer;
#[cfg(not(feature = "fuzzing"))]
use dotnet_types::WellKnown;
use dotnet_types::members::MethodDescription;
use dotnet_utils::{
    ArenaId,
    gc::GCHandle,
    sync::{Arc, Ordering},
};
use dotnet_value::StackValue;
#[cfg(not(feature = "fuzzing"))]
use dotnet_value::{
    object::{HeapStorage, ObjectRef},
    string::CLRString,
};
use dotnet_vm_data::ManagedException;
#[cfg(not(feature = "fuzzing"))]
use dotnet_vm_ops::{LoaderOps, MemoryOps};
use dotnetdll::prelude::{BaseType, MethodType, ParameterType};

#[cfg(feature = "multithreading")]
use crate::gc::arena::ArenaOwner;

#[cfg(test)]
thread_local! {
    pub(crate) static WAIT_TEST_HOOK: std::cell::RefCell<Option<Box<dyn FnOnce()>>> = std::cell::RefCell::new(None);
}

#[cfg(all(test, feature = "multithreading"))]
type WorkerStartTestHook = Box<dyn FnOnce(&mut Executor) -> ExecutorResult>;
#[cfg(all(test, feature = "multithreading"))]
type RetiredQuiescenceTestHook = Box<dyn FnMut(bool)>;

// Inject an early worker failure with live state before the common teardown.
#[cfg(all(test, feature = "multithreading"))]
thread_local! {
    static WORKER_START_TEST_HOOK: std::cell::RefCell<Option<WorkerStartTestHook>> = std::cell::RefCell::new(None);
    static RETIRED_QUIESCENCE_TEST_HOOK: std::cell::RefCell<Option<RetiredQuiescenceTestHook>> = std::cell::RefCell::new(None);
}

enum NativeWaitSleep {
    Monitor(Arc<crate::sync::SyncBlock>, u64, Option<std::time::Instant>),
    Finalizers,
    Sleep(Option<std::time::Instant>),
    Initializer(Box<dyn dotnet_vm_ops::effects::NativeInitSleeper>),
}

/// Thread-affine execution owner. Neither Executor nor its extracted arena
/// guard may be sent to or shared with another OS thread.
pub struct Executor {
    _affinity: std::marker::PhantomData<std::rc::Rc<()>>,
    #[cfg(feature = "multithreading")]
    owner: ArenaOwner,
    shared: Arc<SharedGlobalState>,
    /// Thread ID for this executor
    thread_id: ArenaId,
    registered: bool,
    /// Owner-held registration; moved into `ArenaGuard` by `extract_arena`.
    #[cfg(feature = "multithreading")]
    registration: std::cell::RefCell<Option<dotnet_utils::gc::ArenaRegistration>>,
    #[cfg(not(feature = "multithreading"))]
    arena: Box<GCArena>,
    #[cfg(feature = "fuzzing")]
    pub instruction_budget: Option<u64>,
    #[cfg(not(feature = "multithreading"))]
    gc_tick_counter: u32,
}

#[derive(Clone, Debug)]
pub enum ExecutorResult {
    Exited(u8),
    Threw(ManagedException),
    Error(VmError),
}

impl std::fmt::Display for ExecutorResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ExecutorResult::Exited(code) => write!(f, "Exited with code {}", code),
            ExecutorResult::Threw(exc) => write!(f, "{}", exc),
            ExecutorResult::Error(err) => write!(f, "Internal VM Error: {}", err),
        }
    }
}

impl Executor {
    #[cfg(feature = "fuzzing")]
    fn fuzzing_default_entrypoint_arg<'gc>(
        parameter: &ParameterType<MethodType>,
    ) -> StackValue<'gc> {
        match parameter {
            ParameterType::Value(MethodType::Base(base_type)) => match base_type.as_ref() {
                BaseType::Boolean
                | BaseType::Char
                | BaseType::Int8
                | BaseType::UInt8
                | BaseType::Int16
                | BaseType::UInt16
                | BaseType::Int32
                | BaseType::UInt32 => StackValue::Int32(0),
                BaseType::Int64 | BaseType::UInt64 => StackValue::Int64(0),
                BaseType::Float32 | BaseType::Float64 => StackValue::NativeFloat(0.0),
                BaseType::IntPtr
                | BaseType::UIntPtr
                | BaseType::FunctionPointer(_)
                | BaseType::ValuePointer(_, _) => StackValue::NativeInt(0),
                BaseType::Object
                | BaseType::String
                | BaseType::Array(_, _)
                | BaseType::Vector(_, _)
                | BaseType::Type { .. } => StackValue::null(),
            },
            _ => StackValue::null(),
        }
    }

    #[cfg(feature = "multithreading")]
    fn thread_arena_not_initialized_error() -> VmError {
        VmError::Execution(ExecutionError::InternalError(
            "Thread arena not initialized".into(),
        ))
    }

    fn with_arena<R>(&mut self, f: impl FnOnce(&mut GCArena) -> R) -> Result<R, VmError> {
        #[cfg(feature = "multithreading")]
        {
            self.owner.with(|arena| {
                Ok(f(
                    arena.ok_or_else(Self::thread_arena_not_initialized_error)?
                ))
            })
        }
        #[cfg(not(feature = "multithreading"))]
        {
            Ok(f(&mut self.arena))
        }
    }

    fn with_arena_ref<R>(&self, f: impl FnOnce(&GCArena) -> R) -> Result<R, VmError> {
        #[cfg(feature = "multithreading")]
        {
            self.owner.with(|arena| {
                Ok(f(
                    arena.ok_or_else(Self::thread_arena_not_initialized_error)?
                ))
            })
        }
        #[cfg(not(feature = "multithreading"))]
        {
            Ok(f(&self.arena))
        }
    }

    fn collection_check(&mut self) -> Result<(bool, bool), VmError> {
        self.with_arena(|arena| {
            arena.mutate(|_, c| {
                let full_collect = if c.stack.local.heap.needs_full_collect.get() {
                    c.stack.local.heap.needs_full_collect.set(false);
                    true
                } else {
                    false
                };

                #[cfg(feature = "multithreading")]
                {
                    let collection_requested =
                        full_collect || c.stack.arena.needs_collection().load(Ordering::Acquire);
                    (full_collect, collection_requested)
                }

                #[cfg(not(feature = "multithreading"))]
                {
                    (full_collect, full_collect)
                }
            })
        })
    }

    pub fn tracer_enabled(&self) -> bool {
        self.shared.tracer_enabled.load(Ordering::Relaxed)
    }

    pub fn tracer(&self) -> &Tracer {
        &self.shared.tracer
    }

    pub fn indent(&self) -> usize {
        0
    }

    pub fn get_cache_stats(&self) -> CacheStats {
        self.shared.get_cache_stats()
    }

    pub fn get_runtime_metrics_snapshot(&self) -> RuntimeMetricsSnapshot {
        self.shared.get_runtime_metrics_snapshot()
    }

    pub fn new(shared: Arc<SharedGlobalState>) -> Self {
        #[cfg(feature = "multithreading")]
        let owner = {
            let owner = ArenaOwner::reserve();
            #[cfg(test)]
            ownership_checkpoint("reservation");
            owner
        };
        let shared_clone = Arc::clone(&shared);
        let arena = Box::new(GCArena::new(|_| {
            let local = ArenaLocalState::new();
            ExecutionEngine::new(CallStack::new(shared_clone, local))
        }));

        let thread_id = shared.thread_manager.register_thread();

        #[cfg(feature = "multithreading")]
        owner.install(arena);

        let mut executor = Self {
            _affinity: std::marker::PhantomData,
            shared,
            thread_id,
            registered: true,
            #[cfg(feature = "multithreading")]
            owner,
            #[cfg(feature = "multithreading")]
            registration: std::cell::RefCell::new(None),
            #[cfg(not(feature = "multithreading"))]
            arena,
            #[cfg(feature = "fuzzing")]
            instruction_budget: None,
            #[cfg(not(feature = "multithreading"))]
            gc_tick_counter: 0,
        };
        // From here all fallible initialization is covered by Executor Drop.
        // No arena pointer has been published yet. Capture the exact registration
        // rather than ever falling back to an ID-only removal.
        #[cfg(feature = "multithreading")]
        let arena_identity = {
            let identity = dotnet_utils::gc::try_acquire_lease(thread_id)
                .expect("new Executor arena registration must be live")
                .identity();
            let registration = dotnet_utils::gc::registration_for_identity(identity)
                .expect("new Executor arena registration must be live");
            *executor.registration.get_mut() = Some(registration);
            executor.owner.bind_identity(identity);
            identity
        };
        #[cfg(feature = "multithreading")]
        {
            let handle = ArenaHandle::new(thread_id);
            executor.shared.gc_coordinator.register_arena(handle);
            #[cfg(test)]
            ownership_checkpoint("construction");
        }
        let shared = Arc::clone(&executor.shared);
        executor
            .with_arena(|arena| {
                arena.mutate_root(|_gc, c| {
                    c.stack.thread_id.set(thread_id);
                    #[cfg(feature = "multithreading")]
                    {
                        c.stack.arena_identity = Some(arena_identity);
                        let stack_arena = ArenaHandle::new(thread_id);
                        shared
                            .gc_coordinator
                            .bind_collection_pressure_flag(&stack_arena);
                        c.stack.arena = stack_arena;
                    }
                    #[cfg(not(feature = "multithreading"))]
                    let _ = &shared;
                })
            })
            .expect("newly installed arena");
        executor
    }

    pub fn entrypoint(&mut self, method: MethodDescription) -> Result<(), VmError> {
        #[cfg(not(feature = "fuzzing"))]
        let argv: Vec<String> = std::env::args().skip(1).collect();
        #[cfg(feature = "bench-instrumentation")]
        let metrics = self.shared.metrics.clone();
        #[cfg(feature = "bench-instrumentation")]
        let _metrics_scope = dotnet_metrics::ActiveRuntimeMetricsGuard::enter(&metrics);
        #[cfg(feature = "memory-validation")]
        let thread_id = self.thread_id;
        self.with_arena(|arena| {
            arena.mutate_root(|gc, c| -> Result<(), VmError> {
                let gc_handle = GCHandle::new(
                    gc,
                    #[cfg(feature = "multithreading")]
                    // SAFETY: F1.GcHandleRooted — `arena_inner_gc` is only used while mutating this thread's
                    // root in the owning arena context.
                    unsafe {
                        c.stack.arena_inner_gc()
                    },
                    #[cfg(feature = "memory-validation")]
                    thread_id,
                );

                let shared = c.stack.shared.clone();
                let info = shared
                    .caches
                    .get_method_info(method, &Default::default(), shared.clone())?;
                let mut ctx = c.ves_context(gc_handle);
                #[cfg(not(feature = "fuzzing"))]
                {
                    assert!(
                        !info.signature.instance,
                        "Entrypoint must be static (ECMA-335 II.15.4.1.2)"
                    );
                    assert!(
                        info.source
                            .parent
                            .definition()
                            .generic_parameters
                            .is_empty(),
                        "Entrypoint cannot be declared on a generic type (ECMA-335 II.15.4.1.2)"
                    );
                    let has_supported_return_type = match &info.signature.return_type.1 {
                        None => true,
                        Some(ParameterType::Value(MethodType::Base(base_type))) => {
                            matches!(base_type.as_ref(), BaseType::Int32 | BaseType::UInt32)
                        }
                        _ => false,
                    };
                    assert!(
                        has_supported_return_type,
                        "Entrypoint return type must be void, int32, or uint32 (ECMA-335 II.15.4.1.2)"
                    );
                }
                let entrypoint_args = match info.signature.parameters.len() {
                    0 => vec![],
                    1 => {
                        let entrypoint_param = &info.signature.parameters[0].1;
                        #[cfg(not(feature = "fuzzing"))]
                        {
                            let accepts_string_array = matches!(
                                entrypoint_param,
                                ParameterType::Value(MethodType::Base(base_type))
                                    if matches!(
                                        base_type.as_ref(),
                                        BaseType::Vector(_, element)
                                            if matches!(
                                                element,
                                                MethodType::Base(element_base)
                                                    if matches!(element_base.as_ref(), BaseType::String)
                                            )
                                    )
                            );
                            assert!(
                                accepts_string_array,
                                "Entrypoint parameter must be a string[] vector (ECMA-335 II.15.4.1.2)"
                            );

                            let string_type = ctx.loader().corlib_wkt(WellKnown::String)?;
                            let mut argv_vector = ctx.new_vector(string_type.into(), argv.len())?;

                            for (arg, chunk) in argv
                                .iter()
                                .zip(argv_vector.get_mut().chunks_exact_mut(ObjectRef::SIZE))
                            {
                                let arg_ref =
                                    ObjectRef::new(gc_handle, HeapStorage::Str(CLRString::from(arg)));
                                ctx.register_new_object(&arg_ref);
                                arg_ref.write(chunk);
                            }

                            let argv_ref = ctx.alloc_vec_ref(gc_handle, argv_vector);
                            vec![StackValue::ObjectRef(argv_ref)]
                        }
                        #[cfg(feature = "fuzzing")]
                        {
                            vec![Self::fuzzing_default_entrypoint_arg(entrypoint_param)]
                        }
                    }
                    #[cfg(not(feature = "fuzzing"))]
                    _ => {
                        return Err(VmError::Execution(ExecutionError::TypeMismatch {
                            expected: "zero parameters or a single string[] parameter at entrypoint",
                            actual: format!("{} parameters", info.signature.parameters.len()).into(),
                        }));
                    }
                    #[cfg(feature = "fuzzing")]
                    _ => info
                        .signature
                        .parameters
                        .iter()
                        .map(|parameter| Self::fuzzing_default_entrypoint_arg(&parameter.1))
                        .collect(),
                };

                if let Some(err) = ctx.managed_frame_abort_error(&info) {
                    return Err(err);
                }
                ctx.entrypoint_frame(info, Default::default(), entrypoint_args)?;
                Ok(())
            })
        })??;
        Ok(())
    }

    /// Runs an already-resolved managed `ThreadStart` target in a fresh worker executor.
    ///
    /// The test seam accepts owned resolution metadata and no managed references.
    #[cfg(all(test, feature = "multithreading"))]
    pub(crate) fn run_thread_start(
        shared: Arc<SharedGlobalState>,
        method: MethodDescription,
        generic_inst: dotnet_types::generics::GenericLookup,
        worker: crate::threading::workers::RunningWorker,
    ) {
        Self::run_thread_start_inner(shared, method, generic_inst, None, worker);
    }

    #[cfg(feature = "multithreading")]
    pub(crate) fn run_managed_thread_start(
        shared: Arc<SharedGlobalState>,
        method: MethodDescription,
        generic_inst: dotnet_types::generics::GenericLookup,
        start: crate::threading::local::WorkerStart,
        worker: crate::threading::workers::RunningWorker,
    ) {
        Self::run_thread_start_inner(shared, method, generic_inst, Some(start), worker);
    }

    #[cfg(feature = "multithreading")]
    fn run_thread_start_inner(
        shared: Arc<SharedGlobalState>,
        method: MethodDescription,
        generic_inst: dotnet_types::generics::GenericLookup,
        start: Option<crate::threading::local::WorkerStart>,
        worker: crate::threading::workers::RunningWorker,
    ) {
        let mut executor = Self::new(shared);
        let retained = start.as_ref().map(|start| start.retained.clone());
        let result = match executor.install_thread_start_with_args(method, generic_inst, start) {
            Ok(()) => executor.run_installed_thread_start(),
            Err(error) => ExecutorResult::Error(error),
        };
        let result = match result {
            ExecutorResult::Threw(exception) => {
                executor.shared.report_fatal_exception(exception);
                // Join must never turn the unhandled exception into a catchable guest error.
                ExecutorResult::Exited(0)
            }
            result => result,
        };
        // The finished segment cannot retain its stack or resume after publication.
        let cleanup = executor.with_arena(|arena| {
            arena.mutate_root(|_, c| {
                c.retire_finalizer_segment();
                let execution = &mut c.stack.execution;
                execution
                    .call_state
                    .cleanup_native(&mut execution.frame_stack, &mut execution.evaluation_stack);
                *execution = crate::stack::context::ThreadContext::default();
                if retained.is_some() {
                    c.stack.local.managed_threads.clear_current();
                }
            });
        });
        if let Some(retained) = retained
            && cleanup.is_ok()
        {
            retained.store(false, std::sync::atomic::Ordering::Release);
        }
        let result = cleanup.map_or_else(ExecutorResult::Error, |_| result);
        executor.serve_retired(worker.complete(result));
    }

    #[cfg(feature = "multithreading")]
    fn run_installed_thread_start(&mut self) -> ExecutorResult {
        #[cfg(test)]
        if let Some(hook) = WORKER_START_TEST_HOOK.with(|slot| slot.borrow_mut().take()) {
            return hook(self);
        }
        self.run_inner(false)
    }

    #[cfg(feature = "multithreading")]
    fn serve_retired(mut self, completed: crate::threading::workers::CompletedWorker) {
        let retired = completed.retire();
        let shared = self.shared.clone();
        loop {
            if shared.thread_manager.is_gc_stop_requested() {
                shared
                    .thread_manager
                    .safe_point(self.thread_id, &shared.gc_coordinator);
            }
            let pending = self
                .with_arena_ref(|arena| {
                    arena.mutate(|_, c| c.stack.local.heap.has_pending_finalizers())
                })
                .expect("retired owner must retain its arena");
            if pending && retired.begin_finalizing(&shared) {
                let result = self.run_inner(true);
                match result {
                    ExecutorResult::Threw(exception) => shared.report_fatal_exception(exception),
                    ExecutorResult::Error(_) => shared.abort_signal().request_abort(),
                    ExecutorResult::Exited(_) => {}
                }
                // Publish failures while the registry still sees this owner as Finalizing.
                retired.finish_finalizing();
            }
            if shared.thread_manager.wait_retired() {
                // INVARIANT: F1.RetiredCollectionExclusion — a collection may have
                // queued work after the last probe, so recheck while excluding it.
                let done = shared.gc_coordinator.try_exclude_collection(|| {
                    let pending = self
                        .with_arena_ref(|arena| {
                            arena.mutate(|_, c| c.stack.local.heap.has_pending_finalizers())
                        })
                        .expect("retired owner must retain its arena");
                    if pending && !shared.is_abort_requested() {
                        return false;
                    }
                    // INVARIANT: F1.RetiredCollectionExclusion — freeing this arena is only
                    // safe once no managed code anywhere can still collect, because a trace
                    // reads owner identity through references into every arena. Another owner's
                    // finalizer, a still-running worker or a pending queue keeps this one
                    // serving. Abort abandons pending finalizers, as in the entry-arena wait.
                    let quiescent = shared.workers.is_quiescent(|| {
                        shared.is_abort_requested() || !shared.has_pending_finalizers()
                    });
                    #[cfg(test)]
                    RETIRED_QUIESCENCE_TEST_HOOK.with(|hook| {
                        if let Some(callback) = hook.borrow_mut().as_mut() {
                            callback(quiescent);
                        }
                    });
                    if !quiescent {
                        return false;
                    }
                    retired.begin_shutdown();
                    self.unregister();
                    true
                });
                if done == Some(true) {
                    break;
                }
                // The shutdown signal makes `wait_retired` return immediately, so back off
                // rather than spin while another owner finishes its finalizers.
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
        }
        // Lease holders can need coordinator locks before releasing their leases.
        let retained = self.owner.take().map(RetainedArena::new);
        let registration = self.registration.get_mut().take();
        let drained = retained.is_some_and(|arena| arena.drain(registration));
        drop(self);
        retired.finish_shutdown(drained);
    }

    /// Installs a `ThreadStart` target without applying CLI entrypoint argument rules.
    #[cfg(all(test, feature = "multithreading"))]
    fn install_thread_start(
        &mut self,
        method: MethodDescription,
        generic_inst: dotnet_types::generics::GenericLookup,
    ) -> Result<(), VmError> {
        self.install_thread_start_with_args(method, generic_inst, None)
    }

    #[cfg(feature = "multithreading")]
    fn install_thread_start_with_args(
        &mut self,
        method: MethodDescription,
        generic_inst: dotnet_types::generics::GenericLookup,
        start: Option<crate::threading::local::WorkerStart>,
    ) -> Result<(), VmError> {
        #[cfg(feature = "memory-validation")]
        let thread_id = self.thread_id;
        self.with_arena(|arena| {
            arena.mutate_root(|gc, c| -> Result<(), VmError> {
                let gc_handle = GCHandle::new(
                    gc,
                    #[cfg(feature = "multithreading")]
                    // SAFETY: F1.GcHandleRooted — `arena_inner_gc` is only used while mutating this worker's
                    // root in its owning arena context.
                    unsafe {
                        c.stack.arena_inner_gc()
                    },
                    #[cfg(feature = "memory-validation")]
                    thread_id,
                );
                let shared = c.stack.shared.clone();
                let info = shared
                    .caches
                    .get_method_info(method, &generic_inst, shared.clone())?;
                let args = if let Some(start) = start {
                    // SAFETY: The creating and starting arenas retain these objects in traced
                    // thread records until this worker's cleanup has completed. Those arenas
                    // cannot be destroyed while this worker is running.
                    let decode = |ptr: dotnet_value::object::ObjectPtr| unsafe {
                        dotnet_value::object::ObjectRef::from_storage_ptr(
                            ptr.as_ptr().cast_mut().cast(),
                        )
                    };
                    let thread = decode(start.thread);
                    let delegate = decode(start.delegate);
                    let argument = start
                        .argument
                        .map(decode)
                        .unwrap_or(dotnet_value::object::ObjectRef(None));
                    c.stack.local.managed_threads.set_current(thread);
                    c.stack.local.managed_threads.set_current_id(start.id);
                    vec![
                        StackValue::ObjectRef(delegate),
                        StackValue::ObjectRef(argument),
                    ]
                } else {
                    vec![]
                };
                let mut ctx = c.ves_context(gc_handle);

                if let Some(err) = ctx.managed_frame_abort_error(&info) {
                    return Err(err);
                }
                ctx.entrypoint_frame(info, generic_inst, args)?;
                Ok(())
            })
        })??;
        Ok(())
    }

    /// Decides whether the entry point's `StepResult::Return` ends the run.
    ///
    /// A collection requested by the last instruction can queue finalizers after `run` has
    /// already completed the managed entry point. Returns `Ok(None)` to keep the executor alive
    /// for another iteration so the finalizer-processing phase can start those frames before
    /// reporting program exit; otherwise returns the process exit code.
    fn complete_entrypoint_return(&self) -> Result<Option<u8>, VmError> {
        let has_pending_finalizers = self.with_arena_ref(|arena| {
            arena.mutate(|_, c| c.stack.local.heap.has_pending_finalizers())
        })?;
        if has_pending_finalizers {
            return Ok(None);
        }

        self.with_arena_ref(|arena| {
            arena.mutate(
                |_, c| match c.stack.execution.evaluation_stack.stack.first() {
                    Some(StackValue::Int32(i)) => Ok(Some(*i as u8)),
                    Some(v) => Err(VmError::Execution(ExecutionError::TypeMismatch {
                        expected: "Int32",
                        actual: format!("{:?}", v).into(),
                    })),
                    None => Ok(Some(0)),
                },
            )
        })?
    }

    /// Keep the entry arena participating in GC while foreground workers finish.
    /// `run` calls this before returning. Call it separately if entrypoint setup fails.
    pub fn wait_for_foreground_workers(&mut self) {
        let mut stopping_background = false;
        loop {
            // Observe cancellation before the registry lock: a worker either published
            // Finalizing under that lock or observes the cancellation while holding it.
            let aborted = self.shared.is_abort_requested();
            let idle = || aborted || !self.shared.has_pending_finalizers();
            if stopping_background {
                if self.shared.workers.is_quiescent(idle) {
                    break;
                }
            } else if self
                .shared
                .workers
                .foreground_quiescent(idle, || self.shared.abort_signal().request_abort())
            {
                if !self.shared.is_abort_requested() {
                    break;
                }
                // Keep participating in GC until canceled workers have stopped executing.
                stopping_background = true;
                continue;
            }
            #[cfg(feature = "multithreading")]
            if self.shared.thread_manager.is_gc_stop_requested() {
                self.shared
                    .thread_manager
                    .safe_point(self.thread_id, &self.shared.gc_coordinator);
            }
            if !self.shared.is_abort_requested()
                && self
                    .with_arena_ref(|arena| {
                        arena.mutate(|_, c| c.stack.local.heap.has_pending_finalizers())
                    })
                    .unwrap_or(false)
            {
                let result = self.run_inner(true);
                if !matches!(result, ExecutorResult::Exited(_)) {
                    self.shared.abort_signal().request_abort();
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    }

    /// Runs only the entry point with its arguments already on the stack. Unlike [`Self::run`],
    /// it returns as soon as the entry method does, without waiting for workers or finalizers
    /// (a finalizer that never returns would otherwise block it). The caller must call
    /// [`Self::wait_for_foreground_workers`] before dropping the executor.
    pub fn run_entrypoint(&mut self) -> ExecutorResult {
        let result = self.run_inner(false);
        #[cfg(feature = "multithreading")]
        if let ExecutorResult::Threw(exception) = &result {
            // Publish before waiting for workers: a worker may be blocked on a monitor
            // held by this unhandled entry frame, whose finally will not run.
            self.shared.report_fatal_exception(exception.clone());
        }
        result
    }

    /// Runs the entry point with its arguments already on the stack, and waits for foreground
    /// workers before returning. The first unhandled exception takes precedence over its result.
    pub fn run(&mut self) -> ExecutorResult {
        let result = self.run_entrypoint();
        // Foreground workers can throw after the entry method has returned. Keep the entry
        // arena participating in GC until they stop, then return the first fatal exception.
        self.wait_for_foreground_workers();
        #[cfg(feature = "multithreading")]
        if let Some(exception) = self.shared.fatal_exception() {
            return ExecutorResult::Threw(exception);
        }
        result
    }

    fn run_inner(&mut self, finalizers_only: bool) -> ExecutorResult {
        #[cfg(feature = "bench-instrumentation")]
        let metrics = self.shared.metrics.clone();
        #[cfg(feature = "bench-instrumentation")]
        let _metrics_scope = dotnet_metrics::ActiveRuntimeMetricsGuard::enter(&metrics);
        let result = loop {
            if self.shared.is_abort_requested() {
                break ExecutorResult::Error(VmError::Execution(ExecutionError::Aborted(
                    "Requested by user/timeout".into(),
                )));
            }

            #[cfg(feature = "fuzzing")]
            if let Some(budget) = self.instruction_budget.as_mut() {
                if *budget == 0 {
                    break ExecutorResult::Error(VmError::Execution(
                        ExecutionError::FuzzBudgetExceeded,
                    ));
                }
                *budget -= 1;
            }

            // Incremental collection progress. Only collect debt every 8 ticks to reduce overhead.
            #[cfg(not(feature = "multithreading"))]
            {
                self.gc_tick_counter += 1;
                if self.gc_tick_counter.is_multiple_of(8)
                    && let Err(e) = self.with_arena(|arena| {
                        arena.collect_single_threaded(crate::gc::CollectionWork::Debt);
                    })
                {
                    break ExecutorResult::Error(e);
                }
            }

            let (_full_collect, collection_requested) = match self.collection_check() {
                Ok(v) => v,
                Err(e) => break ExecutorResult::Error(e),
            };

            if collection_requested && let Err(e) = self.perform_full_gc() {
                break ExecutorResult::Error(e);
            }
            if collection_requested {
                #[cfg(feature = "multithreading")]
                if let Err(e) = self.with_arena(|arena| {
                    arena.mutate(|_, c| {
                        c.stack
                            .arena
                            .needs_collection()
                            .store(false, Ordering::Release);
                        c.stack
                            .arena
                            .allocation_counter()
                            .store(0, Ordering::Release);
                    })
                }) {
                    break ExecutorResult::Error(e);
                }
            }

            // Reach a safe point between instructions if requested
            // This check is performed on every loop iteration, but `is_gc_stop_requested()`
            // is optimized to be a fast atomic load that returns false in the common case.
            // For further optimization, consider checking only on:
            // - Backward branches (loops)
            // - Method calls/returns
            // - Long-running operations
            #[cfg(feature = "multithreading")]
            if self.shared.thread_manager.is_gc_stop_requested() {
                if let Err(error) = self.release_initializer_waits() {
                    break ExecutorResult::Error(error);
                }
                self.shared
                    .thread_manager
                    .safe_point(self.thread_id, &self.shared.gc_coordinator);
            }

            #[cfg(feature = "memory-validation")]
            let thread_id = self.thread_id;
            let wait = match self
                .with_arena(|arena| {
                    let (has_pending, mut wait) = arena.mutate(|_, c| {
                        let has_pending = c.stack.local.heap.has_pending_finalizers();
                        let processing = c.stack.local.heap.processing_finalizer.get();
                        let wait = c.stack.execution.call_state.native.last().and_then(|work| {
                            work.wait.as_ref().and_then(|wait| match wait {
                                crate::stack::native_effects::PendingWait::Monitor(wait) => {
                                    wait.epoch.map(|epoch| {
                                        NativeWaitSleep::Monitor(
                                            wait.block.clone(),
                                            epoch,
                                            wait.deadline,
                                        )
                                    })
                                }
                                crate::stack::native_effects::PendingWait::MonitorWait(wait) => {
                                    Some(NativeWaitSleep::Monitor(
                                        wait.wait.block.clone(),
                                        wait.epoch,
                                        wait.wait.sleep_deadline(),
                                    ))
                                }
                                crate::stack::native_effects::PendingWait::Finalizers(_)
                                    if has_pending
                                        || processing
                                        || c.stack.shared.has_pending_finalizers() =>
                                {
                                    Some(NativeWaitSleep::Finalizers)
                                }
                                crate::stack::native_effects::PendingWait::Finalizers(_) => None,
                                crate::stack::native_effects::PendingWait::Sleep(_, deadline) => {
                                    Some(NativeWaitSleep::Sleep(*deadline))
                                }
                                crate::stack::native_effects::PendingWait::Initializer(_, wait) => {
                                    (!wait.ready())
                                        .then(|| NativeWaitSleep::Initializer(wait.sleeper()))
                                }
                            })
                        });
                        (has_pending, wait)
                    });
                    if has_pending {
                        let result = arena.mutate_root(|gc, c| {
                            let gc_handle = GCHandle::new(
                                gc,
                                #[cfg(feature = "multithreading")]
                                // SAFETY: F1.GcHandleRooted — Finalizer processing runs inside this arena's mutate_root
                                // closure, so the inner GC handle belongs to the same arena.
                                unsafe {
                                    c.stack.arena_inner_gc()
                                },
                                #[cfg(feature = "memory-validation")]
                                thread_id,
                            );
                            c.process_pending_finalizers(gc_handle)
                        });
                        match result {
                            StepResult::Error(error) => return Err(error),
                            StepResult::FramePushed => wait = None,
                            _ => {}
                        }
                        if matches!(wait, Some(NativeWaitSleep::Finalizers))
                            && arena.mutate(|_, c| {
                                !c.stack.local.heap.has_pending_finalizers()
                                    && !c.stack.local.heap.processing_finalizer.get()
                                    && !c.stack.shared.has_pending_finalizers()
                            })
                        {
                            wait = None;
                        }
                    }
                    Ok(wait)
                })
                .and_then(|result| result)
            {
                Ok(wait) => wait,
                Err(error) => break ExecutorResult::Error(error),
            };

            if finalizers_only {
                let done = match self.with_arena_ref(|arena| {
                    arena.mutate(|_, c| {
                        c.stack.parked_finalizer_caller.is_none()
                            && !c.stack.local.heap.has_pending_finalizers()
                    })
                }) {
                    Ok(done) => done,
                    Err(error) => break ExecutorResult::Error(error),
                };
                if done {
                    break ExecutorResult::Exited(0);
                }
            }

            // Finalizer work is scheduled first; the returned `NativeWaitSleep` holds no arena
            // lease or branded value, so the actual wait cannot keep a mutation or lease live.
            if let Some(wait) = wait {
                #[cfg(test)]
                WAIT_TEST_HOOK.with(|hook| {
                    if let Some(callback) = hook.borrow_mut().take() {
                        callback();
                    }
                });
                let interval = crate::sync::get_safe_point_yield_duration().clamp(
                    std::time::Duration::from_millis(1),
                    std::time::Duration::from_millis(10),
                );
                match wait {
                    NativeWaitSleep::Monitor(block, epoch, deadline) => {
                        let duration = deadline.map_or(interval, |deadline| {
                            deadline
                                .saturating_duration_since(std::time::Instant::now())
                                .min(interval)
                        });
                        if !duration.is_zero() {
                            block.wait_for_release(epoch, duration);
                        }
                    }
                    NativeWaitSleep::Finalizers => std::thread::sleep(interval),
                    NativeWaitSleep::Sleep(deadline) => {
                        let duration = deadline.map_or(interval, |d| {
                            d.saturating_duration_since(std::time::Instant::now())
                                .min(interval)
                        });
                        if !duration.is_zero() {
                            std::thread::sleep(duration);
                        }
                    }
                    NativeWaitSleep::Initializer(sleeper) => sleeper.wait(interval),
                }
            }

            #[cfg(feature = "memory-validation")]
            let thread_id = self.thread_id;
            let step_result = match self.with_arena(|arena| {
                arena.mutate_root(|gc, c| {
                    let gc_handle = GCHandle::new(
                        gc,
                        #[cfg(feature = "multithreading")]
                        // SAFETY: F1.GcHandleRooted — Instruction stepping runs inside this arena's mutate_root
                        // closure, so the inner GC handle belongs to the same arena.
                        unsafe {
                            c.stack.arena_inner_gc()
                        },
                        #[cfg(feature = "memory-validation")]
                        thread_id,
                    );
                    c.run(gc_handle)
                })
            }) {
                Ok(v) => v,
                Err(e) => break ExecutorResult::Error(e),
            };

            match step_result {
                StepResult::Return => match self.complete_entrypoint_return() {
                    Ok(Some(exit_code)) => break ExecutorResult::Exited(exit_code),
                    Ok(None) => continue,
                    Err(e) => break ExecutorResult::Error(e),
                },
                StepResult::MethodThrew(exc) => {
                    break ExecutorResult::Threw(exc);
                }
                StepResult::Error(e) => {
                    break ExecutorResult::Error(e);
                }
                _ => {}
            }

            {
                let gc_bytes =
                    match self.with_arena_ref(|arena| arena.metrics().total_gc_allocation()) {
                        Ok(v) => v,
                        Err(e) => break ExecutorResult::Error(e),
                    };

                #[cfg(feature = "multithreading")]
                {
                    // Update the arena's metrics directly
                    if let Err(e) = self.with_arena(|arena| {
                        arena.mutate(|_, c| {
                            c.stack
                                .arena
                                .gc_allocated_bytes()
                                .store(gc_bytes, Ordering::Relaxed);
                        })
                    }) {
                        break ExecutorResult::Error(e);
                    }

                    // Update global metrics from aggregated values
                    let total_gc = self.shared.gc_coordinator.total_gc_allocation();
                    self.shared.metrics.update_gc_metrics(total_gc as u64);
                }
                #[cfg(not(feature = "multithreading"))]
                {
                    self.shared.metrics.update_gc_metrics(gc_bytes as u64);
                }
            }
        };

        if let Err(error) = self.with_arena(|arena| {
            arena.mutate_root(|_, c| {
                c.retire_finalizer_segment();
                let execution = &mut c.stack.execution;
                for frame in execution
                    .frame_stack
                    .frames
                    .iter_mut()
                    .chain(execution.frame_stack.suspended_frames.iter_mut())
                    .chain(execution.frame_stack.pooled_frames.iter_mut())
                {
                    frame.monitor_retry = None;
                    frame.monitor_wait_retry = None;
                }
                execution
                    .call_state
                    .cleanup_native(&mut execution.frame_stack, &mut execution.evaluation_stack);
            });
        }) {
            return ExecutorResult::Error(error);
        }
        // Thread unregistration is handled by `Executor`'s `Drop` impl,
        // which runs when the Executor is dropped after this function returns.
        self.shared.tracer.flush();
        result
    }

    fn release_initializer_waits(&mut self) -> Result<(), VmError> {
        self.with_arena(|arena| {
            arena.mutate_root(|_, engine| {
                engine
                    .stack
                    .execution
                    .call_state
                    .release_initializer_waits()?;
                if let Some(parked) = engine.stack.parked_finalizer_caller.as_mut() {
                    parked.call_state.release_initializer_waits()?;
                }
                Ok(())
            })
        })?
    }

    /// Perform a full GC collection with stop-the-world coordination.
    fn perform_full_gc(&mut self) -> Result<(), VmError> {
        self.release_initializer_waits()?;
        #[cfg(feature = "multithreading")]
        {
            use std::time::Instant;
            let start_time = Instant::now();

            vm_trace_gc_collection_start!(self, 0, "allocation pressure");

            // 1. Enter a typestate collection session. Drop restores Idle if
            //    we unwind before the explicit `finish()` below.
            let coordinator = Arc::clone(&self.shared.gc_coordinator);
            let Some(session) = coordinator.begin_collection() else {
                return Ok(()); // Another thread is already collecting
            };

            self.with_arena(|arena| {
                arena.mutate(|_, c| {
                    vm_debug!(c.stack, "GC: Coordinated stop-the-world collection started");
                });
            })?;

            // 2. Request stop-the-world pause and bind it with the collection
            //    session into a single guard that enforces drop order:
            //    session cleanup before thread resume.
            let thread_manager = Arc::clone(&self.shared.thread_manager);
            let stw_guard = thread_manager.request_stop_the_world();
            let gc_cycle = GcCycleGuard::new(session, stw_guard);

            // 3. Perform coordinated GC across all arenas
            gc_cycle.collect_all_arenas(self.thread_id);

            // 4. End the STW cycle before recording pause metrics so the sample
            //    covers the full pause boundary, including coordinator cleanup
            //    and thread-resume handoff.
            drop(gc_cycle);
            let duration = start_time.elapsed();
            self.shared.metrics.record_gc_pause(duration);

            vm_trace_gc_collection_end!(self, 0, 0, duration.as_micros() as u64);

            self.with_arena(|arena| {
                arena.mutate(|_, c| {
                    vm_debug!(
                        c.stack,
                        "GC: Coordinated collection completed in {:?}",
                        duration
                    );
                });
            })?;
        }
        #[cfg(not(feature = "multithreading"))]
        {
            use std::time::Instant;
            let start_time = Instant::now();
            vm_trace_gc_collection_start!(self, 0, "allocation pressure");

            if !self.shared.is_abort_requested() {
                // Perform full collection with finalization (mimics multithreading behavior)
                self.with_arena(|arena| {
                    arena.collect_single_threaded(crate::gc::CollectionWork::Full);
                })?;
            }

            let duration = start_time.elapsed();
            self.shared.metrics.record_gc_pause(duration);
            vm_trace_gc_collection_end!(self, 0, 0, duration.as_micros() as u64);
        }
        Ok(())
    }
}

impl Executor {
    /// Extract this owner's arena at most once. The thread-affine guard may
    /// outlive the Executor, but cannot release or access its TLS reservation.
    /// The reservation remains until Executor Drop, even if the guard drops first.
    /// Further arena operations fail after extraction. Guard Drop frees storage
    /// only after an exact completed lease drain; interrupted cleanup retains it.
    #[cfg(feature = "multithreading")]
    pub fn extract_arena(&self) -> Option<ArenaGuard> {
        let arena = self.owner.take()?;
        Some(ArenaGuard {
            arena: Some(arena),
            registration: self.registration.borrow_mut().take(),
            _affinity: std::marker::PhantomData,
        })
    }
}

/// Thread-affine extracted storage. Drop drains its original registration,
/// retaining storage if draining cannot complete (including during unwind).
#[cfg(feature = "multithreading")]
pub struct ArenaGuard {
    arena: Option<Box<GCArena>>,
    registration: Option<dotnet_utils::gc::ArenaRegistration>,
    _affinity: std::marker::PhantomData<std::rc::Rc<()>>,
}

/// Armed before any fallible cleanup: incomplete teardown retains storage,
/// including on unwind. No destructor or logging is needed to retain it.
#[cfg(feature = "multithreading")]
struct RetainedArena(std::mem::ManuallyDrop<Box<GCArena>>);

#[cfg(feature = "multithreading")]
impl RetainedArena {
    fn new(arena: Box<GCArena>) -> Self {
        Self(std::mem::ManuallyDrop::new(arena))
    }

    fn drain(self, registration: Option<dotnet_utils::gc::ArenaRegistration>) -> bool {
        // INVARIANT: F1.ExecutorArenaRetained — only an exact, completed drain
        // disarms retention. Missing handles and interrupted drains leak.
        if let Some(registration) = registration {
            #[cfg(test)]
            ownership_checkpoint("drain");
            if registration.unregister_and_drain() == dotnet_utils::gc::TeardownOutcome::Drained {
                drop(std::mem::ManuallyDrop::into_inner(self.0));
                return true;
            }
        }
        false
    }
}

#[cfg(all(test, feature = "multithreading"))]
fn ownership_checkpoint(_point: &'static str) {
    OWNERSHIP_PANIC.with(|fault| {
        if fault.get() == Some(_point) {
            fault.set(None);
            panic!("injected ownership panic at {_point}");
        }
    });
}

#[cfg(all(test, feature = "multithreading"))]
thread_local! {
    static OWNERSHIP_PANIC: std::cell::Cell<Option<&'static str>> = const { std::cell::Cell::new(None) };
}

#[cfg(feature = "multithreading")]
impl Drop for ArenaGuard {
    fn drop(&mut self) {
        let retained = self.arena.take().map(RetainedArena::new);
        #[cfg(test)]
        ownership_checkpoint("guard");
        if let Some(retained) = retained {
            retained.0.mutate(|_, c| {
                c.stack
                    .shared
                    .publish_finalizers(&c.stack.local.heap, false);
            });
            let _ = retained.drain(self.registration.take());
        }
    }
}

impl Executor {
    fn unregister(&mut self) {
        if !self.registered {
            return;
        }
        let _ = self.with_arena_ref(|arena| {
            arena.mutate(|_, c| {
                c.stack
                    .shared
                    .publish_finalizers(&c.stack.local.heap, false);
            })
        });
        #[cfg(feature = "multithreading")]
        self.shared.gc_coordinator.unregister_arena(self.thread_id);

        crate::threading::IS_PERFORMING_GC.set(false);
        clear_tracing_state();
        self.shared.thread_manager.unregister_thread(self.thread_id);
        self.registered = false;
    }
}

impl Drop for Executor {
    fn drop(&mut self) {
        let _ = self.with_arena_ref(|arena| {
            arena.mutate(|_, c| {
                c.stack
                    .shared
                    .publish_finalizers(&c.stack.local.heap, false);
            })
        });
        #[cfg(feature = "multithreading")]
        let retained = self.owner.take().map(RetainedArena::new);
        #[cfg(all(test, feature = "multithreading"))]
        ownership_checkpoint("executor");
        self.unregister();
        #[cfg(feature = "multithreading")]
        if let Some(retained) = retained {
            let _ = retained.drain(self.registration.get_mut().take());
        }
    }
}

#[cfg(test)]
#[path = "executor_teardown_tests.rs"]
mod executor_teardown_tests;

#[cfg(test)]
#[path = "executor_collection_tests.rs"]
mod executor_collection_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use dotnet_assemblies::AssemblyLoader;
    use dotnet_value::object::ObjectRef;

    /// Guards the pending-finalizer branch of the entry-point return: a collection requested by
    /// the final instruction can queue finalizers after `run` completed `Main`, and the executor
    /// must keep looping so they start before exit. A managed fixture cannot reach this branch
    /// (the collect intrinsic yields and drains finalizers before `ret` executes), so the
    /// pending state is seeded directly.
    #[test]
    fn entrypoint_return_defers_exit_while_finalizers_pending() {
        #[allow(
            clippy::arc_with_non_send_sync,
            reason = "the executor unit test keeps shared state on its sole executor"
        )]
        let shared = Arc::new(SharedGlobalState::new(Arc::new(
            AssemblyLoader::new_bare("mock_root_executor".to_string())
                .expect("Failed to create mock AssemblyLoader"),
        )));
        let mut executor = Executor::new(shared);

        assert_eq!(
            executor.complete_entrypoint_return().unwrap(),
            Some(0),
            "no pending finalizers: the entry point return must report exit"
        );

        executor
            .with_arena(|arena| {
                arena.mutate(|_, c| {
                    c.stack
                        .local
                        .heap
                        .pending_finalization
                        .borrow_mut()
                        .push(ObjectRef(None));
                })
            })
            .unwrap();
        assert_eq!(
            executor.complete_entrypoint_return().unwrap(),
            None,
            "pending finalizers must keep the executor running instead of exiting"
        );

        executor
            .with_arena(|arena| {
                arena.mutate(|_, c| c.stack.local.heap.pending_finalization.borrow_mut().clear())
            })
            .unwrap();
        assert_eq!(executor.complete_entrypoint_return().unwrap(), Some(0));
    }
}

#[cfg(test)]
#[path = "native_substring_tests.rs"]
pub(crate) mod native_substring_tests;

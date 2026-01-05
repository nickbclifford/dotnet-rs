use crate::{
    types::members::MethodDescription,
    value::StackValue,
    vm::{
        arena_storage::THREAD_ARENA,
        gc_coordinator::ArenaHandle,
        stack::{ArenaLocalState, CallStack, GCArena, SharedGlobalState},
        MethodInfo, StepResult,
    },
    vm_msg,
};
use parking_lot::{Condvar, Mutex};
use std::sync::{atomic::AtomicBool, atomic::AtomicUsize, Arc};

pub struct Executor {
    shared: Arc<SharedGlobalState<'static>>,
    /// Thread ID for this executor
    thread_id: u64,
}

#[derive(Clone, Debug)]
pub enum ExecutorResult {
    Exited(u8),
    Threw, // TODO: well-typed exceptions
}

impl Executor {
    pub fn new(shared: Arc<SharedGlobalState<'static>>) -> Self {
        let shared_clone = Arc::clone(&shared);
        let arena = Box::new(GCArena::new(|gc| {
            let local = ArenaLocalState::new();
            CallStack::new(gc, shared_clone, local)
        }));

        let thread_id = shared.thread_manager.register_thread();

        // Register with GC coordinator
        let handle = ArenaHandle {
            thread_id,
            allocation_counter: Arc::new(AtomicUsize::new(0)),
            needs_collection: Arc::new(AtomicBool::new(false)),
            current_command: Arc::new(Mutex::new(None)),
            command_signal: Arc::new(Condvar::new()),
            finish_signal: Arc::new(Condvar::new()),
        };
        shared.gc_coordinator.register_arena(handle.clone());
        crate::vm::gc_coordinator::set_current_arena_handle(handle);

        THREAD_ARENA.with(|cell| {
            *cell.borrow_mut() = Some(arena);
        });

        // Set thread id in arena
        THREAD_ARENA.with(|cell| {
            let mut arena_opt = cell.borrow_mut();
            let arena = arena_opt.as_mut().unwrap();
            arena.mutate(|_, c| {
                c.thread_id.set(thread_id);
            });
        });

        Self { shared, thread_id }
    }

    pub fn entrypoint(&mut self, method: MethodDescription) {
        // TODO: initialize argv (entry point args are either string[] or nothing, II.15.4.1.2)
        THREAD_ARENA.with(|cell| {
            let mut arena_opt = cell.borrow_mut();
            let arena = arena_opt.as_mut().unwrap();
            arena.mutate_root(|gc, c| {
                c.entrypoint_frame(
                    gc,
                    MethodInfo::new(method, &Default::default(), c.loader()),
                    Default::default(),
                    vec![],
                )
            });
        });
    }

    // assumes args are already on stack
    pub fn run(&mut self) -> ExecutorResult {
        let result = loop {
            // Perform incremental GC progress with finalization support
            // In a real VM this would be tuned based on allocation pressure
            THREAD_ARENA.with(|cell| {
                let mut arena_opt = cell.borrow_mut();
                let arena = arena_opt.as_mut().unwrap();
                if let Some(marked) = arena.mark_debt() {
                    marked.finalize(|fc, c| c.finalize_check(fc));
                }
            });

            let full_collect = THREAD_ARENA.with(|cell| {
                let mut arena_opt = cell.borrow_mut();
                let arena = arena_opt.as_mut().unwrap();
                arena.mutate(|_, c| {
                    if c.heap().needs_full_collect.get() {
                        c.heap().needs_full_collect.set(false);
                        true
                    } else {
                        false
                    }
                })
            });

            if full_collect || crate::vm::gc_coordinator::is_current_arena_collection_requested() {
                // For multithreading: coordinate stop-the-world pause
                self.perform_full_gc();
                crate::vm::gc_coordinator::reset_current_arena_collection_requested();
            }

            // Reach a safe point between instructions if requested
            if self.shared.thread_manager.is_gc_stop_requested() {
                self.shared
                    .thread_manager
                    .safe_point(self.thread_id, &self.shared.gc_coordinator);
            }

            THREAD_ARENA.with(|cell| {
                let mut arena_opt = cell.borrow_mut();
                let arena = arena_opt.as_mut().unwrap();
                arena.mutate_root(|gc, c| c.process_pending_finalizers(gc));
            });

            let step_result = THREAD_ARENA.with(|cell| {
                let mut arena_opt = cell.borrow_mut();
                let arena = arena_opt.as_mut().unwrap();
                arena.mutate_root(|gc, c| c.step(gc))
            });

            let frames_empty = || THREAD_ARENA.with(|cell| {
                let arena_opt = cell.borrow();
                let arena = arena_opt.as_ref().unwrap();
                arena.mutate(|_, c| c.execution.frames.is_empty())
            });

            match step_result {
                StepResult::MethodReturned => {
                    let was_auto_invoked = THREAD_ARENA.with(|cell| {
                        let mut arena_opt = cell.borrow_mut();
                        let arena = arena_opt.as_mut().unwrap();
                        arena.mutate_root(|gc, c| {
                            let frame = c.execution.frames.last().unwrap();
                            let val = frame.state.info_handle.is_cctor || frame.is_finalizer;
                            c.return_frame(gc);
                            val
                        })
                    });

                    if frames_empty() {
                        let exit_code = THREAD_ARENA.with(|cell| {
                            let arena_opt = cell.borrow();
                            let arena = arena_opt.as_ref().unwrap();
                            arena.mutate(|_, c| match c.bottom_of_stack() {
                                Some(StackValue::Int32(i)) => i as u8,
                                Some(v) => panic!("invalid value for entrypoint return: {:?}", v),
                                None => 0,
                            })
                        });
                        break ExecutorResult::Exited(exit_code);
                    } else if !was_auto_invoked {
                        // step the caller past the call instruction
                        THREAD_ARENA.with(|cell| {
                            let mut arena_opt = cell.borrow_mut();
                            let arena = arena_opt.as_mut().unwrap();
                            arena.mutate_root(|_, c| c.increment_ip());
                        });
                    }
                }
                StepResult::MethodThrew => {
                    THREAD_ARENA.with(|cell| {
                        let mut arena_opt = cell.borrow_mut();
                        let arena = arena_opt.as_mut().unwrap();
                        arena.mutate(|_, c| {
                            vm_msg!(c, "Exception thrown: {:?}", c.execution.exception_mode);
                        });
                    });
                    if frames_empty() {
                        break ExecutorResult::Threw;
                    }
                }
                StepResult::InstructionStepped | StepResult::YieldForGC => {}
            }
            // TODO(gc): poll arena for stats
        };

        // Unregister thread when execution completes
        self.shared.thread_manager.unregister_thread(self.thread_id);

        self.shared.tracer.lock().flush();
        result
    }

    /// Perform a full GC collection with stop-the-world coordination.
    fn perform_full_gc(&mut self) {
        use std::time::Instant;
        let start_time = Instant::now();

        // 1. Mark that we are starting collection (acquires coordinator lock)
        let _gc_lock = match self.shared.gc_coordinator.start_collection() {
            Some(guard) => guard,
            None => return, // Another thread is already collecting
        };

        THREAD_ARENA.with(|cell| {
            let mut arena_opt = cell.borrow_mut();
            let arena = arena_opt.as_mut().unwrap();
            arena.mutate(|_, c| {
                vm_msg!(c, "GC: Coordinated stop-the-world collection started");
            });
        });

        // 2. Request stop-the-world pause and wait for all threads to reach safe points
        let _stw_guard = self.shared.thread_manager.request_stop_the_world();

        // 3. Perform coordinated GC across all arenas
        self.shared
            .gc_coordinator
            .collect_all_arenas(self.thread_id);

        // 4. Record metrics and log completion
        let duration = start_time.elapsed();
        self.shared.metrics.record_gc_pause(duration);

        // Mark collection as finished (releases coordinator lock)
        self.shared.gc_coordinator.finish_collection();

        THREAD_ARENA.with(|cell| {
            let mut arena_opt = cell.borrow_mut();
            let arena = arena_opt.as_mut().unwrap();
            arena.mutate(|_, c| {
                vm_msg!(c, "GC: Coordinated collection completed in {:?}", duration);
            });
        });
    }
}

impl Drop for Executor {
    fn drop(&mut self) {
        // Ensure thread is unregistered if not already done
        self.shared.thread_manager.unregister_thread(self.thread_id);

        self.shared.gc_coordinator.unregister_arena(self.thread_id);
    }
}

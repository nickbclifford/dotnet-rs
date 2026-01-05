use crate::{
    types::members::MethodDescription,
    value::StackValue,
    vm::{
        stack::{ArenaLocalState, CallStack, GCArena, SharedGlobalState},
        MethodInfo, StepResult,
    },
    vm_msg,
};

#[cfg(feature = "multithreaded-gc")]
use crate::vm::{arena_storage::THREAD_ARENA, gc_coordinator::ArenaHandle};

#[cfg(feature = "multithreaded-gc")]
use crate::vm::sync::{AtomicBool, AtomicUsize, Condvar, Mutex};

use crate::vm::sync::Arc;

pub struct Executor {
    shared: Arc<SharedGlobalState<'static>>,
    /// Thread ID for this executor
    thread_id: u64,
    #[cfg(not(feature = "multithreaded-gc"))]
    arena: Box<crate::vm::stack::GCArena>,
}

#[derive(Clone, Debug)]
pub enum ExecutorResult {
    Exited(u8),
    Threw, // TODO: well-typed exceptions
}

impl Executor {
    fn with_arena<R>(&mut self, f: impl FnOnce(&mut crate::vm::stack::GCArena) -> R) -> R {
        #[cfg(feature = "multithreaded-gc")]
        {
            THREAD_ARENA.with(|cell| {
                let mut arena_opt = cell.borrow_mut();
                let arena = arena_opt.as_mut().expect("Thread arena not initialized");
                f(arena)
            })
        }
        #[cfg(not(feature = "multithreaded-gc"))]
        {
            f(&mut self.arena)
        }
    }

    fn with_arena_ref<R>(&self, f: impl FnOnce(&crate::vm::stack::GCArena) -> R) -> R {
        #[cfg(feature = "multithreaded-gc")]
        {
            THREAD_ARENA.with(|cell| {
                let arena_opt = cell.borrow();
                let arena = arena_opt.as_ref().expect("Thread arena not initialized");
                f(arena)
            })
        }
        #[cfg(not(feature = "multithreaded-gc"))]
        {
            f(&self.arena)
        }
    }

    pub fn new(shared: Arc<SharedGlobalState<'static>>) -> Self {
        let shared_clone = Arc::clone(&shared);
        let arena = Box::new(GCArena::new(|gc| {
            let local = ArenaLocalState::new();
            CallStack::new(gc, shared_clone, local)
        }));

        #[cfg(feature = "multithreading")]
        let thread_id = shared.thread_manager.register_thread();
        #[cfg(not(feature = "multithreading"))]
        let thread_id = 1u64;

        // Register with GC coordinator
        #[cfg(feature = "multithreaded-gc")]
        {
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
        }

        #[cfg(feature = "multithreaded-gc")]
        THREAD_ARENA.with(|cell| {
            *cell.borrow_mut() = Some(arena);
        });

        // Set thread id in arena
        #[cfg(feature = "multithreaded-gc")]
        THREAD_ARENA.with(|cell| {
            let mut arena_opt = cell.borrow_mut();
            let arena = arena_opt.as_mut().unwrap();
            arena.mutate(|_, c| {
                c.thread_id.set(thread_id);
            });
        });

        #[cfg(not(feature = "multithreaded-gc"))]
        arena.mutate(|_, c| {
            c.thread_id.set(thread_id);
        });

        Self {
            shared,
            thread_id,
            #[cfg(not(feature = "multithreaded-gc"))]
            arena,
        }
    }

    pub fn entrypoint(&mut self, method: MethodDescription) {
        // TODO: initialize argv (entry point args are either string[] or nothing, II.15.4.1.2)
        #[cfg(feature = "multithreaded-gc")]
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

        #[cfg(not(feature = "multithreaded-gc"))]
        self.arena.mutate_root(|gc, c| {
            c.entrypoint_frame(
                gc,
                MethodInfo::new(method, &Default::default(), c.loader()),
                Default::default(),
                vec![],
            )
        });
    }

    // assumes args are already on stack
    pub fn run(&mut self) -> ExecutorResult {
        let result = loop {
            // Perform incremental GC progress with finalization support
            // In a real VM this would be tuned based on allocation pressure
            #[cfg(feature = "multithreaded-gc")]
            self.with_arena(|arena| {
                if let Some(marked) = arena.mark_debt() {
                    marked.finalize(|fc, c| c.finalize_check(fc));
                }
            });
            #[cfg(not(feature = "multithreaded-gc"))]
            if let Some(marked) = self.arena.mark_debt() {
                marked.finalize(|fc, c| c.finalize_check(fc));
            }

            let full_collect = {
                #[cfg(feature = "multithreaded-gc")]
                {
                    self.with_arena(|arena| {
                        arena.mutate(|_, c| {
                            if c.heap().needs_full_collect.get() {
                                c.heap().needs_full_collect.set(false);
                                true
                            } else {
                                false
                            }
                        })
                    })
                }
                #[cfg(not(feature = "multithreaded-gc"))]
                {
                    self.arena.mutate(|_, c| {
                        if c.heap().needs_full_collect.get() {
                            c.heap().needs_full_collect.set(false);
                            true
                        } else {
                            false
                        }
                    })
                }
            };

            #[cfg(feature = "multithreaded-gc")]
            if full_collect || crate::vm::gc_coordinator::is_current_arena_collection_requested() {
                // For multithreading: coordinate stop-the-world pause
                self.perform_full_gc();
                crate::vm::gc_coordinator::reset_current_arena_collection_requested();
            }

            #[cfg(not(feature = "multithreaded-gc"))]
            if full_collect {
                // Perform full collection with finalization (mimics multithreaded-gc behavior)
                let mut marked = None;
                while marked.is_none() {
                    marked = self.arena.mark_all();
                }
                if let Some(marked) = marked {
                    marked.finalize(|fc, c| c.finalize_check(fc));
                }
                self.arena.collect_all();
            }

            // Reach a safe point between instructions if requested
            #[cfg(feature = "multithreading")]
            if self.shared.thread_manager.is_gc_stop_requested() {
                #[cfg(feature = "multithreaded-gc")]
                self.shared
                    .thread_manager
                    .safe_point(self.thread_id, &self.shared.gc_coordinator);

                #[cfg(not(feature = "multithreaded-gc"))]
                self.shared
                    .thread_manager
                    .safe_point(self.thread_id, &Default::default()); // Use default GCCoordinator stub
            }

            self.with_arena(|arena| {
                arena.mutate_root(|gc, c| c.process_pending_finalizers(gc));
            });

            let step_result = self.with_arena(|arena| {
                arena.mutate_root(|gc, c| c.step(gc))
            });

            let frames_empty = |executor: &Self| {
                executor.with_arena_ref(|arena| {
                    arena.mutate(|_, c| c.execution.frames.is_empty())
                })
            };

            match step_result {
                StepResult::MethodReturned => {
                    let was_auto_invoked = self.with_arena(|arena| {
                        arena.mutate_root(|gc, c| {
                            let frame = c.execution.frames.last().unwrap();
                            let val = frame.state.info_handle.is_cctor || frame.is_finalizer;
                            c.return_frame(gc);
                            val
                        })
                    });

                    if frames_empty(self) {
                        let exit_code = self.with_arena_ref(|arena| {
                            arena.mutate(|_, c| match c.bottom_of_stack() {
                                Some(StackValue::Int32(i)) => i as u8,
                                Some(v) => panic!("invalid value for entrypoint return: {:?}", v),
                                None => 0,
                            })
                        });
                        break ExecutorResult::Exited(exit_code);
                    } else if !was_auto_invoked {
                        // step the caller past the call instruction
                        self.with_arena(|arena| {
                            arena.mutate_root(|_, c| c.increment_ip());
                        });
                    }
                }
                StepResult::MethodThrew => {
                    self.with_arena(|arena| {
                        arena.mutate(|_, c| {
                            vm_msg!(c, "Exception thrown: {:?}", c.execution.exception_mode);
                        });
                    });
                    if frames_empty(self) {
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
    #[cfg(feature = "multithreaded-gc")]
    fn perform_full_gc(&mut self) {
        use std::time::Instant;
        let start_time = Instant::now();

        // 1. Mark that we are starting collection (acquires coordinator lock)
        let coordinator = Arc::clone(&self.shared.gc_coordinator);
        let _gc_lock = match coordinator.start_collection() {
            Some(guard) => guard,
            None => return, // Another thread is already collecting
        };

        self.with_arena(|arena| {
            arena.mutate(|_, c| {
                vm_msg!(c, "GC: Coordinated stop-the-world collection started");
            });
        });

        // 2. Request stop-the-world pause and wait for all threads to reach safe points
        let thread_manager = Arc::clone(&self.shared.thread_manager);
        let _stw_guard = thread_manager.request_stop_the_world();

        // 3. Perform coordinated GC across all arenas
        self.shared
            .gc_coordinator
            .collect_all_arenas(self.thread_id);

        // 4. Record metrics and log completion
        let duration = start_time.elapsed();
        self.shared.metrics.record_gc_pause(duration);

        // Mark collection as finished (releases coordinator lock)
        self.shared.gc_coordinator.finish_collection();

        self.with_arena(|arena| {
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

        #[cfg(feature = "multithreaded-gc")]
        self.shared.gc_coordinator.unregister_arena(self.thread_id);
    }
}

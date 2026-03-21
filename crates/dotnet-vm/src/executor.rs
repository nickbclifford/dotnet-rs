use crate::{
    StepResult,
    dispatch::ExecutionEngine,
    error::{ExecutionError, VmError},
    exceptions::ManagedException,
    gc::coordinator::*,
    metrics::CacheStats,
    stack::{
        CallStack, GCArena,
        ops::{CallOps, ReflectionOps, VesOps},
    },
    state::{ArenaLocalState, SharedGlobalState},
    threading::ThreadManagerOps,
    tracer::Tracer,
};
use dotnet_types::members::MethodDescription;
use dotnet_utils::{
    ArenaId,
    gc::GCHandle,
    sync::{Arc, Ordering},
};
use dotnet_value::StackValue;

#[cfg(feature = "multithreading")]
use crate::gc::arena::THREAD_ARENA;

pub struct Executor {
    shared: Arc<SharedGlobalState>,
    /// Thread ID for this executor
    thread_id: ArenaId,
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
    fn with_arena<R>(&mut self, f: impl FnOnce(&mut GCArena) -> R) -> R {
        #[cfg(feature = "multithreading")]
        {
            THREAD_ARENA.with(|cell| {
                let mut arena_opt = cell.borrow_mut();
                let arena = arena_opt.as_mut().expect("Thread arena not initialized");
                f(arena)
            })
        }
        #[cfg(not(feature = "multithreading"))]
        {
            f(&mut self.arena)
        }
    }

    fn with_arena_ref<R>(&self, f: impl FnOnce(&GCArena) -> R) -> R {
        #[cfg(feature = "multithreading")]
        {
            THREAD_ARENA.with(|cell| {
                let arena_opt = cell.borrow();
                let arena = arena_opt.as_ref().expect("Thread arena not initialized");
                f(arena)
            })
        }
        #[cfg(not(feature = "multithreading"))]
        {
            f(&self.arena)
        }
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

    pub fn dump_last_instructions(&self) -> String {
        self.shared.last_instructions.lock().unwrap().dump()
    }

    pub fn new(shared: Arc<SharedGlobalState>) -> Self {
        let shared_clone = Arc::clone(&shared);
        let mut arena = Box::new(GCArena::new(|_| {
            let local = ArenaLocalState::new(shared_clone.statics.clone());
            ExecutionEngine::new(CallStack::new(shared_clone, local))
        }));

        let thread_id = shared.thread_manager.register_thread();

        // Register with GC coordinator
        #[cfg(feature = "multithreading")]
        {
            let handle = ArenaHandle::new(thread_id);
            shared.gc_coordinator.register_arena(handle.clone());
        }

        // Set thread id in arena and pre-initialize reflection
        arena.mutate_root(|gc, c| {
            c.stack.thread_id.set(thread_id);
            #[cfg(feature = "multithreading")]
            {
                c.stack.arena = ArenaHandle::new(thread_id);
            }

            let gc_handle = GCHandle::new(
                gc,
                #[cfg(feature = "multithreading")]
                // SAFETY: `arena_inner_gc` is only used while mutating this thread's arena
                // root, where the arena handle and GC token are in sync.
                unsafe {
                    c.stack.arena_inner_gc()
                },
                #[cfg(feature = "memory-validation")]
                thread_id,
            );

            let mut ctx = c.stack.ves_context(gc_handle);
            ctx.pre_initialize_reflection();
        });

        #[cfg(feature = "multithreading")]
        THREAD_ARENA.with(|cell| {
            *cell.borrow_mut() = Some(arena);
        });

        Self {
            shared,
            thread_id,
            #[cfg(not(feature = "multithreading"))]
            arena,
            #[cfg(feature = "fuzzing")]
            instruction_budget: None,
            #[cfg(not(feature = "multithreading"))]
            gc_tick_counter: 0,
        }
    }

    pub fn entrypoint(&mut self, method: MethodDescription) {
        // TODO: initialize argv (entry point args are either string[] or nothing, II.15.4.1.2)
        #[cfg(feature = "memory-validation")]
        let thread_id = self.thread_id;
        self.with_arena(|arena| {
            arena.mutate_root(|gc, c| {
                let gc_handle = GCHandle::new(
                    gc,
                    #[cfg(feature = "multithreading")]
                    // SAFETY: `arena_inner_gc` is only used while mutating this thread's
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
                    .get_method_info(method, &Default::default(), shared.clone())
                    .expect("Failed to resolve entrypoint");
                c.ves_context(gc_handle)
                    .entrypoint_frame(info, Default::default(), vec![])
                    .expect("Failed to set up entrypoint frame")
            });
        });
    }

    // assumes args are already on stack
    pub fn run(&mut self) -> ExecutorResult {
        let result = loop {
            if self.shared.abort_requested.load(Ordering::Relaxed) {
                break ExecutorResult::Error(VmError::Execution(ExecutionError::Aborted(
                    "Requested by user/timeout".to_string(),
                )));
            }

            #[cfg(feature = "fuzzing")]
            if let Some(budget) = self.instruction_budget.as_mut() {
                if *budget == 0 {
                    return ExecutorResult::Error(VmError::Execution(
                        ExecutionError::FuzzBudgetExceeded,
                    ));
                }
                *budget -= 1;
            }

            // Perform incremental GC progress with finalization support
            // In a real VM this would be tuned based on allocation pressure
            #[cfg(not(feature = "multithreading"))]
            {
                self.gc_tick_counter += 1;
                // Only collect debt every 8 ticks to reduce overhead
                if self.gc_tick_counter.is_multiple_of(8) {
                    self.with_arena(|arena| {
                        arena.collect_debt();
                    });
                }
            }

            #[cfg(feature = "multithreading")]
            let (_full_collect, collection_requested) = self.with_arena(|arena| {
                arena.mutate(|_, c| {
                    let full_collect = if c.stack.local.heap.needs_full_collect.get() {
                        c.stack.local.heap.needs_full_collect.set(false);
                        true
                    } else {
                        false
                    };
                    let collection_requested =
                        full_collect || c.stack.arena.needs_collection().load(Ordering::Acquire);
                    (full_collect, collection_requested)
                })
            });

            #[cfg(not(feature = "multithreading"))]
            let (_full_collect, collection_requested) = self.with_arena(|arena| {
                arena.mutate(|_, c| {
                    let full_collect = if c.stack.local.heap.needs_full_collect.get() {
                        c.stack.local.heap.needs_full_collect.set(false);
                        true
                    } else {
                        false
                    };
                    (full_collect, full_collect)
                })
            });

            if collection_requested {
                self.perform_full_gc();
                #[cfg(feature = "multithreading")]
                self.with_arena(|arena| {
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
                });
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
                self.shared
                    .thread_manager
                    .safe_point(self.thread_id, &self.shared.gc_coordinator);
            }

            #[cfg(feature = "memory-validation")]
            let thread_id = self.thread_id;
            self.with_arena(|arena| {
                // Optimization: Only mutate the root to process finalizers if there are actually pending finalizers
                let has_pending = arena.mutate(|_, c| c.stack.local.heap.has_pending_finalizers());
                if has_pending {
                    let _ = arena.mutate_root(|gc, c| {
                        let gc_handle = GCHandle::new(
                            gc,
                            #[cfg(feature = "multithreading")]
                            // SAFETY: Finalizer processing runs inside this arena's mutate_root
                            // closure, so the inner GC handle belongs to the same arena.
                            unsafe {
                                c.stack.arena_inner_gc()
                            },
                            #[cfg(feature = "memory-validation")]
                            thread_id,
                        );
                        c.ves_context(gc_handle).process_pending_finalizers()
                    });
                }
            });

            #[cfg(feature = "memory-validation")]
            let thread_id = self.thread_id;
            let step_result = self.with_arena(|arena| {
                arena.mutate_root(|gc, c| {
                    let gc_handle = GCHandle::new(
                        gc,
                        #[cfg(feature = "multithreading")]
                        // SAFETY: Instruction stepping runs inside this arena's mutate_root
                        // closure, so the inner GC handle belongs to the same arena.
                        unsafe {
                            c.stack.arena_inner_gc()
                        },
                        #[cfg(feature = "memory-validation")]
                        thread_id,
                    );
                    c.run(gc_handle)
                })
            });

            match step_result {
                StepResult::Return => {
                    let exit_code = self.with_arena_ref(|arena| {
                        arena.mutate(|_, c| {
                            match c.stack.execution.evaluation_stack.stack.first() {
                                Some(StackValue::Int32(i)) => *i as u8,
                                Some(v) => panic!("invalid value for entrypoint return: {:?}", v),
                                None => 0,
                            }
                        })
                    });
                    break ExecutorResult::Exited(exit_code);
                }
                StepResult::MethodThrew(exc) => {
                    break ExecutorResult::Threw(exc);
                }
                StepResult::Error(e) => {
                    break ExecutorResult::Error(e);
                }
                _ => {}
            }

            {
                let (gc_bytes, external_bytes) = self.with_arena_ref(|arena| {
                    let metrics = arena.metrics();
                    (
                        metrics.total_gc_allocation(),
                        metrics.total_external_allocation(),
                    )
                });

                #[cfg(feature = "multithreading")]
                {
                    // Update the arena's metrics directly
                    self.with_arena(|arena| {
                        arena.mutate(|_, c| {
                            c.stack
                                .arena
                                .gc_allocated_bytes()
                                .store(gc_bytes, Ordering::Relaxed);
                            c.stack
                                .arena
                                .external_allocated_bytes()
                                .store(external_bytes, Ordering::Relaxed);
                        })
                    });

                    // Update global metrics from aggregated values
                    let total_gc = self.shared.gc_coordinator.total_gc_allocation();
                    let total_ext = self.shared.gc_coordinator.total_external_allocation();
                    self.shared
                        .metrics
                        .update_gc_metrics(total_gc as u64, total_ext as u64);
                }
                #[cfg(not(feature = "multithreading"))]
                {
                    self.shared
                        .metrics
                        .update_gc_metrics(gc_bytes as u64, external_bytes as u64);
                }
            }
        };

        // Thread unregistration is now handled by ThreadCleanupGuard::drop
        // which runs when this function returns or panics.
        self.shared.tracer.flush();
        result
    }

    /// Perform a full GC collection with stop-the-world coordination.
    fn perform_full_gc(&mut self) {
        #[cfg(feature = "multithreading")]
        {
            use std::time::Instant;
            let start_time = Instant::now();

            vm_trace_gc_collection_start!(self, 0, "allocation pressure");

            // 1. Mark that we are starting collection (acquires coordinator lock)
            let coordinator = Arc::clone(&self.shared.gc_coordinator);
            let Some(_gc_lock) = coordinator.start_collection() else {
                return; // Another thread is already collecting
            };

            self.with_arena(|arena| {
                arena.mutate(|_, c| {
                    vm_debug!(c.stack, "GC: Coordinated stop-the-world collection started");
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

            vm_trace_gc_collection_end!(self, 0, 0, duration.as_micros() as u64);

            self.with_arena(|arena| {
                arena.mutate(|_, c| {
                    vm_debug!(
                        c.stack,
                        "GC: Coordinated collection completed in {:?}",
                        duration
                    );
                });
            });
        }
        #[cfg(not(feature = "multithreading"))]
        {
            use std::time::Instant;
            let start_time = Instant::now();
            vm_trace_gc_collection_start!(self, 0, "allocation pressure");

            if !self.shared.abort_requested.load(Ordering::Relaxed) {
                // Perform full collection with finalization (mimics multithreading behavior)
                self.with_arena(|arena| {
                    // Try to mark all remaining objects. If finish_marking() returns None,
                    // the arena is not in a markable phase (e.g., already in Collecting
                    // or Sleep phase). In that case, fall through to finish_cycle() which
                    // handles all phases correctly by performing a full collection cycle.
                    if let Some(marked) = arena.finish_marking() {
                        crate::gc::finalize_arena(marked);
                    }
                    // finish_cycle() performs a full collection regardless of current phase.
                    // It internally calls do_collection with no early stop, which handles
                    // Mark, Collecting, and Sleep phases correctly.
                    arena.finish_cycle();
                });
            }

            let duration = start_time.elapsed();
            self.shared.metrics.record_gc_pause(duration);
            vm_trace_gc_collection_end!(self, 0, 0, duration.as_micros() as u64);
        }
    }
}

impl Executor {
    /// Extract the thread-local arena without destroying it.
    ///
    /// In multithreading mode, this removes the arena from THREAD_ARENA and returns it
    /// as an opaque guard. The arena memory will be destroyed when the guard is dropped.
    ///
    /// This is used in test scenarios where the arena lifetime must extend beyond the
    /// Executor's lifetime to avoid use-after-free when other threads may still access
    /// shared static references.
    ///
    /// After calling this, the Executor's drop will skip clearing THREAD_ARENA.
    #[cfg(feature = "multithreading")]
    pub fn extract_arena() -> Option<ArenaGuard> {
        THREAD_ARENA.with(|cell| cell.borrow_mut().take().map(ArenaGuard))
    }
}

#[cfg(feature = "multithreading")]
pub struct ArenaGuard(#[allow(dead_code)] Box<GCArena>);

impl Drop for Executor {
    fn drop(&mut self) {
        #[cfg(feature = "multithreading")]
        {
            // 1. Unregister from GC coordinator FIRST.
            // This ensures no new STW collection will target this thread.
            self.shared.gc_coordinator.unregister_arena(self.thread_id);
        }

        // 2. Clear other thread-locals.
        crate::threading::IS_PERFORMING_GC.set(false);
        clear_tracing_state();

        // 3. Unregister from ThreadManager.
        // This keeps the thread "visible" to any already-in-progress STW collections
        // until we are mostly torn down, ensuring they wait for us to finish.
        self.shared.thread_manager.unregister_thread(self.thread_id);

        #[cfg(feature = "multithreading")]
        {
            // 4. Clear THREAD_ARENA LAST (if it hasn't been extracted already).
            // We must keep the arena memory alive until AFTER we have unregistered from
            // the ThreadManager. This ensures that any concurrent GC that was waiting
            // for us will see the thread as 'Exited' and proceed, but the memory
            // will still be valid if they happen to encounter a dangling reference
            // to this arena before we finish dropping.
            //
            // Note: In some test scenarios, the arena may have been extracted via
            // extract_arena() to defer its destruction. In that case, this is a no-op.
            THREAD_ARENA.with(|cell| {
                *cell.borrow_mut() = None;
            });
        }
    }
}

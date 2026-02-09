use crate::{
    MethodInfo, StepResult,
    dispatch::ExecutionEngine,
    metrics::CacheStats,
    stack::{
        CallStack, GCArena,
        ops::{CallOps, ReflectionOps, VesOps},
    },
    state::{ArenaLocalState, SharedGlobalState},
    sync::MutexGuard,
    threading::ThreadManagerOps,
    tracer::Tracer,
};
use dotnet_types::members::MethodDescription;
use dotnet_utils::sync::{Arc, Ordering};
use dotnet_value::StackValue;

#[cfg(feature = "multithreaded-gc")]
use crate::vm_debug;

#[cfg(feature = "multithreaded-gc")]
use super::gc::{arena::THREAD_ARENA, coordinator::*};

pub struct Executor {
    shared: Arc<SharedGlobalState<'static>>,
    /// Thread ID for this executor
    thread_id: u64,
    #[cfg(not(feature = "multithreaded-gc"))]
    arena: Box<GCArena>,
}

#[derive(Clone, Debug)]
pub enum ExecutorResult {
    Exited(u8),
    Threw, // TODO: well-typed exceptions
    Error(crate::error::VmError),
}

impl Executor {
    fn with_arena<R>(&mut self, f: impl FnOnce(&mut GCArena) -> R) -> R {
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

    fn with_arena_ref<R>(&self, f: impl FnOnce(&GCArena) -> R) -> R {
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

    pub fn tracer_enabled(&self) -> bool {
        self.shared.tracer_enabled.load(Ordering::Relaxed)
    }

    pub fn tracer(&self) -> MutexGuard<'_, Tracer> {
        self.shared.tracer.lock()
    }

    pub fn indent(&self) -> usize {
        0
    }

    pub fn get_cache_stats(&self) -> CacheStats {
        self.shared.get_cache_stats()
    }

    pub fn new(shared: Arc<SharedGlobalState<'static>>) -> Self {
        let shared_clone = Arc::clone(&shared);
        let mut arena = Box::new(GCArena::new(|_| {
            let local = ArenaLocalState::new();
            ExecutionEngine::new(CallStack::new(shared_clone, local))
        }));

        let thread_id = shared.thread_manager.register_thread();

        // Register with GC coordinator
        #[cfg(feature = "multithreaded-gc")]
        {
            let handle = ArenaHandle::new(thread_id);
            shared.gc_coordinator.register_arena(handle.clone());
            set_current_arena_handle(handle);
        }

        // Set thread id in arena and pre-initialize reflection
        arena.mutate_root(|gc, c| {
            c.stack.thread_id.set(thread_id);
            let mut ctx = c.stack.ves_context();
            ctx.pre_initialize_reflection(gc);
        });

        #[cfg(feature = "multithreaded-gc")]
        THREAD_ARENA.with(|cell| {
            *cell.borrow_mut() = Some(arena);
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
        self.with_arena(|arena| {
            arena.mutate_root(|gc, c| {
                let shared = c.stack.shared.clone();
                let info = MethodInfo::new(method, &Default::default(), shared);
                c.ves_context()
                    .entrypoint_frame(gc, info, Default::default(), vec![])
            });
        });
    }

    // assumes args are already on stack
    pub fn run(&mut self) -> ExecutorResult {
        let result = loop {
            // Perform incremental GC progress with finalization support
            // In a real VM this would be tuned based on allocation pressure
            #[cfg(not(feature = "multithreaded-gc"))]
            self.with_arena(|arena| {
                if let Some(marked) = arena.mark_debt() {
                    marked.finalize(|fc, c| {
                        c.stack
                            .local
                            .heap
                            .finalize_check(fc, &c.stack.shared, c.stack.indent())
                    });
                }
            });

            let full_collect = self.with_arena(|arena| {
                arena.mutate(|_, c| {
                    if c.stack.local.heap.needs_full_collect.get() {
                        c.stack.local.heap.needs_full_collect.set(false);
                        true
                    } else {
                        false
                    }
                })
            });

            #[cfg(feature = "multithreaded-gc")]
            let collection_requested = full_collect || is_current_arena_collection_requested();
            #[cfg(not(feature = "multithreaded-gc"))]
            let collection_requested = full_collect;

            if collection_requested {
                self.perform_full_gc();
                #[cfg(feature = "multithreaded-gc")]
                reset_current_arena_collection_requested();
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
                let coordinator = {
                    #[cfg(feature = "multithreaded-gc")]
                    {
                        &self.shared.gc_coordinator
                    }
                    #[cfg(not(feature = "multithreaded-gc"))]
                    {
                        &Default::default()
                    }
                };
                self.shared
                    .thread_manager
                    .safe_point(self.thread_id, coordinator);
            }

            self.with_arena(|arena| {
                let _ = arena.mutate_root(|gc, c| c.ves_context().process_pending_finalizers(gc));
            });

            let step_result = self.with_arena(|arena| arena.mutate_root(|gc, c| c.run(gc)));

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
                StepResult::MethodThrew => {
                    break ExecutorResult::Threw;
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

                #[cfg(feature = "multithreaded-gc")]
                {
                    update_current_arena_metrics(gc_bytes, external_bytes);

                    // Update global metrics from aggregated values
                    let total_gc = self.shared.gc_coordinator.total_gc_allocation();
                    let total_ext = self.shared.gc_coordinator.total_external_allocation();
                    self.shared
                        .metrics
                        .update_gc_metrics(total_gc as u64, total_ext as u64);
                }
                #[cfg(not(feature = "multithreaded-gc"))]
                {
                    self.shared
                        .metrics
                        .update_gc_metrics(gc_bytes as u64, external_bytes as u64);
                }
            }
        };

        // Unregister thread when execution completes
        self.shared.thread_manager.unregister_thread(self.thread_id);

        self.shared.tracer.lock().flush();
        result
    }

    /// Perform a full GC collection with stop-the-world coordination.
    fn perform_full_gc(&mut self) {
        use crate::{vm_trace_gc_collection_end, vm_trace_gc_collection_start};
        #[cfg(feature = "multithreaded-gc")]
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
        #[cfg(not(feature = "multithreaded-gc"))]
        {
            use std::time::Instant;
            let start_time = Instant::now();
            vm_trace_gc_collection_start!(self, 0, "allocation pressure");

            // Perform full collection with finalization (mimics multithreaded-gc behavior)
            self.with_arena(|arena| {
                let mut marked = None;
                while marked.is_none() {
                    marked = arena.mark_all();
                }
                if let Some(marked) = marked {
                    marked.finalize(|fc, c| {
                        c.stack
                            .local
                            .heap
                            .finalize_check(fc, &c.stack.shared, c.stack.indent())
                    });
                }
                arena.collect_all();
            });

            let duration = start_time.elapsed();
            self.shared.metrics.record_gc_pause(duration);
            vm_trace_gc_collection_end!(self, 0, 0, duration.as_micros() as u64);
        }
    }
}

impl Drop for Executor {
    fn drop(&mut self) {
        // Ensure thread is unregistered if not already done
        self.shared.thread_manager.unregister_thread(self.thread_id);

        #[cfg(feature = "multithreaded-gc")]
        {
            self.shared.gc_coordinator.unregister_arena(self.thread_id);

            // Clear thread-local GC state to prevent leakage across tests
            THREAD_ARENA.with(|cell| {
                *cell.borrow_mut() = None;
            });
            clear_tracing_state();
        }
    }
}

use crate::{
    types::members::MethodDescription,
    value::StackValue,
    vm::{stack::GCArena, MethodInfo, StepResult},
    vm_msg,
};
use std::sync::Arc;

pub struct Executor {
    arena: &'static mut GCArena,
    /// Thread ID for this executor (if using new_with_global architecture)
    thread_id: Option<u64>,
}

#[derive(Clone, Debug)]
pub enum ExecutorResult {
    Exited(u8),
    Threw, // TODO: well-typed exceptions
}

impl Executor {
    pub fn new(arena: &'static mut GCArena) -> Self {
        Self {
            arena,
            thread_id: None,
        }
    }

    /// Create a new executor and register it with the global thread manager.
    /// This should be used with the new_with_global() architecture.
    pub fn new_with_thread_manager(arena: &'static mut GCArena) -> Self {
        // Register this thread with the thread manager
        let thread_id = arena.mutate(|_, c| {
            let id = c.global.thread_manager.register_thread();
            c.thread_id.set(id);
            Some(id)
        });

        Self { arena, thread_id }
    }

    pub fn entrypoint(&mut self, method: MethodDescription) {
        // TODO: initialize argv (entry point args are either string[] or nothing, II.15.4.1.2)
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
            // GC Safe Point: Check if stop-the-world is requested
            // If using thread manager, pause here if GC is happening on another thread
            if let Some(tid) = self.thread_id {
                self.arena.mutate(|_, c| {
                    c.global.thread_manager.safe_point(tid);
                });
            }

            // Perform incremental GC mark phase
            // For multithreading: only the GC coordinator should do this during stop-the-world
            if let Some(marked) = self.arena.mark_all() {
                marked.finalize(|fc, c| c.finalize_check(fc));
            }

            let full_collect = self.arena.mutate(|_, c| {
                if c.heap().needs_full_collect.get() {
                    c.heap().needs_full_collect.set(false);
                    true
                } else {
                    false
                }
            });

            if full_collect {
                // For multithreading: coordinate stop-the-world pause
                self.perform_full_gc();
            }

            self.arena
                .mutate_root(|gc, c| c.process_pending_finalizers(gc));

            match self.arena.mutate_root(|gc, c| c.step(gc)) {
                StepResult::MethodReturned => {
                    let was_auto_invoked = self.arena.mutate_root(|gc, c| {
                        let frame = c.execution.frames.last().unwrap();
                        let val = frame.state.info_handle.is_cctor || frame.is_finalizer;
                        c.return_frame(gc);
                        val
                    });

                    if self.arena.mutate(|_, c| c.execution.frames.is_empty()) {
                        let exit_code = self.arena.mutate(|_, c| match c.bottom_of_stack() {
                            Some(StackValue::Int32(i)) => i as u8,
                            Some(v) => panic!("invalid value for entrypoint return: {:?}", v),
                            None => 0,
                        });
                        break ExecutorResult::Exited(exit_code);
                    } else if !was_auto_invoked {
                        // step the caller past the call instruction
                        self.arena.mutate_root(|_, c| c.increment_ip());
                    }
                }
                StepResult::MethodThrew => {
                    self.arena.mutate(|_, c| {
                        vm_msg!(c, "Exception thrown: {:?}", c.execution.exception_mode);
                    });
                    if self.arena.mutate(|_, c| c.execution.frames.is_empty()) {
                        break ExecutorResult::Threw;
                    }
                }
                StepResult::InstructionStepped => {}
            }
            // TODO(gc): poll arena for stats
        };

        // Unregister thread when execution completes
        if let Some(tid) = self.thread_id.take() {
            self.arena.mutate(|_, c| {
                c.global.thread_manager.unregister_thread(tid);
            });
        }

        self.arena.mutate(|_, c| c.tracer().flush());
        result
    }

    /// Perform a full GC collection with stop-the-world coordination.
    ///
    /// If the executor is using the new architecture with a thread manager,
    /// this will coordinate a stop-the-world pause across all threads before
    /// performing the collection. Otherwise, it falls back to single-threaded GC.
    fn perform_full_gc(&mut self) {
        use std::time::Instant;
        let start_time = Instant::now();

        // Check if we're using the new architecture with thread manager
        // We escape the Arc<ThreadManager> because it is 'static and can leave the arena.mutate call.
        let thread_manager = self.arena.mutate(|_, c| {
            Some(Arc::clone(&c.global.thread_manager))
        });

        if let Some(tm) = thread_manager {
            self.arena.mutate(|_, c| {
                vm_msg!(
                    c,
                    "GC: Manual collection triggered (stop-the-world coordination)"
                );
            });

            // Request stop-the-world pause and wait for all threads to reach safe points
            let _stw_guard = tm.request_stop_the_world();

            // Perform GC while all other threads are paused at safe points
            let mut marked = None;
            while marked.is_none() {
                marked = self.arena.mark_all();
            }
            if let Some(marked) = marked {
                marked.finalize(|fc, c| c.finalize_check(fc));
            }
            self.arena.collect_all();
        } else {
            // Legacy single-threaded GC
            self.arena.mutate(|_, c| {
                vm_msg!(c, "GC: Manual collection triggered");
            });
            let mut marked = None;
            while marked.is_none() {
                marked = self.arena.mark_all();
            }
            if let Some(marked) = marked {
                marked.finalize(|fc, c| c.finalize_check(fc));
            }
            self.arena.collect_all();
        }

        let duration = start_time.elapsed();
        self.arena.mutate(|_, c| {
            c.global.metrics.record_gc_pause(duration);
            vm_msg!(c, "GC: Manual collection completed in {:?}", duration);
        });
    }
}

impl Drop for Executor {
    fn drop(&mut self) {
        // Ensure thread is unregistered if not already done
        if let Some(tid) = self.thread_id {
            self.arena.mutate(|_, c| {
                c.global.thread_manager.unregister_thread(tid);
            });
        }
    }
}

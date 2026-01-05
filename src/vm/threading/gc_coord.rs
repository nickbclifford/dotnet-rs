use super::basic::{get_current_thread_id, IS_PERFORMING_GC, ThreadManager, ThreadState};
use crate::vm::gc::coordinator::{GCCommand, GCCoordinator};
use crate::vm::sync::{MutexGuard, Ordering};

pub struct StopTheWorldGuard<'a> {
    manager: &'a ThreadManager,
    _lock: MutexGuard<'a, ()>,
    start_time: std::time::Instant,
}

impl<'a> StopTheWorldGuard<'a> {
    pub(super) fn new(
        manager: &'a ThreadManager,
        lock: MutexGuard<'a, ()>,
        start_time: std::time::Instant,
    ) -> Self {
        IS_PERFORMING_GC.set(true);
        Self {
            manager,
            _lock: lock,
            start_time,
        }
    }

    pub fn elapsed_micros(&self) -> u64 {
        self.start_time.elapsed().as_micros() as u64
    }
}

impl<'a> Drop for StopTheWorldGuard<'a> {
    fn drop(&mut self) {
        self.manager.resume_threads();
        IS_PERFORMING_GC.set(false);
    }
}

impl ThreadManager {
    #[inline]
    pub fn is_gc_stop_requested(&self) -> bool {
        self.gc_stop_requested.load(Ordering::Acquire)
    }

    pub fn safe_point(&self, managed_id: u64, coordinator: &GCCoordinator) {
        if !self.is_gc_stop_requested() {
            return;
        }

        if IS_PERFORMING_GC.get() {
            return;
        }

        let thread_info = {
            let threads = self.threads.lock();
            threads.get(&managed_id).cloned()
        };

        if let Some(thread) = thread_info {
            thread.set_state(ThreadState::AtSafePoint);
            self.threads_at_safepoint.fetch_add(1, Ordering::AcqRel);
            self.all_threads_stopped.notify_all();

            {
                let mut guard = self.gc_coordination.lock();
                while self.gc_stop_requested.load(Ordering::Acquire) {
                    if coordinator.has_command(managed_id) {
                        if let Some(command) = coordinator.get_command(managed_id) {
                            self.execute_gc_command(command, coordinator);
                            coordinator.command_finished(managed_id);
                        }
                    }
                    self.all_threads_stopped.wait(&mut guard);
                }
            }

            self.threads_at_safepoint.fetch_sub(1, Ordering::Release);
            thread.set_state(ThreadState::Running);
        }
    }

    pub fn execute_gc_command(&self, command: GCCommand, coordinator: &GCCoordinator) {
        execute_gc_command_for_current_thread(command, coordinator);
    }

    pub fn safe_point_traced(
        &self,
        managed_id: u64,
        coordinator: &GCCoordinator,
        tracer: &crate::vm::gc::tracer::Tracer,
        location: &str,
    ) {
        if tracer.is_enabled() && self.is_gc_stop_requested() {
            tracer.trace_thread_safepoint(0, managed_id, location);
        }
        self.safe_point(managed_id, coordinator);
    }

    pub fn request_stop_the_world(&self) -> StopTheWorldGuard<'_> {
        let mut guard = self.gc_coordination.lock();
        let start_time = std::time::Instant::now();
        const WARN_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(1);
        let mut warned = false;

        self.gc_stop_requested.store(true, Ordering::Release);

        loop {
            let thread_count = self.thread_count();
            let current_id = self.current_thread_id();
            let target_stopped = if current_id.is_some() {
                thread_count.saturating_sub(1)
            } else {
                thread_count
            };

            if self.threads_at_safepoint.load(Ordering::Acquire) >= target_stopped {
                break;
            }

            if !warned && start_time.elapsed() > WARN_TIMEOUT {
                eprintln!("[GC WARNING] Stop-the-world pause taking longer than expected:");
                eprintln!("  Total threads: {}", thread_count);
                eprintln!(
                    "  Threads at safe point: {}",
                    self.threads_at_safepoint.load(Ordering::Acquire)
                );

                let threads = self.threads.lock();
                eprintln!("  Threads not at safe point:");
                for (tid, thread) in threads.iter() {
                    if thread.get_state() != ThreadState::AtSafePoint {
                        eprintln!(
                            "    - Thread ID {}: {:?} (native: {:?})",
                            tid,
                            thread.get_state(),
                            thread.native_id
                        );
                    }
                }
                drop(threads);
                warned = true;
            }

            self.all_threads_stopped.wait(&mut guard);
        }

        if warned {
            eprintln!(
                "[GC] Stop-the-world completed after {} ms",
                start_time.elapsed().as_millis()
            );
        }

        StopTheWorldGuard::new(self, guard, start_time)
    }

    pub fn request_stop_the_world_traced(
        &self,
        tracer: &crate::vm::gc::tracer::Tracer,
    ) -> StopTheWorldGuard<'_> {
        let thread_count = self.thread_count();
        if tracer.is_enabled() {
            tracer.trace_stw_start(0, thread_count);
        }
        self.request_stop_the_world()
    }

    pub(super) fn resume_threads(&self) {
        self.gc_stop_requested.store(false, Ordering::Release);
        self.all_threads_stopped.notify_all();
    }
}

pub fn execute_gc_command_for_current_thread(
    command: GCCommand,
    coordinator: &GCCoordinator,
) {
    use crate::vm::gc::arena::THREAD_ARENA;
    match command {
        GCCommand::CollectAll => {
            THREAD_ARENA.with(|cell| {
                if let Ok(mut arena_opt) = cell.try_borrow_mut() {
                    if let Some(arena) = arena_opt.as_mut() {
                        let thread_id = get_current_thread_id();
                        crate::vm::gc::coordinator::set_currently_tracing(Some(thread_id));

                        arena.mutate(|_, c| {
                            c.local.heap.cross_arena_roots.borrow_mut().clear();
                        });

                        let mut marked = None;
                        while marked.is_none() {
                            marked = arena.mark_all();
                        }
                        if let Some(marked) = marked {
                            marked.finalize(|fc, c| c.finalize_check(fc));
                        }
                        arena.collect_all();
                        crate::vm::gc::coordinator::set_currently_tracing(None);

                        for (target_id, ptr) in
                            crate::vm::gc::coordinator::take_found_cross_arena_refs()
                        {
                            coordinator.record_cross_arena_ref(target_id, ptr);
                        }
                    }
                }
            });
        }
        GCCommand::MarkObjects(ptrs) => {
            THREAD_ARENA.with(|cell| {
                if let Ok(mut arena_opt) = cell.try_borrow_mut() {
                    if let Some(arena) = arena_opt.as_mut() {
                        let thread_id = get_current_thread_id();
                        crate::vm::gc::coordinator::set_currently_tracing(Some(thread_id));

                        arena.mutate(|_, c| {
                            let mut roots = c.local.heap.cross_arena_roots.borrow_mut();
                            for ptr in ptrs {
                                roots.insert(ptr);
                            }
                        });

                        let mut marked = None;
                        while marked.is_none() {
                            marked = arena.mark_all();
                        }
                        if let Some(marked) = marked {
                            marked.finalize(|fc, c| c.finalize_check(fc));
                        }
                        arena.collect_all();

                        crate::vm::gc::coordinator::set_currently_tracing(None);
                        for (target_id, ptr) in
                            crate::vm::gc::coordinator::take_found_cross_arena_refs()
                        {
                            coordinator.record_cross_arena_ref(target_id, ptr);
                        }
                    }
                }
            });
        }
    }
}

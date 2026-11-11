//! VM-owned native workers and their managed completion states.

use crate::ExecutorResult;
#[cfg(feature = "multithreading")]
use dotnet_utils::sync::{Arc, Mutex};
#[cfg(feature = "multithreading")]
use dotnet_value::object::ObjectPtr;
#[cfg(feature = "multithreading")]
use std::collections::HashMap;
#[cfg(feature = "multithreading")]
use std::thread::JoinHandle;

type Outcome = std::thread::Result<ExecutorResult>;

#[cfg(feature = "multithreading")]
enum Lifecycle {
    Running,
    Completed(Option<Outcome>),
    Retired(Option<Outcome>),
    Finalizing(Option<Outcome>),
    ShuttingDown(Option<Outcome>),
    Drained(Option<Outcome>),
    Leaked(Option<Outcome>),
}

#[cfg(feature = "multithreading")]
struct Entry {
    lifecycle: Lifecycle,
    handle: Option<JoinHandle<()>>,
    background: bool,
}

#[cfg(feature = "multithreading")]
#[derive(Default)]
struct RegistryState {
    entries: Vec<Entry>,
    // Keys and delegate pointers are inert: their allocations remain rooted by arena-local
    // records while the worker can use them, and the registry never dereferences them.
    // Completed entries may outlive their allocations and are replaced on address reuse.
    threads: HashMap<ObjectPtr, ManagedThreadMetadata>,
}

#[cfg(feature = "multithreading")]
#[derive(Clone)]
pub(crate) struct ManagedThreadMetadata {
    pub(crate) id: i32,
    pub(crate) delegate: Option<ObjectPtr>,
    pub(crate) parameterized: bool,
    pub(crate) started: bool,
    pub(crate) background: bool,
    pub(crate) completion_index: Option<usize>,
    pub(crate) retained: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

/// The completion and the handle share one lifecycle lock, but only the registry owns handles.
#[cfg(feature = "multithreading")]
#[derive(Default)]
pub(crate) struct WorkerRegistry {
    state: Arc<Mutex<RegistryState>>,
}

#[cfg(feature = "multithreading")]
#[derive(Clone)]
pub(crate) struct WorkerCompletion {
    state: Arc<Mutex<RegistryState>>,
    index: usize,
}

#[cfg(feature = "multithreading")]
impl WorkerCompletion {
    pub(crate) fn is_finished(&self) -> bool {
        matches!(
            self.state.lock().entries[self.index].lifecycle,
            Lifecycle::Completed(_)
                | Lifecycle::Retired(_)
                | Lifecycle::Finalizing(_)
                | Lifecycle::ShuttingDown(_)
                | Lifecycle::Drained(_)
                | Lifecycle::Leaked(_)
        )
    }

    pub(crate) fn take_outcome(&self) -> Option<Outcome> {
        match &mut self.state.lock().entries[self.index].lifecycle {
            Lifecycle::Running => None,
            Lifecycle::Completed(outcome)
            | Lifecycle::Retired(outcome)
            | Lifecycle::Finalizing(outcome)
            | Lifecycle::ShuttingDown(outcome)
            | Lifecycle::Drained(outcome)
            | Lifecycle::Leaked(outcome) => outcome.take(),
        }
    }

    #[cfg(test)]
    pub(crate) fn index(&self) -> usize {
        self.index
    }

    #[cfg(test)]
    pub(crate) fn phase(&self) -> &'static str {
        match self.state.lock().entries[self.index].lifecycle {
            Lifecycle::Running => "running",
            Lifecycle::Completed(_) => "completed",
            Lifecycle::Retired(_) => "retired",
            Lifecycle::Finalizing(_) => "finalizing",
            Lifecycle::ShuttingDown(_) => "shutting down",
            Lifecycle::Drained(_) => "drained",
            Lifecycle::Leaked(_) => "leaked",
        }
    }
}

#[cfg(feature = "multithreading")]
impl WorkerRegistry {
    pub(crate) fn register_managed_thread(
        &self,
        thread: ObjectPtr,
        metadata: ManagedThreadMetadata,
    ) {
        self.state.lock().threads.insert(thread, metadata);
    }

    pub(crate) fn managed_thread(&self, thread: ObjectPtr) -> Option<ManagedThreadMetadata> {
        self.state.lock().threads.get(&thread).cloned()
    }

    pub(crate) fn begin_managed_thread(&self, thread: ObjectPtr) -> Option<ManagedThreadMetadata> {
        let mut state = self.state.lock();
        let metadata = state.threads.get_mut(&thread)?;
        if metadata.started {
            return None;
        }
        metadata.started = true;
        Some(metadata.clone())
    }

    pub(crate) fn set_background(&self, thread: ObjectPtr, background: bool) -> bool {
        let mut state = self.state.lock();
        let Some(metadata) = state.threads.get_mut(&thread) else {
            return false;
        };
        metadata.background = background;
        if let Some(index) = metadata.completion_index {
            state.entries[index].background = background;
        }
        true
    }

    pub(crate) fn managed_completion(&self, thread: ObjectPtr) -> Option<WorkerCompletion> {
        let state = self.state.lock();
        let index = state.threads.get(&thread)?.completion_index?;
        Some(WorkerCompletion {
            state: self.state.clone(),
            index,
        })
    }
    #[cfg(test)]
    pub(crate) fn spawn_owner(
        &self,
        run: impl FnOnce(RunningWorker) + Send + 'static,
    ) -> WorkerCompletion {
        self.spawn_owner_inner(None, false, run)
    }

    pub(crate) fn spawn_managed_owner(
        &self,
        thread: ObjectPtr,
        run: impl FnOnce(RunningWorker) + Send + 'static,
    ) -> WorkerCompletion {
        self.spawn_owner_inner(Some(thread), false, run)
    }

    pub(crate) fn spawn_pool_owner(
        &self,
        run: impl FnOnce(RunningWorker) + Send + 'static,
    ) -> WorkerCompletion {
        self.spawn_owner_inner(None, true, run)
    }

    fn spawn_owner_inner(
        &self,
        thread: Option<ObjectPtr>,
        pool_background: bool,
        run: impl FnOnce(RunningWorker) + Send + 'static,
    ) -> WorkerCompletion {
        let mut state = self.state.lock();
        let index = state.entries.len();
        let background = pool_background
            || thread.is_some_and(|thread| {
                let metadata = state.threads.get_mut(&thread).expect("registered thread");
                metadata.completion_index = Some(index);
                metadata.background
            });
        let published = Arc::clone(&self.state);
        let handle = std::thread::spawn(move || {
            let token = RunningWorker {
                state: Arc::clone(&published),
                index,
            };
            // A panic must update the registry to Leaked; otherwise shutdown sees Running forever.
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run(token)));
            if let Err(panic) = result {
                let mut state = published.lock();
                let outcome = match &mut state.entries[index].lifecycle {
                    Lifecycle::Running => Some(Err(panic)),
                    Lifecycle::Completed(outcome)
                    | Lifecycle::Retired(outcome)
                    | Lifecycle::Finalizing(outcome)
                    | Lifecycle::ShuttingDown(outcome)
                    | Lifecycle::Drained(outcome)
                    | Lifecycle::Leaked(outcome) => outcome.take(),
                };
                state.entries[index].lifecycle = Lifecycle::Leaked(outcome);
            }
        });
        state.entries.push(Entry {
            lifecycle: Lifecycle::Running,
            handle: Some(handle),
            background,
        });
        WorkerCompletion {
            state: Arc::clone(&self.state),
            index,
        }
    }

    #[cfg(test)]
    pub(crate) fn spawn_with_completion_hook(
        &self,
        run: impl FnOnce() -> ExecutorResult + Send + 'static,
        after_publish: impl FnOnce() + Send + 'static,
    ) -> WorkerCompletion {
        self.spawn_owner(move |running| {
            let completed = running.complete(run());
            after_publish();
            completed.retire();
        })
    }

    #[cfg(test)]
    fn spawn(&self, run: impl FnOnce() -> ExecutorResult + Send + 'static) -> WorkerCompletion {
        self.spawn_with_completion_hook(run, || {})
    }

    #[cfg(test)]
    pub(crate) fn has_running(&self) -> bool {
        self.state
            .lock()
            .entries
            .iter()
            .any(|entry| matches!(entry.lifecycle, Lifecycle::Running))
    }

    /// Whether no worker can run managed code: none is `Running` or `Finalizing`, and
    /// `finalizers_idle` holds. The predicate runs under the same registry lock that
    /// `begin_finalizing` takes, so a retired owner cannot start finalizing between the
    /// two observations, and a finalizer cannot start another worker after this returns.
    pub(crate) fn is_quiescent(&self, finalizers_idle: impl FnOnce() -> bool) -> bool {
        let state = self.state.lock();
        Self::quiescent_locked(&state) && finalizers_idle()
    }

    /// Decide process exit and request cancellation while holding the lifecycle lock, so
    /// background changes and worker publication cannot race the decision.
    pub(crate) fn foreground_quiescent(
        &self,
        finalizers_idle: impl FnOnce() -> bool,
        cancel_background: impl FnOnce(),
    ) -> bool {
        let state = self.state.lock();
        if state.entries.iter().any(|entry| {
            matches!(entry.lifecycle, Lifecycle::Finalizing(_))
                || matches!(entry.lifecycle, Lifecycle::Running) && !entry.background
        }) || !finalizers_idle()
        {
            return false;
        }
        if state
            .entries
            .iter()
            .any(|entry| matches!(entry.lifecycle, Lifecycle::Running))
        {
            cancel_background();
        }
        true
    }

    fn quiescent_locked(state: &RegistryState) -> bool {
        !state.entries.iter().any(|entry| {
            matches!(
                entry.lifecycle,
                Lifecycle::Running | Lifecycle::Finalizing(_)
            )
        })
    }

    /// Waits until no worker can run managed code, signals the retired owners, then joins every
    /// native thread, including workers that a retired owner starts while finishing finalizers
    /// after the signal.
    pub(crate) fn shutdown(&self, signal: impl FnOnce()) {
        // The caller has stopped being a GC participant before waiting here. A `Finalizing`
        // worker can start another worker, so it must finish before the handle set is final.
        loop {
            if Self::quiescent_locked(&self.state.lock()) {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        signal();
        loop {
            let handles = self
                .state
                .lock()
                .entries
                .iter_mut()
                .filter_map(|entry| entry.handle.take())
                .collect::<Vec<_>>();
            if handles.is_empty() {
                break;
            }
            for handle in handles {
                let _ = handle.join();
            }
        }
    }
}

/// Only the owner that completed managed execution may enter the retirement service.
#[cfg(feature = "multithreading")]
pub(crate) struct RunningWorker {
    state: Arc<Mutex<RegistryState>>,
    index: usize,
}

#[cfg(feature = "multithreading")]
pub(crate) struct CompletedWorker(RunningWorker);

#[cfg(feature = "multithreading")]
pub(crate) struct RetiredWorker(RunningWorker);

#[cfg(feature = "multithreading")]
impl RunningWorker {
    pub(crate) fn attach_pool_thread(&self, thread: ObjectPtr) {
        let mut state = self.state.lock();
        let metadata = state
            .threads
            .get_mut(&thread)
            .expect("registered pool thread");
        metadata.background = true;
        metadata.completion_index = Some(self.index);
    }

    pub(crate) fn complete(self, result: ExecutorResult) -> CompletedWorker {
        {
            let mut state = self.state.lock();
            assert!(matches!(
                state.entries[self.index].lifecycle,
                Lifecycle::Running
            ));
            state.entries[self.index].lifecycle = Lifecycle::Completed(Some(Ok(result)));
        }
        CompletedWorker(self)
    }
}

#[cfg(feature = "multithreading")]
impl CompletedWorker {
    pub(crate) fn retire(self) -> RetiredWorker {
        let mut state = self.0.state.lock();
        let entry = &mut state.entries[self.0.index];
        let Lifecycle::Completed(outcome) = &mut entry.lifecycle else {
            panic!("retirement requires completed worker");
        };
        entry.lifecycle = Lifecycle::Retired(outcome.take());
        drop(state);
        RetiredWorker(self.0)
    }
}

#[cfg(feature = "multithreading")]
impl RetiredWorker {
    pub(crate) fn begin_finalizing(&self, shared: &crate::state::SharedGlobalState) -> bool {
        let mut state = self.0.state.lock();
        if shared.is_abort_requested() {
            return false;
        }
        let entry = &mut state.entries[self.0.index];
        let Lifecycle::Retired(outcome) = &mut entry.lifecycle else {
            panic!("finalization requires a retired worker");
        };
        entry.lifecycle = Lifecycle::Finalizing(outcome.take());
        true
    }

    pub(crate) fn finish_finalizing(&self) {
        let mut state = self.0.state.lock();
        let entry = &mut state.entries[self.0.index];
        let Lifecycle::Finalizing(outcome) = &mut entry.lifecycle else {
            panic!("finalization completion requires a finalizing worker");
        };
        entry.lifecycle = Lifecycle::Retired(outcome.take());
    }

    pub(crate) fn begin_shutdown(&self) {
        let mut state = self.0.state.lock();
        let entry = &mut state.entries[self.0.index];
        let Lifecycle::Retired(outcome) = &mut entry.lifecycle else {
            panic!("shutdown requires a retired worker");
        };
        entry.lifecycle = Lifecycle::ShuttingDown(outcome.take());
    }

    pub(crate) fn finish_shutdown(self, drained: bool) {
        let mut state = self.0.state.lock();
        let entry = &mut state.entries[self.0.index];
        let Lifecycle::ShuttingDown(outcome) = &mut entry.lifecycle else {
            panic!("shutdown completion requires a shutting down worker");
        };
        entry.lifecycle = if drained {
            Lifecycle::Drained(outcome.take())
        } else {
            Lifecycle::Leaked(outcome.take())
        };
    }
}

// Managed workers cannot be constructed by the single-threaded backend.
#[cfg(not(feature = "multithreading"))]
#[derive(Default)]
pub(crate) struct WorkerRegistry {}

#[cfg(not(feature = "multithreading"))]
#[derive(Clone)]
pub(crate) struct WorkerCompletion;

#[cfg(not(feature = "multithreading"))]
impl WorkerCompletion {
    pub(crate) fn is_finished(&self) -> bool {
        false
    }
    pub(crate) fn take_outcome(&self) -> Option<Outcome> {
        None
    }
}

#[cfg(not(feature = "multithreading"))]
impl WorkerRegistry {
    pub(crate) fn managed_completion(
        &self,
        _thread: dotnet_value::object::ObjectPtr,
    ) -> Option<WorkerCompletion> {
        None
    }
    pub(crate) fn is_quiescent(&self, finalizers_idle: impl FnOnce() -> bool) -> bool {
        finalizers_idle()
    }
    pub(crate) fn foreground_quiescent(
        &self,
        finalizers_idle: impl FnOnce() -> bool,
        _cancel_background: impl FnOnce(),
    ) -> bool {
        finalizers_idle()
    }
    pub(crate) fn shutdown(&self) {}
}

#[cfg(all(test, feature = "multithreading"))]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    #[test]
    fn completed_outcomes_remain_consumable_after_retirement() {
        use crate::error::{ExecutionError, VmError};
        use dotnet_vm_data::ManagedException;

        let results = [
            ExecutorResult::Exited(42),
            ExecutorResult::Threw(ManagedException {
                type_name: "Exception".into(),
                message: None,
                stack_trace: None,
            }),
            ExecutorResult::Error(VmError::Execution(ExecutionError::Aborted("test".into()))),
        ];
        for (index, result) in results.into_iter().enumerate() {
            let state = Arc::new(Mutex::new(RegistryState {
                entries: vec![Entry {
                    lifecycle: Lifecycle::Running,
                    handle: None,
                    background: false,
                }],
                threads: HashMap::new(),
            }));
            let completion = WorkerCompletion {
                state: state.clone(),
                index: 0,
            };
            let completed = RunningWorker { state, index: 0 }.complete(result);
            assert!(completion.is_finished());
            if index == 1 {
                assert!(matches!(
                    completion.take_outcome(),
                    Some(Ok(ExecutorResult::Threw(_)))
                ));
            }
            completed.retire();
            assert!(matches!(
                completion.state.lock().entries[0].lifecycle,
                Lifecycle::Retired(_)
            ));
            match index {
                0 => assert!(matches!(
                    completion.take_outcome(),
                    Some(Ok(ExecutorResult::Exited(42)))
                )),
                1 => assert!(completion.take_outcome().is_none()),
                _ => assert!(matches!(
                    completion.take_outcome(),
                    Some(Ok(ExecutorResult::Error(_)))
                )),
            }
        }
    }

    #[test]
    fn abort_prevents_retired_worker_from_starting_finalizers() {
        let state = Arc::new(Mutex::new(RegistryState {
            entries: vec![Entry {
                lifecycle: Lifecycle::Running,
                handle: None,
                background: false,
            }],
            threads: HashMap::new(),
        }));
        let retired = RunningWorker {
            state: state.clone(),
            index: 0,
        }
        .complete(ExecutorResult::Exited(42))
        .retire();
        let loader =
            dotnet_assemblies::AssemblyLoader::new_bare("worker-abort-test".into()).unwrap();
        let shared = crate::state::SharedGlobalState::new(Arc::new(loader));
        shared.abort_signal().request_abort();
        assert!(!retired.begin_finalizing(&shared));
        assert!(matches!(
            state.lock().entries[0].lifecycle,
            Lifecycle::Retired(_)
        ));
    }

    #[test]
    fn finalizing_preserves_managed_completion() {
        let state = Arc::new(Mutex::new(RegistryState {
            entries: vec![Entry {
                lifecycle: Lifecycle::Running,
                handle: None,
                background: false,
            }],
            threads: HashMap::new(),
        }));
        let completion = WorkerCompletion {
            state: state.clone(),
            index: 0,
        };
        let retired = RunningWorker { state, index: 0 }
            .complete(ExecutorResult::Exited(42))
            .retire();
        let loader =
            dotnet_assemblies::AssemblyLoader::new_bare("worker-finalizing-test".into()).unwrap();
        let shared = crate::state::SharedGlobalState::new(Arc::new(loader));
        assert!(retired.begin_finalizing(&shared));
        assert!(completion.is_finished());
        assert!(matches!(
            completion.take_outcome(),
            Some(Ok(ExecutorResult::Exited(42)))
        ));
        retired.finish_finalizing();
        assert!(matches!(
            completion.state.lock().entries[0].lifecycle,
            Lifecycle::Retired(None)
        ));
    }

    #[test]
    fn shutdown_requires_retirement_and_records_failed_drain() {
        let state = Arc::new(Mutex::new(RegistryState {
            entries: vec![Entry {
                lifecycle: Lifecycle::Running,
                handle: None,
                background: false,
            }],
            threads: HashMap::new(),
        }));
        let completion = WorkerCompletion {
            state: state.clone(),
            index: 0,
        };
        let retired = RunningWorker { state, index: 0 }
            .complete(ExecutorResult::Exited(42))
            .retire();
        let loader =
            dotnet_assemblies::AssemblyLoader::new_bare("worker-shutdown-test".into()).unwrap();
        let shared = crate::state::SharedGlobalState::new(Arc::new(loader));
        assert!(retired.begin_finalizing(&shared));
        let rejected =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| retired.begin_shutdown()));
        assert!(rejected.is_err());
        retired.finish_finalizing();
        retired.begin_shutdown();
        let rejected =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| retired.begin_shutdown()));
        assert!(rejected.is_err());
        retired.finish_shutdown(false);
        assert_eq!(completion.phase(), "leaked");
        assert!(matches!(
            completion.take_outcome(),
            Some(Ok(ExecutorResult::Exited(42)))
        ));
    }

    #[test]
    fn quiescence_requires_no_running_or_finalizing_worker_and_idle_finalizers() {
        let registry = WorkerRegistry::default();
        registry.state.lock().entries.push(Entry {
            lifecycle: Lifecycle::Running,
            handle: None,
            background: false,
        });
        assert!(!registry.is_quiescent(|| true));
        let running = RunningWorker {
            state: Arc::clone(&registry.state),
            index: 0,
        };
        let completed = running.complete(ExecutorResult::Exited(0));
        assert!(registry.is_quiescent(|| true));
        assert!(!registry.is_quiescent(|| false));
        let retired = completed.retire();
        assert!(registry.is_quiescent(|| true));
        let loader =
            dotnet_assemblies::AssemblyLoader::new_bare("worker-quiescence-test".into()).unwrap();
        let shared = crate::state::SharedGlobalState::new(Arc::new(loader));
        assert!(retired.begin_finalizing(&shared));
        // The finalizer predicate must not decide the answer while a worker can still run.
        assert!(!registry.is_quiescent(|| true));
        retired.finish_finalizing();
        assert!(registry.is_quiescent(|| true));
    }

    #[test]
    fn foreground_exit_cancels_only_after_foreground_and_finalizers_are_idle() {
        use std::cell::Cell;

        let registry = WorkerRegistry::default();
        registry.state.lock().entries.push(Entry {
            lifecycle: Lifecycle::Running,
            handle: None,
            background: false,
        });
        let canceled = Cell::new(false);
        assert!(!registry.foreground_quiescent(|| true, || canceled.set(true)));
        registry.state.lock().entries[0].background = true;
        assert!(!registry.foreground_quiescent(|| false, || canceled.set(true)));
        assert!(!canceled.get());
        assert!(registry.foreground_quiescent(|| true, || canceled.set(true)));
        assert!(canceled.get());
        assert!(!registry.is_quiescent(|| true));
    }

    #[test]
    fn shutdown_joins_worker_started_after_signal() {
        use std::sync::mpsc;

        let registry = Arc::new(WorkerRegistry::default());
        let (retired_tx, retired_rx) = mpsc::channel();
        let (start_tx, start_rx) = mpsc::channel();
        let (late_tx, late_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let parent_registry = registry.clone();
        registry.spawn_owner(move |running| {
            let _retired = running.complete(ExecutorResult::Exited(0)).retire();
            let _ = retired_tx.send(());
            if start_rx.recv_timeout(Duration::from_secs(10)).is_ok() {
                parent_registry.spawn_owner(move |running| {
                    let _ = late_tx.send(());
                    let _ = release_rx.recv_timeout(Duration::from_secs(10));
                    let _ = running.complete(ExecutorResult::Exited(0)).retire();
                });
            }
        });
        let retired = retired_rx.recv_timeout(Duration::from_secs(10)).is_ok();
        let (signal_tx, signal_rx) = mpsc::channel();
        let (done_tx, done_rx) = mpsc::channel();
        let shutdown_registry = registry.clone();
        let shutdown = std::thread::spawn(move || {
            shutdown_registry.shutdown(|| {
                let _ = signal_tx.send(());
            });
            let _ = done_tx.send(());
        });
        let signaled = signal_rx.recv_timeout(Duration::from_secs(10)).is_ok();
        let deadline = Instant::now() + Duration::from_secs(10);
        while registry.state.lock().entries[0].handle.is_some() && Instant::now() < deadline {
            std::thread::yield_now();
        }
        let first_taken = registry.state.lock().entries[0].handle.is_none();
        let _ = start_tx.send(());
        let late_started = late_rx.recv_timeout(Duration::from_secs(10)).is_ok();
        let returned_early = done_rx.recv_timeout(Duration::from_millis(200)).is_ok();
        let _ = release_tx.send(());
        let completed = returned_early || done_rx.recv_timeout(Duration::from_secs(10)).is_ok();
        shutdown.join().unwrap();
        registry.shutdown(|| {});
        assert!(retired && signaled && first_taken && late_started && completed);
        assert!(
            !returned_early,
            "shutdown returned before joining the late worker"
        );
    }

    #[test]
    fn worker_registry_publishes_and_reaps_success_and_panic() {
        let registry = WorkerRegistry::default();
        let success = registry.spawn(|| ExecutorResult::Exited(42));
        let panic = registry.spawn(|| panic!("worker failure"));
        let deadline = Instant::now() + Duration::from_secs(5);
        while registry.has_running() {
            assert!(Instant::now() < deadline);
            std::thread::yield_now();
        }
        assert!(matches!(
            success.take_outcome(),
            Some(Ok(ExecutorResult::Exited(42)))
        ));
        assert!(success.is_finished());
        assert!(success.take_outcome().is_none());
        assert!(matches!(
            panic.state.lock().entries[panic.index].lifecycle,
            Lifecycle::Leaked(_)
        ));
        assert!(matches!(panic.take_outcome(), Some(Err(_))));
        registry.shutdown(|| {});
        registry.shutdown(|| {});
    }
}

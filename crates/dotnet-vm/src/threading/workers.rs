//! VM-owned native workers and their managed completion states.

use crate::ExecutorResult;
#[cfg(feature = "multithreading")]
use dotnet_utils::sync::{Arc, Mutex};
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
}

#[cfg(feature = "multithreading")]
#[derive(Default)]
struct RegistryState {
    entries: Vec<Entry>,
}

/// The completion and the handle share one lifecycle lock, but only the registry owns handles.
#[cfg(feature = "multithreading")]
#[derive(Default)]
pub(crate) struct WorkerRegistry {
    state: Arc<Mutex<RegistryState>>,
}

#[cfg(feature = "multithreading")]
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
    pub(crate) fn spawn_owner(
        &self,
        run: impl FnOnce(RunningWorker) + Send + 'static,
    ) -> WorkerCompletion {
        let mut state = self.state.lock();
        let index = state.entries.len();
        let published = Arc::clone(&self.state);
        let handle = std::thread::spawn(move || {
            let token = RunningWorker {
                state: Arc::clone(&published),
                index,
            };
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

    pub(crate) fn has_running(&self) -> bool {
        self.state
            .lock()
            .entries
            .iter()
            .any(|entry| matches!(entry.lifecycle, Lifecycle::Running))
    }

    pub(crate) fn has_finalizing(&self) -> bool {
        self.state
            .lock()
            .entries
            .iter()
            .any(|entry| matches!(entry.lifecycle, Lifecycle::Finalizing(_)))
    }

    pub(crate) fn shutdown(&self, signal: impl FnOnce()) {
        // The caller has stopped being a GC participant before waiting here. A worker may
        // start another worker, so observe the complete set under the same lock.
        let handles = loop {
            let mut state = self.state.lock();
            if state
                .entries
                .iter()
                .all(|entry| !matches!(entry.lifecycle, Lifecycle::Running))
            {
                break state
                    .entries
                    .iter_mut()
                    .filter_map(|entry| entry.handle.take())
                    .collect::<Vec<_>>();
            }
            drop(state);
            std::thread::sleep(std::time::Duration::from_millis(1));
        };
        signal();
        for handle in handles {
            let _ = handle.join();
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
    pub(crate) fn has_running(&self) -> bool {
        false
    }
    pub(crate) fn has_finalizing(&self) -> bool {
        false
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
                }],
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
    fn finalizing_preserves_managed_completion() {
        let state = Arc::new(Mutex::new(RegistryState {
            entries: vec![Entry {
                lifecycle: Lifecycle::Running,
                handle: None,
            }],
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
            }],
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

#![cfg(all(loom, feature = "multithreading"))]

//! Bounded models using the production registry and Loom-scheduled threads/signals.

use dotnet_vm::ExecutorResult;
use loom::{
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    thread,
};
use std::time::Duration;

#[allow(dead_code)]
#[path = "../src/threading/workers.rs"]
mod workers;
use workers::WorkerRegistry;

fn check(f: impl Fn() + Send + Sync + 'static) {
    let mut builder = loom::model::Builder::new();
    builder.max_threads = 3;
    builder.preemption_bound = Some(2);
    builder.max_branches = 20_000;
    builder.max_duration = None;
    builder.max_permutations = None;
    builder.check(f);
}

#[test]
fn completion_wait_publication_and_join() {
    check(|| {
        let registry = WorkerRegistry::default();
        let cleaned = Arc::new(AtomicBool::new(false));
        let cleaned_worker = cleaned.clone();
        let completion = registry.spawn_owner(move |running| {
            running
                .complete_with_cleanup(ExecutorResult::Exited(42), || {
                    cleaned_worker.store(true, Ordering::Release);
                })
                .retire();
        });
        assert_eq!(completion.index(), 0);
        completion.wait(Duration::from_millis(1));
        assert!(completion.is_finished());
        assert!(cleaned.load(Ordering::Acquire));
        assert!(matches!(
            completion.take_outcome(),
            Some(Ok(ExecutorResult::Exited(42)))
        ));
        assert!(completion.take_outcome().is_none());
        registry.shutdown(|| {});
        registry.shutdown(|| {});
        assert_eq!(completion.phase(), "retired");
    });
}

#[test]
fn abort_finalization_and_shutdown_exclude_collection() {
    check(|| {
        let registry = Arc::new(WorkerRegistry::default());
        let abort = Arc::new(AtomicBool::new(false));
        let collection = Arc::new(Mutex::new(()));
        let torn_down = Arc::new(AtomicBool::new(false));
        let wake = Arc::new((Mutex::new(false), Condvar::new()));
        let exits = Arc::new(AtomicUsize::new(0));
        let (worker_abort, worker_collection, worker_torn, worker_wake, worker_exits) = (
            abort.clone(),
            collection.clone(),
            torn_down.clone(),
            wake.clone(),
            exits.clone(),
        );
        let worker_registry = registry.clone();
        let completion = registry.spawn_owner(move |running| {
            let retired = running.complete(ExecutorResult::Exited(7)).retire();
            if retired.begin_finalizing(|| worker_abort.load(Ordering::Acquire)) {
                assert!(!worker_torn.load(Ordering::Acquire));
                retired.finish_finalizing();
            }
            let mut signaled = worker_wake.0.lock().unwrap();
            while !*signaled {
                signaled = worker_wake.1.wait(signaled).unwrap();
            }
            drop(signaled);
            let _guard = worker_collection.lock().unwrap();
            assert!(worker_abort.load(Ordering::Acquire));
            assert!(worker_registry.is_quiescent(|| true));
            retired.begin_shutdown();
            drop(_guard);
            retired.finish_shutdown(true);
            worker_exits.fetch_add(1, Ordering::SeqCst);
        });
        let (collector_registry, collector_collection, collector_abort, collector_torn) = (
            registry.clone(),
            collection.clone(),
            abort.clone(),
            torn_down.clone(),
        );
        let collector = thread::spawn(move || {
            let _guard = collector_collection.lock().unwrap();
            if collector_abort.load(Ordering::Acquire) && collector_registry.is_quiescent(|| true) {
                collector_torn.store(true, Ordering::Release);
            }
        });
        abort.store(true, Ordering::Release);
        registry.shutdown(|| {
            *wake.0.lock().unwrap() = true;
            wake.1.notify_all();
        });
        collector.join().unwrap();
        registry.shutdown(|| {});
        assert_eq!(exits.load(Ordering::SeqCst), 1);
        assert_eq!(completion.phase(), "drained");
        assert!(matches!(
            completion.take_outcome(),
            Some(Ok(ExecutorResult::Exited(7)))
        ));
        assert!(completion.take_outcome().is_none());
    });
}

#[test]
fn finalizing_blocks_quiescence_until_finished() {
    check(|| {
        let registry = WorkerRegistry::default();
        let phase = Arc::new((Mutex::new((false, false)), Condvar::new()));
        let worker_phase = phase.clone();
        let completion = registry.spawn_owner(move |running| {
            let retired = running.complete(ExecutorResult::Exited(1)).retire();
            assert!(retired.begin_finalizing(|| false));
            let mut state = worker_phase.0.lock().unwrap();
            state.0 = true;
            worker_phase.1.notify_all();
            while !state.1 {
                state = worker_phase.1.wait(state).unwrap();
            }
            drop(state);
            retired.finish_finalizing();
            retired.begin_shutdown();
            retired.finish_shutdown(false);
        });
        let mut state = phase.0.lock().unwrap();
        while !state.0 {
            state = phase.1.wait(state).unwrap();
        }
        assert!(!registry.is_quiescent(|| true));
        assert_eq!(completion.phase(), "finalizing");
        state.1 = true;
        phase.1.notify_all();
        drop(state);
        registry.shutdown(|| {});
        assert_eq!(completion.phase(), "leaked");
        assert!(matches!(
            completion.take_outcome(),
            Some(Ok(ExecutorResult::Exited(1)))
        ));
        assert_eq!(registry.unjoined_count(), 0);
    });
}

#[test]
fn stable_indices_and_handles_across_multiple_workers() {
    check(|| {
        let registry = WorkerRegistry::default();
        let first = registry.spawn_owner(|running| {
            running.complete(ExecutorResult::Exited(11)).retire();
        });
        let second = registry.spawn_owner(|running| {
            running.complete(ExecutorResult::Exited(22)).retire();
        });
        assert_eq!((first.index(), second.index()), (0, 1));
        assert_eq!(registry.unjoined_count(), 2);
        registry.shutdown(|| {});
        assert_eq!(registry.unjoined_count(), 0);
        registry.shutdown(|| {});
        assert_eq!(registry.unjoined_count(), 0);
        assert!(matches!(
            first.take_outcome(),
            Some(Ok(ExecutorResult::Exited(11)))
        ));
        assert!(matches!(
            second.take_outcome(),
            Some(Ok(ExecutorResult::Exited(22)))
        ));
        assert!(first.take_outcome().is_none());
        assert!(second.take_outcome().is_none());
    });
}

#[test]
fn shutdown_joins_worker_spawned_after_signal() {
    check(|| {
        let registry = Arc::new(WorkerRegistry::default());
        let wake = Arc::new((Mutex::new(false), Condvar::new()));
        let late = Arc::new(Mutex::new(None));
        let (owner_registry, owner_wake, owner_late) =
            (registry.clone(), wake.clone(), late.clone());
        let first = registry.spawn_owner(move |running| {
            let retired = running.complete(ExecutorResult::Exited(3)).retire();
            let mut signaled = owner_wake.0.lock().unwrap();
            while !*signaled {
                signaled = owner_wake.1.wait(signaled).unwrap();
            }
            drop(signaled);
            let completion = owner_registry.spawn_owner(|running| {
                running.complete(ExecutorResult::Exited(4)).retire();
            });
            *owner_late.lock().unwrap() = Some(completion);
            retired.begin_shutdown();
            retired.finish_shutdown(true);
        });
        registry.shutdown(|| {
            *wake.0.lock().unwrap() = true;
            wake.1.notify_all();
        });
        assert_eq!(registry.unjoined_count(), 0);
        assert_eq!(first.phase(), "drained");
        let second = late.lock().unwrap().take().unwrap();
        assert_eq!((first.index(), second.index()), (0, 1));
        assert!(matches!(
            first.take_outcome(),
            Some(Ok(ExecutorResult::Exited(3)))
        ));
        assert!(matches!(
            second.take_outcome(),
            Some(Ok(ExecutorResult::Exited(4)))
        ));
        registry.shutdown(|| {});
        assert_eq!(registry.unjoined_count(), 0);
    });
}

#[test]
fn abort_prevents_new_finalization() {
    check(|| {
        let registry = WorkerRegistry::default();
        let aborted = Arc::new(AtomicBool::new(true));
        let worker_abort = aborted.clone();
        let completion = registry.spawn_owner(move |running| {
            let retired = running.complete(ExecutorResult::Exited(0)).retire();
            assert!(!retired.begin_finalizing(|| worker_abort.load(Ordering::Acquire)));
            retired.begin_shutdown();
            retired.finish_shutdown(true);
        });
        registry.shutdown(|| {});
        assert_eq!(completion.phase(), "drained");
    });
}

#[test]
fn panic_publishes_leaked_outcome_and_wakes_waiter() {
    check(|| {
        let registry = WorkerRegistry::default();
        let completion = registry.spawn_owner(|_| panic!("worker failed"));
        completion.wait(Duration::from_millis(1));
        assert_eq!(completion.phase(), "leaked");
        assert!(matches!(completion.take_outcome(), Some(Err(_))));
        assert!(completion.take_outcome().is_none());
        registry.shutdown(|| {});
        assert_eq!(registry.unjoined_count(), 0);
    });
}

#[test]
fn release_excludes_collection_and_shutdown_joins_once() {
    check(|| {
        let registry = Arc::new(WorkerRegistry::default());
        // This models collection exclusion and registration, not gc-arena marking.
        let registered = Arc::new(Mutex::new(true));
        let drains = Arc::new(AtomicUsize::new(0));
        let (worker_registered, worker_drains) = (registered.clone(), drains.clone());
        let completion = registry.spawn_owner(move |running| {
            let retired = running.complete(ExecutorResult::Exited(42)).retire();
            let releasing = {
                let mut registered = worker_registered.lock().unwrap();
                let releasing = retired.begin_release().unwrap();
                assert!(*registered);
                *registered = false;
                releasing
            };
            // Drain takes place after dropping collection exclusion.
            worker_drains.fetch_add(1, Ordering::SeqCst);
            releasing.finish(true);
        });
        let collector_registered = registered.clone();
        let collector_drains = drains.clone();
        let collector = thread::spawn(move || {
            let registered = collector_registered.lock().unwrap();
            if *registered {
                // A registered owner is still available for this collection snapshot.
                assert_eq!(collector_drains.load(Ordering::SeqCst), 0);
            }
        });
        registry.shutdown(|| {});
        collector.join().unwrap();
        registry.shutdown(|| {});
        assert!(!*registered.lock().unwrap());
        assert_eq!(drains.load(Ordering::SeqCst), 1);
        assert_eq!(registry.joins_completed(), 1);
        assert_eq!(registry.unjoined_count(), 0);
        assert_eq!(completion.phase(), "released");
        assert!(matches!(
            completion.take_outcome(),
            Some(Ok(ExecutorResult::Exited(42)))
        ));
        assert!(completion.take_outcome().is_none());
    });
}

#[test]
fn release_reaper_and_shutdown_join_once() {
    check(|| {
        let registry = Arc::new(WorkerRegistry::default());
        let drains = Arc::new(AtomicUsize::new(0));
        let worker_drains = drains.clone();
        let completion = registry.spawn_owner(move |running| {
            let retired = running.complete(ExecutorResult::Exited(17)).retire();
            let releasing = retired.begin_release().unwrap();
            worker_drains.fetch_add(1, Ordering::SeqCst);
            releasing.finish(true);
        });
        let reaper_registry = registry.clone();
        let reaper = thread::spawn(move || reaper_registry.reap_released());
        registry.shutdown(|| {});
        reaper.join().unwrap();
        registry.reap_released();
        registry.shutdown(|| {});
        assert_eq!(drains.load(Ordering::SeqCst), 1);
        assert_eq!(registry.joins_completed(), 1);
        assert!(completion.is_joined());
        assert_eq!(completion.phase(), "released");
        assert_eq!(completion.index(), 0);
        assert!(matches!(
            completion.take_outcome(),
            Some(Ok(ExecutorResult::Exited(17)))
        ));
        assert!(completion.take_outcome().is_none());
    });
}

#[test]
fn failed_release_preserves_consumed_outcome_through_shutdown() {
    check(|| {
        let registry = WorkerRegistry::default();
        let phase = Arc::new((Mutex::new((false, false)), Condvar::new()));
        let worker_phase = phase.clone();
        let completion = registry.spawn_owner(move |running| {
            let retired = running.complete(ExecutorResult::Exited(5)).retire();
            let releasing = retired.begin_release().unwrap();
            let mut state = worker_phase.0.lock().unwrap();
            state.0 = true;
            worker_phase.1.notify_all();
            while !state.1 {
                state = worker_phase.1.wait(state).unwrap();
            }
            drop(state);
            // Dropping an unfinished guard publishes Leaked, not Released.
            drop(releasing);
        });
        let mut state = phase.0.lock().unwrap();
        while !state.0 {
            state = phase.1.wait(state).unwrap();
        }
        assert_eq!(completion.phase(), "releasing");
        assert!(matches!(
            completion.take_outcome(),
            Some(Ok(ExecutorResult::Exited(5)))
        ));
        state.1 = true;
        phase.1.notify_all();
        drop(state);
        registry.shutdown(|| {});
        assert_eq!(completion.phase(), "leaked");
        assert!(completion.take_outcome().is_none());
        assert_eq!(registry.joins_completed(), 1);
    });
}

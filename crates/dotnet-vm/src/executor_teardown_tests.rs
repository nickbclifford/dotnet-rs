//! Executor arena teardown ordering with active cross-arena leases.
//!
//! The witness is a real GC allocation in the Executor's arena, but carries no GC
//! pointers and is never dereferenced across threads. The ordering tests do not
//! collect; the stress recreation also drives real collections.

use super::*;
use dotnet_assemblies::AssemblyLoader;
use gc_arena::{Collect, Gc};
use std::sync::{
    Arc as StdArc,
    atomic::{AtomicUsize, Ordering as AtomicOrdering},
};

#[derive(Collect)]
#[collect(require_static)]
struct DropWitness(StdArc<AtomicUsize>);

impl Drop for DropWitness {
    fn drop(&mut self) {
        self.0.fetch_add(1, AtomicOrdering::SeqCst);
    }
}

fn shared() -> Arc<SharedGlobalState> {
    #[allow(
        clippy::arc_with_non_send_sync,
        reason = "the single-thread configuration keeps shared state on its Executor's thread"
    )]
    Arc::new(SharedGlobalState::new(Arc::new(
        AssemblyLoader::new_bare("executor_teardown".to_string()).unwrap(),
    )))
}

fn allocate_witness(executor: &mut Executor, drops: StdArc<AtomicUsize>) {
    executor
        .with_arena(|arena| {
            arena.mutate(|gc, _| {
                let _witness = Gc::new(gc, DropWitness(drops));
            });
        })
        .unwrap();
}

#[cfg(feature = "multithreading")]
struct ShutdownOnDrop(Arc<SharedGlobalState>);

#[cfg(feature = "multithreading")]
impl Drop for ShutdownOnDrop {
    fn drop(&mut self) {
        self.0.shutdown_workers();
    }
}

#[cfg(not(feature = "multithreading"))]
#[test]
fn executor_single_arena_cleanup_control() {
    let drops = StdArc::new(AtomicUsize::new(0));
    let mut executor = Executor::new(shared());
    allocate_witness(&mut executor, StdArc::clone(&drops));
    assert_eq!(drops.load(AtomicOrdering::SeqCst), 0);
    drop(executor);
    assert_eq!(drops.load(AtomicOrdering::SeqCst), 1);
}

#[cfg(feature = "multithreading")]
#[test]
fn retired_owner_keeps_registration_until_shutdown() {
    use dotnet_utils::gc::cross_arena::try_acquire_lease;
    use std::time::Duration;

    let state = shared();
    let worker_state = state.clone();
    let drops = StdArc::new(AtomicUsize::new(0));
    let worker_drops = drops.clone();
    let (ready, receiver) = std::sync::mpsc::channel();
    let completion = state.workers.spawn_owner(move |running| {
        let mut executor = Executor::new(worker_state);
        allocate_witness(&mut executor, worker_drops);
        ready.send(executor.thread_id).unwrap();
        executor.serve_retired(running.complete(ExecutorResult::Exited(0)));
    });
    let id = match receiver.recv_timeout(Duration::from_secs(30)) {
        Ok(id) => id,
        Err(error) => {
            state.shutdown_workers();
            panic!("worker did not become ready: {error}");
        }
    };
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    while !completion.is_finished() && std::time::Instant::now() < deadline {
        std::thread::yield_now();
    }
    let finished = completion.is_finished();
    let lease = try_acquire_lease(id);
    let alive = lease.is_some();
    let before_shutdown = drops.load(AtomicOrdering::SeqCst);
    let shutdown_state = state.clone();
    let shutdown = std::thread::spawn(move || shutdown_state.shutdown_workers());
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    while lease.as_ref().is_some_and(|lease| lease.is_valid())
        && std::time::Instant::now() < deadline
    {
        std::thread::yield_now();
    }
    let removed = lease.as_ref().is_some_and(|lease| !lease.is_valid());
    let collection_lock_available = state.gc_coordinator.try_exclude_collection(|| ()).is_some();
    let during_drain = drops.load(AtomicOrdering::SeqCst);
    let phase_during_drain = completion.phase();
    // Release before joining or asserting, including on a failed observation.
    drop(lease);
    shutdown.join().unwrap();
    assert!(finished);
    assert!(alive);
    assert_eq!(before_shutdown, 0);
    assert!(removed);
    assert!(
        collection_lock_available,
        "lease drain must not hold the collection lock"
    );
    assert_eq!(during_drain, 0);
    assert_eq!(phase_during_drain, "shutting down");
    assert_eq!(completion.phase(), "drained");
    assert!(try_acquire_lease(id).is_none());
    assert_eq!(drops.load(AtomicOrdering::SeqCst), 1);
}

#[cfg(feature = "multithreading")]
#[test]
fn retired_shutdown_waits_for_inflight_collection_and_lease() {
    use crate::{
        gc::coordinator::GcCycleGuard, test_utils::worker::Worker, threading::ThreadManagerOps,
    };
    use dotnet_utils::gc::cross_arena::try_acquire_lease;
    use std::{sync::mpsc, time::Duration};

    let state = shared();
    let _cleanup = ShutdownOnDrop(state.clone());
    let worker_state = state.clone();
    let (ready_tx, ready_rx) = mpsc::channel();
    let completion = state.workers.spawn_owner(move |running| {
        let executor = Executor::new(worker_state);
        ready_tx.send(executor.thread_id).unwrap();
        executor.serve_retired(running.complete(ExecutorResult::Exited(0)));
    });
    let id = ready_rx.recv_timeout(Duration::from_secs(30)).unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    while !completion.is_finished() && std::time::Instant::now() < deadline {
        std::thread::yield_now();
    }
    let completed = completion.is_finished();
    let collector = Executor::new(state.clone());
    let (lease_ready_tx, lease_ready_rx) = mpsc::channel();
    let (probe_tx, probe_rx) = mpsc::channel();
    let (removed_tx, removed_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let holder = std::thread::spawn(move || {
        // This host thread is not a GC participant and never crosses a safepoint.
        let lease = try_acquire_lease(id).unwrap();
        lease_ready_tx.send(()).unwrap();
        if probe_rx.recv_timeout(Duration::from_secs(30)).is_ok() {
            let deadline = std::time::Instant::now() + Duration::from_secs(30);
            while lease.is_valid() && std::time::Instant::now() < deadline {
                std::thread::yield_now();
            }
            let _ = removed_tx.send(!lease.is_valid());
            let _ = release_rx.recv_timeout(Duration::from_secs(30));
        }
        drop(lease);
    });
    let holder = Worker::new(holder, (probe_tx, release_tx));
    lease_ready_rx
        .recv_timeout(Duration::from_secs(30))
        .unwrap();
    let shutdown;
    let coordinator = state.gc_coordinator.clone();
    let session = coordinator.begin_collection().unwrap();
    let stw = state.thread_manager.request_stop_the_world();
    let cycle = GcCycleGuard::new(session, stw);

    let (signaled_tx, signaled_rx) = mpsc::channel();
    let (done_tx, done_rx) = mpsc::channel();
    let shutdown_state = state.clone();
    let shutdown_thread = std::thread::spawn(move || {
        shutdown_state.workers.shutdown(|| {
            shutdown_state.thread_manager.shutdown_retired();
            signaled_tx.send(()).unwrap();
        });
        let _ = done_tx.send(());
    });
    shutdown = Worker::new(shutdown_thread, holder);
    let signaled = signaled_rx.recv_timeout(Duration::from_secs(30)).is_ok();
    let valid_during_collection = try_acquire_lease(id).is_some();
    cycle.collect_all_arenas(collector.thread_id);
    drop(cycle);
    let _ = shutdown.resources().resources().0.send(());
    let removed = removed_rx
        .recv_timeout(Duration::from_secs(30))
        .unwrap_or(false);
    let rejected = try_acquire_lease(id).is_none();
    let held_up_shutdown = done_rx.try_recv().is_err();
    let phase_during_drain = completion.phase();
    let _ = shutdown.resources().resources().1.send(());
    shutdown.finish().unwrap();
    drop(collector);
    assert!(completed);
    assert!(signaled);
    assert!(valid_during_collection);
    assert!(removed);
    assert!(rejected);
    assert!(held_up_shutdown);
    assert_eq!(phase_during_drain, "shutting down");
    assert_eq!(completion.phase(), "drained");
}

#[cfg(feature = "multithreading")]
#[test]
fn worker_panic_drains_lease_before_shutdown() {
    use dotnet_utils::gc::cross_arena::try_acquire_lease;
    use std::{sync::mpsc, time::Duration};

    let state = shared();
    let _cleanup = ShutdownOnDrop(state.clone());
    let drops = StdArc::new(AtomicUsize::new(0));
    let worker_drops = drops.clone();
    let worker_state = state.clone();
    let (ready_tx, ready_rx) = mpsc::channel();
    let (start_tx, start_rx) = mpsc::channel();
    let completion = state.workers.spawn_owner(move |_running| {
        let mut executor = Executor::new(worker_state);
        allocate_witness(&mut executor, worker_drops);
        ready_tx.send(executor.thread_id).unwrap();
        let _ = start_rx.recv_timeout(Duration::from_secs(30));
        panic!("worker failed while owning an arena");
    });
    let id = ready_rx.recv_timeout(Duration::from_secs(30)).unwrap();
    let lease = try_acquire_lease(id).unwrap();
    start_tx.send(()).unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    while lease.is_valid() && std::time::Instant::now() < deadline {
        std::thread::yield_now();
    }
    let removed = !lease.is_valid();
    let rejected = try_acquire_lease(id).is_none();
    let alive = drops.load(AtomicOrdering::SeqCst) == 0;
    let shutdown_state = state.clone();
    let shutdown = std::thread::spawn(move || shutdown_state.shutdown_workers());
    drop(lease);
    shutdown.join().unwrap();
    assert!(removed && rejected && alive);
    assert_eq!(completion.phase(), "leaked");
    assert!(matches!(completion.take_outcome(), Some(Err(_))));
    assert_eq!(drops.load(AtomicOrdering::SeqCst), 1);
}

#[cfg(feature = "multithreading")]
#[test]
fn retired_self_held_lease_leaks_storage_at_shutdown() {
    use dotnet_utils::gc::cross_arena::try_acquire_lease;
    use std::{sync::mpsc, time::Duration};

    let state = shared();
    let _cleanup = ShutdownOnDrop(state.clone());
    let drops = StdArc::new(AtomicUsize::new(0));
    let worker_drops = drops.clone();
    let worker_state = state.clone();
    let (ready_tx, ready_rx) = mpsc::channel();
    let completion = state.workers.spawn_owner(move |running| {
        let mut executor = Executor::new(worker_state);
        allocate_witness(&mut executor, worker_drops);
        let lease = try_acquire_lease(executor.thread_id).unwrap();
        ready_tx.send(executor.thread_id).unwrap();
        executor.serve_retired(running.complete(ExecutorResult::Exited(0)));
        drop(lease);
    });
    let id = ready_rx.recv_timeout(Duration::from_secs(30)).unwrap();
    state.shutdown_workers();
    assert_eq!(completion.phase(), "leaked");
    assert!(try_acquire_lease(id).is_none());
    assert_eq!(drops.load(AtomicOrdering::SeqCst), 0);
}

#[cfg(feature = "multithreading")]
#[test]
fn worker_start_failure_retires_and_shutdown_reaps() {
    use crate::test_utils::parameterless_i32_signature;
    use dotnet_types::{TypeDescription, generics::GenericLookup, members::MethodDescription};
    use dotnetdll::{prelude::*, resolved::members::Method};
    use std::time::Duration;

    let loader = AssemblyLoader::new_bare("worker-start-failure".into()).unwrap();
    let (mut res, _, ty) = make_test_assembly!("InvalidStart.dll", "InvalidStart", "Worker");
    let index = res.push_method(
        ty,
        Method::new(
            Accessibility::Public,
            parameterless_i32_signature(),
            "Start",
            Some(make_test_method!(
                max_stack: 1,
                instructions: vec![Instruction::LoadConstantInt32(0), Instruction::Return],
                locals: vec![LocalVariable::Variable {
                    custom_modifiers: vec![],
                    pinned: false,
                    by_ref: false,
                    var_type: MethodType::MethodGeneric(42),
                }]
            )),
        ),
    );
    let res = loader.register_owned_assembly(res);
    let ty = TypeDescription::new(res.clone(), ty);
    let index = ty
        .definition()
        .methods
        .iter()
        .position(|m| std::ptr::eq(m, &res.definition()[index]))
        .unwrap();
    let method = MethodDescription::new(
        ty,
        GenericLookup::default(),
        res,
        MethodMemberIndex::Method(index),
    );
    let state = Arc::new(SharedGlobalState::new(Arc::new(loader)));
    let _cleanup = ShutdownOnDrop(state.clone());
    let worker_state = state.clone();
    let completion = state.workers.spawn_owner(move |running| {
        Executor::run_thread_start(worker_state, method, GenericLookup::default(), running);
    });
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    while !completion.is_finished() && std::time::Instant::now() < deadline {
        std::thread::yield_now();
    }
    let outcome = completion.take_outcome();
    state.shutdown_workers();
    assert_eq!(completion.phase(), "drained");
    match outcome {
        Some(Ok(ExecutorResult::Error(error))) => assert!(
            format!("{error:?}").contains("GenericIndexOutOfBounds"),
            "unexpected startup error: {error:?}"
        ),
        other => panic!("expected startup failure, got {other:?}"),
    }
}

#[cfg(feature = "multithreading")]
fn check_teardown(extracted: bool, unwind: bool) {
    use crate::test_utils::worker::Worker;
    use dotnet_utils::gc::cross_arena::try_acquire_lease;
    use std::{
        sync::mpsc,
        thread,
        time::{Duration, Instant},
    };

    let drops = StdArc::new(AtomicUsize::new(0));
    let worker_drops = StdArc::clone(&drops);
    let (ready_tx, ready_rx) = mpsc::channel();
    let (start_tx, start_rx) = mpsc::channel();
    let (done_tx, done_rx) = mpsc::channel();
    // Loader initialization can be slow under Miri; do it before starting any
    // protocol deadline, then move the real shared state to the Executor thread.
    let state = shared();
    let worker = thread::spawn(move || {
        let mut executor = Executor::new(state);
        let id = executor.thread_id;
        allocate_witness(&mut executor, worker_drops);
        if extracted {
            let guard = executor.extract_arena().expect("Executor arena");
            drop(executor);
            ready_tx.send(id).unwrap();
            if start_rx.recv_timeout(Duration::from_secs(10)).is_ok() {
                drop(guard);
            }
        } else {
            ready_tx.send(id).unwrap();
            if start_rx.recv_timeout(Duration::from_secs(10)).is_ok() {
                if unwind {
                    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        let _executor = executor;
                        panic!("independent recreation: unwind with an active lease");
                    }));
                    assert!(result.is_err());
                } else {
                    drop(executor);
                }
            }
        }
        let _ = done_tx.send(());
    });

    let mut worker = Worker::new(worker, (start_tx, None));
    let id = ready_rx
        .recv_timeout(Duration::from_secs(10))
        .expect("Executor ready");
    worker.resources_mut().1 =
        Some(try_acquire_lease(id).expect("actual Executor arena registered"));
    let lease = worker.resources().1.as_ref().unwrap();
    let before = drops.load(AtomicOrdering::SeqCst);
    worker.resources().0.send(()).unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while lease.is_valid() && Instant::now() < deadline {
        thread::yield_now();
    }
    let closing = !lease.is_valid();
    let rejected = closing && try_acquire_lease(id).is_none();
    let during = drops.load(AtomicOrdering::SeqCst);
    let completed_while_held = done_rx.try_recv().is_ok();

    // Always release before any assertions: a failing regression must not strand
    // Executor or ArenaGuard in unregister_arena's lease drain.
    drop(worker.resources_mut().1.take());
    let completed = completed_while_held || done_rx.recv_timeout(Duration::from_secs(10)).is_ok();
    worker.finish().unwrap();
    let after = drops.load(AtomicOrdering::SeqCst);
    assert_eq!(before, 0, "witness died before teardown");
    assert!(closing, "teardown did not reach registry closing");
    assert!(rejected, "new leases must be rejected while closing");
    assert!(
        !completed_while_held,
        "teardown returned while a lease was held"
    );
    assert!(completed, "teardown did not complete after lease release");
    assert_eq!(after, 1, "witness must be destroyed exactly once");
    assert_eq!(
        during, 0,
        "arena allocation destroyed before active lease drained"
    );
}

/// A driver panic, both before readiness and with a real lease held, must
/// disconnect the worker, release the lease, and join before unwind returns.
#[cfg(feature = "multithreading")]
#[test]
fn worker_driver_failure_releases_and_joins() {
    use crate::test_utils::worker::Worker;
    use dotnet_utils::gc::cross_arena::try_acquire_lease;
    use std::{panic::AssertUnwindSafe, sync::mpsc, thread, time::Duration};

    struct Joined(StdArc<AtomicUsize>, thread::ThreadId);
    impl Drop for Joined {
        fn drop(&mut self) {
            if thread::current().id() == self.1 {
                self.0.fetch_add(1, AtomicOrdering::SeqCst);
            }
        }
    }

    for after_ready in [false, true] {
        let drops = StdArc::new(AtomicUsize::new(0));
        let exited = StdArc::new(AtomicUsize::new(0));
        let joined = StdArc::new(AtomicUsize::new(0));
        let worker_drops = StdArc::clone(&drops);
        let worker_exited = StdArc::clone(&exited);
        let worker_joined = StdArc::clone(&joined);
        let driver_id = thread::current().id();
        let state = shared();
        let result = std::panic::catch_unwind(AssertUnwindSafe(|| {
            let (ready_tx, ready_rx) = mpsc::channel();
            let (release_tx, release_rx) = mpsc::channel::<()>();
            let handle = thread::spawn(move || {
                let mut executor = Executor::new(state);
                allocate_witness(&mut executor, worker_drops);
                let _ = ready_tx.send(executor.thread_id);
                let _ = release_rx.recv(); // disconnect is cancellation
                drop(executor);
                worker_exited.fetch_add(1, AtomicOrdering::SeqCst);
                Joined(worker_joined, driver_id)
            });
            let mut worker = Worker::new(handle, (release_tx, None));
            if after_ready {
                let id = ready_rx.recv_timeout(Duration::from_secs(30)).unwrap();
                worker.resources_mut().1 = Some(try_acquire_lease(id).unwrap());
            }
            panic!("injected driver failure");
        }));
        assert!(result.is_err());
        assert_eq!(drops.load(AtomicOrdering::SeqCst), 1, "arena released");
        assert_eq!(exited.load(AtomicOrdering::SeqCst), 1, "worker exited");
        assert_eq!(
            joined.load(AtomicOrdering::SeqCst),
            1,
            "join result consumed"
        );
    }
    let worker = Worker::new(thread::spawn(|| panic!("injected worker failure")), ());
    assert!(
        worker.finish().is_err(),
        "finish must report worker failure"
    );
    let driver = std::panic::catch_unwind(|| {
        let (tx, rx) = mpsc::channel::<()>();
        let handle = thread::spawn(move || {
            let _ = rx.recv();
            panic!("worker failure during driver unwind");
        });
        let _worker = Worker::new(handle, tx);
        panic!("injected driver failure with a failing worker");
    });
    assert!(
        driver.is_err(),
        "worker failure must not cause a double panic"
    );
}

#[cfg(feature = "multithreading")]
#[test]
fn executor_normal_teardown_active_lease() {
    check_teardown(false, false);
}

#[cfg(feature = "multithreading")]
#[test]
fn executor_extracted_teardown_active_lease() {
    check_teardown(true, false);
}

/// Unwinding an actual Executor
/// must still drain another thread's lease before its arena is destroyed.
#[cfg(feature = "multithreading")]
#[test]
fn executor_unwind_teardown_active_lease() {
    check_teardown(false, true);
}

/// Repeated real arena collection
/// followed by a separate, unrooted teardown witness. A lease does not root objects.
#[cfg(not(feature = "multithreading"))]
#[test]
fn arena_lifetime_teardown_stress() {
    let state = shared();
    let rounds = if cfg!(miri) { 2 } else { 20 };
    for _ in 0..rounds {
        let collected = StdArc::new(AtomicUsize::new(0));
        let teardown = StdArc::new(AtomicUsize::new(0));
        let mut executor = Executor::new(Arc::clone(&state));
        allocate_witness(&mut executor, StdArc::clone(&collected));
        executor.with_arena(|arena| arena.finish_cycle()).unwrap();
        assert_eq!(collected.load(AtomicOrdering::SeqCst), 1);
        allocate_witness(&mut executor, StdArc::clone(&teardown));
        assert_eq!(teardown.load(AtomicOrdering::SeqCst), 0);
        drop(executor);
        assert_eq!(teardown.load(AtomicOrdering::SeqCst), 1);
    }
}

/// Collect two actual Executor
/// arenas, then deliberately hold a lease across B's exit to test the drain.
/// The collection witness is already reclaimed before the teardown witness is
/// allocated; no claim is made that the lease roots either allocation.
#[cfg(feature = "multithreading")]
#[test]
fn arena_lifetime_teardown_stress() {
    use crate::{gc::coordinator::GcCycleGuard, test_utils::worker::Worker};
    use dotnet_utils::gc::cross_arena::try_acquire_lease;
    use std::{
        sync::mpsc,
        thread,
        time::{Duration, Instant},
    };

    const DEADLINE: Duration = Duration::from_secs(30);
    let state = shared(); // Loader setup is outside the protocol deadlines.
    // Schedules cycle through {normal, extracted} x {lease held across exit,
    // lease released before exit}; Miri runs one round of each.
    let rounds = if cfg!(miri) { 4 } else { 20 };
    let mut observations = Vec::new();

    for round in 0..rounds {
        let extracted = round % 2 == 1;
        let hold_lease = round / 2 % 2 == 0;
        let collected = StdArc::new(AtomicUsize::new(0));
        let teardown = StdArc::new(AtomicUsize::new(0));
        let worker_collected = StdArc::clone(&collected);
        let worker_teardown = StdArc::clone(&teardown);
        let worker_state = Arc::clone(&state);
        let (registered_tx, registered_rx) = mpsc::channel();
        let (collected_tx, collected_rx) = mpsc::channel();
        let (ready_tx, ready_rx) = mpsc::channel();
        let (exit_tx, exit_rx) = mpsc::channel();
        let (done_tx, done_rx) = mpsc::channel();

        let worker = thread::spawn(move || {
            let mut b = Executor::new(Arc::clone(&worker_state));
            let id = b.thread_id;
            allocate_witness(&mut b, worker_collected);
            registered_tx.send(id).unwrap();
            let deadline = Instant::now() + DEADLINE;
            loop {
                worker_state
                    .thread_manager
                    .safe_point(id, &worker_state.gc_coordinator);
                match collected_rx.try_recv() {
                    Ok(()) => break,
                    Err(mpsc::TryRecvError::Disconnected) => return,
                    Err(mpsc::TryRecvError::Empty) => {
                        assert!(Instant::now() < deadline, "collection signal timed out");
                        thread::yield_now();
                    }
                }
            }
            allocate_witness(&mut b, worker_teardown);
            if extracted {
                let guard = b.extract_arena().expect("owner arena");
                drop(b);
                ready_tx.send(()).unwrap();
                if exit_rx.recv_timeout(DEADLINE).is_ok() {
                    drop(guard);
                }
            } else {
                ready_tx.send(()).unwrap();
                if exit_rx.recv_timeout(DEADLINE).is_ok() {
                    drop(b);
                }
            }
            let _ = done_tx.send(());
        });

        let mut worker = Worker::new(worker, (collected_tx, exit_tx, None));
        let id = registered_rx
            .recv_timeout(DEADLINE)
            .expect("owner registered");
        let a = Executor::new(Arc::clone(&state));
        let coordinator = Arc::clone(&state.gc_coordinator);
        let session = coordinator.begin_collection().expect("idle coordinator");
        let stw = state.thread_manager.request_stop_the_world();
        let cycle = GcCycleGuard::new(session, stw);
        cycle.collect_all_arenas(a.thread_id);
        drop(cycle);
        worker.resources().0.send(()).unwrap();
        ready_rx
            .recv_timeout(DEADLINE)
            .expect("owner ready to exit");
        let collected_before_exit = collected.load(AtomicOrdering::SeqCst);
        let before = teardown.load(AtomicOrdering::SeqCst);
        worker.resources_mut().2 = Some(try_acquire_lease(id).expect("owner is registered"));
        if !hold_lease {
            // Released lease: teardown must not wait and must still free exactly once.
            drop(worker.resources_mut().2.take());
            worker.resources().1.send(()).unwrap();
            let completed = done_rx.recv_timeout(DEADLINE).is_ok();
            worker.finish().expect("owner thread");
            drop(a);
            let after = teardown.load(AtomicOrdering::SeqCst);
            assert_eq!(
                collected_before_exit, 1,
                "round {round}: collection witness"
            );
            assert_eq!(before, 0, "round {round}: premature witness destruction");
            assert!(completed, "round {round}: released-lease teardown hung");
            assert_eq!(after, 1, "round {round}: witness must drop once");
            assert!(
                try_acquire_lease(id).is_none(),
                "round {round}: lease acquired on a torn-down arena"
            );
            continue;
        }
        worker.resources().1.send(()).unwrap();
        let lease = worker.resources().2.as_ref().unwrap();
        let deadline = Instant::now() + DEADLINE;
        while lease.is_valid() && Instant::now() < deadline {
            thread::yield_now();
        }
        let closing = !lease.is_valid();
        let rejected = closing && try_acquire_lease(id).is_none();
        let during = teardown.load(AtomicOrdering::SeqCst);
        let completed_held = done_rx.try_recv().is_ok();
        // Always release before asserting so the unregister drain can complete.
        drop(worker.resources_mut().2.take());
        let completed = completed_held || done_rx.recv_timeout(DEADLINE).is_ok();
        worker.finish().expect("owner thread");
        let after = teardown.load(AtomicOrdering::SeqCst);
        drop(a);
        observations.push((
            round,
            extracted,
            collected_before_exit,
            before,
            closing,
            rejected,
            during,
            completed_held,
            completed,
            after,
        ));
    }

    let mut early_drops = Vec::new();
    for (round, extracted, collected, before, closing, rejected, during, held, completed, after) in
        observations
    {
        assert_eq!(collected, 1, "round {round}: collection witness");
        assert_eq!(before, 0, "round {round}: premature witness destruction");
        assert!(closing && rejected, "round {round}: registry did not close");
        assert!(!held && completed, "round {round}: lease drain completion");
        assert_eq!(after, 1, "round {round}: witness must drop once");
        if during != 0 {
            early_drops.push((round, extracted, during));
        }
    }
    assert!(
        early_drops.is_empty(),
        "arena allocation destroyed before active lease drained: {early_drops:?}"
    );
}

// ---- TLS ownership protocol on real Executors ----

/// A second Executor on a thread that still owns one must fail closed
/// instead of replacing (and destroying) the first arena's TLS Box.
#[cfg(feature = "multithreading")]
#[test]
fn executor_nested_creation_fails_closed() {
    use std::panic::{AssertUnwindSafe, catch_unwind};
    let drops = StdArc::new(AtomicUsize::new(0));
    let worker_drops = StdArc::clone(&drops);
    let (first, second) = (shared(), shared());
    std::thread::spawn(move || {
        let mut first = Executor::new(first);
        allocate_witness(&mut first, worker_drops);
        let nested = catch_unwind(AssertUnwindSafe(|| Executor::new(second)));
        assert!(nested.is_err(), "nested Executor::new must fail closed");
        assert_eq!(
            drops.load(AtomicOrdering::SeqCst),
            0,
            "first arena destroyed by rejected nested creation"
        );
        drop(first);
        assert_eq!(drops.load(AtomicOrdering::SeqCst), 1);
    })
    .join()
    .unwrap();
}

/// An owner thread holding a lease on its own arena must not deadlock in
/// teardown, and must not free the leased arena.
#[cfg(feature = "multithreading")]
#[test]
fn executor_self_held_lease_leaks_instead_of_deadlocking() {
    use crate::test_utils::worker::Worker;
    use dotnet_utils::gc::cross_arena::try_acquire_lease;
    use std::{sync::mpsc, time::Duration};
    let drops = StdArc::new(AtomicUsize::new(0));
    let worker_drops = StdArc::clone(&drops);
    let state = shared();
    let (done_tx, done_rx) = mpsc::channel();
    let worker = std::thread::spawn(move || {
        let mut executor = Executor::new(state);
        allocate_witness(&mut executor, worker_drops);
        let lease = try_acquire_lease(executor.thread_id).unwrap();
        drop(executor); // would spin forever if it waited on its own lease
        let _ = done_tx.send(());
        drop(lease);
    });
    // An actual self-drain deadlock requires an outer process timeout: a join
    // guard cannot kill a stuck thread, and must never detach it as success.
    let worker = Worker::new(worker, ());
    done_rx
        .recv_timeout(Duration::from_secs(30))
        .expect("teardown deadlocked on a self-held lease");
    worker.finish().unwrap();
    assert_eq!(
        drops.load(AtomicOrdering::SeqCst),
        0,
        "leased arena must be leaked, not freed"
    );
}

/// An external ID-only remover that is already draining must not let the
/// owner free the arena while a lease is still held.
#[cfg(feature = "multithreading")]
#[test]
fn executor_external_remover_does_not_release_owner() {
    use crate::test_utils::worker::Worker;
    use dotnet_utils::gc::cross_arena::{try_acquire_lease, unregister_arena};
    use std::{
        sync::mpsc,
        thread,
        time::{Duration, Instant},
    };
    let drops = StdArc::new(AtomicUsize::new(0));
    let worker_drops = StdArc::clone(&drops);
    let state = shared();
    let (ready_tx, ready_rx) = mpsc::channel();
    let (go_tx, go_rx) = mpsc::channel::<()>();
    let (done_tx, done_rx) = mpsc::channel();
    let owner = thread::spawn(move || {
        let mut executor = Executor::new(state);
        allocate_witness(&mut executor, worker_drops);
        ready_tx.send(executor.thread_id).unwrap();
        let _ = go_rx.recv_timeout(Duration::from_secs(30));
        drop(executor);
        let _ = done_tx.send(());
    });
    let owner = Worker::new(owner, go_tx);
    let id = ready_rx.recv_timeout(Duration::from_secs(30)).unwrap();
    let lease = try_acquire_lease(id).unwrap();
    let remover = thread::spawn(move || unregister_arena(id)); // removes entry, drains
    // Remover is dropped first on unwind: release its lease before joining it,
    // then disconnect and join the owner.
    let mut remover = Worker::new(remover, Some(lease));
    let lease = remover.resources().as_ref().unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while lease.is_valid() && Instant::now() < deadline {
        thread::yield_now();
    }
    assert!(!lease.is_valid(), "external removal did not begin");
    owner.resources().send(()).unwrap();
    let owner_done_early = done_rx.recv_timeout(Duration::from_millis(300)).is_ok();
    let freed_early = drops.load(AtomicOrdering::SeqCst);
    drop(remover.resources_mut().take()); // release before either join
    owner.finish().unwrap();
    remover.finish().unwrap();
    assert!(!owner_done_early, "owner returned while a lease was held");
    assert_eq!(freed_early, 0, "arena freed while a lease was held");
    assert_eq!(drops.load(AtomicOrdering::SeqCst), 1);
}

/// An extracted Executor
/// still owns its thread registration. A successor must not occupy its TLS
/// until it is dropped, and its extracted guard must remain independently owned.
#[cfg(feature = "multithreading")]
#[test]
fn extracted_executor_tls_ownership() {
    use crate::test_utils::worker::Worker;
    use dotnet_utils::gc::cross_arena::try_acquire_lease;
    use std::{
        panic::{AssertUnwindSafe, catch_unwind},
        sync::mpsc,
        thread,
        time::Duration,
    };

    fn held_lease(id: ArenaId) -> Worker<bool, mpsc::Sender<()>> {
        let (ready_tx, ready_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let worker = thread::spawn(move || {
            let lease = try_acquire_lease(id).expect("live arena registration");
            ready_tx.send(()).unwrap();
            let _ = release_rx.recv_timeout(Duration::from_secs(30));
            lease.is_valid()
        });
        let worker = Worker::new(worker, release_tx);
        ready_rx.recv_timeout(Duration::from_secs(30)).unwrap();
        worker
    }

    let state = shared();
    let mut rejected = Vec::new();
    for guard_first in [true, false] {
        let first_drops = StdArc::new(AtomicUsize::new(0));
        let second_drops = StdArc::new(AtomicUsize::new(0));
        let mut first = Executor::new(Arc::clone(&state));
        allocate_witness(&mut first, StdArc::clone(&first_drops));
        let first_id = first.thread_id;
        let guard = first.extract_arena().unwrap();
        let first_holder = held_lease(first_id);

        let attempted = catch_unwind(AssertUnwindSafe(|| Executor::new(Arc::clone(&state))));
        rejected.push(attempted.is_err());
        // On the old implementation a successor is possible. Drop it first so
        // this pre-fix assertion test never frees an arena with an active lease.
        if let Ok(successor) = attempted {
            drop(successor);
        }
        let first_still_registered = try_acquire_lease(first_id).is_some();
        let first_alive = first_drops.load(AtomicOrdering::SeqCst) == 0;

        if guard_first {
            first_holder.resources().send(()).unwrap();
            let first_was_valid = first_holder.finish().unwrap();
            drop(guard);
            drop(first);
            assert!(first_was_valid && first_still_registered && first_alive);
            assert!(try_acquire_lease(first_id).is_none());
        } else {
            drop(first);
            let mut successor = Executor::new(Arc::clone(&state));
            allocate_witness(&mut successor, StdArc::clone(&second_drops));
            let second_id = successor.thread_id;
            let second_holder = held_lease(second_id);
            let first_after_executor_drop = try_acquire_lease(first_id).is_some();
            first_holder.resources().send(()).unwrap();
            let first_was_valid = first_holder.finish().unwrap();
            drop(guard);
            let second_still_registered = try_acquire_lease(second_id).is_some();
            let second_alive = second_drops.load(AtomicOrdering::SeqCst) == 0;
            second_holder.resources().send(()).unwrap();
            let second_was_valid = second_holder.finish().unwrap();
            drop(successor);
            assert!(
                first_still_registered
                    && first_alive
                    && first_after_executor_drop
                    && first_was_valid
            );
            assert!(second_still_registered && second_alive && second_was_valid);
            assert!(try_acquire_lease(first_id).is_none());
            assert!(try_acquire_lease(second_id).is_none());
            assert_eq!(second_drops.load(AtomicOrdering::SeqCst), 1);
        }
        assert_eq!(first_drops.load(AtomicOrdering::SeqCst), 1);
    }
    assert_eq!(
        rejected,
        [true, true],
        "extracted Executor allowed a successor before its own drop"
    );
}

static_assertions::assert_not_impl_any!(Executor: Send, Sync);

#[cfg(feature = "multithreading")]
mod ownership {
    use super::*;
    use std::panic::{AssertUnwindSafe, catch_unwind};

    static_assertions::assert_not_impl_any!(ArenaGuard: Send, Sync);

    // Installed only around the explicit drop on this thread. Match the actual
    // registry warning, rather than treating an arbitrary tracing event as proof.
    struct DrainWarning {
        seen: StdArc<AtomicUsize>,
        panic_on_warning: bool,
    }

    impl tracing::Subscriber for DrainWarning {
        fn enabled(&self, metadata: &tracing::Metadata<'_>) -> bool {
            *metadata.level() == tracing::Level::WARN
                && metadata.target() == "dotnet_utils::gc::cross_arena"
        }

        fn new_span(&self, _: &tracing::span::Attributes<'_>) -> tracing::span::Id {
            tracing::span::Id::from_u64(1)
        }

        fn record(&self, _: &tracing::span::Id, _: &tracing::span::Record<'_>) {}

        fn record_follows_from(&self, _: &tracing::span::Id, _: &tracing::span::Id) {}

        fn event(&self, event: &tracing::Event<'_>) {
            struct Message(bool);
            impl tracing::field::Visit for Message {
                fn record_debug(
                    &mut self,
                    field: &tracing::field::Field,
                    value: &dyn std::fmt::Debug,
                ) {
                    if field.name() == "message" {
                        self.0 = format!("{value:?}").contains(
                            "unregister_arena is waiting for outstanding cross-arena leases",
                        );
                    }
                }
            }
            let mut message = Message(false);
            event.record(&mut message);
            if self.enabled(event.metadata()) && message.0 {
                self.seen.fetch_add(1, AtomicOrdering::SeqCst);
                assert!(!self.panic_on_warning, "injected drain warning panic");
            }
        }

        fn enter(&self, _: &tracing::span::Id) {}

        fn exit(&self, _: &tracing::span::Id) {}
    }

    struct Fault;
    impl Fault {
        fn at(point: &'static str) -> Self {
            OWNERSHIP_PANIC.with(|fault| {
                assert_eq!(fault.replace(Some(point)), None);
            });
            Self
        }
    }
    impl Drop for Fault {
        fn drop(&mut self) {
            OWNERSHIP_PANIC.with(|fault| fault.set(None));
        }
    }

    fn panic_message(result: Result<(), Box<dyn std::any::Any + Send>>) -> String {
        let payload = result.expect_err("expected controlled panic");
        if let Some(message) = payload.downcast_ref::<String>() {
            message.clone()
        } else if let Some(message) = payload.downcast_ref::<&str>() {
            (*message).to_owned()
        } else {
            panic!("unexpected non-string panic payload");
        }
    }

    #[test]
    fn forgotten_executor_tls_exit_retains_arena() {
        use crate::test_utils::worker::Worker;
        use std::{sync::mpsc, thread, time::Duration};

        let state = shared();
        let drops = StdArc::new(AtomicUsize::new(0));
        let worker_drops = StdArc::clone(&drops);
        let (ready_tx, ready_rx) = mpsc::channel();
        let (exit_tx, exit_rx) = mpsc::channel();
        let worker = thread::spawn(move || {
            let mut executor = Executor::new(state);
            allocate_witness(&mut executor, worker_drops);
            ready_tx.send(executor.thread_id).unwrap();
            let _ = exit_rx.recv();
            std::mem::forget(executor);
        });
        let worker = Worker::new(worker, exit_tx);
        let id = ready_rx.recv_timeout(Duration::from_secs(30)).unwrap();
        let lease = dotnet_utils::gc::try_acquire_lease(id).unwrap();
        worker.resources().send(()).unwrap();
        worker.finish().unwrap();
        assert_eq!(drops.load(AtomicOrdering::SeqCst), 0);
        drop(lease);
        dotnet_utils::gc::cross_arena::unregister_arena(id);
        assert_eq!(drops.load(AtomicOrdering::SeqCst), 0);
    }

    #[test]
    fn repeated_extraction_reserves_until_owner_drop() {
        let state = shared();
        let mut executor = Executor::new(Arc::clone(&state));
        let drops = StdArc::new(AtomicUsize::new(0));
        allocate_witness(&mut executor, StdArc::clone(&drops));
        let id = executor.thread_id;
        let guard = executor.extract_arena().unwrap();
        assert!(executor.extract_arena().is_none());
        assert!(dotnet_utils::gc::try_acquire_lease(id).is_some());
        drop(guard);
        assert_eq!(drops.load(AtomicOrdering::SeqCst), 1);
        assert!(executor.extract_arena().is_none());
        assert!(dotnet_utils::gc::try_acquire_lease(id).is_none());
        assert!(catch_unwind(AssertUnwindSafe(|| Executor::new(Arc::clone(&state)))).is_err());
        assert!(executor.with_arena(|_| ()).is_err());
        drop(executor);
        drop(Executor::new(state));
    }

    #[test]
    fn constructor_rollback_releases_reservation() {
        let state = shared();
        for point in ["reservation", "construction"] {
            let _fault = Fault::at(point);
            let result = catch_unwind(AssertUnwindSafe(|| drop(Executor::new(Arc::clone(&state)))));
            assert!(
                panic_message(result).contains(&format!("injected ownership panic at {point}"))
            );
            drop(Executor::new(Arc::clone(&state)));
        }
    }

    #[test]
    fn executor_teardown_panic_retains_arena() {
        for point in ["executor", "guard", "drain"] {
            let state = shared();
            let drops = StdArc::new(AtomicUsize::new(0));
            let mut executor = Executor::new(Arc::clone(&state));
            let id = executor.thread_id;
            allocate_witness(&mut executor, StdArc::clone(&drops));
            let lease = dotnet_utils::gc::try_acquire_lease(id).unwrap();
            let guard = (point == "guard").then(|| executor.extract_arena().unwrap());
            let _fault = Fault::at(point);
            let result = catch_unwind(AssertUnwindSafe(|| {
                drop(executor);
                drop(guard);
            }));
            assert!(
                panic_message(result).contains(&format!("injected ownership panic at {point}"))
            );
            assert_eq!(drops.load(AtomicOrdering::SeqCst), 0);
            drop(lease);
            assert_eq!(drops.load(AtomicOrdering::SeqCst), 0);
            // Early injected cleanup failure deliberately leaves bookkeeping;
            // remove it without touching the retained storage.
            state.gc_coordinator.unregister_arena(id);
            state.thread_manager.unregister_thread(id);
            dotnet_utils::gc::cross_arena::unregister_arena(id);
            drop(Executor::new(state));
        }
    }

    #[test]
    fn owner_first_guard_does_not_touch_successor() {
        let state = shared();
        let old_drops = StdArc::new(AtomicUsize::new(0));
        let new_drops = StdArc::new(AtomicUsize::new(0));
        let mut old = Executor::new(Arc::clone(&state));
        allocate_witness(&mut old, StdArc::clone(&old_drops));
        let old_id = old.thread_id;
        let guard = old.extract_arena().unwrap();
        assert!(old.extract_arena().is_none());
        drop(old);
        let mut successor = Executor::new(state);
        allocate_witness(&mut successor, StdArc::clone(&new_drops));
        let new_id = successor.thread_id;
        let lease = dotnet_utils::gc::try_acquire_lease(new_id).unwrap();
        drop(guard);
        assert_eq!(old_drops.load(AtomicOrdering::SeqCst), 1);
        assert!(dotnet_utils::gc::try_acquire_lease(old_id).is_none());
        assert!(lease.is_valid());
        assert_eq!(new_drops.load(AtomicOrdering::SeqCst), 0);
        assert!(successor.with_arena(|_| ()).is_ok());
        drop(lease);
        drop(successor);
        assert_eq!(new_drops.load(AtomicOrdering::SeqCst), 1);
        assert!(dotnet_utils::gc::try_acquire_lease(new_id).is_none());
    }

    #[test]
    fn self_held_drain_panic_and_no_diagnostic() {
        for inject_panic in [false, true] {
            let state = shared();
            let drops = StdArc::new(AtomicUsize::new(0));
            let warnings = StdArc::new(AtomicUsize::new(0));
            let mut executor = Executor::new(Arc::clone(&state));
            let id = executor.thread_id;
            allocate_witness(&mut executor, StdArc::clone(&drops));
            let lease = dotnet_utils::gc::try_acquire_lease(id).unwrap();
            let fault = inject_panic.then(|| Fault::at("drain"));
            let subscriber = DrainWarning {
                seen: StdArc::clone(&warnings),
                panic_on_warning: true,
            };
            let result = catch_unwind(AssertUnwindSafe(|| {
                tracing::subscriber::with_default(subscriber, || drop(executor));
            }));
            drop(fault);
            if inject_panic {
                assert!(panic_message(result).contains("injected ownership panic at drain"));
            } else {
                result.expect("self-held teardown must return without diagnostic panic");
            }
            assert_eq!(warnings.load(AtomicOrdering::SeqCst), 0);
            assert_eq!(drops.load(AtomicOrdering::SeqCst), 0);
            drop(lease);
            dotnet_utils::gc::cross_arena::unregister_arena(id);
            assert_eq!(drops.load(AtomicOrdering::SeqCst), 0);
            drop(Executor::new(state));
        }
    }

    #[test]
    fn drain_warning_panic_retains_arena() {
        use crate::test_utils::worker::Worker;
        use std::{sync::mpsc, thread, time::Duration};

        // Both ordinary and extracted teardown must survive a real subscriber
        // panic *inside* drain_leases while a different thread owns the lease.
        for extracted in [false, true] {
            let state = shared();
            let drops = StdArc::new(AtomicUsize::new(0));
            let warnings = StdArc::new(AtomicUsize::new(0));
            let mut executor = Executor::new(Arc::clone(&state));
            let id = executor.thread_id;
            allocate_witness(&mut executor, StdArc::clone(&drops));
            let guard = extracted.then(|| executor.extract_arena().unwrap());
            let (ready_tx, ready_rx) = mpsc::channel();
            let (release_tx, release_rx) = mpsc::channel::<()>();
            let handle = thread::spawn(move || {
                let lease = dotnet_utils::gc::try_acquire_lease(id).unwrap();
                ready_tx.send(()).unwrap();
                let _ = release_rx.recv();
                drop(lease);
            });
            let worker = Worker::new(handle, release_tx);
            ready_rx.recv_timeout(Duration::from_secs(30)).unwrap();
            let subscriber = DrainWarning {
                seen: StdArc::clone(&warnings),
                panic_on_warning: true,
            };
            let result = catch_unwind(AssertUnwindSafe(|| {
                tracing::subscriber::with_default(subscriber, || {
                    drop(executor);
                    drop(guard);
                });
            }));
            let removed = dotnet_utils::gc::try_acquire_lease(id).is_none();
            let before_release = drops.load(AtomicOrdering::SeqCst);
            worker.finish().unwrap();
            assert!(panic_message(result).contains("injected drain warning panic"));
            assert_eq!(warnings.load(AtomicOrdering::SeqCst), 1);
            assert!(removed, "drain did not remove the registration");
            assert_eq!(before_release, 0);
            assert_eq!(drops.load(AtomicOrdering::SeqCst), 0);
            drop(Executor::new(state));
        }
    }
}

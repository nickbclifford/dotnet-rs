//! Independent recreation (not recovered originals) of Executor arena teardown ordering.
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
        AssemblyLoader::new_bare("executor_teardown_recreation".to_string()).unwrap(),
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

#[cfg(not(feature = "multithreading"))]
#[test]
fn recreation_executor_single_arena_cleanup_control() {
    let drops = StdArc::new(AtomicUsize::new(0));
    let mut executor = Executor::new(shared());
    allocate_witness(&mut executor, StdArc::clone(&drops));
    assert_eq!(drops.load(AtomicOrdering::SeqCst), 0);
    drop(executor);
    assert_eq!(drops.load(AtomicOrdering::SeqCst), 1);
}

#[cfg(feature = "multithreading")]
fn check_teardown(extracted: bool, unwind: bool) {
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

    let id = ready_rx
        .recv_timeout(Duration::from_secs(10))
        .expect("Executor ready");
    let lease = try_acquire_lease(id).expect("actual Executor arena registered");
    let before = drops.load(AtomicOrdering::SeqCst);
    start_tx.send(()).unwrap();
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
    drop(lease);
    let completed = completed_while_held || done_rx.recv_timeout(Duration::from_secs(10)).is_ok();
    let deadline = Instant::now() + Duration::from_secs(10);
    while !worker.is_finished() && Instant::now() < deadline {
        thread::yield_now();
    }
    assert!(worker.is_finished(), "owner join timed out");
    worker.join().unwrap();
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

#[cfg(feature = "multithreading")]
#[test]
fn recreation_executor_normal_teardown_active_lease() {
    check_teardown(false, false);
}

#[cfg(feature = "multithreading")]
#[test]
fn recreation_executor_extracted_teardown_active_lease() {
    check_teardown(true, false);
}

/// Independent recreation (not recovered originals): unwinding an actual Executor
/// must still drain another thread's lease before its arena is destroyed.
#[cfg(feature = "multithreading")]
#[test]
fn recreation_executor_unwind_teardown_active_lease() {
    check_teardown(false, true);
}

/// Independent recreation (not recovered originals): repeated real arena collection
/// followed by a separate, unrooted teardown witness. A lease does not root objects.
#[cfg(not(feature = "multithreading"))]
#[test]
fn m1e_recreation_teardown_stress() {
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

/// Independent recreation (not recovered originals): collect two actual Executor
/// arenas, then deliberately hold a lease across B's exit to test the drain.
/// The collection witness is already reclaimed before the teardown witness is
/// allocated; no claim is made that the lease roots either allocation.
#[cfg(feature = "multithreading")]
#[test]
fn m1e_recreation_teardown_stress() {
    use crate::gc::coordinator::GcCycleGuard;
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
        collected_tx.send(()).unwrap();
        ready_rx
            .recv_timeout(DEADLINE)
            .expect("owner ready to exit");
        let collected_before_exit = collected.load(AtomicOrdering::SeqCst);
        let before = teardown.load(AtomicOrdering::SeqCst);
        let lease = try_acquire_lease(id).expect("owner is registered");
        if !hold_lease {
            // Released lease: teardown must not wait and must still free exactly once.
            drop(lease);
            exit_tx.send(()).unwrap();
            let completed = done_rx.recv_timeout(DEADLINE).is_ok();
            let deadline = Instant::now() + DEADLINE;
            while !worker.is_finished() && Instant::now() < deadline {
                thread::yield_now();
            }
            assert!(worker.is_finished(), "owner join timed out");
            worker.join().expect("owner thread");
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
        exit_tx.send(()).unwrap();
        let deadline = Instant::now() + DEADLINE;
        while lease.is_valid() && Instant::now() < deadline {
            thread::yield_now();
        }
        let closing = !lease.is_valid();
        let rejected = closing && try_acquire_lease(id).is_none();
        let during = teardown.load(AtomicOrdering::SeqCst);
        let completed_held = done_rx.try_recv().is_ok();
        // Always release before asserting so the unregister drain can complete.
        drop(lease);
        let completed = completed_held || done_rx.recv_timeout(DEADLINE).is_ok();
        let deadline = Instant::now() + DEADLINE;
        while !worker.is_finished() && Instant::now() < deadline {
            thread::yield_now();
        }
        assert!(worker.is_finished(), "owner join timed out");
        worker.join().expect("owner thread");
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

// ---- 5.3 / 5.4 / 5.5: ownership protocol recreations (independent, real Executor) ----

/// 5.3: a second Executor on a thread that still owns one must fail closed
/// instead of replacing (and destroying) the first arena's TLS Box.
#[cfg(feature = "multithreading")]
#[test]
fn recreation_executor_nested_creation_fails_closed() {
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

/// 5.4: an owner thread holding a lease on its own arena must not deadlock in
/// teardown, and must not free the leased arena.
#[cfg(feature = "multithreading")]
#[test]
fn recreation_executor_self_held_lease_leaks_instead_of_deadlocking() {
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
    done_rx
        .recv_timeout(Duration::from_secs(30))
        .expect("teardown deadlocked on a self-held lease");
    worker.join().unwrap();
    assert_eq!(
        drops.load(AtomicOrdering::SeqCst),
        0,
        "leased arena must be leaked, not freed"
    );
}

/// 5.5: an external ID-only remover that is already draining must not let the
/// owner free the arena while a lease is still held.
#[cfg(feature = "multithreading")]
#[test]
fn recreation_executor_external_remover_does_not_release_owner() {
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
    let id = ready_rx.recv_timeout(Duration::from_secs(30)).unwrap();
    let lease = try_acquire_lease(id).unwrap();
    let remover = thread::spawn(move || unregister_arena(id)); // removes entry, drains
    let deadline = Instant::now() + Duration::from_secs(10);
    while lease.is_valid() && Instant::now() < deadline {
        thread::yield_now();
    }
    assert!(!lease.is_valid(), "external removal did not begin");
    go_tx.send(()).unwrap();
    let owner_done_early = done_rx.recv_timeout(Duration::from_millis(300)).is_ok();
    let freed_early = drops.load(AtomicOrdering::SeqCst);
    drop(lease); // release before asserting so a failure cannot strand threads
    owner.join().unwrap();
    remover.join().unwrap();
    assert!(!owner_done_early, "owner returned while a lease was held");
    assert_eq!(freed_early, 0, "arena freed while a lease was held");
    assert_eq!(drops.load(AtomicOrdering::SeqCst), 1);
}

/// Independent recreation (not recovered originals): an extracted Executor
/// still owns its thread registration. A successor must not occupy its TLS
/// until it is dropped, and its extracted guard must remain independently owned.
#[cfg(feature = "multithreading")]
#[test]
#[ignore = "finding: pending repair; independent recreation (not recovered originals)"]
fn recreation_extracted_executor_tls_ownership() {
    use dotnet_utils::gc::cross_arena::try_acquire_lease;
    use std::{
        panic::{AssertUnwindSafe, catch_unwind},
        sync::mpsc,
        thread,
        time::Duration,
    };

    fn held_lease(id: ArenaId) -> (mpsc::Sender<()>, thread::JoinHandle<bool>) {
        let (ready_tx, ready_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let worker = thread::spawn(move || {
            let lease = try_acquire_lease(id).expect("live arena registration");
            ready_tx.send(()).unwrap();
            let _ = release_rx.recv_timeout(Duration::from_secs(30));
            lease.is_valid()
        });
        ready_rx.recv_timeout(Duration::from_secs(30)).unwrap();
        (release_tx, worker)
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
        let (release_first, first_holder) = held_lease(first_id);

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
            release_first.send(()).unwrap();
            let first_was_valid = first_holder.join().unwrap();
            drop(guard);
            drop(first);
            assert!(first_was_valid && first_still_registered && first_alive);
            assert!(try_acquire_lease(first_id).is_none());
        } else {
            drop(first);
            let mut successor = Executor::new(Arc::clone(&state));
            allocate_witness(&mut successor, StdArc::clone(&second_drops));
            let second_id = successor.thread_id;
            let (release_second, second_holder) = held_lease(second_id);
            let first_after_executor_drop = try_acquire_lease(first_id).is_some();
            release_first.send(()).unwrap();
            let first_was_valid = first_holder.join().unwrap();
            drop(guard);
            let second_still_registered = try_acquire_lease(second_id).is_some();
            let second_alive = second_drops.load(AtomicOrdering::SeqCst) == 0;
            release_second.send(()).unwrap();
            let second_was_valid = second_holder.join().unwrap();
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

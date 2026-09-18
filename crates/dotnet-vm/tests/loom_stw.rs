#![cfg(loom)]

//! Small loom model of the production stop-the-world handoff.
//!
//! Production threaded types are feature-gated out of the `cfg(loom)`,
//! `--no-default-features` build. This deliberately mirrors their relevant
//! Release/Acquire and RAII sequencing instead.

use dotnet_utils::sync::RwLock;
use loom::sync::{
    Arc,
    atomic::{AtomicBool, AtomicUsize, Ordering},
    mpsc::{Receiver, Sender, channel},
};
use loom::thread;
use std::panic::{AssertUnwindSafe, catch_unwind};

const MUTATOR_COUNT: usize = 2;

/// Run every STW scenario with one collector/model thread and two mutators.
///
/// The explicit `None` assignments keep this test independent of cutoff
/// environment variables: only the population is bounded.
fn check_stw_model(f: impl Fn() + Send + Sync + 'static) {
    let mut builder = loom::model::Builder::new();
    builder.max_threads = 3;
    builder.preemption_bound = None;
    builder.max_permutations = None;
    builder.max_duration = None;
    builder.check(f);
}

struct Shared {
    /// Mirrors `GCCoordinator::stw_in_progress`.
    collection_active: AtomicBool,
    /// Mirrors `ThreadManager::gc_stop_requested`.
    stop_requested: AtomicBool,
    /// Mirrors `ThreadManager::threads_at_safepoint`.
    parked_mutators: AtomicUsize,
    mutator_heap_accesses: AtomicUsize,
    gc_heap_accesses: AtomicUsize,
    resumed_mutators: AtomicUsize,
}

impl Shared {
    fn new() -> Self {
        Self {
            collection_active: AtomicBool::new(false),
            stop_requested: AtomicBool::new(false),
            parked_mutators: AtomicUsize::new(0),
            mutator_heap_accesses: AtomicUsize::new(0),
            gc_heap_accesses: AtomicUsize::new(0),
            resumed_mutators: AtomicUsize::new(0),
        }
    }
}

/// Minimal equivalent of `CollectionSession`.
struct CollectionSession {
    shared: Arc<Shared>,
}

impl CollectionSession {
    fn begin_collection(shared: Arc<Shared>) -> Self {
        assert!(
            !shared.collection_active.load(Ordering::Acquire),
            "begin_collection requires an idle collection state"
        );
        // Production: `GCCoordinator::enter_collecting_state` uses Release.
        shared.collection_active.store(true, Ordering::Release);
        Self { shared }
    }
}

impl Drop for CollectionSession {
    fn drop(&mut self) {
        // Production: `GCCoordinator::finish_collection_inner` uses Release.
        self.shared
            .collection_active
            .store(false, Ordering::Release);
    }
}

/// Minimal equivalent of `ThreadManager`'s stop request and STW guard.
struct ThreadManager {
    shared: Arc<Shared>,
    stop_senders: Vec<Sender<()>>,
    resume_senders: Vec<Sender<()>>,
    parked: Receiver<()>,
}

impl ThreadManager {
    fn request_stop_the_world(self) -> StopTheWorldGuard {
        // Production: `ThreadManager::request_stop_the_world` uses Release.
        self.shared.stop_requested.store(true, Ordering::Release);
        for sender in &self.stop_senders {
            sender.send(()).expect("mutator must be waiting for stop");
        }
        for _ in 0..MUTATOR_COUNT {
            self.parked
                .recv()
                .expect("each mutator must acknowledge its safepoint");
        }

        StopTheWorldGuard {
            shared: self.shared,
            resume_senders: self.resume_senders,
        }
    }
}

/// Minimal equivalent of `StopTheWorldGuard`; its drop resumes mutators.
struct StopTheWorldGuard {
    shared: Arc<Shared>,
    resume_senders: Vec<Sender<()>>,
}

impl Drop for StopTheWorldGuard {
    fn drop(&mut self) {
        // Production: `ThreadManager::resume_threads` uses Release.
        self.shared.stop_requested.store(false, Ordering::Release);
        for sender in &self.resume_senders {
            sender.send(()).expect("parked mutator must be resumable");
        }
    }
}

/// Mirrors `GcCycleGuard`'s required collection-cleanup-before-resume order.
struct GcCycleGuard {
    session: Option<CollectionSession>,
    stw_guard: Option<StopTheWorldGuard>,
}

impl GcCycleGuard {
    fn new(session: CollectionSession, stw_guard: StopTheWorldGuard) -> Self {
        Self {
            session: Some(session),
            stw_guard: Some(stw_guard),
        }
    }
}

impl Drop for GcCycleGuard {
    fn drop(&mut self) {
        // This ordering is the production `GcCycleGuard::drop` contract.
        drop(self.session.take());
        drop(self.stw_guard.take());
    }
}

fn mutator(
    shared: Arc<Shared>,
    ready: Sender<()>,
    stop: Receiver<()>,
    parked: Sender<()>,
    resume: Receiver<()>,
) {
    // A mutator may access its heap before observing a stop request. It must
    // finish that access before publishing its safepoint acknowledgement.
    if !shared.stop_requested.load(Ordering::Acquire) {
        shared.mutator_heap_accesses.fetch_add(1, Ordering::Relaxed);
    }
    ready
        .send(())
        .expect("collector must await mutator readiness");

    stop.recv().expect("collector must request stop");
    assert!(
        shared.stop_requested.load(Ordering::Acquire),
        "a stop notification must publish the stop request"
    );
    // Production safepoints publish with AcqRel after flushing/accessing state.
    shared.parked_mutators.fetch_add(1, Ordering::AcqRel);
    parked
        .send(())
        .expect("collector must await safepoint acknowledgement");

    resume.recv().expect("STW guard must resume parked mutator");
    // The cycle guard must clean up the collection session before resuming us.
    assert!(
        !shared.collection_active.load(Ordering::Acquire),
        "collection cleanup must precede mutator resume"
    );
    assert!(
        !shared.stop_requested.load(Ordering::Acquire),
        "resume notification must publish the cleared stop request"
    );
    shared.mutator_heap_accesses.fetch_add(1, Ordering::Relaxed);
    shared.resumed_mutators.fetch_add(1, Ordering::Release);
}

fn gc_only_heap_access(shared: &Shared) {
    // SAFETY: F1.StwParked — both mutators acknowledged their safepoints with
    // AcqRel, and the collector observed those acknowledgements with Acquire.
    assert_eq!(
        shared.parked_mutators.load(Ordering::Acquire),
        MUTATOR_COUNT,
        "F1.StwParked: every mutator must be parked before GC-only access"
    );
    assert!(
        shared.stop_requested.load(Ordering::Acquire),
        "F1.StwParked: the stop request must remain active during GC-only access"
    );

    let accesses_before = shared.mutator_heap_accesses.load(Ordering::Acquire);
    shared.gc_heap_accesses.fetch_add(1, Ordering::Relaxed);
    assert_eq!(
        shared.mutator_heap_accesses.load(Ordering::Acquire),
        accesses_before,
        "F1.StwParked: mutator heap access changed while all mutators were parked"
    );
}

#[test]
fn normal_stop_the_world_parks_every_mutator_before_gc_access_and_resumes_after_cleanup() {
    check_stw_model(|| {
        let shared = Arc::new(Shared::new());
        let (ready, ready_events) = channel();
        let (parked, parked_events) = channel();
        let mut stop_senders = Vec::with_capacity(MUTATOR_COUNT);
        let mut resume_senders = Vec::with_capacity(MUTATOR_COUNT);
        let mut mutators = Vec::with_capacity(MUTATOR_COUNT);

        for _ in 0..MUTATOR_COUNT {
            let (stop_sender, stop_receiver) = channel();
            let (resume_sender, resume_receiver) = channel();
            stop_senders.push(stop_sender);
            resume_senders.push(resume_sender);
            let shared = Arc::clone(&shared);
            let ready = ready.clone();
            let parked = parked.clone();
            mutators.push(thread::spawn(move || {
                mutator(shared, ready, stop_receiver, parked, resume_receiver)
            }));
        }
        drop(ready);
        drop(parked);

        for _ in 0..MUTATOR_COUNT {
            ready_events
                .recv()
                .expect("each mutator must reach its normal access boundary");
        }
        let session = CollectionSession::begin_collection(Arc::clone(&shared));
        let manager = ThreadManager {
            shared: Arc::clone(&shared),
            stop_senders,
            resume_senders,
            parked: parked_events,
        };
        let stw_guard = manager.request_stop_the_world();
        let cycle_guard = GcCycleGuard::new(session, stw_guard);

        gc_only_heap_access(&shared);
        assert_eq!(shared.gc_heap_accesses.load(Ordering::Acquire), 1);

        drop(cycle_guard);
        for mutator in mutators {
            mutator.join().expect("mutator must complete after resume");
        }

        assert_eq!(
            shared.resumed_mutators.load(Ordering::Acquire),
            MUTATOR_COUNT,
            "both mutators must observe the STW guard's resume"
        );
        assert!(
            !shared.collection_active.load(Ordering::Acquire),
            "collection state must be idle after the cycle guard drops"
        );
    });
}

/// Minimal equivalent of an armed `ResumeOnPanic` before ownership transfers
/// to `StopTheWorldGuard`.
struct ResumeOnPanicModel {
    shared: Arc<Shared>,
    resume_senders: Vec<Sender<()>>,
}

impl Drop for ResumeOnPanicModel {
    fn drop(&mut self) {
        // Production: `ResumeOnPanic::drop` calls `resume_threads` while
        // armed; `ThreadManager::resume_threads` clears this with Release.
        self.shared.stop_requested.store(false, Ordering::Release);
        for sender in &self.resume_senders {
            sender
                .send(())
                .expect("an armed panic guard must resume every parked mutator");
        }
    }
}

fn mutator_waiting_for_panic_resume(
    shared: Arc<Shared>,
    stop: Receiver<()>,
    parked: Sender<()>,
    resume: Receiver<()>,
) {
    stop.recv().expect("collector must request stop");
    assert!(
        shared.stop_requested.load(Ordering::Acquire),
        "stop notification must make the request visible before parking"
    );
    shared.parked_mutators.fetch_add(1, Ordering::AcqRel);
    parked.send(()).expect("collector must await parking");

    resume
        .recv()
        .expect("armed ResumeOnPanic must resume this mutator");
    assert!(
        !shared.stop_requested.load(Ordering::Acquire),
        "ResumeOnPanic release must make resume visible to the mutator"
    );
    shared.resumed_mutators.fetch_add(1, Ordering::Release);
}

#[test]
fn armed_resume_on_panic_resumes_parked_mutators_before_stw_guard_handoff() {
    check_stw_model(|| {
        let shared = Arc::new(Shared::new());
        let (parked, parked_events) = channel();
        let mut stop_senders = Vec::with_capacity(MUTATOR_COUNT);
        let mut resume_senders = Vec::with_capacity(MUTATOR_COUNT);
        let mut mutators = Vec::with_capacity(MUTATOR_COUNT);

        for _ in 0..MUTATOR_COUNT {
            let (stop_sender, stop_receiver) = channel();
            let (resume_sender, resume_receiver) = channel();
            stop_senders.push(stop_sender);
            resume_senders.push(resume_sender);
            let shared = Arc::clone(&shared);
            let parked = parked.clone();
            mutators.push(thread::spawn(move || {
                mutator_waiting_for_panic_resume(shared, stop_receiver, parked, resume_receiver)
            }));
        }
        drop(parked);

        // This models `request_stop_the_world` after its Release stop store
        // and `ResumeOnPanic::new`, but before `StopTheWorldGuard` is created.
        let unwind = catch_unwind(AssertUnwindSafe(|| {
            shared.stop_requested.store(true, Ordering::Release);
            let _panic_guard = ResumeOnPanicModel {
                shared: Arc::clone(&shared),
                resume_senders,
            };
            for sender in &stop_senders {
                sender
                    .send(())
                    .expect("mutator must await the stop request");
            }
            for _ in 0..MUTATOR_COUNT {
                parked_events
                    .recv()
                    .expect("each mutator must park before injected unwind");
            }

            panic!("inject unwind before StopTheWorldGuard handoff");
        }));
        assert!(unwind.is_err(), "the modeled setup panic must be caught");

        for mutator in mutators {
            mutator
                .join()
                .expect("mutator must make progress after panic-guard resume");
        }
        assert!(
            !shared.stop_requested.load(Ordering::Acquire),
            "armed panic guard must publish that the stop request ended"
        );
        assert_eq!(
            shared.resumed_mutators.load(Ordering::Acquire),
            MUTATOR_COUNT,
            "every parked mutator must observe the panic-guard resume"
        );
    });
}

/// State owned by the collector and a parked command handler.
struct CommandState {
    pending: AtomicBool,
    completions: AtomicUsize,
}

impl CommandState {
    fn new() -> Self {
        Self {
            pending: AtomicBool::new(false),
            completions: AtomicUsize::new(0),
        }
    }
}

/// Minimal equivalent of an armed `CommandCompletionGuard`.
struct CommandCompletionGuardModel {
    state: Arc<CommandState>,
    completion: Sender<()>,
}

impl Drop for CommandCompletionGuardModel {
    fn drop(&mut self) {
        // Production: `CommandCompletionGuard<Armed>::drop` calls
        // `GCCoordinator::command_finished`, which clears the command under
        // its lock and notifies the collector waiting for completion.
        assert!(
            self.state.pending.swap(false, Ordering::Release),
            "an armed completion guard requires a pending command"
        );
        self.state.completions.fetch_add(1, Ordering::Release);
        self.completion
            .send(())
            .expect("collector must observe command completion after unwind");
    }
}

fn parked_non_command_mutator(
    shared: Arc<Shared>,
    stop: Receiver<()>,
    parked: Sender<()>,
    resume: Receiver<()>,
) {
    stop.recv().expect("collector must request stop");
    assert!(shared.stop_requested.load(Ordering::Acquire));
    shared.parked_mutators.fetch_add(1, Ordering::AcqRel);
    parked.send(()).expect("collector must await parking");
    resume.recv().expect("collector must resume parked mutator");
    assert!(!shared.stop_requested.load(Ordering::Acquire));
}

fn parked_command_mutator(
    shared: Arc<Shared>,
    stop: Receiver<()>,
    parked: Sender<()>,
    command: Receiver<()>,
    completion: Sender<()>,
    command_state: Arc<CommandState>,
    resume: Receiver<()>,
) {
    stop.recv().expect("collector must request stop");
    assert!(shared.stop_requested.load(Ordering::Acquire));
    shared.parked_mutators.fetch_add(1, Ordering::AcqRel);
    parked.send(()).expect("collector must await parking");

    command
        .recv()
        .expect("collector must publish a command while mutators are parked");
    let unwind = catch_unwind(AssertUnwindSafe(|| {
        // Production arms immediately before `execute_gc_command`.
        let _completion_guard = CommandCompletionGuardModel {
            state: command_state,
            completion,
        };
        panic!("inject unwind during GC command execution");
    }));
    assert!(unwind.is_err(), "the modeled command panic must be caught");

    resume.recv().expect("collector must resume parked mutator");
    assert!(!shared.stop_requested.load(Ordering::Acquire));
}

#[test]
fn armed_command_completion_guard_clears_published_command_after_handler_unwind() {
    check_stw_model(|| {
        let shared = Arc::new(Shared::new());
        let command_state = Arc::new(CommandState::new());
        let (parked, parked_events) = channel();
        let (command, command_receiver) = channel();
        let (completion, completion_events) = channel();
        let (stop_first, stop_first_receiver) = channel();
        let (stop_handler, stop_handler_receiver) = channel();
        let (resume_first, resume_first_receiver) = channel();
        let (resume_handler, resume_handler_receiver) = channel();

        let first_shared = Arc::clone(&shared);
        let first_parked = parked.clone();
        let parked_mutator = thread::spawn(move || {
            parked_non_command_mutator(
                first_shared,
                stop_first_receiver,
                first_parked,
                resume_first_receiver,
            )
        });
        let handler_shared = Arc::clone(&shared);
        let handler_parked = parked.clone();
        let handler_state = Arc::clone(&command_state);
        let command_mutator = thread::spawn(move || {
            parked_command_mutator(
                handler_shared,
                stop_handler_receiver,
                handler_parked,
                command_receiver,
                completion,
                handler_state,
                resume_handler_receiver,
            )
        });
        drop(parked);

        // Mirrors the Release stop publication and two safepoint AcqRel
        // acknowledgements before `CollectionSession` dispatches a command.
        shared.stop_requested.store(true, Ordering::Release);
        stop_first.send(()).unwrap();
        stop_handler.send(()).unwrap();
        for _ in 0..MUTATOR_COUNT {
            parked_events.recv().unwrap();
        }
        assert_eq!(
            shared.parked_mutators.load(Ordering::Acquire),
            MUTATOR_COUNT,
            "a command may execute only after every mutator is parked"
        );

        // Production publishes the command under its lock before notifying its
        // handler; the handler's armed guard clears and notifies on unwind.
        command_state.pending.store(true, Ordering::Release);
        command.send(()).unwrap();
        completion_events
            .recv()
            .expect("collector must not remain blocked after handler unwind");
        assert!(
            !command_state.pending.load(Ordering::Acquire),
            "completion notification must make the cleared command visible"
        );
        assert_eq!(command_state.completions.load(Ordering::Acquire), 1);

        shared.stop_requested.store(false, Ordering::Release);
        resume_first.send(()).unwrap();
        resume_handler.send(()).unwrap();
        parked_mutator.join().expect("parked mutator must resume");
        command_mutator
            .join()
            .expect("command handler must resume after caught unwind");
    });
}

/// Minimal state held by the cross-arena registry and its leases.
///
/// This models `dotnet_utils::gc::cross_arena::ArenaState`; the immutable
/// generation is deliberately non-atomic, as in production.
struct LeaseState {
    active_leases: AtomicUsize,
    is_alive: AtomicBool,
    torn_down: AtomicBool,
    generation: u64,
}

impl LeaseState {
    fn new(generation: u64) -> Self {
        Self {
            active_leases: AtomicUsize::new(0),
            is_alive: AtomicBool::new(true),
            torn_down: AtomicBool::new(false),
            generation,
        }
    }
}

/// Models the `VALID_ARENAS` registry's read/write synchronization.
struct LeaseRegistry {
    state: RwLock<Option<Arc<LeaseState>>>,
}

/// Minimal equivalent of `cross_arena::ArenaLease`.
struct Lease {
    state: Arc<LeaseState>,
    generation: u64,
}

impl Drop for Lease {
    fn drop(&mut self) {
        // Production: `ArenaLease::drop` uses Release before a teardown
        // drain observes this counter with Acquire.
        self.state.active_leases.fetch_sub(1, Ordering::Release);
    }
}

/// Mirrors `try_acquire_lease`: the clone and increment are both under the
/// registry read lock, preventing a writer from removing the state first.
fn acquire_lease(registry: &LeaseRegistry) -> Option<Lease> {
    let registry_guard = registry.state.read();
    let state = registry_guard.as_ref()?.clone();
    let generation = state.generation;
    // Production: `try_acquire_lease` increments with Acquire while its
    // `VALID_ARENAS` read lock is still held.
    state.active_leases.fetch_add(1, Ordering::Acquire);
    drop(registry_guard);

    Some(Lease { state, generation })
}

/// Mirrors the relevant `cross_arena::unregister_arena` lifetime sequence.
fn unregister_and_teardown(
    registry: &LeaseRegistry,
    teardown_published: Sender<u64>,
    teardown_complete: Sender<u64>,
) {
    // Production removes the entry under the `VALID_ARENAS` write lock, so no
    // new read-side acquisition can pass the lease increment after this.
    let state = registry.state.write().take();
    let Some(state) = state else {
        return;
    };

    // Production publishes teardown before waiting for outstanding leases.
    // A held lease may consequently observe `is_alive == false` while its
    // backing memory remains protected by the drain.
    state.is_alive.store(false, Ordering::Release);
    teardown_published
        .send(state.generation)
        .expect("lease holder must observe teardown publication");

    // Production drains using Acquire loads and cooperatively yields. The
    // holder has a handshake that guarantees this loop will terminate without
    // a permutation or duration cutoff.
    while state.active_leases.load(Ordering::Acquire) != 0 {
        loom::thread::yield_now();
    }

    // This is the arena-memory lifetime boundary. It must follow the drain.
    assert_eq!(
        state.active_leases.load(Ordering::Acquire),
        0,
        "final arena teardown must never occur with a live lease"
    );
    state.torn_down.store(true, Ordering::Release);
    teardown_complete
        .send(state.generation)
        .expect("model coordinator must observe final teardown");
}

#[test]
fn cross_arena_lease_holds_teardown_until_release_and_preserves_generation() {
    check_stw_model(|| {
        const GENERATION: u64 = 41;

        let state = Arc::new(LeaseState::new(GENERATION));
        let registry = Arc::new(LeaseRegistry {
            state: RwLock::new(Some(Arc::clone(&state))),
        });
        let (lease_acquired, acquired_generation) = channel();
        let (teardown_published, publication_generation) = channel();
        let (teardown_complete, complete_generation) = channel();
        let (begin_teardown, begin_teardown_signal) = channel();

        let holder_registry = Arc::clone(&registry);
        let holder = thread::spawn(move || {
            let lease = acquire_lease(&holder_registry).expect("registered arena must lease");
            lease_acquired
                .send(lease.generation)
                .expect("model coordinator must observe acquisition");
            let published_generation = publication_generation
                .recv()
                .expect("unregister must publish teardown");

            // `is_alive` is intentionally not an invariant of a held lease:
            // production clears it before its acquire-drain lifetime boundary.
            assert!(!lease.state.is_alive.load(Ordering::Acquire));
            assert_eq!(lease.generation, lease.state.generation);
            assert_eq!(lease.generation, published_generation);
            assert!(
                !lease.state.torn_down.load(Ordering::Acquire),
                "teardown cannot finish while this lease remains live"
            );
            drop(lease);
        });

        let unregister_registry = Arc::clone(&registry);
        let unregister = thread::spawn(move || {
            begin_teardown_signal
                .recv()
                .expect("model coordinator must authorize teardown");
            unregister_and_teardown(&unregister_registry, teardown_published, teardown_complete);
        });

        assert_eq!(acquired_generation.recv().unwrap(), GENERATION);
        begin_teardown.send(()).unwrap();
        assert_eq!(complete_generation.recv().unwrap(), GENERATION);
        holder.join().expect("lease holder must complete");
        unregister.join().expect("unregister must complete");

        assert!(state.torn_down.load(Ordering::Acquire));
        assert_eq!(state.active_leases.load(Ordering::Acquire), 0);
    });
}

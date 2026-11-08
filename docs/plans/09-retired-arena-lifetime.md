# Retain arenas after managed thread exit

**Status:** Complete
**Outcome:** A managed object reachable from another arena or shared static storage remains
accessible and collectible after its allocating managed thread terminates.

## Problem and scope

Unregistering and draining a worker arena at managed exit would leave encoded references in
statics or other arenas with no owner; subsequent production decode would fail with
`OwnerUnavailable`. It would also lose the original inner exception of a failed initializer after
the initializing thread exits. Leases protect active accesses, not future accesses or tracing roots.
Completed workers therefore retain and service their arenas until coordinated shutdown.

This prerequisite covers managed thread termination, shared statics, cross-arena object graphs,
GC and shutdown. It does not change exception identity or reconstruct exception objects. It must
preserve existing protection against access after a registration is actually removed.

## Design

Keep a terminated worker's arena and its exact registration on its owning OS thread, and continue
servicing the coordinator's GC commands there. The worker must stop its ordinary managed entry
point and publish managed completion independently of OS-thread termination: `Thread.Join` now
observes completion without joining the OS worker. A completed owner services collection
commands until explicit shutdown drains its arena.
Keep retired arenas in coordinated mark, cross-arena fixed-point, finalization and sweep; retaining
storage or a lease alone does not perform those duties. Account for retired workers separately
from runnable managed threads in stop-the-world coordination and process-exit decisions.

After publishing managed completion, the retired owner may execute its arena's queued managed
finalizers. `finalize_arena` queues unreachable objects in the owning arena, and finalizer frames
must continue on that owner. Service GC commands at safepoints while running finalizers, including
when a finalizer waits or requests a collection. Once finalizers finish, return to the retirement
service loop; do not resume the completed thread's ordinary managed entry point. Shutdown must
coordinate with any finalizer in progress and prevent a new collection or finalizer from racing
arena destruction.

Define an explicit shutdown protocol that wakes service workers, excludes new collections and
accesses, drains exact-identity leases, and only then frees arena storage. Coordinate shutdown
with in-flight collections, startup failure and unwind; avoid reference cycles that prevent the
shutdown signal. Do not transfer the thread-affine arena or make it `Send` without a separate
soundness proof. Until safe earlier retirement is proven, retain retired arenas until VM
shutdown. The accepted per-worker resource and process-wide ID costs are recorded in the
[worker lifecycle contract](../THREADING_AND_SYNCHRONIZATION.md#managed-worker-lifecycle-and-process-exit).

### Compatibility decisions (2026-11-07)

Match CLR rather than encode new deviations in the shutdown protocol:

- **Foreground exit.** CLR keeps the process alive until every foreground thread finishes, and
  `new Thread` is foreground by default. When Main returns, the VM waits for every running
  worker to complete before shutdown; with `IsBackground` unimplemented, every worker counts as
  foreground. Shutdown never aborts a running worker except on a user/timeout abort request.
- **`GC.WaitForPendingFinalizers`** waits until no arena, live or retired, has a pending or
  in-progress finalizer. A finalizer waiting on its waiter deadlocks, as in CLR.
- **Panicked workers** keep the existing drain-or-leak path rather than retiring.

### Worker lifecycle state machine

One lifecycle per worker, held in a VM-owned worker registry and changed under one lock:
`Running → Completed(outcome) → Retired ⇄ Finalizing`, followed by
`Retired → ShuttingDown → Drained | Leaked`; a panic goes directly to `Leaked`. Join, stop-the-world accounting (`Retired` and a parked
`Finalizing` count as stopped), the exit decision and shutdown read this state instead of
combining flags. Owner-side transitions use typestate where possible (`serve_retired` consumes a
`Completed` token). Introduce only the states each slice needs; do not build the full machine
up front. Each legal transition gets a test; illegal ones are rejected or leak the arena.

### Slices

Tracked as steps 4.3 and 4.6–4.9 of the `supervised/async-effects` checklist:

1. VM-owned worker registry, Join without native join, foreground exit wait, explicit shutdown
   wired into the CLI and harness (workers still exit as today). Done when: a CLR-diffed fixture
   whose Main returns before an unjoined worker finishes prints the worker's output, and a test
   shows Join returning while the native thread is still held.
2. Retirement service loop; Loom for the service handshake; record the accepted resource cost in
   the threading contract. Done when: `retired_arena_static_survives_join_and_collection` is
   enabled and passes in MT.
3. Retired-owner finalizers and global `WaitForPendingFinalizers`. Done when: a CLR-diffed
   fixture observes, from Main, a finalizer that ran on a retired owner.
4. Cross-arena graph fixture and shutdown-race tests across the transition table. Done when: the
   cycle fixture matches CLR, the unrooted control is reclaimed, and each listed race test
   completes under an external timeout.
5. Contract documentation, resource-cost record, full checks. Done when: `./check.sh --fast`
   passes in all three feature configs, the contract is recorded in the GC, threading and invariant
   documents, and this plan is marked Complete.

## Delivery and checks

Implement this as a working runtime change before using the retained failure cache in plan 08.
Use production object encoding and decoding and actual managed worker termination in regressions:

- A worker stores an object in a static, terminates and is joined; another thread reads the same
  object after repeated coordinated collections, matching CLR behavior.
- References across multiple retired and live arenas survive fixed-point tracing, including
  cycles, while an unrooted control is reclaimed and finalization remains correct.
- A failed initializer's original exception and identity survive owner termination and collection;
  multiple waiters receive the cached failure once step 4.4 is implemented.
- Join, collection, worker termination and shutdown races complete without deadlock, including
  failures and unwinding; fresh accesses after actual unregister remain rejected and in-flight
  leases drain before reclamation.
- Run the real-thread regressions with timeouts, relevant collection and teardown suites, pinned
  Miri controls for changed pointer and lifetime paths, Loom for changed synchronization, and
  `./check.sh --fast`. Record discovered tests and evidence limits.

Update the GC, threading and invariant contracts and resume plan 08 step 4.4 only after the
lifetime prerequisite is established. The worker-per-arena resource cost and shutdown semantics
are accepted for this design; changing them requires a plan revision. Never silently discard the
inner exception.

## Baseline regression

The MT integration test `retired_arena_static_survives_join_and_collection` runs a
managed worker that stores an object in a static, joins it, and reads it after three collections.
On 2026-11-07 the CLR returned 42; the VM failed with `OwnerUnavailable` for the worker arena.
It covers the production codec and actual
worker exit, but does not prove cross-arena fixed-point tracing, finalization or shutdown.
Run it under an external timeout: the harness's cooperative abort cannot bound a deadlock.

The shutdown owner must be explicit: each retained `Executor` holds `Arc<SharedGlobalState>`,
so a signal emitted only by `SharedGlobalState::drop` cannot reach those workers. The CLI invokes
explicit shutdown before `exit_fast`, and the in-process harness does so after dropping its entry
executor; owner-side retirement coordinates collection exclusion and lease drain before storage is released.

## Completion

Plan 09 establishes the arena lifetime and global-finalizer prerequisite. Failed-initializer
identity is verified separately by the [static initialization slice](08-async-effects/slice-4-static-initialization.md).
Worker lifecycle, resource cost and final verification evidence are recorded in the GC and
threading contracts and the closeout session entry in `AGENT_MEMORY.md`. The bounded Loom model
does not cover every shutdown interleaving, as recorded in the lifecycle test evidence.

# Plan 08 slice 5: finalizer and monitor waits

**Status:** Notes. Fill in Done-when and open questions before starting.

## Consumers

- `GC.WaitForPendingFinalizers` (`dotnet-vm/src/intrinsics/gc.rs`) spins via `yield_spin`
  while the queue is non-empty or a finalizer is running.
- `Monitor.Enter(object)` and `Monitor.ReliableEnter/Enter(object, ref bool)`
  (`dotnet-intrinsics-threading/src/monitor.rs`) return `Yield` and retry.

## Verified facts and hazards (HEAD 069c8294, read from code, not reproduced)

- `WaitForPendingFinalizers` (`gc.rs:203-216`) returns a bare `Yield` while the per-arena queue
  is non-empty or `processing_finalizer` is set; the call re-executes. Called from inside a
  finalizer it spins forever, because `process_pending_finalizers` returns early while
  `processing_finalizer` is set (`context_ops.rs:1150-1152`). This stays legacy.
- `Enter(object)` (`monitor.rs:80-111`) returns `Yield` and re-executes. `ReliableEnter` and
  `Enter(object, bool&)` (`:113-162`) write `lockTaken` through the pointer's origin; C# `lock`
  uses this overload and `lockTaken` is a caller local, so it needs a frame-home byref.
- `TryEnter` (`:192-291`) keeps its deadline in a thread-local `CURRENT_DEADLINE` (`:12-36`),
  cleared only on a non-`Yield` result, so it is not tied to a particular call. A per-activation
  `Instant` replaces it, so the timeout overloads migrate in this slice.
- MT blocks on the condvar inside the mutation and returns `Yield` only on a GC stop
  (`sync/threaded.rs:139-217`). ST always succeeds and only counts recursion
  (`sync/single_threaded.rs:28-76`).
- Fixtures: `gc/gc_finalization_42`, `generic_finalizer_42`, `gc_resurrection_*`,
  `weak_handle_42`; `threading/monitor_try_enter_timeout_42`,
  `monitor_try_enter_timeout_single_42`, `threading_cross_arena_0`. Missing: contended
  `Enter(object, bool&)`. Rust tests: `sync/threaded.rs:487-560`.

## Introduced here

- A condition-wait protocol: probe, register, recheck. Registrations are owned by the driver,
  not by future destructors. Replies are addressed by generation. A notification only marks a
  registration eligible; the owning executor rechecks inside a fresh mutation.
- Waits happen outside the mutation, with bounded wake intervals and GC-stop participation.
  Deadlines use `Instant`, not poll counts. In ST, runnable managed or finalizer work runs
  before waiting. ST never manufactures readiness and keeps existing blocking and deadlock
  semantics.

## Carried from the archive

- Monitor: a wake replies only "retry probe". The next poll acquires synchronously, so no
  acquired-but-undelivered state exists. Recursion increments once. No cancellation path
  performs an implicit `Monitor.Exit`.
- `WaitForPendingFinalizers` called from inside a finalizer stays legacy until a CLR fixture
  defines the self-wait behavior. Do not clear the processing flag to fake readiness.
- `ReliableEnter(object, ref bool)` needs frame-home byref transport (slice 3). Until then it
  stays legacy.

## Open questions

(Answered: use deterministic schedule tests, not Loom, which is not a dependency; the MT
executor tests already drive schedules through `test_utils::worker::Worker`. The timeout
overloads migrate in this slice.)

## Done when (draft)

- Finalizer-wait and monitor fixtures pass in both configurations. The `yield_spin` and
  monitor `Yield` retry paths are deleted for migrated overloads. Waits respond to MT stops and
  do not spin in ST.

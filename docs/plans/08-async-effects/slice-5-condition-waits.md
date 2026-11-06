# Plan 08 slice 5: finalizer and monitor waits

**Status:** Notes. Fill in Done-when and open questions before starting.

## Consumers

- `GC.WaitForPendingFinalizers` (`dotnet-vm/src/intrinsics/gc.rs`) spins via `yield_spin`
  while the queue is non-empty or a finalizer is running.
- `Monitor.Enter(object)` and `Monitor.ReliableEnter/Enter(object, ref bool)`
  (`dotnet-intrinsics-threading/src/monitor.rs`) return `Yield` and retry.

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

- Use Loom, or deterministic schedule tests, for notification races?
- Timeout overloads (`TryEnter` with timeout): this slice or later?

## Done when (draft)

- Finalizer-wait and monitor fixtures pass in both configurations. The `yield_spin` and
  monitor `Yield` retry paths are deleted for migrated overloads. Waits respond to MT stops and
  do not spin in ST.

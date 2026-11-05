# Native continuations (Rust async as VM effects)

**Status:** Parked — redesign not started
**Outcome (when resumed):** One named native operation keeps its progress across a VM
safepoint instead of restarting on `Yield`.

This is about long-running host operations inside the VM (for example, a large `Substring`
copy that currently restarts when a GC is requested). It is not .NET `Task` scheduling and
not an OS async runtime.

## What exists

- [`experiments/async-effects/`](../../experiments/async-effects/DESIGN.md): a design,
  layered prototype and [gallery](../../experiments/async-effects/gallery/README.md) built on
  `VmEffects`, `fx.vm().await?`, rooted access loans and per-activation roots. It is not wired
  into the VM. Treat its API as a candidate, not a decision.
- An earlier integration attempt was abandoned unmerged. Its lessons are below.

## To resume

Pick one real operation whose progress is lost on `Yield`, name the observable improvement,
and plan one enabled slice: adapter, scheduler hook, result handling and cleanup for that
operation only, single-threaded, with a fallback to the current path for every other call.
Multithreading, managed re-entry, exception boundaries and waits are separate later outcomes.

Done when, for that operation: output is exact under a real collection request after each
chunk, each element is processed once, polling is bounded so abort/budget/collection checks
still run, roots survive collection, cancellation releases futures and roots, and the cost
compared to the direct path is measured.

## Lessons from the abandoned attempt

| Issue found | Resolution to evaluate |
|---|---|
| Registration scanner ignored `cfg` and deduplicated IDs | Select the chosen signatures explicitly before arguments are consumed; leave unrelated registrations alone. |
| ST thread-manager stop predicate was a stub | Use the real heap collection request; test against real collections, not mock flags. |
| Safepoint acknowledgment needed branded state | Restore roots, leave the suspending mutation, run outer checks, then acknowledge in the next mutation. |
| Finalizers disturbed a parked caller | For a finite ST operation, keep queued finalizer work rooted and defer new finalizer frames until the call finishes. |
| Ordinary and indirect calls have different return contracts | Fall back to the legacy path before consuming operands for any call shape not yet implemented. |
| Polling `Ready` depends on completion handling | Build them together. |
| Prototype costs excluded real context/root work | Measure the real path, including retained allocations. |

## Safety requirements for any design

- Futures are `'static` and hold no unconfined GC-branded values or heap borrows. Anything
  carried across suspension is a traced root.
- Nothing held while parked prevents collection (no lock, lease or GC access scope).
- Operands are consumed once; progress, result and IP ownership are unambiguous.
- Cancellation, failure, return and teardown release futures, buffers and roots.

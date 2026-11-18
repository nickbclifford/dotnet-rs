# Retain arenas after managed thread exit

**Status:** Complete (partly superseded by [plan 26](26-thread-lifecycle-and-multicast-fallback.md))
**Outcome:** A managed object reachable from another arena or shared static storage remains
accessible and collectible after its allocating managed thread terminates.

## Problem

Unregistering and draining a worker arena at managed exit left encoded references in statics or
other arenas with no owner, so production decode failed with `OwnerUnavailable`, and a failed
initializer's original inner exception could be lost. Leases protect active accesses, not future
accesses or tracing roots.

## Design

- A terminated worker's arena and its exact registration stay on its owning OS thread, which
  keeps servicing the coordinator's GC commands (`Executor::serve_retired`). The worker never
  resumes its ordinary managed entry point.
- Managed completion is published independently of OS-thread termination; `Thread.Join` observes
  completion without joining the OS worker.
- Retired arenas take part in coordinated mark, the cross-arena fixed point, finalization and
  sweep, and count as stopped for stop-the-world.
- A retired owner may run its own queued finalizers (state `Finalizing`), servicing GC commands
  meanwhile, then returns to the service loop.
- Shutdown excludes collections, drains exact-identity leases and only then frees each arena; if
  the drain cannot complete, the storage is leaked. The arena is never transferred between
  threads. The CLI and the in-process harness call `SharedGlobalState::shutdown_workers`
  explicitly, because each retained `Executor` holds `Arc<SharedGlobalState>`.

Lifecycle (`crates/dotnet-vm/src/threading/workers.rs`): `Running → Completed(outcome) → Retired ⇄
Finalizing`, then `Retired → ShuttingDown → Drained | Leaked`; a panicked worker follows the
drain-or-leak path. Plan 26 added `Retired → Releasing → Released | Leaked` for empty ordinary
arenas, which are now released before shutdown; arenas with live objects, and the pool and
finalizer owners, are still retained until shutdown. The current contract is
[Managed worker lifecycle and process exit](../THREADING_AND_SYNCHRONIZATION.md#managed-worker-lifecycle-and-process-exit).

### Compatibility decisions

These match CLR rather than add deviations:

- **Foreground exit.** The process stays alive until every foreground thread finishes; `new
  Thread` is foreground by default.
- **`GC.WaitForPendingFinalizers`** waits until no arena, live or retired, has a pending or
  in-progress finalizer. A call from a running finalizer returns immediately.
- **Panicked workers** keep the drain-or-leak path rather than retiring.

## Checks

- `retired_arena_static_survives_join_and_collection` (multithreaded integration test): a worker
  stores an object in a static, is joined, and the object is read after three collections. Run
  it under an external timeout; the harness's cooperative abort cannot bound a deadlock.
- Cross-arena cycle fixtures with an unrooted control, retired-owner finalization with a CLR
  comparison, and timeout-bounded join/collection/shutdown race tests.
- Failed-initializer identity across owner retirement is verified by
  [plan 08 slice 4](08-async-effects/slice-4-static-initialization.md).
- The Loom models in `crates/dotnet-vm/tests/loom_workers.rs` cover the registry's transitions; they
  do not cover every shutdown interleaving.

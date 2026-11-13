# Shared GC handles and a dedicated finalizer thread

**Status:** In progress — slices 1–4 complete; slice 5 pending
**Outcome:** Under `multithreading`, finalizers run on one background finalizer thread with its own
identity, GC handles are readable from any thread, and the process exits without waiting for
finalizers; the GC fixtures listed under each slice match `dotnet` through the diff harness.

## Problem

Each arena owns its handle tables and runs its own finalizers on whichever thread it interrupts.
Paths are in `crates/dotnet-runtime-memory/src/heap.rs` (`HeapManager`),
`crates/dotnet-vm/src/intrinsics/gc.rs`, `crates/dotnet-vm/src/dispatch/mod.rs` and
`crates/dotnet-vm/src/executor.rs`.

- **Dependent handles are never traced or cleared.** `dependent_handles` appears in neither
  `HeapManager::trace` nor `finalize_check`/`clear_dead_weak_handles`, so after a sweep
  `InternalGetDependent` can read a freed object. `ConditionalWeakTable` uses these handles; no
  fixture exercises them.
- **Handles are per arena.** `encode_gc_handle` is `(index+1)*4` with no arena bits, and every
  operation indexes the calling arena's `gchandles` (`RefCell`, not `Sync`). From another thread,
  `InternalGet` returns an unrelated object or null, and `InternalFree`/`InternalSet` corrupt an
  unrelated slot. CoreLib's `WeakReference` finalizer calls `InternalFree`, so this must be fixed
  before finalizers move threads.
- **Finalizer identity.** `process_pending_finalizers` parks the interrupted context and runs
  `Finalize` on its OS thread, arena and managed thread ID. Monitors and type-initializer claims
  are keyed by `ArenaId` (`sync/threaded.rs`, `statics.rs`), so a finalizer re-enters locks the
  interrupted code holds and sees `StaticInitResult::Recursive` for an in-progress `.cctor`. On a
  retired worker, `threading_current_thread` allocates a fresh `Thread` with the old ID.
- **Resurrection across arenas.** `ObjectRef::resurrect` (`crates/dotnet-value/src/object/mod.rs`)
  recurses through every reference, recolors foreign objects with the local finalization context,
  and panics past depth 1000. No cross-arena mark runs after Finalize, so a foreign object
  reachable only from a resurrected object may be swept (not reproduced). Long weak handles are
  cleared per arena after local resurrection, so a cross-arena long handle depends on arena order.
  `ReRegisterForFinalize` on a foreign object pushes it onto the caller's untraced queue.
- **Exit waits for finalizers.** `Executor::run` → `wait_for_foreground_workers` requires
  `!has_pending_finalizers()` and no `Finalizing` worker (`workers.rs::foreground_quiescent`), so a
  finalizer that never returns blocks exit. `complete_entrypoint_return` also defers exit while
  local finalizers are pending, even through `run_entrypoint`. The CLR does neither.
- **`GC.WaitForPendingFinalizers`** (`gc.rs`, readiness in `native_effects.rs::advance_finalizers`)
  waits for global quiescence rather than for the finalizers queued when it was called, so
  ongoing allocation elsewhere can starve it.
- **CLR baseline (resolved in slice 2).** The original README claim that the resurrection and
  weak-handle fixtures disagree with the CLR lacked recorded evidence. The fixtures now run
  against CoreLib in the diff harness; the results are recorded below.

## Scope

In: GC handles (all kinds, plus dependent handles), finalizer scheduling and identity, cross-arena
resurrection and long-weak clearing, exit and `WaitForPendingFinalizers` semantics, under
`multithreading`.

Out:
- **Single-threaded finalizer identity.** There is no second OS thread, so the single-threaded
  build keeps running finalizers on the interrupted context. Giving it a separate identity would
  need cooperative scheduling between two contexts on one thread (green threads, ruled out in
  [plan 15](15-corelib-task-and-thread-pool.md)). Record this as a documented deviation. The
  handle, resurrection-walk and exit fixes still apply to single-threaded builds.
- Retired-worker resource cost and thread-static retention (separate README entries): the
  finalizer thread does not change when arenas may be freed.
- Pinned-handle native semantics beyond `AddrOfPinnedObject` (the native call boundary entry).

## Design

- **One shared handle table.** Move handles into `SharedGlobalState` as a thread-safe table of
  `ObjectPtr` + kind, traced from every arena's `CallStack::trace` like
  `SharedReflectionRegistry` (plan 14). Handle values stay index-based; the index is unique once
  the table is global. A reader turns the pointer into a local `ObjectRef` while the table roots
  it (`ObjectRef::from_storage_ptr`). Use the same table in single-threaded builds unless the
  `runtime_primitives` benchmark shows a measurable cost; one path is preferred.
- **Owner partition.** Every collection-time operation on the shared table is done by the arena
  that owns the object concerned (`ObjectRef::owner_id`), so no arena reads another arena's mark
  colors:
  - *Strong and pinned:* each arena traces the entries whose target it owns; the owner marks the
    target directly, so these entries add no cross-arena records.
  - *Short weak:* in `SweepPhaseCommand::ClearWeak` (after the marking fixed point, before any
    Finalize), each arena clears the dead entries whose target it owns. Long weak entries keep
    today's post-resurrection clearing until slice 4 moves it.
  - *Dependent:* slice 1's fixed point (`MarkPhaseCommand::Dependents` → `TraceDependents`,
    repeated until nothing activates) runs over the shared table. In `Dependents`, the key's owner
    activates the entry in a shared active set if the key is live; in `TraceDependents`, each
    arena traces the active values it owns. The key's owner clears both halves in `ClearWeak`
    when the key is dead.
  - Retired arenas keep servicing collection until shutdown
    (`F1.RetiredCollectionExclusion`), so every entry's owner takes part in each collection.
- **Mutation during collection.** A handle allocated or set from another thread while a
  collection is in progress must still be traced. Confirm that the collector re-traces arena
  roots at the end of marking (the property plan 14's registry relies on), and record that
  justification in a `SAFETY`/`INVARIANT` comment that cites the registry.
- **Resurrection.** Iterative walk that stops at foreign edges. The coordinator promotes objects
  awaiting finalization as roots before Finalize (CLR "f-reachable" promotion) so their foreign
  dependents survive, and long weak handles are cleared in a phase after every arena's Finalize.
- **Finalizer thread.** A registered background owner spawned like a pool worker
  (`WorkerRegistry::spawn_pool_owner`, `Executor::run_pool_worker`), with its own arena, `ArenaId`
  and `Thread` object. Owners publish objects awaiting finalization to a shared, traced queue
  during Finalize; the finalizer thread pops, runs `Finalize` with a foreign `this` (already
  supported through leases), and drops the entry. `ReRegisterForFinalize` and
  `SuppressFinalize` on a foreign object post to the owner, applied at its next Finalize phase.
  The per-arena `parked_finalizer_caller` path remains only for single-threaded builds.
- **Exit.** `Executor::run` and `complete_entrypoint_return` stop waiting for finalizers;
  shutdown aborts the finalizer thread at a safepoint and joins it before any arena is freed
  (`F1.RetiredCollectionExclusion`).
- **`WaitForPendingFinalizers`.** Record the queue's enqueue generation at the call and wait until
  the finalizer thread has completed up to it; return immediately on the finalizer thread.

## Slices

1. **Dependent handles.** Trace dependent handles whose primary is live, clear both halves when the
   primary dies (before Finalize, as for short weak handles), and add `conditional_weak_table_42.cs`
   (entries survive with their key, disappear after the key is collected, values kept alive only
   through the table). Diff against the CLR. Run the relevant Miri lane.
2. **CLR baseline.** Port `weak_handle_42.cs` to the real `GCHandle`, register both
   `gc_resurrection_*` fixtures and the port as diff tests (remove `new ...; r = null;` liveness
   dependence by allocating in non-inlined helpers), and record which cases genuinely differ.
   Update the README claim to match.
3. **Shared handle table** (revised 2026-11-13 after the first attempt stopped on
   unspecified cross-arena coordination; see Design, "Owner partition").
   - **3a. Normal, pinned and weak handles.** Move these kinds, including `AddrOfPinnedObject`
     and pinning, to the shared table with owner-partitioned tracing and `ClearWeak` clearing.
     Register `gc/cross_thread_handles_42.cs` (written in the first attempt: a strong `GCHandle`
     and a `WeakReference` created on one thread, read and set from another, before and after the
     creator exits, with collections in between; exits 42 on .NET) as a diff test. Update
     `executor_collection_tests.rs`, which pushes into `gchandles` directly. Add the table to
     `GC_AND_MEMORY_SAFETY.md` roots and the pointer inventory.
   - **3b. Dependent handles.** Move dependent handles to the same table, running slice 1's
     fixed point over it with owner-partitioned activation, tracing and clearing; delete the
     per-arena `dependent_handles` and `active_dependents`. Fixture (`multithreading`): a
     `ConditionalWeakTable` entry added on one thread and looked up on another; a value kept
     alive only through the table survives collection; the entry disappears after its key is
     collected, including after the creating thread exits; a key in one arena with its value in
     another. `conditional_weak_table_42` must still pass in ST and MT.
4. **Cross-arena resurrection.** Iterative walk, f-reachable promotion, long-weak clearing after
   all Finalize phases. Fixtures: a resurrected object whose only path to a foreign object is
   through itself, a 10,000-node resurrected list (today a panic), and a cross-arena long weak
   handle to a resurrected object; each repeated under `GC.Collect` from both threads.
5. **Finalizer thread.** Spawn it at VM start under `multithreading`, add the shared queue and
   owner-posted re-registration, and retire per-arena finalizer interleaving in MT. Fixtures:
   a finalizer taking a lock the main thread holds (blocks rather than re-enters), a finalizer
   touching a type whose `.cctor` is running on another thread (waits rather than seeing it
   half-run), `Thread.CurrentThread` in a finalizer (the same finalizer-thread object every time,
   not the allocating thread), and re-registration from inside `Finalize`. Rewrite
   `retired_finalizer_monitor` and the retired-finalizer tests in `fixtures.rs` and
   `executor_teardown_tests.rs` whose premise was finalizers on the retired owner.
6. **Exit and `WaitForPendingFinalizers`.** Stop waiting for finalizers at exit; generation-based
   `WaitForPendingFinalizers`. Rewrite `shutdown_waits_for_active_retired_finalizer`,
   `entrypoint_return_defers_exit_while_finalizers_pending`,
   `finalizer_after_entrypoint_preserves_exit_value` and `gc/finalizer_after_entrypoint_0.cs`
   (whose oracle requires a finalizer the CLR does not guarantee). Add a fixture where another
   thread keeps producing finalizable garbage while `WaitForPendingFinalizers` must still return.
   Drop `Executor::run_entrypoint` if `run` no longer differs from it.
7. **Docs.** Remove the README entries this plan resolves (cross-thread GC handles, finalization
   and GC compatibility, exit waits, `WaitForPendingFinalizers`), leaving the single-threaded
   identity deviation recorded. Update `GC_AND_MEMORY_SAFETY.md`,
   `THREADING_AND_SYNCHRONIZATION.md`, `INVARIANT_REGISTRY.md` (F1.FinalizerPublication,
   F1.GcHandleRooted) and the pointer inventory.

## Done when

- `conditional_weak_table_42`, the cross-thread handle fixture, the resurrection fixtures and the
  finalizer-thread fixtures match `dotnet` in MT, with and without `validation-all`, and the
  existing GC diff tests still pass.
- No handle operation reads an arena-local table under `multithreading`; dependent handles are
  traced and cleared.
- A finalizer that never returns does not delay process exit.
- Miri (cross-arena lane) and the Loom STW model cover the shared handle table and the
  f-reachable queue; the Loom model includes the finalizer thread in stop-the-world.

## Ask the maintainer only if

- The shared handle table costs measurably more than the per-arena table on single-threaded
  benchmarks (then decide whether single-threaded builds keep a local table).
- The owner partition cannot express an operation without reading a foreign arena's mark colors,
  or the collector does not re-trace roots at the end of marking.
- Running a finalizer with a foreign `this` needs a new access protocol beyond leases.
- Slice 2 shows a CLR disagreement that a dedicated finalizer thread does not explain.
- Stopping the finalizer thread at shutdown requires freeing arenas while a finalizer is
  mid-call.

## Slice 2 baseline

With non-inlined allocation helpers and the real CoreLib `GCHandle`, `gc_resurrection_42`,
`gc_resurrection_0`, and `weak_handle_42` match .NET 10 in the multithreaded host diff
harness: exit codes 42, 0, and 42 respectively, with empty stdout on both runtimes. These
cases do not test finalizer-thread identity or cross-arena handles.

## Slice 3a benchmark

The no-default-features `runtime_primitives` benchmark was run against 03cb031d and the
shared-table change (10 samples, one-second warmup and measurement). The stack benchmark
measured 18.21 ns before and 17.46 ns after; vector tracing measured 1.6605 ns before and
1.6618 ns after. Neither benchmark exercises GC-handle operations directly, so these numbers
do not establish their cost.

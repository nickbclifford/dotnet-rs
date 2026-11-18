# Shared GC handles and a dedicated finalizer thread

**Status:** Complete
**Outcome:** Under `multithreading`, finalizers run on one background finalizer thread with its own
identity, GC handles are readable from any thread, and the process exits without waiting for
finalizers; the GC fixtures match `dotnet` through the diff harness.

## What was built

- **One shared handle table.** `SharedGcHandles` (`crates/dotnet-runtime-memory/src/handles.rs`)
  holds every handle kind, including dependent handles, as provenance-preserving `ObjectPtr`s,
  traced from every arena. Handle values stay index-based. A reader roots the result before its
  next safepoint. Single-threaded builds use the same table.
- **Owner partition.** Each collection-time operation is done by the arena that owns the object:
  strong and pinned targets are traced by their owner; short weak entries are cleared by their
  owner in `SweepPhaseCommand::ClearWeak`, before any arena finalizes; dependent entries run a
  shared fixed point (`MarkPhaseCommand::Dependents` then `TraceDependents`) in which key owners
  activate live keys and value owners trace active values, and key owners clear dead pairs. No
  arena reads another arena's mark colors. See
  [Shared GC handle roots](../GC_AND_MEMORY_SAFETY.md#shared-gc-handle-roots).
- **Resurrection.** An iterative walk that stops at foreign edges; objects awaiting finalization
  are promoted as roots before Finalize (CLR f-reachable promotion), and long weak handles are
  cleared after every arena's Finalize.
- **Finalizer thread.** A registered background owner with its own arena and `Thread` object.
  Owners publish finalizable objects to the shared, traced `Finalizers` queue; the finalizer thread
  claims and runs them. `ReRegisterForFinalize`/`SuppressFinalize` on a foreign object are posted
  to the owner. See [Global finalizer completion](../GC_AND_MEMORY_SAFETY.md#global-finalizer-completion).
- **Exit and `WaitForPendingFinalizers`.** Exit no longer waits for finalizers; shutdown stops the
  finalizer thread at a safepoint before arenas are freed. `WaitForPendingFinalizers` waits for the
  enqueue generation observed at the call and returns immediately on the finalizer thread.
- **Fixed alongside:** a stop-the-world start race (an initiator that failed to take
  `gc_coordination` while a thread was registering parked at its own safepoint; now it parks only
  while another initiator holds the lock, `stw_initiator_holds_lock`, and re-asserts the stop
  request after acquiring it), and a `ConditionalWeakTable` crash on its ninth entry caused by the
  missing `DependentHandle.InternalGetTarget` intrinsic.

Without `multithreading`, finalizers run in a separate segment on the interrupted arena; plan 26
gave that segment its own `Thread` and owner key (see
[COMPATIBILITY](../COMPATIBILITY.md#remaining-boundaries) for the remaining deferral gap).

## Accepted cost

The shared table is not slower single-threaded (`bench-thin`, `--no-default-features`, 30
samples, alternating runs):

| Case | Before (two runs) | After (two runs) |
|---|---|---|
| `gc_handle_ops` | 462.6 / 466.0 ms | 460.7 / 464.3 ms |
| `weak_reference_get` | 81.3 / 80.8 ms | 80.1 / 81.0 ms |
| `conditional_weak_table_ops` | 752.8 / 749.8 ms | 744.1 / 740.0 ms |

```bash
taskset -c 12-15 cargo bench --profile bench-thin -p dotnet-benchmarks --no-default-features --bench end_to_end -- \
  'gc_handle_ops|weak_reference_get|conditional_weak_table_ops' --sample-size 30 --warm-up-time 2 --measurement-time 5
```

## Checks

- CLR differential fixtures in `crates/dotnet-cli/tests/fixtures/gc/`, among them
  `conditional_weak_table_42`, `conditional_weak_table_growth_42`, `cross_thread_handles_42`,
  `cross_thread_dependent_handles_42`, `cross_arena_resurrection_42`, the `finalizer_*` fixtures,
  `gc_resurrection_42`/`_0` and `weak_handle_42` (the last three match .NET 10 with exit codes 42,
  0 and 42 and empty stdout).
- Loom (`crates/dotnet-runtime-memory/tests/loom_shared_tables.rs`, preemption bound 3):
  `shared_handles_publish_and_clear_by_owner_partition` and
  `finalizer_queue_roots_claimed_and_pending_objects_while_worker_is_parked`. Loom does not track
  gc-arena mark colors, so reading a foreign arena's colors is not detected (disabling the owner
  filter in `clear_dead` is not caught).

  ```bash
  RUSTFLAGS="--cfg loom" cargo nextest run -p dotnet-runtime-memory --test loom_shared_tables --features multithreading
  ```
- Pinned Tree Borrows Miri singletons (multithreading):
  `foreign_short_weak_handle_clears_before_the_owner_finalizes`,
  `dependent_handle_chain_survives_and_clears` and
  `dedicated_finalizer_thread_claims_runs_and_completes_collected_finalizer`
  ([CI](../CI.md#miri)).
- Stop-the-world start race: `registered_initiator_waits_out_registration_without_parking` and
  `queued_initiator_reasserts_stop_request_after_previous_resume`.

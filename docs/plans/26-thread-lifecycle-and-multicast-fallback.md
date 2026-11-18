# Thread lifecycle reclamation, single-threaded finalizer identity and the multicast fallback

**Status:** Complete
**Outcome:** A completed managed worker whose arena holds no live objects releases its arena and OS
thread before VM shutdown; a thread's `[ThreadStatic]` values become collectible when it exits;
single-threaded finalizers never re-enter monitors or initializers held by the code they interrupt
and observe their own `Thread` identity; the production `WorkerRegistry` runs under Loom; and
`MulticastState` is deleted because every multicast `Invoke` shape uses native effects. The
Newtonsoft and EF InMemory host rungs still match `dotnet`.

## What was built

### Multicast without a fallback

`try_start_multicast` admits every valid shape before operand commitment, and the `MulticastState`
continuation, its dispatch step and its return and tail-call branches were deleted
(`newobj` keeps `InitializerResume::Retry`). See
[Multicast invocation](../DELEGATES_AND_DISPATCH.md#multicast-invocation).

- **Byref returns:** a `NativeOutput` byref variant accepts a non-`Transient` `ManagedPtr`; the body
  pushes the last target's rooted pointer unchanged (no reflection boxing).
- **`TypedReference` arguments:** a root cell holds the pointer descriptor and the original type
  descriptor.
- **Ref structs and byref targets containing managed pointers:** admitted with the existing
  tracing. Heap, `FrameHome`, static and cross-arena fields are traced recursively by
  `LayoutManager::trace` and re-resolved on access; `Unmanaged` fields keep the guest's
  `F10.RawMemoryAccessValid` obligation; a `Transient` field tag can be written but is rejected when
  read back, so it is not a valid stored encoding.
- A `Transient` byref argument raises `InvalidProgramException` before commitment.

### Single-threaded finalizers

- The single-threaded `SyncBlock` (`sync/single_threaded.rs`) records an owner key and recursion,
  so a non-owner `Exit`, `Wait` or `Pulse` throws `SynchronizationLockException`.
- Monitors, initializer claims, `[ThreadStatic]` lookups and managed `Thread` records key on the
  executing segment's logical thread. Under `multithreading` that is the arena's `ArenaId`; without
  it the finalizer segment has a reserved key and its own rooted `Thread`. Arena identity and
  pointer encoding are unchanged.
- Automatic finalizers run only when the interrupted context owns no monitor and no `Initializing`
  claim; otherwise they stay queued and are retried at later safepoints. `GC.WaitForPendingFinalizers`
  forces a run; if a finalizer then needs a monitor or initializer the parked caller owns, the VM
  reports its single-threaded deadlock error (`dotnet` would hang). A program that holds a lock for
  its whole run never finalizes on this build ([COMPATIBILITY](../COMPATIBILITY.md#remaining-boundaries)).

### Thread statics

At managed completion, under the registry publication lock, the thread's `[ThreadStatic]` entries
are removed (a drop guard covers unwind), so `Join` cannot observe completion before cleanup. Only
the owning thread creates storage, and a lookup for an exited owner is an error rather than fresh
storage. `F6.ThreadStaticRoot` now reads "live until its owning thread completes"; a raw address
taken with `conv.u` dangles after release, like any guest unmanaged pointer.

### Canonical objects in a long-lived arena

Newly materialized literals, `String.Empty` and shared reflection objects are allocated by the
dedicated finalizer executor through the
[canonical allocation service](../THREADING_AND_SYNCHRONIZATION.md#canonical-allocation-service)
(`threading/canonical.rs`, multithreaded only). The service installs the empty string before opening
admission, so empty-string paths never suspend, and services requests on every idle turn and every
`run_inner` iteration. A non-owner miss parks through the initializer wait machinery
(`NativeInitWait`) and retries with IP and stack unchanged. The maintainer approved one exception:
when `String.Intern(existingObject)` wins the table race, that exact object stays canonical in its
original arena, which is then retained until shutdown. Copying objects at retirement and weak
canonical tables were rejected because they cannot preserve identity for escaped references, weak
handles or `ConditionalWeakTable` keys.

### Releasing empty retired arenas

1. At retirement, and after each owner-side finalizer run, `Executor::clear_retired_roots` clears
   arena-local roots (hydrated reflection caches, the current `Thread`, the managed-pointer decode
   cache, finished execution state, completed child records).
2. After a coordinated sweep, a `Retired` owner checks gc-arena's `Metrics::total_gc_count() == 0`
   (not `HeapManager::live_object_count`, which misses `ObjectRef::new` allocations).
3. If empty, it takes `try_exclude_collection`, rechecks, moves to `Releasing`, and unregisters from
   the coordinator and thread manager. Outside those locks it drains exact leases
   (`RetainedArena::drain`); only a successful drain frees storage and publishes `Released`; a
   failure, self-held lease or unwind publishes `Leaked`.
4. `WorkerRegistry` keeps the entry so completion indices and outcomes stay stable. Live executors
   join released handles; shutdown joins the rest and waits for in-flight joins.

This narrowed `F1.RetiredCollectionExclusion`. The full lifecycle is in
[Managed worker lifecycle and process exit](../THREADING_AND_SYNCHRONIZATION.md#managed-worker-lifecycle-and-process-exit).

### Loom model of the production registry

`crates/dotnet-vm/tests/loom_workers.rs` compiles the production `WorkerRegistry` and models
spawn, completion, retirement, finalization, release and shutdown against a collection-exclusion
adapter, with at most three threads, two preemptions and 20,000 branches, and notification-driven
waits (Loom has no timeouts). `loom_stw.rs` still models the stop-the-world handoff.

```bash
DOTNET_SKIP_BUILD=1 RUSTFLAGS="--cfg loom" cargo nextest run -p dotnet-vm --test loom_workers --no-default-features --features multithreading
```

## Out of scope

- **Recycling arena IDs.** Monitor owners, static-init owners and static metadata can retain numeric
  keys, so IDs stay monotonic and the 65,535-per-process cap remains (plans README, "Retired-worker
  arena-ID capacity").
- **Pool workers** are capped and never retire.
- **A single-threaded scheduler**; the cost is the deferral gap above.
- **Arenas with live objects** are retained until shutdown.
- **gc-arena mark colors under Loom** are not modeled.

## Known issue

A diagnostic Miri run of `thread_static_gc_on_worker_exit` through the atomic field-store path
exposed a pre-existing pointer-provenance failure (the aligned byte-to-integer atomic path discards
provenance when storing object references); the pinned regression does not cover that store path.
Tracked in the plans README, "Shared raw-memory access".

## Accepted cost

The `string`, `dispatch` and `runtime_primitives` (`eval_stack_push_i32_pop_safe`,
`vector_trace_i32`) benchmarks showed no regression beyond noise in either configuration;
`string --test` instruction counts matched within 0.03%. MT `dispatch` moved from 371.16 to
382.11 ms (about 3%, below the 5% investigation threshold).

## Checks

- Multicast: `delegates/delegate_multicast_{byref_return,typed_reference,ref_struct,span,pointer,unmanaged_ref}_42.cs`
  and the other multicast and `statics/cctor_filter_*` CLR differential fixtures.
- Single-threaded finalizers: `gc/finalizer_thread_static_identity_42.cs`,
  `finalizer_monitor_wrong_owner_42.cs`, `finalizer_lock_deferral_42.cs` and
  `finalizer_cctor_deferral_42.cs`.
- Thread statics: `threading/thread_static_gc_42.cs`, `thread_local_lifetime_42.cs`, and the pinned
  Miri singleton `thread_static_gc_on_worker_exit`.
- Canonical objects: `crates/dotnet-vm/src/canonical_homing_tests.rs` (multithreaded only) and the
  `threading/canonical_*_42.cs` fixtures.
- Release: `empty_retired_arena_releases_before_shutdown`, `retired_owner_clears_dead_local_roots`,
  `sequential_empty_workers_keep_native_resources_bounded` (1,000 short-lived workers keep the live
  OS-thread count bounded), `release_drain_allows_collection_and_races_shutdown_reaper` and
  `released_worker_live_reaper_races_shutdown` (`executor_teardown_tests.rs`, run under an external
  timeout), plus the still-passing `retired_arena_static_survives_join_and_collection`.

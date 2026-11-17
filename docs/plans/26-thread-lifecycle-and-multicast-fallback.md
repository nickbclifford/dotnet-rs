# Thread lifecycle reclamation, single-threaded finalizer identity and the multicast fallback

**Status:** Not started
**Outcome:** A completed managed worker whose arena holds no live objects releases its arena and OS
thread before VM shutdown; a thread's `[ThreadStatic]` values become collectible when it exits;
single-threaded finalizers never re-enter monitors or initializers held by the code they interrupt
and observe their own `Thread` identity; the production `WorkerRegistry` runs under Loom; and
`MulticastState` is deleted because every multicast `Invoke` shape uses native effects. Each is
checked by the fixtures and tests named in its slice, and the Newtonsoft and EF InMemory host rungs
still match `dotnet`.

Closes these README "Not yet planned" entries: **Fallback continuations for excluded shapes**,
**Single-threaded finalizer identity**, **Thread-static retention**, **Loom model coverage**, and
the per-worker OS-thread and arena half of **Retired-worker resource cost** (the 65,535 process-wide
arena-ID cap stays there; see Scope).

## Problem

### Multicast fallback (`MulticastState`)

Multicast `Invoke` selects native effects in `try_start_multicast`
(`crates/dotnet-vm/src/stack/native_effects.rs`) and otherwise falls back to the legacy
`MulticastState` continuation (`crates/dotnet-vm-data/src/stack.rs`, `handle_multicast_step` in
`crates/dotnet-vm/src/dispatch/mod.rs`, the entry in `crates/dotnet-intrinsics-delegates/src/invoke.rs`,
and the return and tail-call checks in `stack/context.rs` and `stack/call_ops_impl.rs`; about 230
lines). It falls back for:

- **Byref returns**: `multicast_output` maps any `Ref` return to `Unsupported`, and `NativeOutput`
  (`stack/context.rs`) has no byref variant (`native_return_tests.rs` asserts the rejection).
- **`TypedReference` arguments**: `RootedValue::stash` (`crates/dotnet-vm-ops/src/effects.rs`) has no
  arm for `StackValue::TypedRef`.
- **Ref-struct values and byref targets containing managed pointers**: `multicast_value_shape` and the
  `Ref` arm require `!has_managed_ptrs()`.

Plan 16 kept the fallback for these shapes ("Kept") and ruled out ref-struct transport as scope creep
for fear of `Transient`/`Unmanaged` pointer transport. A `Transient` pointer cannot be stored in a
field at all (`crates/dotnet-value/src/pointer/serde.rs`), so a ref-struct value never carries one.
Its `FrameHome`, heap, static and cross-arena fields are already traced by `LayoutManager::trace` and
re-resolved on access, and `Unmanaged` fields carry the trust already granted to `Unmanaged` byrefs
(`F10.RawMemoryAccessValid`). Fixtures: `delegate_multicast_ref_struct_42.cs` (expected selection
counts async/argument/return = 0/2/1), `delegate_multicast_byref_return_42.cs` (0/0/1) and
`delegate_multicast_typed_reference_42.cs` (0/1/0), checked by
`multicast_selection_precedes_operand_commitment` in
`crates/dotnet-cli/tests/integration_tests_impl/fixtures.rs` and by the diff harness.

### Single-threaded finalizers

Without `multithreading`, `process_pending_finalizers` (`dispatch/mod.rs`) swaps the interrupted
`ThreadContext` into `parked_finalizer_caller` and runs `Finalize` in a fresh segment. Identity is
per arena, not per segment, so the finalizer:

- **re-enters monitors silently.** The single-threaded `SyncBlock` (`sync/single_threaded.rs`)
  records no owner: `try_enter` always succeeds and `is_owned` is `recursion_count > 0`. A finalizer can
  enter, exit, wait on or pulse a monitor the interrupted code holds, with no
  `SynchronizationLockException`.
- **reads partially initialized statics.** `StaticsManager::init` (`statics.rs`) returns `Recursive`
  for a `.cctor` the interrupted code is running, and the native probe treats that as complete.
- **shares `Thread.CurrentThread`, `Environment.CurrentManagedThreadId` and `[ThreadStatic]`
  storage.** These come from `local.managed_threads` and `CallStack::thread_id` (`threading/local.rs`,
  `stack/context_ops.rs`, `instructions/objects/fields.rs`).

Plan 20 left this out because two contexts on one OS thread would need a scheduler (plan 15, "Out").
This plan avoids the scheduler: it runs finalizers only when the interrupted code holds nothing a
finalizer could contend for.

### Thread-static retention

`StaticStorageManager.thread_statics` (`statics.rs`) is keyed by `(type, generics, ArenaId)`, created
lazily by `get_for_thread`, never pruned, and traced by every arena (`CallStack::trace` →
`shared.statics.trace`). `F6.ThreadStaticRoot` keeps each allocation live "while any serialized field
address can resolve to it". The CLR frees a thread's thread-static values when it exits, so objects
reachable only from them become collectible and finalizable.

Static byrefs (`PointerOrigin::Static` with `thread_id: Some(owner)`) are not traced; every access
re-resolves through `get_for_metadata`. Valid programs cannot carry a byref to another thread's
thread static (byrefs cannot be captured by `ThreadStart`, boxed or stored in heap objects). Only
`conv.u` on a managed pointer produces a raw `Unmanaged` address into the storage.

### Retired-worker resource cost

A completed worker stays `Retired` in `WorkerRegistry` (`threading/workers.rs`) and services GC
commands in `Executor::serve_retired` (`executor.rs`) until `SharedGlobalState::shutdown_workers`.
Its arena is freed only after the shutdown signal and global quiescence (`F1.RetiredCollectionExclusion`).
Every managed `Thread` therefore costs one OS thread and one arena until exit. Nothing can say whether
a retired arena still holds live objects: the coordinator has no per-arena liveness summary, and
`HeapManager::live_object_count` misses allocations made through `ObjectRef::new`. gc-arena 0.7's
`Metrics::total_gc_count()` reports live `Gc` allocations after a sweep, but it is never consulted.

Besides thread statics, two shared tables pin worker-allocated objects permanently:
`SharedGlobalState::interned_strings` (`stack/stack_ops_impl.rs`) and `SharedReflectionRegistry`
(`state.rs`). The first thread to `ldstr` a literal or materialize a `RuntimeType` owns the canonical
object, and both tables' `SAFETY` comments rely on "owner arenas are retained until shutdown". If
nothing changes, almost every worker arena stays non-empty forever.

### Loom coverage

`dotnet-vm` with `multithreading` compiles under `cfg(loom)` as of 6e29f56d (CI checks it), but no model
drives it. `crates/dotnet-vm/tests/loom_stw.rs` mirrors the stop-the-world handoff by hand and does not
compile the production `WorkerRegistry`. Retirement and shutdown transitions are unmodelled.

## Scope

In:

- Native transport for every multicast shape the fallback handles, then deleting `MulticastState`
  and everything only it uses (`delegate_method_info`, `delegate_call_frame`, the multicast arms of
  the return and tail-call checks). `InitializerResume::Retry` stays because `newobj` uses it.
- Single-threaded monitor owners, a separate finalizer identity, and deferring finalizers to points
  where the interrupted code holds no monitor and no in-progress initializer.
- Releasing a thread's thread-static storage at managed completion (both builds).
- Allocating canonical objects (interned strings, shared reflection objects) in an arena that lives
  until shutdown.
- Releasing an empty retired worker's arena and OS thread (`multithreading`).
- A Loom model that compiles the production `WorkerRegistry`, extended for the new transition.

Out, with the reason:

- **Recycling arena IDs.** Tag-5 `ObjectRef` decoding is ID-only, and monitor owners, static-init
  owners and the static metadata registry key by bare `ArenaId`. Recycling would need
  generation-checked decoding and a purge of every ID-keyed table. IDs stay monotonic, so the
  65,535-per-process cap remains in "Not yet planned".
- **Pool workers.** They are capped at `max(2, available_parallelism)` and deliberately never retire
  between work items; their arenas need no early release.
- **A single-threaded scheduler.** A finalizer is never parked so the interrupted code can resume
  (plan 15's exclusion stands). The cost is the residual deferral gap under Design.
- **Arenas with live objects.** A retired arena that still owns a reachable object (for example one
  stored in an ordinary static) is retained until shutdown, as the CLR keeps the object alive.
- **gc-arena mark colors under Loom.** Loom cannot track them; the owner-partition limit recorded in
  plan 20 stays.

## Design

### Multicast

Admit each shape in `try_start_multicast` with the same before-commitment checks it uses now:

- **Byref returns:** add a `NativeOutput` byref variant whose validation accepts a non-`Transient`
  `ManagedPtr`. The target's return already flows through `deliver_native_return` → `region.root`,
  which stashes non-`Transient` pointers (the transport plan 19 relies on), so the body pushes the
  fetched pointer unchanged. Multicast pushes the raw pointer; it does not reuse reflection's
  dereference-and-box result path.
- **`TypedReference`:** a `RootCellValue` variant holding the pointer descriptor and `type_desc`,
  with stash and fetch arms and the existing origin rule.
- **Ref structs and byref targets containing managed pointers:** drop `!has_managed_ptrs()` from
  `multicast_value_shape` and the `Ref` arm. Write the justification above (no `Transient` in fields,
  traced origins, `Unmanaged` trust) at the admission site.

### Single-threaded finalizers

- **Monitor owners.** The single-threaded `SyncBlock` records an owner key and a recursion count, like
  the threaded one, so a non-owner `Exit`, `Wait` or `Pulse` throws `SynchronizationLockException` as
  the CLR does.
- **Logical thread key.** Monitors, initializer claims, `[ThreadStatic]` lookups and managed
  `Thread` records key on the executing segment's logical thread, not on `CallStack::thread_id`. In
  multithreaded builds that is the arena's `ArenaId`, so nothing changes there. In single-threaded
  builds the finalizer segment gets one reserved key and its own `Thread` object, created once.
  Pointer serialization and arena identity keep using `ArenaId`.
- **Deferral.** `process_pending_finalizers` starts a segment only when the interrupted context owns
  no monitor and no `Initializing` claim. Otherwise the finalizers stay queued and are retried at the
  next safepoint. `GC.WaitForPendingFinalizers` forces the run. If the finalizer then needs a monitor
  or initializer the parked caller owns, the run raises the existing single-threaded "would deadlock"
  internal error, as an infinite `Monitor.Wait` does. In the CLR, the finalizer thread would block
  while the caller waits on it, which is also a deadlock. Residual gap: a program that holds a lock
  for its whole run never finalizes on this build. Record it in COMPATIBILITY.md.

### Thread statics

At managed completion, after the thread's outcome is published and before `serve_retired`, remove
every `thread_statics` entry keyed by the completing thread. Only the owning thread may create its
storage. A `get_for_metadata` lookup for an exited thread's key returns an error, never fresh zeroed
storage. Valid IL cannot reach that path; an unsafe program that does gets an internal error rather
than silent new storage. A raw address taken with `conv.u` dangles after release, under the same
trust as any other guest unmanaged pointer. Rewrite `F6.ThreadStaticRoot` as "live until its owning
thread completes".

### Canonical objects

Interned strings and `SharedReflectionRegistry` objects are allocated by a thread whose arena lives
until shutdown, not by whichever thread asks first. Today the entry and finalizer arenas live that
long. Implement the cheapest option that keeps CLR identity (a canonical object is created once and
never collected):

1. a lookup miss on a worker sends an allocation request to the entry or finalizer executor through
   the existing native-effect wait, and the worker resumes with a cross-arena reference; or
2. the canonical object is created in the requesting arena and copied to the long-lived arena when
   its owner retires. Identity breaks if any reference to the original survives, so option 2 holds only
   if the registry can prove no reference does.

Weak canonical entries (recreate on demand) are rejected: they change identity observable through
weak handles and `ConditionalWeakTable` keys, which conflicts with matching CLR semantics.

### Releasing empty retired arenas

1. At retirement, the worker clears arena-local state that roots its own objects
   (`ReflectionLocalState` caches, `local.managed_threads`, the managed-pointer decode cache).
2. After each coordinated collection it services (after sweep and its own finalizers), a `Retired`
   owner checks `total_gc_count() == 0`.
3. If the arena is empty, the owner takes `try_exclude_collection` and re-checks under that lock, then
   transitions `Retired → Released`. It unregisters from the coordinator and thread manager, drains
   exact leases through the existing `RetainedArena::drain` path (leaking on a self-held lease, as
   today), drops the arena and returns from its OS thread.
4. `WorkerRegistry` keeps the entry so completion indices stay stable. Shutdown joins `Released`
   handles and skips their drain.

This narrows `F1.RetiredCollectionExclusion`: an arena may be freed before shutdown only under
collection exclusion, after a collection proves it holds no live `Gc`. Nothing can then name its
objects. Cross-arena references to it were traced, weak handles to it were cleared by its own
`ClearWeak`, and f-reachable promotion keeps any object a pending finalizer can reach. Static and
cross-arena byrefs re-resolve or are token-checked.

## Slices

Slices 1–3, 4–5, 6 and 7–9 touch independent subsystems. Run them in this order unless a later group
becomes urgent.

### Slice 1: multicast byref returns

Add the `NativeOutput` byref variant and admit `Ref` returns. `delegate_multicast_byref_return_42.cs`
expects 1/0/0, and `native_return_tests.rs` asserts acceptance plus the `Transient` rejection.

### Slice 2: multicast `TypedReference`

Add the `RootCellValue` variant and admit `TypedReference` arguments.
`delegate_multicast_typed_reference_42.cs` expects 1/0/0, and a collection during the multicast call
keeps the referenced local valid (native unit test).

### Slice 3: ref structs, then delete `MulticastState`

Relax the managed-pointer rule. `delegate_multicast_ref_struct_42.cs` expects 3/0/0, and a new fixture
multicasts a `SpanAction`-style delegate over a `Span<int>` taken from `stackalloc` and from an array.
Then delete `MulticastState` and everything only it uses, and remove the "Kept" row from plan 16. The
selection test now asserts no fallback counter exists.

### Slice 4: single-threaded monitor owners and finalizer identity

Owner tracking in `sync/single_threaded.rs`, the logical thread key, and a finalizer `Thread`.
Differential fixtures (single-threaded and multithreaded, each matching `dotnet`):

- a finalizer reports a `ManagedThreadId` and `Thread.CurrentThread.IsBackground` different from
  Main's;
- a finalizer's `[ThreadStatic]` value is distinct from Main's;
- `Monitor.Exit` on a lock the current thread doesn't own throws `SynchronizationLockException`.

### Slice 5: single-threaded finalizer deferral

Gate `process_pending_finalizers` on the interrupted context's owned monitors and initializer claims.
Fixtures:

- an object becomes unreachable while Main holds lock `L` and allocates; the finalizer (which takes `L`)
  runs only after Main releases `L`, matching `dotnet`'s output order;
- the same inside a `.cctor`;
- `WaitForPendingFinalizers` while holding `L`, with a finalizer that takes `L`, gives the "would
  deadlock" error. This one is VM-only: `dotnet` hangs, so the diff harness can't run it.

### Slice 6: release thread statics at thread exit

Remove the completing thread's entries, add the owner-only creation rule and the exited-thread lookup
error, and rewrite `F6.ThreadStaticRoot`. Fixtures matching `dotnet` under `multithreading`:

- a worker stores a finalizable object only in a `[ThreadStatic]` field and exits; after `Join`,
  `GC.Collect` and `WaitForPendingFinalizers`, the finalizer has run;
- `ThreadLocal<T>` values of an exited thread are released (`Values` with `trackAllValues: true` and a
  `WeakReference` to the value).

`retired_arena_static_survives_join_and_collection` (ordinary statics) still passes. Pinned Miri
shard: the thread-static GC fixture. This slice applies to single-threaded builds only through the
entry thread, which never exits before shutdown, so nothing changes there.

### Slice 7: canonical objects in a long-lived arena

Choose option 1 or 2 from Design and record why. Fixtures:

- a worker interns a literal and materializes a `RuntimeType` first and exits; Main then observes
  the same identity (`ReferenceEquals`, `String.IsInterned`, and a `ConditionalWeakTable` keyed by
  the `Type`) across collections;
- the `string`, `dispatch` and `runtime_primitives` benches show no regression beyond noise in
  single-threaded and multithreaded builds. Report `perf stat` instruction counts as well as times;
  see the README entry on the `string` bench.

### Slice 8: production `WorkerRegistry` under Loom

Compile `threading/workers.rs` into a Loom model, either as a `#[cfg(all(loom, test))]` module or through
`#[path]` as `loom_shared_tables.rs` does. Model spawn, complete, retire, finalize and shutdown against
a collection exclusion. Add it to the `loom` CI job. Replace any `loom_stw.rs` assertion this model
covers with the production version; keep `loom_stw.rs` for the stop-the-world handoff.

### Slice 9: release empty retired arenas

Implement the release steps under Design and extend the slice 8 model with `Retired → Released`
racing a collection request and shutdown. Tests (`multithreading`, run under an external timeout):

- a worker that allocates only temporaries and exits releases its arena: its OS thread is joined
  before shutdown and its `ArenaId` is unregistered;
- a worker whose object is kept in an ordinary static is not released, and the object survives
  (the existing retired-arena regression);
- 1,000 sequential short-lived workers keep the live OS-thread count bounded (a `/proc/self/task`
  count, or an equivalent exposed by the registry);
- a collection requested concurrently with release, and shutdown concurrently with release, complete
  without deadlock or use-after-free. Pinned Miri shard for the release path.

### Slice 10: documentation

Update THREADING_AND_SYNCHRONIZATION.md (lifecycle, single-threaded finalizers, resource cost),
GC_AND_MEMORY_SAFETY.md (retired worker arenas, thread statics), INVARIANT_REGISTRY.md
(`F1.RetiredCollectionExclusion`, `F6.ThreadStaticRoot`, the interned-string and reflection-registry
`SAFETY` comments), and COMPATIBILITY.md (the deferral gap). In the README, remove the entries this
plan closes and shrink "Retired-worker resource cost" to the ID cap.

## Done when

- All new and updated fixtures match `dotnet` in both base feature sets, except the VM-only deadlock
  case in slice 5.
- `MulticastState` and its helpers are gone; `rg MulticastState crates` finds nothing.
- A thread-static-only object of an exited thread is finalized, and 1,000 short-lived workers do not
  accumulate OS threads.
- The Loom job runs the production `WorkerRegistry` model.
- `./check.sh --fast`, the full `dotnet-vm` unit tests (both feature sets), clippy `-D warnings`, the
  doc-drift check and the Loom and pinned Miri jobs pass.
- The Newtonsoft and EF InMemory host rungs still match `dotnet`.

## Ask the maintainer only if

- Ref-struct transport in slice 3 needs anything beyond relaxing the value-shape rule: a new pointer
  origin, `Transient` support, or a change to `LayoutManager::trace`.
- Slice 4's logical key would have to change `ArenaId` itself, or pointer serialization.
- Slice 7 finds neither canonical-object option workable without changing observable identity, or
  either one costs more than 5% on a benchmark.
- `total_gc_count()` is non-zero for an idle retired arena after slice 9's cleanup, and the remaining
  allocations can't be attributed and cleared.
- Releasing an arena needs a change to the Tag-5 codec or to `RegistrationToken` semantics.

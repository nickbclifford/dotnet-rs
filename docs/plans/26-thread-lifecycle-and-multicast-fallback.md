# Thread lifecycle reclamation, single-threaded finalizer identity and the multicast fallback

**Status:** Complete
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

At the start of this plan, multicast `Invoke` selected native effects for most shapes and used a
legacy continuation for the rest. Slices 1–3 admit byref returns, typed references and ref-struct
values and remove that continuation. `try_start_multicast` selects before operand commitment;
invalid transient byrefs are rejected there.

A writer can emit a `Transient` tag, but metadata and resolved readers reject it; it is not a
valid round-trippable stored field representation. Heap, `FrameHome`, static and cross-arena
fields are traced recursively by `LayoutManager::trace` and re-resolved on access. `Unmanaged`
fields carry the existing guest trust (`F10.RawMemoryAccessValid`). The ref-struct selection
fixture expects 3/0/0; a stackalloc/array span fixture exercises collection, order, exceptions
and initialization.

### Single-threaded finalizers

Without `multithreading`, `process_pending_finalizers` (`dispatch/mod.rs`) swaps the interrupted
`ThreadContext` into `parked_finalizer_caller` and runs `Finalize` in a fresh segment. Before this
plan, identity was per arena rather than per segment, so the finalizer:

- **re-enters monitors silently.** The single-threaded `SyncBlock` (`sync/single_threaded.rs`)
  recorded no owner: `try_enter` always succeeded and `is_owned` tested only recursion. A finalizer could
  enter, exit, wait on or pulse a monitor the interrupted code held, with no
  `SynchronizationLockException`.
- **reads partially initialized statics.** `StaticsManager::init` (`statics.rs`) returns `Recursive`
  for a `.cctor` the interrupted code was running, and the native probe treated that as complete.
- **shares `Thread.CurrentThread`, `Environment.CurrentManagedThreadId` and `[ThreadStatic]`
  storage.** These came from `local.managed_threads` and `CallStack::thread_id` (`threading/local.rs`,
  `stack/context_ops.rs`, `instructions/objects/fields.rs`).

Plan 20 left this out because two contexts on one OS thread would need a scheduler (plan 15, "Out").
This plan avoids the scheduler: it runs finalizers only when the interrupted code holds nothing a
finalizer could contend for.

### Thread-static retention

At the start of this plan, `StaticStorageManager.thread_statics` (`statics.rs`) was keyed by
`(type, generics, ArenaId)`, created lazily by `get_for_thread`, never pruned, and traced by every arena (`CallStack::trace` →
`shared.statics.trace`). The former `F6.ThreadStaticRoot` kept each allocation live "while any serialized field
address can resolve to it". The CLR frees a thread's thread-static values when it exits, so objects
reachable only from them become collectible and finalizable.

Static byrefs (`PointerOrigin::Static` with `thread_id: Some(owner)`) are not traced; every access
re-resolves through `get_for_metadata`. Valid programs cannot carry a byref to another thread's
thread static (byrefs cannot be captured by `ThreadStart`, boxed or stored in heap objects). Only
`conv.u` on a managed pointer produces a raw `Unmanaged` address into the storage.

### Retired-worker resource cost

At the start of this plan, a completed worker remained `Retired`, servicing GC commands until
`SharedGlobalState::shutdown_workers`; every managed `Thread` cost one OS thread and one arena
until shutdown. `HeapManager::live_object_count` misses allocations made through `ObjectRef::new`;
release instead uses gc-arena 0.7's `Metrics::total_gc_count()` after a coordinated sweep.

Besides thread statics, shared interned-string and reflection tables could pin worker-allocated
objects permanently. The canonical service homes new literals, empty strings and shared reflection
objects in its long-lived arena. An explicit `Intern(existingObject)` winner still pins its original
live-object arena to preserve identity.

### Loom coverage

`dotnet-vm` with `multithreading` compiles under `cfg(loom)`. The production worker registry is
modeled in `loom_workers`, while `loom_stw` independently models the stop-the-world handoff.

## Scope

In:

- Native transport for every multicast shape the fallback handles, then deleting `MulticastState`
  and everything only it uses (`delegate_method_info`, `delegate_call_frame`, the multicast arms of
  the return and tail-call checks). `InitializerResume::Retry` stays because `newobj` uses it.
- Single-threaded monitor owners, a separate finalizer identity, and deferring finalizers to points
  where the interrupted code holds no monitor and no in-progress initializer.
- Releasing a thread's thread-static storage at managed completion (both builds).
- Allocating newly materialized literal and empty strings and shared reflection objects in an arena
  that lives until shutdown. Explicit `String.Intern(existingObject)` instead preserves the winning
  input's identity and retains its live-object arena.
- Releasing an empty retired worker's arena and OS thread (`multithreading`).
- A Loom model that compiles the production `WorkerRegistry`, extended for the new transition.

Out, with the reason:

- **Recycling arena IDs.** Tag-5 cross-arena references already validate registration tokens, but
  monitor owners, static-init owners and static metadata can retain numeric keys. Recycling would
  require auditing and purging every ID-keyed table. IDs stay monotonic, so the
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
  `multicast_value_shape` and the `Ref` arm. Write the justification above (no valid stored `Transient` encoding,
  traced origins, `Unmanaged` trust) at the admission site.

### Single-threaded finalizers

- **Monitor owners.** The single-threaded `SyncBlock` records an owner key and a recursion count, like
  the threaded one, so a non-owner `Exit`, `Wait` or `Pulse` throws `SynchronizationLockException` as
  the CLR does.
- **Logical thread key.** Monitors, initializer claims, `[ThreadStatic]` lookups and managed
  `Thread` records key on the executing segment's logical thread, not on `CallStack::thread_id`. In
  multithreaded builds that is the arena's `ArenaId`, so nothing changes there. In single-threaded
  builds the finalizer segment gets one reserved key and its own `Thread` object, created once.
  Arena identity and object-reference encoding keep using `ArenaId`; cross-arena managed-pointer
  encoding continues to use its registration token.
- **Deferral.** `process_pending_finalizers` starts a segment only when the interrupted context owns
  no monitor and no `Initializing` claim. Otherwise the finalizers stay queued and are retried at the
  next safepoint. `GC.WaitForPendingFinalizers` forces the run. If the finalizer then needs a monitor
  or initializer the parked caller owns, the run raises the existing single-threaded "would deadlock"
  internal error, as an infinite `Monitor.Wait` does. In the CLR, the finalizer thread would block
  while the caller waits on it, which is also a deadlock. Residual gap: a program that holds a lock
  for its whole run never finalizes on this build. Record it in COMPATIBILITY.md.

### Thread statics

At managed completion, after the thread's outcome is published and before `serve_retired`, remove
every `thread_statics` entry keyed by the completing thread. The registry lock covers publication
and cleanup, so Join cannot observe completion until cleanup finishes. A guard also closes the
owner on unwind before the registry publishes the panic. Only the owning thread may create its
storage. A `get_for_metadata` lookup for an exited thread's key returns an error, never fresh zeroed
storage. Valid IL cannot reach that path; an unsafe program that does gets an internal error rather
than silent new storage. A raw address taken with `conv.u` dangles after release, under the same
trust as any other guest unmanaged pointer. Rewrite `F6.ThreadStaticRoot` as "live until its owning
thread completes".

### Canonical objects

Choose option 1 for newly materialized literal and empty strings and every shared reflection object:
a lookup miss sends an owned allocation request to the dedicated finalizer executor via a native
effect, and the requester resumes with the rooted canonical reference. The finalizer arena remains
live until shutdown, including before the first finalizer and during native waits. Single-threaded
allocation remains local. The service rechecks misses and handles nested materialization locally.

The maintainer approved an exception for `String.Intern(existingObject)`: if its input wins the
table race, the exact input remains canonical in its original arena, which is a genuinely live-object
arena retained until shutdown. A losing input is not retained by the table. This does not authorize
homing ordinary string allocations.

Copying objects to a long-lived arena on retirement cannot preserve identity: references can escape
through statics, weak handles, `ConditionalWeakTable` keys, caches and other threads. Weak canonical
entries (recreate on demand) are likewise rejected because they change observable identity.

### Releasing empty retired arenas

1. At retirement, the worker clears arena-local state that roots its own objects
   (`ReflectionLocalState` caches, `local.managed_threads`, the managed-pointer decode cache).
2. After each coordinated collection it services (after sweep and its own finalizers), a `Retired`
   owner checks `total_gc_count() == 0`.
3. If the arena is empty, the owner takes `try_exclude_collection` and rechecks zero and lifecycle
   eligibility, transitions `Retired → Releasing`, publishes finalizer state and unregisters from the
   coordinator and thread manager. It transfers storage and its exact registration to a retention
   guard. Outside collection exclusion and registry locks, it drains exact leases via
   `RetainedArena::drain`. Only a successful drain frees storage and publishes `Released`; failure,
   a self-held lease or unwind publishes `Leaked`.
4. `WorkerRegistry` keeps the entry so completion indices and outcomes stay stable. The live reaper
   joins ready released handles outside locks; shutdown joins any remaining handles, waits for
   in-flight joins and does not repeat their arena drain.

This narrows `F1.RetiredCollectionExclusion`: before shutdown, unregistration requires collection
exclusion after a sweep proves zero total `Gc` allocations and finalizer effects have settled;
lease draining and freeing follow outside exclusion. Cross-arena references are traced, weak handles
are cleared by the owner's `ClearWeak`, and f-reachable promotion keeps pending-finalizer objects
alive. Static and cross-arena byrefs re-resolve or are token-checked.

## Slices

Slices 1–3 are complete. Implement and accept slices 4 and 5 together: owner-correct finalizer
identity makes automatic finalizer deferral necessary for the existing fast suite, and deferral must
include the forced-wait path so explicit waits do not stall. Slice 6 follows because its storage
ownership depends on the logical identity in slice 4. Slices 6, 7 and 8 precede slice 9: thread-static
cleanup and canonical homing remove avoidable roots, and the production Loom model supplies the
baseline to extend for release races. Slice 10 follows all implementation slices. The existing order
among slices 6, 7 and 8 satisfies these dependencies; they do not require an additional reorder.

### Slice 1: multicast byref returns

Add the `NativeOutput` byref variant and admit `Ref` returns. `delegate_multicast_byref_return_42.cs`
expects 1/0/0, and `native_return_tests.rs` asserts acceptance plus the `Transient` rejection.

**Evidence (2026-11-17):** Native activation validates the descriptor origin and delivers the last
rooted pointer without reflection boxing. The byref-return fixture exercises static and heap origins,
write-through and collection; native return tests cover unmanaged aliasing, mismatched result and
`Transient` rejection. After rebuilding fixtures, `native_return_tests | native_substring_tests`
passed 119/119 without features and 170/170 with `multithreading`; the selection test passed 1/1
in each build, with the fixture selecting 1/0/0. The fixture's CLR differential passed in both builds
with explicitly rebuilt binaries. Both descriptor Miri selectors passed 1/1 with zero ignored in each
build on nightly-2026-05-27. Both all-target clippy `-D warnings` legs, doc drift (59/59), and
rustdoc broken-link checking passed. `./check.sh --fast` passed with 1,151, 1,460 and 1,477
tests run in its three feature sets (58, 7 and 6 other tests skipped, respectively).

### Slice 2: multicast `TypedReference`

Add the `RootCellValue` variant and admit `TypedReference` arguments.
`delegate_multicast_typed_reference_42.cs` expects 1/0/0, and a collection during the multicast call
keeps the referenced local valid (native unit test).

**Evidence (2026-11-17):** A traced typed-reference root stores the original type descriptor and
uses the same validated pointer descriptor as byrefs. The executor regression collects between
managed child calls with a caller local home and verifies write-through; the enabled fixture also
collects between its multicast targets. After fixture rebuild and nonzero test listing,
`native_return_tests | native_substring_tests` passed 120/120 without features and 171/171 with
`multithreading`; the selection test passed 1/1 in each build, with the typed-reference fixture
selecting 1/0/0. The fixture's CLR differential passed in both builds using explicitly rebuilt
binaries. Both pinned descriptor Miri tests and the typed-reference heap-root test passed 1/1 with
zero ignored in each build on nightly-2026-05-27. The executor local-home regression also passed
1/1 with zero ignored under Miri in each build. Both all-target clippy `-D warnings` legs, doc
drift (59/59) and rustdoc broken-link checking passed.
`./check.sh --fast` passed with 1,153, 1,462 and 1,479 tests run in its three feature sets
(58, 7 and 6 other tests skipped, respectively).

### Slice 3: ref structs, then delete `MulticastState`

Relax the managed-pointer rule. `delegate_multicast_ref_struct_42.cs` expects 3/0/0, and a new fixture
multicasts a `SpanAction`-style delegate over a `Span<int>` taken from `stackalloc` and from an array.
Then delete `MulticastState` and everything only it uses, and remove the "Kept" row from plan 16. The
selection test now asserts no fallback counter exists.

**Evidence (2026-11-17):** Pointer-bearing value layouts and byref targets now select the
native activation, with valid origin checks unchanged; unsupported operands are rejected before
commitment. The ref-struct fixture selects 3 native calls and tests ref-field replacement through a byref, last-result aliasing and GC;
the span fixture selects 4 and exercises stackalloc, array write-through, ordering, a throwing
target/filter and a cctor under collection. A rooted nested pointer-field regression proves owner
retention and eventual release. A by-value unmanaged/function pointer fixture also verifies native
selection, write-through and last-result identity; its root preserves pointer provenance while
leaving unmanaged memory access under the existing guest trust rule. The legacy frame state, step, return/tail-call branches, host
helpers and initializer override are gone; `newobj` retains `InitializerResume::Retry`.
Source search for the removed state and helpers found no Rust production matches, and selection
asserts native counts and rejects other path events with a source guard.

After rebuilding fixtures, the native return/substring filters passed 120/120 without features and
171/171 with multithreading; the selection and two ref-struct differential tests passed 3/3 in each
build; the enhanced ref-struct and pointer fixtures and selection reruns each passed 2/2 in both builds. Explicitly rebuilt CLI binaries matched CLR stdout and exit for all 26 applicable multicast
and cctor-filter sources without features and all 27 with multithreading (the contended worker
fixture requires multithreading). Nightly-2026-05-27 Miri with CI flags passed the heap, frame-home,
typed-reference, unmanaged-pointer and nested-value root tests, the recursive layout test and executor typed-reference
collection test in both builds, each 1 passed and 0 ignored. Both all-target clippy legs, doc drift
(59/59), rustdoc broken-link check and support-slot verification passed. The frame-size guard now
reflects the 48-byte reduction in both builds.
The isolated `./check.sh --fast` rerun passed 1,159/1,159 without features, 1,468/1,468 with
multithreading and 1,485/1,485 with multithreading plus validation-all; 58, 7 and 6 other tests
were skipped, respectively.

### Slice 4: single-threaded monitor owners and finalizer identity

Owner tracking in `sync/single_threaded.rs`, the logical thread key, and a finalizer `Thread`.
Differential fixtures (single-threaded and multithreaded, each matching `dotnet`):

- a finalizer reports a `ManagedThreadId` and `Thread.CurrentThread.IsBackground` different from
  Main's;
- a finalizer's `[ThreadStatic]` value is distinct from Main's;
- `Monitor.Exit` on a lock the current thread doesn't own throws `SynchronizationLockException`.

**Initial evidence (2026-11-17; before joint acceptance):** Owner-aware ST monitors, the separate logical
owner and rooted finalizer `Thread` are active; allocation identity and pointer encoding are
unchanged. Focused sync/statics/finalizer/native tests passed 131/131 ST and 192/192 MT; focused
CLI finalizer/monitor tests passed 27/27 ST and 51/51 MT. All three identity, thread-static and
wrong-owner differential fixtures matched CLR in each build, with exit 42 verified by the enabled
CLI tests. Pinned Miri finalizer-home, finalizer-wait and filter-segment singletons passed 1/1 in
both builds. The finalizer-exception and finalizer-self-wait Miri singletons also passed 1/1 in
each build. Separately, full workspace MT and MT+validation-all nextest runs passed 1,472/1,472
and 1,489/1,489 respectively (7 and 6 skipped). Both clippy legs, support slots, feature-matrix
listing, doc drift and broken-link rustdoc check passed. Skipped tests are not counted.

`./check.sh --fast` fails in the ST `conditional_weak_table_growth` differential test. A full ST
`nextest --no-fail-fast` ran 1,167 tests: 1,165 passed and the two enabled versions of that fixture
failed (58 skipped). The CLR and the unmodified baseline both exit 42. Instrumentation showed an
automatically scheduled `ConditionalWeakTable.Container.Finalize` trying to enter a monitor held
by the parked `ConditionalWeakTable.Add` caller. Correct ownership reports the required deadlock;
allowing reentry breaks this slice, while deferring automatic finalizers is explicitly assigned to
slice 5 and a single-threaded scheduler is excluded. The maintainer authorized implementing and
accepting slices 4 and 5 together to resolve this dependency; neither is accepted until their
combined behavior and required checks pass.

### Slice 5: single-threaded finalizer deferral

Gate `process_pending_finalizers` on the interrupted context's owned monitors and initializer claims.
Fixtures:

- an object becomes unreachable while Main holds lock `L` and allocates; the finalizer (which takes `L`)
  runs only after Main releases `L`, matching `dotnet`'s output order;
- the same inside a `.cctor`;
- `WaitForPendingFinalizers` while holding `L`, with a finalizer that takes `L`, gives the "would
  deadlock" error. This one is VM-only: `dotnet` hangs, so the diff harness can't run it.

**Joint slices 4–5 acceptance (2026-11-16):** Automatic ST finalizer preparation now consults
the authoritative monitor and initializer ownership tables before touching the pending queue.
The executor retries after ownership ends; an actual pending `WaitForPendingFinalizers` forces
execution. A finalizer's self-wait returns immediately. Tests cover recursive monitor release,
initializer claims and both initializer fast paths, forced monitor deadlock, and segment restoration.
The VM-only deadlock source is outside generated success fixtures and the differential script
rejects its marker; the script guard has a regression test.

The focused no-feature VM sync/statics/finalizer/native suite passed 131/131 before the additional
monitor-retry test; the full no-feature fast suite included that test and passed 1,173/1,173.
The focused MT VM suite passed 193/193 and the MT CLI finalizer/monitor/growth suite passed 57/57,
both under timeout. The focused ST CLI deferral, wrong-owner and growth tests passed 9/9. The
three identity/TLS/ownership fixtures and two deferral fixtures each matched CLR stdout and exit
in both explicitly rebuilt base binaries. The isolated fast gate passed 1,173/1,173 ST, 1,476/1,476
MT and 1,493/1,493 MT plus validation-all (58, 7 and 6 others skipped); it includes both clippy
legs. With the pinned nightly and CI Miri flags, finalizer-home, finalizer-wait and finalizer-self-wait
singletons each passed 1/1 in both builds, and the new ST monitor-retry and initializer-owner tests
each passed 1/1, with none ignored. The VM-only script guard, doc drift (59/59), rustfmt and
broken-intra-doc-link rustdoc check passed; rustdoc retained an existing private-link warning.

### Slice 6: release thread statics at thread exit

Remove the completing thread's entries, add the owner-only creation rule and the exited-thread lookup
error, and rewrite `F6.ThreadStaticRoot`. Fixtures matching `dotnet` under `multithreading`:

- a worker stores a finalizable object only in a `[ThreadStatic]` field and exits; after `Join`,
  `GC.Collect` and `WaitForPendingFinalizers`, the finalizer has run;
- `ThreadLocal<T>` untracked values of an exited thread are released, while `Values` with
  `trackAllValues: true` retains values until disposal and any snapshots are released, as on CLR.

`retired_arena_static_survives_join_and_collection` (ordinary statics) still passes. Pinned Miri
shard: the thread-static GC fixture. This slice applies to single-threaded builds only through the
entry thread, which never exits before shutdown, so nothing changes there.

**Evidence (2026-11-17):** Managed completion closes its logical owner's storage under the registry
publication lock; a drop guard covers unwind. Owner-only creation, nonowner reads, close/creation
races, post-exit resolver errors, error/unwind cleanup and completion ordering have unit coverage.
The MT fixture selection ran 9/9, including the ordinary-static retirement regression; MT
workers/teardown/statics/thread-static VM selection ran 44/44 and ST statics/API selection 4/4.
The separately built MT binary matched CLR for the two new lifetime fixtures and the existing
thread-static and reflection fixtures. Worker fixtures are MT-only. Full VM libraries passed 265/265
ST and 396/396 MT, with five skipped in each. The fast gate passed 1,174/1,174 ST, 1,485/1,485 MT
and 1,502/1,502 MT+validation-all (60, 7 and 6 skipped). Both all-target clippy legs passed.

The pinned MT `thread_static_gc_on_worker_exit` ran 1 passed, 0 failed, 0 ignored with the CI flags.
It uses the production indirect static-write path, collection and finalizer worker, and checks live
and exited ownership. The existing pinned static-store-abandonment singleton passed 1/0/0 in both
builds; the MT owner/close resolver singleton also passed 1/0/0 under Miri. The CI include row and
FOREIGN manifest guard are enabled. Manifest and
advisory script tests passed 4/4 and 6/6; slot verification, doc drift and broken-link rustdoc passed
(the existing private rustdoc-link warning remains). The initial fast run caught an older raw-memory
test assuming nonowner creation; it was updated to create entries as their owners and rerun in both
builds. A first diagnostic Miri run using the unchanged atomic field-store path instead exposed a
pre-existing pointer-provenance failure there; the pinned regression does not claim to validate that
separate store path. See the agent memory for the follow-up.

### Slice 7: canonical objects in a long-lived arena

The approved option 1 homes newly materialized literals, empty strings and shared reflection objects
in the long-lived finalizer arena. Explicit `String.Intern(existingObject)` keeps the winning input
in its original, retained live-object arena. Copying at retirement cannot preserve identity for
escaped references, weak handles or CWT keys; weak canonical tables likewise cannot preserve identity.
All string and shared reflection funnels are active. The existing MT type fixture
now also checks a worker-first `Type` through a weak reference and `ConditionalWeakTable` after
releasing its ordinary strong roots and collecting repeatedly. Before homing,
prebuilt-fixture nextest passed 1/1 ST interning and 4/4 MT interning/type/member selections; explicit
ST interning and MT type/member/interning differential runs matched CLR. The MT interning fixture
also keeps a weak reference to the actual winning input and verifies it still identifies the canonical
object after other references are cleared and collections run. Its fixture and differential nextest
cases passed 2/2, and a feature-explicit MT differential run matched CLR. This checks the approved
identity exception, not allocation homing. **Step 7.1 (active string service) is implemented**
(2026-11-17): `threading/canonical.rs` holds the MT-only queue (one mutex, per-request completion,
interest-counted cancellation, atomic close that resolves every request with shutdown). The finalizer
executor installs the empty string, then opens admission, and services a bounded batch inside its own
`mutate_root` on every idle turn and every `run_inner` iteration (so also while a guest finalizer runs or
is parked); it wakes on a condvar and closes admission before retiring its arena. A non-owner `ldstr`
miss parks via `NativeEntry::Canonical` and retries the instruction with IP and stack unchanged; the
owner and executors with no service allocate directly. Explicit `Intern` is untouched. Deviations from
the Design text: (1) the empty key is installed at service start, so `String.Empty` and all empty
constructors/operations never suspend and need no continuation (5 constructors + 2 operations read the
prebuilt object via `push_empty_string`); (2) canonical waits reuse `NativeEffect::Initializer` /
`PendingWait::Initializer` through `NativeInitWait` instead of a new `NativeEffect::Canonical`, so the
generation validation is shared; a GC stop releases the wait and the future re-probes. Evidence: `canonical_homing`
lists 8 MT tests (owner ArenaId, cancel before/after publication, simultaneous misses, shutdown,
terminal failure) and 0 ST tests (service is MT-only); fixture `canonical_strings_mt_42` plus the existing
interning/reflection/canonical selections pass in ST (128) and MT (142); `diff_run.sh` matched CLR for
string_interning_42 (ST, MT), string_interning_mt_42 and canonical_strings_mt_42 (MT); clippy `-D warnings`
both legs, `./check.sh --fast` (1512 passed) and doc drift pass. Bench (bench-fat, same settings, Criterion
time vs baseline): ST string 331.5–332.6 ms (329–333), dispatch 373.3–374.5 (373–378), eval_stack 17.2 ns
(16–17), vector_trace 1.262 ns; MT string 315–318 ms (342–353), dispatch 374–378 (371–378), eval_stack
16.9–17.3, vector_trace 1.20–1.30. No time regression; raw process instruction counts differ only
through adaptive iteration counts (ST string 160.4G ±4.6% vs 153–168G; MT string 169.1G vs 131.8G
with ~8% fewer ns/iter, so the count reflects more iterations, not per-op cost). At that point reflection homing and consolidated acceptance remained unfinished. Fixtures:

- a worker interns a literal and materializes a `RuntimeType` first and exits; Main then observes
  the same identity (`ReferenceEquals`, `String.IsInterned`, and a `ConditionalWeakTable` keyed by
  the `Type`) across collections;
- the `string`, `dispatch` and `runtime_primitives` benches show no regression beyond noise in
  single-threaded and multithreaded builds. Report `perf stat` instruction counts as well as times;
  see the README entry on the `string` bench.

#### Assembly increment evidence

The assembly funnel is active: a selector checks the rooted table before committing the receiver;
misses root the receiver in a native activation and request the owned `ResolutionS` from the finalizer
executor. The owner materializes without guest code, publishes the reverse resolution before the
forward entry, and the requester hydrates its local caches from the winner. Failure wakes the waiter
without publishing an object. A closed service now fails an already-waiting miss instead of silently
allocating on retry. ST and cache hits retain synchronous allocation/lookup; explicit `Intern` remains
unchanged. Other reflection funnels were pending at that increment, so it was not full reflection
homing. The reflection work was split into independently activated increments with their own gates;
consolidated acceptance follows all of them.

Fixture build passed. The MT `canonical_homing` selector listed and passed 10 tests, including owner,
shared requests, assembly failure, cancellation and shutdown; it has no ST tests because the service is
MT-only (zero selected is an exclusion, not a pass). The listed string/reflection/canonical CLI
selection passed 128 ST and 144 MT. The new assembly race/Join fixture and member fixture matched CLR
in MT; `dynamic_method_reflection_42` matched in both builds. The assembly fixture is MT-only and
excluded from ST. Both explicit all-target clippy legs passed; `./check.sh --fast` passed 1,174 ST,
1,499 MT and 1,516 MT+validation-all tests (62, 7 and 6 other tests skipped). Slot verification and
matrix enumeration passed. The finalizer-wait/nested-property coverage remains with the other
reflection kinds.

**Assembly increment benchmarks.** Both sides used `cargo bench --profile bench-fat -p
dotnet-benchmarks --bench TARGET --no-default-features --no-run`, with `--features multithreading`
added for MT, separate `end_to_end` and `runtime_primitives` targets, and executable `--list`. Each
case used `perf stat -r 5 -e instructions,cycles -- "$BENCH" CASE --sample-size 10 --warm-up-time 1
--measurement-time 3 --bench`. MT baseline was rebuilt from the unchanged tree with the exact
no-default-features flags; binaries were captured inside `target/bench-fat/deps`. Rows show ranges
covering the five Criterion intervals, process-average instruction/cycle counts, and process elapsed
seconds; G is billions. Logs are in `/tmp/dotnet-canon-before72` and `/tmp/dotnet-canon-after72`.

| Build/case | Criterion before → after | Instructions G before → after | Cycles G before → after | Elapsed s before → after |
|---|---:|---:|---:|---:|
| ST string | 328.53–331.98 → 326.91–330.17 ms | 167.808 → 167.801 | 38.527 → 38.382 | 6.494 → 6.463 |
| ST dispatch | 369.80–373.30 → 371.35–377.48 ms | 76.018 → 76.020 | 34.237 → 34.381 | 5.712 → 5.741 |
| ST eval_stack_push_i32_pop_safe | 15.899–17.492 → 16.713–17.079 ns | 59.109 → 58.827 | 23.171 → 23.450 | 4.182 → 4.219 |
| ST vector_trace_i32 | 1.2600–1.2677 → 1.2573–1.2678 ns | 142.093 → 141.326 | 23.788 → 23.725 | 4.405 → 4.384 |
| MT string | 313.47–319.98 → 314.55–319.76 ms | 169.059 → 169.044 | 37.397 → 37.202 | 6.266 → 6.251 |
| MT dispatch | 372.58–380.66 → 371.75–378.63 ms | 78.716 → 78.709 | 34.871 → 34.652 | 5.768 → 5.746 |
| MT eval_stack_push_i32_pop_safe | 16.697–17.776 → 16.147–17.836 ns | 57.727 → 59.164 | 23.244 → 23.371 | 4.185 → 4.207 |
| MT vector_trace_i32 | 1.2287–1.2414 → 1.2307–1.2427 ns | 144.106 → 143.984 | 23.661 → 23.641 | 4.368 → 4.370 |

As a workload-adjusted process-count proxy, instructions divided by the average reported measured
iterations were, before → after: ST string 16.781G → 16.780G, dispatch 7.602G → 7.602G,
eval_stack 330.59 → 330.49, vector_trace 59.21 → 59.38; MT string 16.906G → 16.904G, dispatch
7.872G → 7.871G, eval_stack 339.57 → 335.39, vector_trace 60.04 → 59.99. These are **not**
per-operation counts: `perf` also counts startup and adaptive warm-up, while Criterion reports only
measurement iterations (10 for end-to-end, roughly 142–184M for eval_stack and 2.4B for vector).
ST string and dispatch were repeated without competing work after an initially contaminated run;
ST string's repeat counted 167.808G at ±0.00%, versus the earlier contaminated 130.936G. Primitive
samples, especially eval_stack, retain calibration variance (MT pre-count ±3.04%). The matched
intervals and adjusted proxies show no confirmed >5% cost; no performance decision is requested.

#### Token and synchronized-entry increment evidence

The type, method and field `ldtoken` instructions now probe the shared table before allocating a
handle or touching the evaluation stack. On a miss the finalizer executor materializes the object;
the native continuation retries the instruction with its stack and instruction pointer unchanged.
Static synchronized entry similarly requests the declaring type before setting the lock or entering
its monitor. Keys own only metadata; cache hits borrow keys and do not create request payloads. The
owner uses fallible host-only materializers, checks required wrapper ancestry before layout, and
asserts the candidate owner before publication; completed winners are rooted by the shared registry.
ST retains the direct path. Existing unconverted reflection producers may still win a race in another
arena: this increment does not claim complete reflection homing. The remaining
increment retains the property/enumeration work, finalizer-wait and nested-failure tests and every
original acceptance gate. This split keeps an independently exercised instruction/entry path and
its own gates without moving the complete-slice acceptance.

Fixture build succeeded. The `canonical_homing` selector listed and ran 13/13 MT tests, including
execution of all three token instructions with service-owner assertions, a shared miss and missing
core/ancestor failure propagation; ST has zero because the service tests are MT-only (not a pass). The string/reflection/canonical selection
listed and passed 128/128 ST and 146/146 MT; the worker fixture is excluded in ST. A first ST run
exposed the missing generated-test exclusion (128 passed, 1 failed); the exclusion was added and
the selection rerun successfully. Feature-explicit CLI differential runs matched CLR for
`dynamic_method_reflection_42` and `string_interning_42` in both builds, and in MT also for
`canonical_token_mt_42`, `canonical_type_objects_42`, `canonical_member_objects_42`,
`canonical_assembly_mt_42`, `canonical_strings_mt_42` and `string_interning_mt_42`. The new fixture
includes worker-first token and synchronized-entry execution, Join, weak/CWT survival and a
simultaneous miss. Both all-target clippy legs passed with `-D warnings`; `./check.sh --fast` passed
1,174 ST, 1,504 MT and 1,521 MT+validation tests (63, 7 and 6 skipped). The initial MT clippy run
failed on a single-element loop in the new test, which was simplified and rerun successfully.
Doc drift passed 59/59; `cargo run -p xtask -- verify-slots` passed 21 slots, feature-matrix
enumeration succeeded, and the broken-intra-doc-link rustdoc check passed with the pre-existing
private-link warning.

**Initial benchmark and stopping-trigger investigation.** Bench-fat `end_to_end` and
`runtime_primitives` were built separately using `cargo bench --profile bench-fat -p dotnet-benchmarks --bench TARGET --no-default-features
--no-run`, repeating with `--features multithreading`; each executable was listed and immediately
measured by `perf stat -r 5 -e instructions,cycles -- "$BENCH" CASE --sample-size 10 --warm-up-time 1
--measurement-time 3 --bench`. The following ranges cover five Criterion confidence intervals;
process-average instructions/cycles are in billions, elapsed in seconds. These are the baseline
and the first implementation, not the optimized result. Logs are in
`/tmp/dotnet-canon-before74` and `/tmp/dotnet-canon-after74-final`.

| Build/case | Criterion before → after | Instructions G before → after | Cycles G before → after | Elapsed s before → after |
|---|---:|---:|---:|---:|
| ST string | 326.13–333.40 → 347.81–359.05 ms | 167.806 → 130.927 | 38.664 → 32.938 | 6.509 → 5.495 |
| ST dispatch | 373.09–379.69 → 373.49–383.74 ms | 76.011 → 76.015 | 34.642 → 34.469 | 5.772 → 5.776 |
| ST eval_stack_push_i32_pop_safe | 16.265–17.180 → 15.892–17.436 ns | 59.321 → 58.199 | 23.201 → 23.029 | 4.185 → 4.157 |
| ST vector_trace_i32 | 1.2571–1.2653 → 1.2615–1.2673 ns | 138.886 → 141.894 | 23.529 → 23.755 | 4.353 → 4.402 |
| MT string | 315.02–321.65 → 315.59–323.86 ms | 169.053 → 169.040 | 37.315 → 37.482 | 6.269 → 6.319 |
| MT dispatch | 372.46–379.19 → 371.64–379.89 ms | 78.707 → 78.711 | 34.748 → 34.725 | 5.761 → 5.755 |
| MT eval_stack_push_i32_pop_safe | 16.238–17.708 → 16.993–17.835 ns | 59.248 → 58.464 | 23.268 → 23.535 | 4.213 → 4.243 |
| MT vector_trace_i32 | 1.2351–1.2432 → 1.2342–1.2508 ns | 143.715 → 143.492 | 23.597 → 23.613 | 4.374 → 4.377 |

The ST string slowdown exceeded 5%; an unchanged-HEAD rebuild and a repeat of the changed build
produced 325.58–331.29 ms versus 344.81–356.31 ms (five intervals each). Process instructions were
160.438G ±4.60% versus 130.933G ±0.00%, affected by adaptive warm-up; ten measurement iterations
were reported in each run. A fixed single Criterion `--test` invocation under the same perf events
reported 19.837G versus 19.835G instructions, 9.179G versus 9.378G cycles and 1.169 versus 1.200 s
including startup. The maintainer was asked at the stopping trigger and directed optimization,
not acceptance of the cost. Keeping the original direct reflection materializer on ST and limiting
the fallible service materializers and reflection request variants to MT restored the preliminary
ST string interval to 324.10–328.92 ms; process instructions were 167.797G, cycles 38.307G and
elapsed 6.446 s. Its single-invocation count was 19.827G instructions, 8.993G cycles and 1.178 s.
The repeat and preliminary optimization logs are in `/tmp/dotnet-canon-repeat74` and
`/tmp/dotnet-canon-opt74`; the first regression is not treated as an accepted trade-off.

**Optimized comparison.** The complete protocol was then repeated against the same pre-change
baseline, with logs under `/tmp/dotnet-canon-after74-owner`. Events are user-mode instructions and
cycles as reported by this host; the ranges again cover all five Criterion intervals.

| Build/case | Criterion before → optimized | Instructions G before → optimized | Cycles G before → optimized | Elapsed s before → optimized |
|---|---:|---:|---:|---:|
| ST string | 326.13–333.40 → 327.36–333.04 ms | 167.806 → 167.819 | 38.664 → 38.967 | 6.509 → 6.560 |
| ST dispatch | 373.09–379.69 → 370.82–376.27 ms | 76.011 → 76.019 | 34.642 → 34.638 | 5.772 → 5.777 |
| ST eval_stack_push_i32_pop_safe | 16.265–17.180 → 15.875–17.183 ns | 59.321 → 59.073 | 23.201 → 23.080 | 4.185 → 4.174 |
| ST vector_trace_i32 | 1.2571–1.2653 → 1.2596–1.2714 ns | 138.886 → 141.745 | 23.529 → 23.721 | 4.353 → 4.397 |
| MT string | 315.02–321.65 → 326.34–338.29 ms | 169.053 → 161.617 | 37.315 → 37.183 | 6.269 → 6.254 |
| MT dispatch | 372.46–379.19 → 370.27–379.70 ms | 78.707 → 78.702 | 34.748 → 35.002 | 5.761 → 5.797 |
| MT eval_stack_push_i32_pop_safe | 16.238–17.708 → 16.466–17.306 ns | 59.248 → 59.058 | 23.268 → 23.186 | 4.213 → 4.197 |
| MT vector_trace_i32 | 1.2351–1.2432 → 1.2058–1.2129 ns | 143.715 → 145.984 | 23.597 → 23.379 | 4.374 → 4.337 |

Instructions divided by average reported measurement iterations are a workload-adjusted proxy,
not true per-operation counts because process totals include setup and adaptive warm-up. Before →
optimized they are: ST string 16.781G → 16.782G, dispatch 7.601G → 7.602G, eval-stack 329.56 →
330.75, vector 59.86 → 59.06; MT string 16.905G → 16.162G, dispatch 7.871G → 7.870G,
eval-stack 335.87 → 336.32 and vector 59.88 → 58.39. End-to-end cases report ten measurement
iterations; eval-stack reported 169–187 million, vector 2.0–2.5 billion. The MT string count varied
±4.60% because warm-up changed; the observed time difference is below 5% on the mean but is noisy.
An unchanged-HEAD and optimized MT repeat under the same commands yielded 315.76–325.62 versus
327.99–341.07 ms (five Criterion intervals), or about 4% on the average point estimates. Process
counts were 169.057G ±0.00% versus 154.198G ±5.91% instructions, 38.019G versus 36.620G cycles and
6.376 versus 6.099 s; startup and warm-up differ. The fixed single `--test` comparison yielded
19.852G versus 19.859G instructions, 9.510G versus 9.933G cycles and 1.204 versus 1.260 s
including startup (after elapsed ±2.00%). These results do not confirm a >5% cost; the MT
loop-placement and calibration variability remain a limit, not a waiver of the trigger. Repeat
logs are in `/tmp/dotnet-canon-repeat74-mt`.

**After failure-path hardening.** The full protocol was repeated after adding fallible ancestry
checks, using `/tmp/dotnet-canon-after74-last`. The initial MT string result exceeded the stopping
threshold and was repeated with unchanged and changed binaries.

| Build/case | Criterion before → hardened | Instructions G before → hardened | Cycles G before → hardened | Elapsed s before → hardened |
|---|---:|---:|---:|---:|
| ST string | 326.13–333.40 → 323.02–334.14 ms | 167.806 → 145.688 | 38.664 → 34.560 | 6.509 → 5.760 |
| ST dispatch | 373.09–379.69 → 373.12–379.60 ms | 76.011 → 76.034 | 34.642 → 34.819 | 5.772 → 5.805 |
| ST eval_stack_push_i32_pop_safe | 16.265–17.180 → 17.015–17.508 ns | 59.321 → 59.104 | 23.201 → 23.572 | 4.185 → 4.283 |
| ST vector_trace_i32 | 1.2571–1.2653 → 1.2608–1.2775 ns | 138.886 → 141.585 | 23.529 → 23.667 | 4.353 → 4.405 |
| MT string | 315.02–321.65 → 351.40–354.26 ms | 169.053 → 131.882 | 37.315 → 32.953 | 6.269 → 5.476 |
| MT dispatch | 372.46–379.19 → 378.06–386.94 ms | 78.707 → 78.718 | 34.748 → 35.207 | 5.761 → 5.845 |
| MT eval_stack_push_i32_pop_safe | 16.238–17.708 → 16.495–18.051 ns | 59.248 → 58.204 | 23.268 → 22.974 | 4.213 → 4.191 |
| MT vector_trace_i32 | 1.2351–1.2432 → 1.2154–1.3065 ns | 143.715 → 143.533 | 23.597 → 23.438 | 4.374 → 4.369 |

Workload-adjusted process-count proxies before → hardened (not per-operation because of startup and
adaptive warm-up): ST string 16.781G → 14.569G, dispatch 7.601G → 7.603G, eval-stack 329.56 →
330.56, vector 59.86 → 58.99; MT string 16.905G → 13.188G, dispatch 7.871G → 7.872G,
eval-stack 335.87 → 338.40, vector 59.88 → 58.83. The sharp string count difference reflects
warm-up calibration, not fewer measured iterations (ten each); it does not waive the time trigger.

The MT repeat confirmed a slowdown: 318.96–323.26 ms before versus 351.26–355.59 ms after.
Process totals were 169.072G versus 131.881G instructions, 38.211G versus 33.578G cycles, and
6.410 versus 5.547 s; both report ten measurement iterations but different adaptive warm-up.
Fixed single-invocation `--test` counts were 19.853G versus 19.855G instructions and 9.656G
versus 9.868G cycles, with 1.215 versus 1.247 s including startup. The maintainer was asked and
authorized further optimization rather than acceptance. Marking the fallible ancestry validation
as cold and non-inlined preserves its checks while isolating it from hot code: the preliminary
MT string run was 320.46–325.89 ms, 169.066G instructions, 38.109G cycles and 6.392 s; its
single-invocation run was 19.846G instructions, 9.298G cycles and 1.173 s. Full matching benches
are repeated on this implementation. Logs: `/tmp/dotnet-canon-repeat74-final` and
`/tmp/dotnet-canon-mt-opt1`.

**Final comparison.** With the cold ancestry check, both build sets and all four cases were
remeasured with the same commands and settings; logs are in `/tmp/dotnet-canon-after74-cold`.
Ranges cover five Criterion intervals and counters are per-process averages in billions.

| Build/case | Criterion before → final | Instructions G before → final | Cycles G before → final | Elapsed s before → final |
|---|---:|---:|---:|---:|
| ST string | 326.13–333.40 → 328.15–341.52 ms | 167.806 → 130.935 | 38.664 → 31.725 | 6.509 → 5.292 |
| ST dispatch | 373.09–379.69 → 373.78–381.18 ms | 76.011 → 76.040 | 34.642 → 35.005 | 5.772 → 5.868 |
| ST eval_stack_push_i32_pop_safe | 16.265–17.180 → 16.076–17.540 ns | 59.321 → 58.244 | 23.201 → 22.854 | 4.185 → 4.163 |
| ST vector_trace_i32 | 1.2571–1.2653 → 1.2625–1.2894 ns | 138.886 → 141.650 | 23.529 → 23.728 | 4.353 → 4.417 |
| MT string | 315.02–321.65 → 321.92–329.95 ms | 169.053 → 169.060 | 37.315 → 38.236 | 6.269 → 6.442 |
| MT dispatch | 372.46–379.19 → 374.03–381.39 ms | 78.707 → 78.733 | 34.748 → 34.886 | 5.761 → 5.822 |
| MT eval_stack_push_i32_pop_safe | 16.238–17.708 → 16.864–17.933 ns | 59.248 → 58.653 | 23.268 → 23.488 | 4.213 → 4.254 |
| MT vector_trace_i32 | 1.2351–1.2432 → 1.2073–1.2120 ns | 143.715 → 145.925 | 23.597 → 23.352 | 4.374 → 4.338 |

Instructions divided by mean reported measurement iterations, a process-count proxy including
startup and adaptive warm-up rather than per-operation counts, were before → final: ST string
16.781G → 13.093G, dispatch 7.601G → 7.604G, eval-stack 329.56 → 332.82, vector 59.86 → 59.02;
MT string 16.905G → 16.906G, dispatch 7.871G → 7.873G, eval-stack 335.87 → 337.09, vector
59.88 → 58.37. Both string and dispatch measured ten iterations; primitives used 170–185 million
and 2.0–2.5 billion iterations respectively. ST string's large raw count change is warm-up
calibration; the fixed-run and prior repeated comparisons above show the limitation. The MT string
preliminary optimized and full runs are within 5% of the matched baselines; no confirmed >5%
regression remains. Earlier triggered regressions and maintainer direction are retained above.

#### Field type accessor increment evidence

The remaining reflection work spans many producers and rooted enumerations, so it is split again:
`FieldInfo` type accessors form a separately exercised increment; all other producers, nested
properties, enumerations and the full original acceptance remain in the following increment. `GetFieldType` and `GetDeclaringType` select the
native path before consuming the receiver on a service miss, root the receiver over the wait,
and hydrate the shared type winner before pushing it. Hits and ST use the synchronous path.
The executor regression checks service ownership for worker-first `FieldType` and `DeclaringType`
misses; the fixture races two workers, joins them and checks identity after collections. Its generic
field also exposes the pre-existing constructed-generic `DeclaringType` mismatch recorded in the
plans README: a CLR differential probe fails identically on unchanged HEAD and this increment.
Other reflection producers are still capable of winning a race outside the owner, so this is not
complete homing.

Fixture build passed. The listed `canonical_homing` selector ran 14/14 MT tests under 300 s; ST
selects zero (service MT-only; not a pass). The listed CLI string/reflection/canonical selection
passed 128/128 ST and 147/147 MT, with the worker fixture explicitly excluded in ST. Fresh
feature-explicit CLI binaries matched CLR for `dynamic_method_reflection_42` and
`string_interning_42` in both builds, and in MT for `canonical_field_access_mt_42`,
`canonical_token_mt_42`, `canonical_type_objects_42`, `canonical_member_objects_42`,
`canonical_assembly_mt_42`, `canonical_strings_mt_42` and `string_interning_mt_42`. Both explicit
all-target clippy legs passed with `-D warnings`; `./check.sh --fast` passed 1,174 ST, 1,506 MT
and 1,523 MT+validation tests (64, 7 and 6 skipped). The slot verifier passed 21 slots and feature
matrix enumeration succeeded. Doc drift passed 59/59 and the broken-intra-doc-link rustdoc check
passed with the existing private-link warning.

The before/after `bench-fat` binaries were built with the Design commands for each target and
feature set, listed and measured using `perf stat -r 5 -e instructions,cycles` and Criterion's
10 samples, 1 s warm-up and 3 s measurement. Each time range spans the five confidence
intervals; counters are per-process averages in billions and elapsed is seconds. Logs are
`/tmp/dotnet-canon-before75` and `/tmp/dotnet-canon-after75`.

| Build/case | Criterion before → after | Instructions G | Cycles G | Elapsed s |
|---|---:|---:|---:|---:|
| ST string | 330.97–338.47 → 329.96–333.74 ms | 138.311 → 153.048 | 33.171 → 35.699 | 5.519 → 6.009 |
| ST dispatch | 375.63–379.40 → 371.70–378.39 ms | 76.024 → 76.017 | 34.816 → 34.392 | 5.814 → 5.758 |
| ST eval_stack_push_i32_pop_safe | 16.403–17.414 → 16.041–17.439 ns | 59.028 → 57.850 | 23.288 → 22.778 | 4.245 → 4.156 |
| ST vector_trace_i32 | 1.2634–1.2738 → 1.2657–1.3401 ns | 141.332 → 141.275 | 23.634 → 23.787 | 4.404 → 4.434 |
| MT string | 321.17–325.30 → 316.91–326.53 ms | 169.055 → 169.071 | 37.892 → 37.685 | 6.375 → 6.359 |
| MT dispatch | 374.76–379.07 → 375.33–384.27 ms | 78.715 → 78.712 | 34.669 → 34.904 | 5.774 → 5.807 |
| MT eval_stack_push_i32_pop_safe | 16.465–17.586 → 16.579–17.739 ns | 58.854 → 58.325 | 22.954 → 23.118 | 4.153 → 4.202 |
| MT vector_trace_i32 | 1.2079–1.2207 → 1.2100–1.2195 ns | 145.929 → 145.763 | 23.363 → 23.332 | 4.344 → 4.346 |

Dividing process instructions by mean reported measurement iterations yields a workload proxy,
not true per-operation counts because startup and adaptive warm-up are included. Before → after:
ST string 13.831G → 15.305G, dispatch 7.602G → 7.602G, eval-stack 330.50 → 332.86 and vector
59.38 → 59.36; MT string 16.906G → 16.907G, dispatch 7.872G → 7.871G, eval-stack 336.31 →
338.31 and vector 58.37 → 58.31. End-to-end cases each measured ten iterations; eval-stack used
166–183 million and vector 2.3–2.5 billion. ST string's raw process counts varied ±5.33% and
±5.90% with adaptive warm-up. Criterion point-estimate means changed by at most +2.51%; the one
ST vector outlier does not establish a >5% mean regression. To investigate ST string calibration,
`perf stat -r 5 -e instructions,cycles -- "$BENCH" string --test` on freshly rebuilt unchanged and
changed binaries gave 19.838G → 19.840G instructions, 9.149G → 9.438G cycles and 1.194 → 1.204 s
including startup. The fixed invocation confirms the process-count difference is warm-up variance;
no confirmed >5% cost was observed. Fixed-run logs are `/tmp/canon75-fixed-before.log` and
`/tmp/canon75-fixed-after.log`.

**Step 7.6 (remaining reflection callers) is implemented** (2026-11-17). Every shared type, method,
constructor, field, property and assembly producer reachable from a non-owner executor now selects the
canonical service before committing operands (`await_canonical_object` / `await_reflection_object`),
including nested property dependencies, enumeration/array producers, delegate method objects, dynamic
method creation and `Object.GetType`; hits, ST and the owner remain synchronous. A temporary assertion
in the synchronous reflection allocation funnel (worker with an active service) did not fire across the
full MT integration suite, confirming no exercised caller still publishes a worker-owned winner. The
assertion was removed. A constrained value-type `GetType` call now selects the constraint's Type object
before boxing the receiver, because a retry re-executes the instruction and the boxed stack would
otherwise fail (`fields_enum_frame_homes_42`). New `canonical_reflection_mt_42.cs` (type, method,
constructor, field, property, enumerated property, assembly, delegate and dynamic-method identity with
weak/CWT survival) and `canonical_join_race_mt_42.cs` (barrier simultaneous misses with Main and a
finalizer joining workers) match CLR; `canonical_homing` gained reverse-cache, nested-failure and
finalizer-wait/Join-race executor tests. Verification: fixtures build; `canonical_homing` 17/17 MT (ST
lists zero, MT-only service); prebuilt string/reflection/canonical CLI selections 128/128 ST, 151/151
MT; feature-explicit `diff_run.sh` matched CLR for dynamic_method_reflection and string_interning in
both builds and for canonical_member_objects, canonical_type_objects, canonical_{assembly,field_access,
token,strings,reflection,join_race}_mt and string_interning_mt in MT (the two worker-thread
member/type fixtures are MT-only); both clippy `-D warnings` legs, `./check.sh --fast` (1,530 passed),
doc drift 59/59, rustdoc links and slot verification passed. Benchmark (bench-fat, sample 10, 1 s/3 s,
`perf stat -r 5`; baseline built from a clean HEAD worktree, interleaved runs), before → after,
instructions G / cycles G: ST string 169.00 → 160.43 / 39.06 → 36.98; ST dispatch 77.21 → 76.02 /
34.76 → 34.18; ST eval_stack 59.43 → 58.99 / 23.10 → 22.85; ST vector 141.96 → 142.10 / 23.77 → 23.78;
MT string 169.04 → 169.04 / 37.09 → 37.61; MT dispatch 78.70 → 78.70 / 34.44 → 34.65; MT eval_stack
59.09 → 59.50 / 23.17 → 23.41; MT vector 142.34 → 139.56 / 23.49 → 24.01. Criterion MT string
313–318 → 319–323 ms (+1.9%), MT dispatch 370–373 → 372–378 ms, MT vector 1.2045 → 1.2943 ns (the
same code-placement mode as the recorded pre-change baseline 1.2938–1.3016 ns, with fewer
instructions). No confirmed >5% cost. Logs are in `/tmp/canon76`. Reflection allocation outside the
service for parameter, attribute, array and handle objects is unchanged by design; consolidated
acceptance (7.3) remains.

#### Consolidated Slice 7 acceptance (2026-11-17)

Option 1 and the explicit-Intern exception above are accepted for this slice. The finalizer owner
homes newly materialized canonical strings and reflection objects; it does not copy a winning
`Intern(existingObject)` input, whose original arena remains a retained live-object arena. Copying
at retirement cannot rewrite escaped identities, and weak tables would lose canonical identity.
No ordinary-string allocation redesign or live-object arena release is included. The allocation
funnel audit and affected tracing/lookup SAFETY explanations are closed below and in code; overall
Status remains In progress for later slices.

Fixture build (`cargo run -p xtask -- fixtures build`) passed. Both selected suites were listed:
`canonical_homing` has 17 enabled MT tests, all passed under `timeout 300s`; ST lists zero because
the service is MT-only (exclusion, not a pass). With prebuilt fixtures, the listed
`test(string_interning) | test(reflection) | test(canonical_)` CLI selection passed 128 ST and
151 MT, MT under `timeout 300s`. Full `cargo nextest run -p dotnet-vm --lib --no-default-features`
passed 265/265 ST; the explicitly MT-enabled command under `timeout 600s` passed 413/413 (5 other
tests skipped in each). Explicit ST/MT CLI builds followed by `DOTNET_RS_BIN="$PWD/target/debug/dotnet-rs"
timeout 180s bash scripts/diff_run.sh SOURCE` matched CLR for `string_interning_42` and
`dynamic_method_reflection_42` in both builds, and in MT for `string_interning_mt_42`,
`canonical_{strings,assembly,token,field_access,reflection,join_race}_mt_42` and
`canonical_{type,member}_objects_42`. The worker-thread fixtures are excluded in ST.

Both explicit `cargo clippy --all-targets --no-default-features [--features multithreading] --
-D warnings` legs passed; brackets denote the optional MT argument. `./check.sh --fast` passed
1,174 ST, 1,513 MT and 1,530 MT+validation tests (66/7/6 other tests skipped). The standalone
MT reflection crate check, `cargo run -p xtask -- verify-slots` (21 slots),
`cargo run -p xtask -- matrix test-features`, `bash scripts/check_doc_drift.sh` (59/59) and
`DOTNET_SKIP_BUILD=1 RUSTDOCFLAGS="-D rustdoc::broken_intra_doc_links" cargo doc --no-deps
--no-default-features` passed. Rustdoc emitted its existing private-item-link warning. An initial
harness accidentally supplied empty feature arguments and the CLI build failed; those results were
excluded and all feature-explicit lists, suites, builds and differential runs were repeated above.
Loom, Miri and the two project host rungs remain part of later plan acceptance, not claimed here.

**Whole-slice before/after performance.** Before is the pre-service tree `f8bf8e55`; after is
`8338adc0` plus comment-only safety edits. Separate target directories kept the ST and MT
executables distinct. Both targets were built and listed in each tree using
`cargo bench --profile bench-fat -p dotnet-benchmarks --bench TARGET --no-default-features --no-run`,
repeated with `--features multithreading`; `TARGET` was `end_to_end` and `runtime_primitives`.
For each listed binary/case, interleaved before/after on the same host used exactly
`perf stat -r 5 -e instructions,cycles -- "$BENCH" CASE --sample-size 10 --warm-up-time 1
--measurement-time 3 --bench`. No other benchmark or test ran during these measurements. The
Criterion ranges span all five confidence intervals; instruction and elapsed percentages are
perf's five-run variation. Cycles are process-average counts. The final column divides the
process-average instructions by the mean reported measurement iteration count; it is a
**workload-normalized proxy**, not true per-operation instructions, because the numerator includes
startup and adaptive warm-up. Criterion's rounded iteration counts also limit precision.

| Build / case | Criterion range before → after | Instructions before → after | Cycles before → after | Elapsed s before → after | Instructions / measurement iteration proxy |
|---|---:|---:|---:|---:|---:|
| ST string | 321.74–331.26 → 327.27–333.91 ms | 169,026,858,178 ±0.72% → 169,302,236,111 ±0.88% | 38,980,239,086 → 39,319,146,908 | 6.613 ±2.05% → 6.683 ±2.75% | 1.69e+10 → 1.693e+10 |
| ST dispatch | 370.07–376.79 → 369.31–374.96 ms | 77,239,849,392 ±1.59% → 77,407,218,010 ±1.78% | 34,952,286,933 → 35,021,557,414 | 5.870 ±2.53% → 5.877 ±2.51% | 7.724e+09 → 7.741e+09 |
| ST eval_stack_push_i32_pop_safe | 15.835–17.231 → 15.837–17.192 ns | 58,940,083,110 ±0.99% → 58,900,875,995 ±0.72% | 23,290,994,475 → 22,958,394,630 | 4.198 ±1.28% → 4.135 ±1.36% | 330.4 → 330.9 |
| ST vector_trace_i32 | 1.2439–1.2695 → 1.2617–1.2774 ns | 142,070,288,478 ±0.13% → 142,080,547,041 ±0.05% | 23,716,453,933 → 23,804,804,536 | 4.391 ±0.08% → 4.409 ±0.11% | 59.2 → 59.2 |
| MT string | 345.49–354.46 → 320.12–323.57 ms | 131,826,190,480 ±0.00% → 169,043,484,666 ±0.00% | 32,615,649,381 → 37,700,364,037 | 5.430 ±0.15% → 6.345 ±0.12% | 1.318e+10 → 1.69e+10 |
| MT dispatch | 368.73–374.71 → 379.12–384.11 ms | 78,619,700,426 ±0.00% → 78,703,188,630 ±0.01% | 34,461,046,148 → 35,178,813,476 | 5.701 ±0.08% → 5.836 ±0.10% | 7.862e+09 → 7.87e+09 |
| MT eval_stack_push_i32_pop_safe | 16.429–17.606 → 16.406–17.962 ns | 56,807,787,665 ±3.33% → 58,386,330,640 ±0.83% | 22,652,501,377 → 23,297,314,457 | 4.108 ±1.46% → 4.228 ±0.93% | 341.8 → 337.9 |
| MT vector_trace_i32 | 1.2988–1.3069 → 1.2993–1.3207 ns | 138,801,284,374 ±0.11% → 138,786,148,019 ±0.05% | 23,905,364,416 → 23,939,067,951 | 4.437 ±0.04% → 4.445 ±0.08% | 60.35 → 60.34 |

Reported measurement iteration ranges before → after: ST string: 10–10 → 10–10; ST dispatch: 10–10 → 10–10; ST eval_stack_push_i32_pop_safe: 1.73e+08–1.85e+08 → 1.75e+08–1.84e+08; ST vector_trace_i32: 2.4e+09–2.4e+09 → 2.4e+09–2.4e+09; MT string: 10–10 → 10–10; MT dispatch: 10–10 → 10–10; MT eval_stack_push_i32_pop_safe: 1.36e+08–1.81e+08 → 1.7e+08–1.8e+08; MT vector_trace_i32: 2.3e+09–2.3e+09 → 2.3e+09–2.3e+09.
The MT string raw count and proxy increase despite a faster Criterion result because process-wide
counts include adaptive warm-up; it cannot be attributed to per-operation cost. To control that,
`perf stat -r 5 -e instructions,cycles -- "$BENCH" string --test` on the same binaries ran one
fixed Criterion invocation per process: ST before/after 19,845,731,686 / 19,846,100,705 instructions,
9,457,665,970 / 9,642,266,900 cycles and 1.198 / 1.210 s; MT 19,843,813,800 / 19,847,966,996
instructions, 9,545,659,064 / 9,437,716,338 cycles and 1.205 / 1.179 s. Fixed counts include
startup, but match workload and differ by at most 0.03%; perf variation was at most 0.02% for
instructions and 0.99% for cycles. ST primitive time ranges overlap; MT dispatch's median changed
371.16 → 382.11 ms (about 3%), with comparable normalized instruction proxy. MT vector ranges
overlap. No confirmed >5% cost was found. The MT string code-placement sensitivity documented in
the README and these startup/warm-up effects limit inference from wall time or raw totals alone.
Full build/list/run and fixed logs are retained at `/tmp/canon-accept/` for this run.

#### Slice 7 Design

This is the original implementation specification; the acceptance and deviation evidence above
records the actual implementation. Status remains In progress for later slices. Reflection was
activated in assembly, token/synchronized-entry, field-accessor and remaining-caller increments,
each with its own fast gate, clippy and performance checks, followed by consolidated acceptance.

**Transport.** Add MT-only `CanonicalKey::{String(Vec<u16>), Type(RuntimeType),
Method(MethodDescription, GenericLookup), Field(FieldDescription, GenericLookup),
Property(MethodDescription, GenericLookup), Assembly(ResolutionS)}`. Property uses the existing
accessor/lookup key; do not invent a name-only key. Method covers constructors too. Add an owned
request `{id, key, completion: Arc<...>}` and terminal completion `Result<(), CanonicalError>`;
the response deliberately carries no pointer: completion means retry the rooted authoritative table
lookup. Errors distinguish cancellation, shutdown and materialization failure. Never send worker Gc
handles, borrowed UTF-16, heap storage or native activation references. The bounded service batch
must drop queue locks before materializing. Use one queue mutex and completion event/epoch per
request; no extra synchronization on ST or table hits. Reuse native generation validation, adding
`NativeEffect::Canonical(generation)` and `PendingWait::Canonical(generation, completion)`.

**Progress and lifetime.** Record the dedicated finalizer executor's ArenaId at startup before
accepting requests. In `executor.rs`, service a bounded batch inside its own active `mutate_root`
from the idle loop (881–962) AND each `run_inner` iteration before native wait selection
(982–1165). The latter runs even when a guest finalizer is executing: service is host allocation,
not guest callback dispatch, and must neither enter a constructor nor acquire a guest monitor or
initializer claim. Resolve layouts without running cctors. A request produced by this executor is
materialized directly rather than queued. Do not recursively enter `run_inner`.

The existing wait loop releases mutation/arena access before bounded 1–10 ms sleeps
(executor.rs:1164 onward). Extend that polling path to canonical completions; retain GC safepoints
on each turn. Thus Main parked in Join, a worker parked on a monitor, and a finalizer parked in
Join/event/monitor/initializer waits continue servicing collection; the finalizer also services
canonical requests. Guest finalizers running IL reach the same per-instruction service point.
This fixes allocation-service cycles only; ordinary guest lock/Join cycles retain existing semantics.
Do not hold DashMap guards, heap borrows, mutation access or arena leases across any wait.
Shutdown closes admission atomically with queue ownership, resolves every queued completion with
shutdown, and wakes sleepers before joining the owner; abort uses the same drain. A canceled
activation drops its completion interest, never rolls back an already-published winner. Shutdown
must not destroy the finalizer arena while any executor can read shared tables. Test service startup
before the first guest finalizer and shutdown races explicitly.

**Publication and safety.** Recheck the shared table after dequeuing. Build under finalizer-arena
mutation with temporary objects rooted locally; complete initialization before an entry operation.
The entry winner is authoritative, even if explicit Intern published meanwhile. Publish before
signaling completion. Only explicit Intern may publish a worker-owned input; it keeps exact identity
(stack_ops_impl.rs:113 and interning.rs:16–48). Losing candidates acquire no shared roots. Shared
reflection tables are in state.rs:272–299; local cache roots and shared-table tracing already exist
(state.rs:703 onward). Preserve tracing in every arena and the cross-arena coordinator's active
mutator/collection protocol. Pointer reconstruction must happen only under requester mutation from
a still-rooted shared entry, following reflection_ops_impl.rs:83–106; completion itself is not a GC
root. Assert allocation ArenaId equals service ArenaId before publishing each newly allocated
canonical object; assert the ST local arena instead in ST. Do not assert finalizer ownership for
explicit Intern winners, including when a literal request loses to one.

**Nested reflection.** Extract host-only materializers from common.rs and type_members.rs. Build
property name, accessor MethodInfo and declaring Type via local recursive materialization on the
owner; no self-request, wait or managed constructor. Type/method/field index assignment retains
existing shared forward/reverse index helpers. Install reverse assembly resolution before publishing
its object; install local forward/reverse cache entries using the winning object only. Requester
retries hydrate local caches from shared winners, including reverse assembly resolution. Keep
in-progress temporary objects rooted; a failing parent is never published. Dependencies may be
published independently once complete. Detect recursive same-key construction as an internal error,
rather than publish a partial object. Reflection arrays, parameter objects, handles, attributes,
resource streams and ordinary strings are not shared canonical entries and remain caller allocations.

**Allocation-funnel audit (closed).** The actual implementation uses a preinstalled empty
string and preflights reflection keys before consuming operands, rather than converting every
synchronous helper into a suspendable API. Each retry leaves the original operands rooted and
unchanged; enumeration preflights the complete set before constructing its result. The owner
materializes nested property dependencies directly. The shared forward and reverse entries and
local caches use the published winner.

| Producer | Active route |
|---|---|
| `ldstr` / `push_interned_string` | Copy the miss key, queue the finalizer owner, and retry the instruction; hits are synchronous. |
| `String.Empty`, `push_string_or_empty`, five constructors and two computed-empty operations | The owner installs empty before service admission; consumed-operand paths synchronously push that rooted winner. Without a service, the retained local arena supplies it. |
| `Intern` / `IsInterned` | Synchronous lookup or atomic publication of the existing input; the winning input's arena stays live. |
| `ldtoken`, static synchronized entry, constrained calls, `Object.GetType`, delegate method metadata | Select the required type/method/field before changing stack, frame or monitor state; retry after a miss. |
| Runtime assembly and `FieldInfo` type accessors | Native selections root receivers and await the owner; shared and local forward/reverse caches hydrate from the winner. |
| Reflection type queries/construction, methods, parameters, fields and member enumeration | Preflight owned keys before operand commitment; all result keys are present before enumeration builds arrays. |
| Dynamic method and nested property construction | Native dynamic-method selection or owner-local recursive property materialization; dependencies are published complete before parent publication. |

Closure searched `push_interned_string`, `push_string_or_empty`, `get_runtime_type`,
`get_runtime_method_obj`, `get_runtime_field_obj`, `reflection_cache_runtime_` and the selection
sites across Rust sources. Trait declarations and mocks are not producers. The pre-initialization
helper has no runtime caller. `FastAllocateString`, ordinary strings, arrays, parameters,
attributes and handles are not canonical shared entries. The prior increment's temporary
worker-allocation assertion was exercised across the full MT integration suite; it is coverage,
not proof of unexecuted paths.

**Enabled tests and acceptance mapping.** Existing `string_interning_42`,
`string_interning_mt_42`, `canonical_type_objects_42` and `canonical_member_objects_42` fixtures remain
required; list selectors before running and do not count MT-only fixtures as ST coverage. Add enabled
VM tests under selector `canonical_homing` with direct owner-ArenaId assertions; guest identity
fixtures alone pass before homing and cannot establish success. Add guest sources
`canonical_strings_mt_42.cs` (worker-first literal/Empty, ReferenceEquals, IsInterned, weak/CWT,
all five empty constructors and both computed-empty operations), `canonical_reflection_mt_42.cs`
(type, method, constructor, field, property, assembly identity, nested property dependencies, weak/CWT),
and `canonical_join_race_mt_42.cs` (barrier simultaneous misses with Main joining workers and a
finalizer joining a requesting worker). Use nonliteral UTF-16 for explicit Intern race winner and
assert winning-input ReferenceEquals/weak survival. Add executor tests `canonical_homing_cancel`,
`canonical_homing_shutdown`, `canonical_homing_finalizer_wait`, `canonical_homing_nested_failure`
and `canonical_homing_retry_operands`; test both cancel-before-service and cancel-after-publication,
no partial parents, no duplicate side effects, rooted operands across forced collection. All new VM
tests are enabled, not ignored. Worker fixtures are MT only; exercise local string/reflection APIs
and consumed-operand continuation regressions in ST too. Each MT test run has an external timeout.

The baseline logs were verified present at `/tmp/dotnet-canon-baseline` during this design run.
Benchmark baseline is the table below, not a new acceptance result. Inspect retained executables
under target/bench-fat/deps and logs under /tmp/dotnet-canon-baseline if available; retain Cargo's
executable location (a copy outside deps failed previously). Build end_to_end and runtime_primitives
separately with `cargo bench --profile bench-fat -p dotnet-benchmarks --bench TARGET
--no-default-features --no-run`, repeating with `--features multithreading`. List each executable.
Use `perf stat -r 5 -e instructions,cycles -- "$BENCH" CASE --sample-size 10
--warm-up-time 1 --measurement-time 3 --bench` for string/dispatch and the two named primitive
cases below. Match host/profile/features/settings and report Criterion intervals plus instructions,
cycles and elapsed. Raw process counts include adaptive warm-up/startup; record measured iteration
counts and normalize workload counts, using matched fixed-iteration measurement if needed rather
than dividing unequal process totals blindly. Repeat unchanged baseline and changed binary under
matched conditions when ST startup/warm-up variance obscures attribution. Ask on confirmed >5%
cost; no implementation slice may silently defer that decision to 7.3.


**Pre-change benchmark baseline (2026-11-17):** `bench-fat`, explicit no-default-features (ST)
versus `--features multithreading` (MT); Criterion sample size 10, 1 s warm-up and 3 s measurement;
`perf stat -r 5 -e instructions,cycles`. `runtime_primitives` is a separate Criterion executable,
not a case in `end_to_end`. Both executable lists were checked for the selected cases. The time
ranges below span all five reported Criterion confidence intervals; counts and elapsed times are
per-process averages with `perf`'s reported variability. These are pre-change results only, not
acceptance evidence for the unfinished slice.

| Build | Case | Criterion time range | Instructions | Cycles | Elapsed |
|---|---|---:|---:|---:|---:|
| ST | string | 329.45–333.26 ms | 153,055,712,071 ±5.90% | 35,888,897,060 ±5.01% | 6.001 s ±5.43% |
| ST | dispatch | 372.57–377.66 ms | 76,033,568,404 ±0.01% | 34,700,350,302 ±0.23% | 5.766 s ±0.20% |
| ST | eval_stack_push_i32_pop_safe | 15.932–17.218 ns | 58,475,232,608 ±0.74% | 22,746,817,136 ±1.28% | 4.122 s ±1.32% |
| ST | vector_trace_i32 | 1.2607–1.2679 ns | 141,372,399,163 ±0.23% | 23,651,751,717 ±0.15% | 4.391 s ±0.15% |
| MT | string | 341.73–353.28 ms | 131,842,966,931 ±0.01% | 32,729,634,633 ±0.17% | 5.434 s ±0.16% |
| MT | dispatch | 371.09–377.87 ms | 78,628,344,621 ±0.01% | 34,674,897,951 ±0.16% | 5.743 s ±0.16% |
| MT | eval_stack_push_i32_pop_safe | 16.555–17.252 ns | 58,628,124,473 ±3.09% | 23,096,015,159 ±1.90% | 4.185 s ±1.86% |
| MT | vector_trace_i32 | 1.2938–1.3016 ns | 136,258,164,718 ±2.33% | 23,764,291,279 ±0.98% | 4.394 s ±0.99% |

Criterion adapts iteration counts and `perf` includes startup, warm-up and sampling, so raw process
counts are not per-operation counts; compare with matching settings and normalize for iteration
counts. In particular, ST string's initial process-count variability was 5.90%. A repeat using the
unchanged executable gave 327.25–331.25 ms, 167,794,790,680 instructions ±0.00%, 38,410,242,283
cycles ±0.14% and 6.482 s ±0.23%; the process counts differ despite the same 10 measured iterations
because startup and warm-up are also counted. Repeat and normalize noisy comparisons before judging
the 5% stopping threshold. The initial attempt to run a copied executable outside Cargo's `deps`
directory failed to find its profile and is excluded.

### Slice 8: production `WorkerRegistry` under Loom

Compile `threading/workers.rs` into a Loom model, either as a `#[cfg(all(loom, test))]` module or through
`#[path]` as `loom_shared_tables.rs` does. Model spawn, complete, retire, finalize and shutdown against
a collection exclusion. Add it to the `loom` CI job. Replace any `loom_stw.rs` assertion this model
covers with the production version; keep `loom_stw.rs` for the stop-the-world handoff.

**Implementation and model bounds.** `loom_workers` includes the production registry source and
uses Loom threads, handles, completion locks and notification. The executor passes its abort
predicate into the same registry-locked boundary exercised by the model. Collection exclusion and
retired-owner wake are modeled adapters; gc-arena tracing and mark colors are outside the model.
The suite uses at most three threads, two preemptions, 20,000 branches, no permutation or time
cutoff, and notification-driven waits (Loom has no timeout). The independent STW handoff and lease
models remain in `loom_stw`; only the mirrored abort/finalization assertion is replaced.

**Acceptance evidence.** After listing the enabled tests, `DOTNET_SKIP_BUILD=1 RUSTFLAGS="--cfg loom"
cargo nextest run -p dotnet-vm --test loom_workers --no-default-features --features multithreading`
passed 7/7; `loom_stw --no-default-features` passed 8/8 and `loom_shared_tables --features
multithreading` passed 2/2. The MT VM all-target Loom compile and an additional Loom-target clippy
check passed. Fixture build succeeded; the listed MT `threading::workers::tests` and
`executor_teardown_tests` selection passed 34/34 under `timeout 300s` (384 unrelated tests skipped).
Both all-target clippy `-D warnings` feature legs passed. `./check.sh --fast` passed 1,174/1,513/1,530
(ST/MT/MT+validation-all; 66/7/6 skipped). The cross-arena manifest and Miri-advisory script tests
passed 4/4 and 6/6, slot verification passed 21, and feature-matrix enumeration succeeded.
Doc drift and the broken-intra-doc-link rustdoc check passed (the existing private-link warning
remains). An optional broader Python script sweep found a pre-existing catalog failure in
`test_pointer_provenance.py`: it expects two conditional ignores absent from unchanged HEAD
source; this is recorded in the plans README, not counted as a pass. A concurrent doc-drift
invocation also failed without retained diagnostics; the sequential rerun passed. The release
transition, pinned Miri release selector and final host-rung acceptance remain in later slices.

### Slice 9: release empty retired arenas

**Slice accepted:** Retirement-root cleanup, safe release with ownership/unregister/exact-drain,
live reaping and the native 1,000-worker bound are active. Collection and shutdown races are tested
natively and under production Loom; the release singleton is pinned and passed under Miri. The
releasing guard accompanies release activation, and the original requirements below passed as
recorded in the increment and acceptance evidence. Overall Status remains In progress for the
later documentation and plan acceptance.

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

**Retirement-root cleanup evidence (2026-11-17).** The first enabled increment calls
`Executor::clear_retired_roots` on entering retirement and after retired finalizer execution,
including failed runs. It retires interrupted finalizer segments, cleans native activations and
finished stack state, clears hydrated reflection object caches/current Thread/decode cache, and
prunes completed child records. Active and unstarted child records remain authoritative; shared
statics, GC handles, intern winners, reflection registry and finalization queues are unchanged.
Reflection metadata vectors retain stable indices. No arena is released by this increment, so the
existing retention/exclusion contract remains in force until the release increment narrows it.

The enabled `retired_owner_clears_dead_local_roots` regression uses the actual retirement service
and coordinated collector: weak witnesses for stack/current Thread/decode/reflection/completed-child
roots clear, while active-child and shared strong-handle witnesses survive. It also consumes the
preserved completion outcome. `retired_root_cleanup_reaches_zero_after_sweep` reaches exactly zero
`Metrics::total_gc_count()` after a full sweep, including an independently allocated Gc drop witness;
repeated cleanup/collection also reaches zero. Both listed tests passed. The production-service
regression is pinned in MT `vm-teardown`, the `MT_TEARDOWN` manifest and its exact 1/0/0 guard.
Pinned nightly Miri with CI Tree Borrows/disable-isolation/ignore-leaks flags passed that exact
singleton (1 passed, 0 failed, 0 ignored); permissive integer-pointer warnings remain. The existing
MT `thread_static_gc_on_worker_exit`, `retired_self_held_lease_leaks_storage_at_shutdown` and
`worker_panic_drains_lease_before_shutdown` pinned singletons also passed, each exactly 1/0/0 under
`timeout 1800s` with the same flags.

Fixture build passed. Listed workers/teardown/statics/finalizer VM selections passed 22/22 ST and
62/62 MT (MT under `timeout 300s`); listed prebuilt MT TLS/ThreadLocal/ordinary-static CLI selection
passed 9/9 under the same timeout. A freshly built explicit MT CLI matched CLR using
`DOTNET_RS_BIN` and `timeout 180s bash scripts/diff_run.sh` for `thread_static_gc_42.cs`,
`thread_local_lifetime_42.cs`, `thread_retired_creator_42.cs` and `retired_arena_cycles_42.cs`.
These worker sources are MT-only; ST regressions use the VM selection and fast gate.

Existing listed Loom suites passed production workers 7/7, STW 8/8 and shared tables 2/2 with
`RUSTFLAGS="--cfg loom"`; MT all-target Loom check passed. No release transitions or gc-arena mark
colors are claimed under these models. Both explicit all-target clippy `-D warnings` feature legs
passed. `./check.sh --fast` passed 1,174/1,515/1,532 (66/7/6 skipped). Cross-arena manifest and
Miri-advisory scripts passed 4/4 and 6/6, slots passed 21 and feature-matrix enumeration passed.
The added CI row initially exposed two stale expected row totals in the manifest test; updating
those totals fixed both failures. The optional complete Python script sweep still fails only the
pre-existing `test_pointer_provenance.py::test_catalog_matches_source_and_shards` expectation of two
conditional ignores absent from unchanged source; the test and source match HEAD. Other script
files passed. Doc drift passed 59/59. Rustdoc broken-link checking passed, with the existing
private-link warning.

The complete release protocol, before-shutdown unregister/join, 1,000-worker bound, release race
models, release-specific Miri singleton and narrowing of retention SAFETY statements remain in
CHECKLIST 9.2–9.5 as described above. Original slice-9 acceptance is preserved, not counted as complete.

**Empty-arena release increment (9.2).** The production Sweep command advances an owner-local
completed-sweep boundary. Retirement snapshots it after dead-root cleanup and again after any
owner-side finalizer run, requiring a later sweep before release. The owner checks exact zero total
Gc allocations, no pending finalizers and no retained child records, then rechecks under collection
exclusion. Registry eligibility excludes pool and dedicated-finalizer entries. An armed releasing
guard preserves the stable completion entry and its possibly consumed outcome across
`Retired → Releasing → Released | Leaked`. Finalizer publication precedes storage extraction;
coordinator/thread unregistration occurs under exclusion. Exact registration drain and storage
freeing occur outside collection and lifecycle locks, with no later arena access/allocation.
Unwind and failed/self-held drains record Leaked; interrupted exact unregistration may also retain
its lease registration, as in the existing leak-on-unwind contract. Shutdown joins terminal owners
and never repeats their arena drain. A collector finishing after an owner's safepoint may still
hold exclusion; the owner retries that boundary without depending on a future collection wake.

Four enabled executor regressions cover before-shutdown unregister with a blocking external lease,
weak clearing, collection/lifecycle lock availability during drain, consumed outcomes, successful
release and repeated shutdown; self-held and injected-unwind drains; empty pool/finalizer retention;
and pending work/unstarted-child retention even in a zero-allocation arena. Existing executor
strong-handle/live-child tests and managed ordinary-static/cycle/TLS/finalizer regressions exercise
nonempty retention. The existing in-flight collection/shutdown test accepts either the empty-owner
release sequence or shutdown sequence while requiring the held lease to delay completion.
F1 and the owner-ID trace SAFETY premise describe the activated empty-arena exception.
Live handle reaping, native thousand-worker bounds, new release-race Loom models and the distinct
pinned release singleton/CI registration remain in 9.3–9.5; the whole slice is not yet accepted.

**Release-increment verification.** `cargo run -p xtask -- fixtures build` passed (both the
initial slow sandboxed build and the bounded SDK-cache-access retry completed successfully).
Listed and ran VM lib selection `test(executor_teardown_tests) | test(workers::tests) |
test(thread_static) | test(thread_local) | test(retired) | test(finalizer)`: 22/22 ST and 66/66 MT,
with MT under `timeout 300s`. Listed and ran prebuilt MT CLI integration selection `test(retired) |
test(thread_static) | test(thread_local) | test(finalizer)`: 46/46 under `timeout 300s`, including
ordinary-static/cycle CLR comparisons. Existing production-worker, STW and shared-table Loom suites
passed 7/7, 8/8 and 2/2 using `DOTNET_SKIP_BUILD=1 RUSTFLAGS="--cfg loom"`; MT all-target Loom
compile passed. These are the existing models; release-specific models remain in 9.4.

Pinned nightly-2026-05-27 Miri listed and passed each exact MT singleton
`empty_retired_owner_unregisters_then_drains_before_shutdown`,
`empty_retired_owner_failed_or_unwound_drain_is_leaked`,
`retired_self_held_lease_leaks_storage_at_shutdown` and `worker_panic_drains_lease_before_shutdown`
in `executor::executor_teardown_tests`, with `DOTNET_SKIP_BUILD=1`, CI Tree Borrows/disable-isolation/
ignore-leaks flags and `timeout 1800s`: each reports 1 passed, 0 failed, 0 ignored. Existing
integer-to-pointer/provenance warnings remain; intentional leak paths use the CI leak-ignore flag.
Both explicit all-target clippy `-D warnings` legs passed. `./check.sh --fast` passed 1,174 ST,
1,519 MT and 1,536 MT+validation-all (66/7/6 unrelated skips); slot verification passed 21 within
the gate. Doc drift passed 59/59, broken-intra-doc-link rustdoc passed with the existing private-link
warning, and formatting/diff checks passed. Logs are `/tmp/release-*`.

An initial shutdown regression asserted only ShuttingDown/Drained despite the now-eligible
post-sweep empty release; updated its expected paired transitions and reran successfully. An initial
pre-drain injected-panic assertion wrongly expected exact unregistration despite interruption before
the drain; the corrected test verifies the existing retained-registration leak contract and cleans
its test registration after joining. Clippy initially rejected a nested assertion's displaced unsafe
comment; moved the unsafe read immediately below its proof and reran both legs. No scope/codec/ID
change was required. Only 9.2 is accepted here; reaping, stress/race models, the distinct pinned
release CI singleton and whole-slice acceptance remain in 9.3–9.5.

**Live-reaper increment.** Released owners queue their stable entry only after the worker closure
has finished its arena work. Live executor dispatch, foreground waiting and finalizer-service loops
reap finished native threads: they take handles under the registry lock, exclude self-join, and join
outside locks. Native `is_finished` avoids parking a GC participant on thread-local destructors;
not-yet-finished handles stay queued. Shutdown takes unreaped handles and waits for live-path joins
already in progress. Completion entries and outcomes remain stable, and no arena or registration
protocol changes. Loom lacks `is_finished`; its modeled closure publishes readiness before its
final return and the join remains outside the registry lock.

The separate enabled `empty_retired_arena_releases_before_shutdown` executor regression drives a
real collection, checks the drop witness, exact registration removal, coordinator and thread-manager
counts, live-path native join and retained outcome before shutdown. The Linux native stress repeats
this for 1,000 sequential workers, explicitly collecting each and checking the coordinator registry,
thread manager, leases and the OS task IDs created by the test; it allows transient `/proc` removal
after join but requires at most one and waits for zero. This is insensitive to pre-existing helper
threads. The stress is intentionally excluded from Miri; the small regression is not.

**Reaper-increment verification.** Fixture build passed. Listed then ran timeout-300s MT VM
`test(executor_teardown_tests) | test(workers::tests) | test(thread_static) | test(thread_local) |
test(retired) | test(finalizer)`: 68 passed. Listed then ran prebuilt timeout-300s MT CLI
`test(thread_static) | test(thread_local) | test(retired_arena_static_survives_join_and_collection)`:
9 passed, including ordinary-static survival. Both explicit all-target clippy `-D warnings` feature
legs passed. Existing production-worker Loom suite listed and passed 7/7 under timeout; it does not
yet model the new release/reaper races. `./check.sh --fast` passed 1,174 ST, 1,521 MT and 1,538
MT+validation-all tests (66/7/6 unrelated skips). Doc drift and broken-intra-doc-link rustdoc checks
passed; rustdoc retains its existing private-link warning. An initial OS assertion expected immediate
`/proc` disappearance on join; the test now bounds and waits for kernel task removal and passed.
Logs are `/tmp/reap-final-*`. Only this increment is complete: release/collection/shutdown native and
Loom races, the pinned Miri/CI singleton and complete Slice 9 acceptance remain in 9.4 and 9.5.


**Release-race increment (9.4).** Shutdown previously accounted for live-reaper joins but not
handles claimed by another concurrent shutdown: the second caller could return before the first
finished joining. Both paths now atomically account for claims under the registry lock, join outside
it, and wait for all outstanding claims. The executor race regression holds an exact lease across
release, performs another real coordinated collection while drain is blocked, then verifies two
shutdown callers and a live reaper wait/complete with one join and a consumed stable outcome. A
second native regression repeats the live-reaper/shutdown race after an actual empty-owner sweep
and release. No IDs, token semantics, finalizer eligibility or allocation ownership changed.

The production `loom_workers` model now exercises Retired → Releasing → Released with a modeled
collection-exclusion/registration lock and concurrent shutdown, competing live/shutdown handle
claims, and an unfinished release guard publishing Leaked while a consumed outcome stays consumed.
The production registry, release guard, signals and thread handles run under Loom; collection state
and drain are abstract counters/locks, not gc-arena color or lease implementations. The existing
independent STW and shared-table models remain. As before, the registry models use up to three Loom
threads and preemption bound 2; native `is_finished` has no Loom equivalent.

Listed then ran the MT executor teardown/worker/thread-static/finalizer selection under
`timeout 300s`: 68 passed, 0 failed, including both new races and the 1,000-worker native stress.
Listed then ran Loom workers 10/10, STW 8/8 and shared tables 2/2 with
`DOTNET_SKIP_BUILD=1 RUSTFLAGS="--cfg loom"` and timeouts; the MT all-target Loom compile passed.
Fixture build passed, then the listed prebuilt MT CLI TLS/ordinary-static selection passed 9/9
under `timeout 300s`. Both explicit all-target clippy `-D warnings` legs passed. The fast gate
passed 1,174/1,523/1,540 ST/MT/MT+validation tests (66/7/6 unrelated skips); doc drift passed
59/59 and broken-link rustdoc passed with the existing private-link warning. An initial additional
Loom model for two
independent shutdown callers timed out due to exhaustive busy-yield schedules; it was replaced by
the deterministic native lease-held two-shutdown regression, not counted as a Loom pass. The
small release Miri/CI singleton and complete Slice 9 acceptance remain in 9.5.

**Complete Slice 9 acceptance (2026-11-17).** The MT `vm-teardown` matrix has a unique
`empty-release` shard for `executor::executor_teardown_tests::empty_retired_arena_releases_before_shutdown`;
`MT_TEARDOWN` and the diagnostics runner require exactly 1 passed, 0 failed, 0 ignored. With
nightly-2026-05-27, `MIRIFLAGS="-Zmiri-tree-borrows -Zmiri-disable-isolation -Zmiri-ignore-leaks"`,
`--exact --test-threads=1` and `timeout 1800s`, that singleton passed 1/0/0. The existing MT
`retired_owner_clears_dead_local_roots`, `retired_self_held_lease_leaks_storage_at_shutdown`,
`worker_panic_drains_lease_before_shutdown`, `retired_shutdown_waits_for_inflight_collection_and_lease`,
`ownership::self_held_drain_panic_and_no_diagnostic` and
`executor::native_substring_tests::thread_static_gc_on_worker_exit` pinned singletons also each
passed 1/0/0 with those flags and timeout. Miri emitted the existing permissive integer-pointer
warnings; the 1,000-worker stress is native-only.

After `cargo run -p xtask -- fixtures build`, the listed prebuilt MT VM lib selection
`test(workers) | test(executor_teardown_tests) | test(thread_static) | test(finalizer)` passed
69/69 under `timeout 300s`, including all release/cleanup, failed-drain, finalizer, race and
1,000-worker regressions. The listed prebuilt MT CLI thread-static, ThreadLocal and ordinary-static
survival selection passed 9/9 under the same timeout. Listed Loom workers, STW and shared tables
passed 10/10, 8/8 and 2/2 under timeout with `DOTNET_SKIP_BUILD=1 RUSTFLAGS="--cfg loom"`;
MT all-target Loom compile passed. These bounded models abstract collection/drain, not gc-arena
mark colors, and the native tests exercise real collection and lease drain.

The F1 and affected SAFETY audit checked `serve_retired`, zero/pending/child rechecks,
`ReleasingWorker`, exact registration drain, reaping/shutdown and shared canonical roots against
`F1.RetiredCollectionExclusion`, `F1.ExecutorArenaRetained` and the lookup/trace comments. The
checks and ownership guard support the documented conditions: unregistration is under exclusion,
while waiting for leases and freeing are outside it; live canonical winners keep their arenas
nonempty. Tag-5 still resolves its nonreused registration token. The design summary above was
corrected to distinguish Releasing from successfully drained Released.

Cross-arena manifest and advisory script tests passed 4/4 and 6/6; the initial manifest run caught
stale row and MT-selector totals, which were updated and rerun. Both explicit all-target clippy
`-D warnings` legs passed. `./check.sh --fast` passed 1,174 ST, 1,523 MT and 1,540 MT+validation
(66/7/6 unrelated skips). Slot verification passed 21; doc drift passed 59/59, and rustdoc
broken-link checking passed with the existing private-link warning. Detailed logs: `/tmp/slice95`.
Hosted CI was not dispatched. Slice 10 still owns cross-cutting documentation and final plan gates.

### Slice 10: documentation

Reconcile THREADING_AND_SYNCHRONIZATION.md (lifecycle, owner identity, finalizers, worker resource
cost), GC_AND_MEMORY_SAFETY.md (retired arenas, thread statics and canonical roots),
INVARIANT_REGISTRY.md (`F1.RetiredCollectionExclusion`, `F5.TracesEveryGcRef` and
`F6.ThreadStaticRoot`), and COMPATIBILITY.md (the single-threaded deferral gap). Remove the five
closed plans-README entries and leave the monotonic 65,535 arena-ID cap; retain the separate MT
string code-placement warning. Preserve option 1 for canonical homing, its benchmark results and the
approved `String.Intern(existingObject)` exception: the exact winning input keeps its original live
arena until shutdown. No all-string allocator migration, ID recycling or live-object arena release
is included.

**Acceptance evidence (2026-11-17).** The lifecycle narrative now distinguishes allocation ArenaId
from execution/managed-thread identity, states that ST monitor ownership is checked without host
thread locks, describes a separate stable ST finalizer Thread/owner key, automatic deferral and the
forced-wait deadlock, and explains `[ThreadStatic]` closure before completion publication. Retired
workers release eligible empty arenas and joined OS threads; live-object, pool and finalizer arenas
remain retained as specified. Shared reflection and intern tables identify their long-lived arena
roots and the explicit-Intern live-object exception. The four closed plan-26 README entries were removed, and the retired-worker entry was shrunk to
the unrecycled ID cap; the MT string warning remains. The
stale pointer-diagnostic README follow-up and its test expectation were reconciled: the two numeric
address-formation controls stay enabled and are not conditionally ignored.

A CI selector audit found five `miri-cross-arena` rows still naming tests deleted with the former
reflection retry/legacy initializer paths. Those no-match rows were removed from both configurations,
with the workflow manifest and CI documentation synchronized (98 matrix entries). A fresh
`cargo nextest list` audit found all remaining exact native-bridge, VM-native and teardown selectors
enabled (27 no-features and 65 multithreading). The production WorkerRegistry, STW and shared-table
Loom suites passed 10/10, 8/8 and 2/2; the MT all-target `cfg(loom)` compile passed. The models stay
within the recorded three-thread/preemption bounds and do not model gc-arena mark colors. No hosted
CI workflow was dispatched.

Final verification: `cargo run -p xtask -- fixtures build` succeeded. Feature-explicit CLR
`diff_run.sh` runs matched stdout and exit for 29 applicable ST fixtures and 45 MT fixtures; the
MT-only worker/canonical/reflection/TLS cases were excluded from ST. The VM-only forced-deadlock
fixture was never passed to `dotnet`; its marker/skip regression is in the passing script suite.
Four fresh-binary `--host` rungs matched `dotnet`: Newtonsoft and EF InMemory with no features and
with `multithreading`. Existing ignored host-project `obj/` caches initially caused duplicate
assembly attributes; they were moved aside without deletion during each host build and restored
after the exact rungs passed.

The explicit TLS-finalization, empty-release and 1,000-worker selection passed 3/3 under
`timeout 300s`; each 1,000-worker run performs collection and verifies the native-resource bound.
Full `dotnet-vm --lib` nextest passed 265/265 ST and 423/423 MT (five skipped in each; MT under
`timeout 600s`). `./check.sh --fast` passed 1,174 ST, 1,523 MT and 1,540 MT+validation-all tests
(66/7/6 skips). Both all-target clippy `-D warnings` legs passed.

Pinned nightly-2026-05-27 Miri used CI's Tree Borrows/disable-isolation/ignore-leaks flags. The 68
applicable exact singletons passed 1/0/0: pointer-transport descriptors and native-effect bridge
controls in both configurations; native activation/return and initializer publication/retirement
paths; ST finalizer-segment, wait, monitor-retry, initializer-owner and condition-wait tests; and MT
finalizer/condition-wait, foreign-child and cross-arena pointer, thread-static GC, retired-root/release,
exact-lease drain, panic-retention and monitor-owner tests. Dependency integer-to-pointer warnings
remain. Combined Miri invocations reached the local 300-second command cap; every selected singleton
was then run exactly under its 1,800-second bound and passed. The obsolete reflection-retry selector
returned zero matches before its stale CI rows were removed, and is not counted as a pass.

The complete script-test/doc-lint loop passed; cross-arena/Miri-advisory tests passed 4/4 and 6/6;
slots verified 21; the feature matrix enumerated successfully; doc drift passed 59/59; and rustdoc
broken-intra-doc-links passed. No Tag-5, RegistrationToken, ArenaId, pointer-serialization or
allocation-identity semantics changed.

## Done when

- All new and updated fixtures match `dotnet` in both base feature sets where applicable, except the
  VM-only deadlock case in slice 5; feature-specific worker fixtures are run only in multithreading.
- `MulticastState` and its helpers are gone; `rg MulticastState crates` finds nothing.
- A thread-static-only object of an exited thread is finalized, and 1,000 short-lived workers do not
  accumulate OS threads.
- The production Loom job runs the `WorkerRegistry` model.
- `./check.sh --fast`, both full `dotnet-vm` unit-test base sets, both clippy `-D warnings` legs,
  doc lint/drift/intra-doc links, all Loom suites and applicable pinned Miri jobs pass.
- The Newtonsoft and EF InMemory `--host` rungs match `dotnet` with both explicitly built binaries.

## Ask the maintainer only if

- Ref-struct transport in slice 3 needs anything beyond relaxing the value-shape rule: a new pointer
  origin, `Transient` support, or a change to `LayoutManager::trace`.
- Slice 4's logical key would have to change `ArenaId` itself, or pointer serialization.
- Slice 7 finds neither canonical-object option workable without changing observable identity, or
  either one costs more than 5% on a benchmark.
- `total_gc_count()` is non-zero for an idle retired arena after slice 9's cleanup, and the remaining
  allocations can't be attributed and cleared.
- Releasing an arena needs a change to the Tag-5 codec or to `RegistrationToken` semantics.

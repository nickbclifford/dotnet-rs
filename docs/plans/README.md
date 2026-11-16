# Plans

The project goal is a maintainable ECMA-335 runtime whose supported behavior and safety
boundaries are understandable. Prefer one working runtime path over more infrastructure,
audits or documents.

## Queue

In priority order. Each plan's own `Status:` line is the only record of its status.

| ID | Plan | Note |
|---|---|---|
| 23 | [Runtime behavior gaps: static field reflection, Math FCalls, Array.Fill, Span.CopyTo, signed byref offsets and generic arity](23-runtime-behavior-gaps.md) | Validated runtime surface and remaining limits are recorded in the plan |
| 25 | [Implement `DynamicMethod`](25-dynamicmethod-implementation.md) | Slice 1 complete; generics, `Module`, seam retirement and lifetime remain |
| 24 | [Investigate Reflection.Emit and `DynamicMethod`](24-reflection-emit-investigation.md) | Complete; decision is a `DynamicMethod` implementation (plan 25), no `TypeBuilder`; the access violation is fixed |
| 21 | [Correctness backlog: byte-read bounds, pointer slots, feature forwarding, thread IDs and the timer flake](21-correctness-backlog.md) | Complete |
| 22 | [String interning](22-string-interning.md) | Complete; literal, empty and explicit interning identity across threads |
| 18 | [Managed API gaps against the CLR](18-managed-api-clr-gaps.md) | Complete; Newtonsoft and EF host rungs match `dotnet` |
| 19 | [Reflection invocation matches the CLR](19-reflection-invoke-clr-parity.md) | Complete; wrapping, byref returns, success-only copy-back and ambiguity match the CLR |
| 20 | [Shared GC handles and a dedicated finalizer thread](20-gc-handles-and-finalizer-thread.md) | Complete; shared handle table, cross-arena resurrection and the dedicated finalizer thread |
| 17 | [Inspection-driven code quality refactor](17-inspection-quality-refactor.md) | Ordered semantic, readability and dependency slices with an inspection ledger |
| 16 | [Retire legacy continuations and move suspending intrinsics to effects](16-retire-legacy-continuations.md) | Complete (slices 1–8); the multicast continuation for excluded shapes is deliberately kept |
| 15 | [CoreLib `Task` and the thread pool](15-corelib-task-and-thread-pool.md) | CoreLib tasks/resources, capped background pool workers, unnamed events and managed timers are active |
| 14 | [Canonical reflection objects across threads](14-canonical-reflection-objects.md) | Complete; shared canonical type, member and assembly objects |
| 13 | [Async intrinsic API: the body is the implementation](13-async-intrinsic-api.md) | Complete; body validation, inline dispatch and optional fallbacks verified; measured Substring fast path retained |
| 12 | [Managed threading intrinsics](12-managed-threading-intrinsics.md) | Complete; managed threads and monitor condition waits are supported |
| 11 | [Exception control flow: handlers inside handlers, unhandled worker exceptions](11-exception-control-flow.md) | Complete; nested handlers, fatal worker exceptions and filter exception boundary verified |
| 09 | [Retain arenas after managed thread exit](09-retired-arena-lifetime.md) | Establishes the retained-arena lifecycle contract |
| 05 | [Safe APIs whose soundness rests on unchecked caller premises](05-safe-api-premises.md) | Validates typed field and memory-operation premises |
| 06 | [Cross-arena collection fixture through production decoding](06-cross-arena-collection-fixture.md) | Fixtures use production storage decoding |
| 07 | [Nested filters keep earlier suspended frames](07-nested-filter-suspension.md) | Nested filter segments preserve suspended frames |
| 08 | [Async/await VM effects](08-async-effects.md) | Current paths and slice evidence are in [08-async-effects/](08-async-effects/) |
| 10 | [Async effects hardening](10-async-effects-hardening.md) | Complete; post-merge contract and documentation gaps in 08 closed |
| 04 | [Native continuations: Substring proof](04-native-continuations.md) | Measured large-call cost accepted; scheduler contract and behavior controls retained for 08 slice 1 |
| 03 | [Repair enum and named scalar frame homes](03-frame-home-enum-representation.md) | Done; scalar bounds, opcode/growth coverage and historical reproductions verified |
| 02 | [Stable homes for addressable locals and arguments](02-stable-frame-homes.md) | Done; measured primitive cost accepted |
| 01 | [Executor teardown ownership](01-executor-teardown-ownership.md) | Done; kept as a pointer to the contract |

Plan IDs normally follow dependency order. Never reuse an ID; a repair or prerequisite found
while executing a plan gets the next free ID, even when it must precede that plan.

## Not yet planned

Known problems without a plan. Write a plan for one only when it is chosen as the next work.
Entries that plans 18–20 cover stay here until the plan's docs slice removes them; each plan
lists the entries it closes.
Evidence and IDs (p01…, R01…) are defined in the
[pointer-provenance inventory](../pointer-provenance/INVENTORY.md).

- **Warm managed resource lookup cost:** after plan 15 removed the `SR` resource-string
  intrinsic, `exception_messages` went from about 3.4 ms to about 121 ms; accepted for
  CLR-matching messages (no VM-side decoder). Serving manifest resources as an
  `UnmanagedMemoryStream`, as the CLR does, cut the cold first load (`resource_cold_load`
  94 → 8 ms) and `exception_messages` to about 38 ms, but `resource_warm_lookup` rose from about
  31 to 38 ms (not investigated). A warm lookup is about 35 managed calls and about 230 µs, and
  that cost is general per-call overhead, not resource-specific: flat profiles are dominated by
  rebuilding `MethodDescription`, `ResolutionContext` and `VmResolverService` and by virtual
  resolution on every call. Caching resolved targets per call site would help the whole VM.
  Plan 15 has the older numbers and commands.
- **Gaps behind the System.Text.Json fixture** (none involve Emit,
  [plan 24](24-reflection-emit-investigation.md) slice 3; the Emit-dependent failures are
  [plan 25](25-dynamicmethod-implementation.md)):
  - `JsonSerializer.Deserialize<double[]>` throws `NotSupportedException: Derived classes must
    provide an implementation` while STJ creates the array's type info: a `Type` member the
    `DotnetRs.RuntimeType` stub doesn't override.
  - `new Dictionary<string, T>()` (no comparer) throws from `GetAlternateLookup<ReadOnlySpan<char>>`
    ("The collection's comparer does not support the requested operation"); with
    `StringComparer.Ordinal`/`OrdinalIgnoreCase` it works.
  - An Expression tree built from C# that captures a local or sets a field in an object
    initializer stops in `FieldInfo.GetFieldFromHandle`.
  - `Type.GetType(string)` and `Assembly.GetType(string)` hit the `QCall` DLL error.
- **Delegate creation gaps:** `MethodInfo.CreateDelegate<T>(object target)` (a closed delegate)
  stops with "Reflection runtime type object missing index field", so the target appears to be read
  where the delegate type is expected; the static `Delegate.CreateDelegate(Type, MethodInfo)` and
  `(Type, object, MethodInfo)` overloads are missing from `DotnetRs.Delegate`. Open delegates via
  `MethodInfo.CreateDelegate(Type)` work.
- **SQLite connection-string keywords:** `new SqliteConnection("Data Source=:memory:")` throws
  `ArgumentException: Connection string keyword 'data source' is not supported` on the VM
  (`SqliteConnectionStringBuilder.GetIndex`); `dotnet` accepts it. This is the first wall for an EF Core
  SQLite rung and is not an Emit failure
  ([evidence](evidence/24-demand-survey.txt), probe 4).
- **ParameterInfo custom attributes:** `DotnetRs.ParameterInfo` returns no custom attributes
  (`GetCustomAttributes`, `IsDefined`, `GetCustomAttributesData` are hard-coded empty), so
  `ParamArrayAttribute` and other parameter attributes are invisible to reflection. This is why
  `Activator.CreateInstance` with `BindingFlags.CreateInstance` can't bind a `params` constructor
  ([plan 24](24-reflection-emit-investigation.md), slice 2).
- **Userland breadth is untracked:** the only real-application rungs are the Newtonsoft
  roundtrip (`crates/dotnet-cli/tests/host_projects/newtonsoft_roundtrip`) and the EF Core
  InMemory smoke test
  (`crates/dotnet-benchmarks/fixtures/ef_inmemory`). No list of gaps beyond those exists; the
  former EF gap backlog and userland roadmap were deleted. Pick the next application rung
  (for example EF Core SQLite, which [COMPATIBILITY.md](../COMPATIBILITY.md) lists as
  unvalidated) and record its failures here before planning fixes.
- **Fallback continuations for excluded shapes:** `MulticastState` remains for ref-struct
  values and byref targets containing managed pointers, byref returns and `TypedReference`.
  Non-null heap, frame-home, static, unmanaged and MT cross-arena byref arguments with
  pointer-free targets use native effects; transient byrefs are invalid. The managed-body-only
  cctor and span equality slow path routes remain non-inline. Non-inline
  intrinsics use a registered fallback or return `NotImplemented` when absent. Plan 16 retired every other
  legacy continuation (reflection return, monitor retry, static-init retry, yield-retry
  intrinsics, `VmContinuation::RetryInstruction`). Substring retains
  its measured valid-small fast path ([plan 13](13-async-intrinsic-api.md#substring-fast-path-measurement)).
- **Single-threaded finalizer identity:** without `multithreading`, finalizers run on the
  interrupted managed context and share its thread identity, including monitor and initializer ownership.
- **Positive managed-pointer offsets beyond the compact range:** `ManagedPtr` uses signed 32-bit
  displacements, so positive offsets above `i32::MAX` are not representable. Negative offsets
  may be formed and carried, but access remains checked against the owner's extent; this does not
  permit out-of-range dereferences. See [the memory-safety contract](../GC_AND_MEMORY_SAFETY.md#managed-pointer-displacement-and-access).
- **Static field addresses:** `ldsflda; conv.u` writes and static payload alignment
  (`p12_ldsflda_convu_stind_static`).
- **Instance field alignment:** `FieldStorage` uses `Vec<u8>`, which does not promise the
  alignment required by `get_field_mut_local`; callers of that path can trip its alignment
  assertion under Miri.
- **Native call boundary:** payload addresses, ABI returns, by-ref/out copy-back and pinned
  lifetime for P/Invoke (R06/R07). The current cross-arena raw-escape and by-value
  managed-pointer aggregate rejections stay until this is done.
- **Shared raw-memory access:** one protocol for competing plain, volatile, atomic, block and
  raw-alias accesses to shared storage (heap, statics) under `multithreading` (R09/R12).
  `FieldStorage`'s aligned byte-to-integer atomic path also discards pointer provenance when
  storing object references; a subsequent static-storage GC trace fails under Miri.
- **Safe byte writes into reference slots:** `Vector::get_mut`, `FieldStorage::with_data_mut` and
  `ObjectRef::with_data_mut` hand out `&mut [u8]` over storage the collector traces, so safe code
  can write forged pointer bytes; `ObjectPtr::with_data_mut` also skips the owner barrier
  (`ThreadSafeLock::borrow_mut_unbarriered`). Fold into R09/R12 and R10/R11.
- **Resumable bulk operations and barriers:** `cpblk`/`initblk`/`Memmove`/`Array.Copy`/`Fill`
  that keep progress across safepoints and apply GC write barriers to reference slots (R10/R11).
- **Retired-worker resource cost:** each completed explicit managed worker retains one OS thread
  and one arena until VM shutdown; process-wide arena IDs are capped at 65,535. Pool workers never
  retire between requests and are capped at `max(2, available_parallelism)`. Earlier safe retirement
  remains unplanned.
- **Thread-static retention:** thread-static storage and its object references remain rooted until
  VM shutdown, including after an explicit worker exits, so outstanding byrefs remain valid.
  Reclaiming these values when no byref can reach them remains unplanned.
- **String benchmark is code-placement sensitive in MT:** `string` moved +7% in plan 16 slice 6
  (and +33%, then −8%, in slice 3) with unchanged instruction counts, because its hot
  `IsNullOrWhiteSpace` loop shifts across cache-line boundaries as the binary changes. Measured
  2026-11-14: layouts differ by 6–8% with identical instructions per iteration, and the slow
  layouts have twice the frontend stalls. Building with `-C llvm-args=-align-loops=64` gave the
  fast time on the two layouts tried; check more layouts and the `dispatch` bench before adding
  it to the bench profile. Until then, compare `perf stat` instructions before attributing a
  `string` change to a commit.
- **Loom model coverage:** the hand-written `loom_stw.rs` model does not compile the production
  `WorkerRegistry` and does not cover every stop-the-world, retirement or shutdown interleaving.
  `dotnet-vm` does not compile under `cfg(loom)` with `multithreading`, so only
  `dotnet-runtime-memory/tests/loom_shared_tables.rs` drives production types (`SharedGcHandles`,
  `Finalizers`). It does not model the owner-partition color reads as data races: loom does not
  track gc-arena's mark colors.

## Writing a plan

A plan is a short, current document an agent can act on. Keep detailed history in git.
Retain validation conclusions, reproducible check commands, accepted costs and unresolved
limits in the relevant plan or subsystem doc. The pointer inventory records current
safety coverage; dated transcripts do not define current requirements.

- **Header:** `Status:` (Not started / In progress / Parked / Complete) and a one-sentence
  **Outcome** that a test can check.
- **Problem:** what goes wrong today, with code paths and test names.
- **Scope:** what is in, and what is out with the reason. Use the spec (ECMA-335 section
  numbers) to rule out cases that valid programs cannot produce.
- **Design:** the chosen approach, briefly. Reuse existing mechanisms where one fits.
- **Slices:** ordered increments. Each slice ends with the VM using the new path and its tests
  enabled. A slice never builds support that only a later slice uses.
- **Done when:** the checkable exit conditions.
- **Ask the maintainer only if:** a short list of specific triggers.

If a slice turns out to need much more than its description (new modules, or support for
cases outside the stated scope), stop and revise the plan rather than splitting the work into
more preparatory steps.

# Plans

The project goal is a maintainable ECMA-335 runtime whose supported behavior and safety
boundaries are understandable. Prefer one working runtime path over more infrastructure,
audits or documents.

## Queue

Every plan below is complete; newest first. Each plan's own `Status:` line is the only record of
its status.

| ID | Plan | Note |
|---|---|---|
| 27b | [Consolidate tests around intended contracts](27b-test-consolidation.md) | Complete; audit findings resolved with behavior and safety coverage preserved |
| 27a | [Audit test contracts and evidence](27a-test-audit.md) | Complete; every test family inspected, findings in the [ledger](27-test-audit-ledger.md) |
| 26 | [Thread lifecycle reclamation, single-threaded finalizer identity and the multicast fallback](26-thread-lifecycle-and-multicast-fallback.md) | Complete; empty worker arenas released, ST finalizer identity, multicast without a fallback |
| 25 | [Implement `DynamicMethod`](25-dynamicmethod-implementation.md) | Complete except eviction of unreachable dynamic methods (see Not yet planned) |
| 24 | [Investigate Reflection.Emit and `DynamicMethod`](24-reflection-emit-investigation.md) | Complete; `DynamicMethod` via a synthetic module, no `TypeBuilder`; the access violation is fixed |
| 23 | [Runtime behavior gaps: static field reflection, Math FCalls, Array.Fill, Span.CopyTo, signed byref offsets and generic arity](23-runtime-behavior-gaps.md) | Complete; differential fixtures match `dotnet` |
| 22 | [String interning](22-string-interning.md) | Complete; literal, empty and explicit interning identity across threads |
| 21 | [Correctness backlog: byte-read bounds, pointer slots, feature forwarding, thread IDs and the timer flake](21-correctness-backlog.md) | Complete; timer flake not reproduced, diagnostics improved |
| 20 | [Shared GC handles and a dedicated finalizer thread](20-gc-handles-and-finalizer-thread.md) | Complete; shared handle table, cross-arena resurrection and the dedicated finalizer thread |
| 19 | [Reflection invocation matches the CLR](19-reflection-invoke-clr-parity.md) | Complete; wrapping, byref returns, success-only copy-back and ambiguity match the CLR |
| 18 | [Managed API gaps against the CLR](18-managed-api-clr-gaps.md) | Complete; Newtonsoft and EF host rungs match `dotnet` |
| 17 | [Inspection-driven code quality refactor](17-inspection-quality-refactor.md) | Complete; shared semantic contracts replace the inspected duplication |
| 16 | [Retire legacy continuations and move suspending intrinsics to effects](16-retire-legacy-continuations.md) | Complete; suspending intrinsics use effects |
| 15 | [CoreLib `Task` and the thread pool](15-corelib-task-and-thread-pool.md) | Complete; CoreLib tasks and resources, capped pool workers, unnamed events and managed timers |
| 14 | [Canonical reflection objects across threads](14-canonical-reflection-objects.md) | Complete; shared canonical type, member and assembly objects |
| 13 | [Async intrinsic API: the body is the implementation](13-async-intrinsic-api.md) | Complete; body-only registration, inline dispatch at IL call sites, optional fallbacks |
| 12 | [Managed threading intrinsics](12-managed-threading-intrinsics.md) | Complete; managed threads and monitor condition waits |
| 11 | [Exception control flow: handlers inside handlers, unhandled worker exceptions](11-exception-control-flow.md) | Complete; nested handlers, fatal worker exceptions, filter exception boundary |
| 10 | [Async effects hardening](10-async-effects-hardening.md) | Complete; effect control and static initialization are explicit state machines |
| 09 | [Retain arenas after managed thread exit](09-retired-arena-lifetime.md) | Complete; retained-arena lifecycle contract |
| 08 | [Async/await VM effects](08-async-effects.md) | Complete; native effects for suspending intrinsics |
| 07 | [Nested filters keep earlier suspended frames](07-nested-filter-suspension.md) | Complete; nested filter segments preserve suspended frames |
| 06 | [Cross-arena collection fixture through production decoding](06-cross-arena-collection-fixture.md) | Complete; fixtures use production storage decoding |
| 05 | [Safe APIs whose soundness rests on unchecked caller premises](05-safe-api-premises.md) | Complete; typed field and memory-operation premises enforced |
| 04 | [Native continuations: Substring proof](04-native-continuations.md) | Complete; superseded by plan 08, measured large-call cost accepted |
| 03 | [Repair enum and named scalar frame homes](03-frame-home-enum-representation.md) | Complete; enum and named-scalar homes keep scalar semantics |
| 02 | [Stable homes for addressable locals and arguments](02-stable-frame-homes.md) | Complete; measured primitive cost accepted |
| 01 | [Executor teardown ownership](01-executor-teardown-ownership.md) | Complete; pointer to the contract |

Plan IDs normally follow dependency order. Never reuse an ID; a repair or prerequisite found
while executing a plan gets the next free ID, even when it must precede that plan.

## Not yet planned

Known problems without a plan. Write a plan for one only when it is chosen as the next work.
Evidence and IDs (p01…, R01…) are defined in the
[pointer-provenance inventory](../pointer-provenance/INVENTORY.md).

- **Negative GC generation validation** ([CLI-11](27-audit/cleanup-cli.md)):
  `gc_collect_negative_count_smoke_42` exercises `GC.Collect(-1)` without claiming CLR parity.
  Reproduce the negative-generation behavior against CLR and decide/fix validation separately
  from this smoke fixture's contract. (The CLR-side behavior is unconfirmed; the VM ignores the
  argument in `gc_collect_1`.)
- **Metadata failures hidden by boolean classification and reflection probes**
  ([27b library scope](27-audit/cleanup-libraries.md), [reflection scope](27-audit/cleanup-cli.md)):
  concrete comparison and assignability propagate resolution errors, while existing bool
  classifiers (`is_class`, `is_interface`, `is_nullable`, `has_default_constructor`, and the
  compatibility `is_value_type`) can reduce lookup failures to a classifier result. Generic
  special-constraint validation still uses those legacy classifiers. Reflection also retains
  optional nullable/primitive probes and constructor-candidate shape handling that can discard
  lookup errors. Audit those query contracts and carry genuine metadata failures to the typed
  VM-error boundary without confusing an ordinary negative query result with failed resolution.
- **Eviction of unreachable `DynamicMethod`s:** compiled dynamic methods and their metadata
  arenas are never reclaimed. Safe eviction needs GC-visible owners for delegates, `ParameterInfo`
  and `ldftn` function pointers (today a bare method index), cross-arena marking for the shared
  reflection table, pruning of loader registries and descriptor caches, and a fix for
  `ResolutionS::definition`'s `'static` metadata references. See
  [plan 25's eviction analysis](25-dynamicmethod-implementation.md#eviction-analysis).
- **`DynamicMethod` limits:** managed vararg calls throw `InvalidProgramException` (as the Linux
  CLR does; other platforms untested), and `owner`/`skipVisibility` are not enforced because the
  VM enforces no general member visibility. See
  [plan 25](25-dynamicmethod-implementation.md#limits-and-out-of-scope).
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
- **Gaps behind the System.Text.Json fixture** (none involve Emit;
  [plan 24](24-reflection-emit-investigation.md) slice 3):
  - `JsonSerializer.Deserialize<double[]>` throws `NotSupportedException: Derived classes must
    provide an implementation` while STJ creates the array's type info: a `Type` member the
    `DotnetRs.RuntimeType` stub doesn't override (unconfirmed).
  - `new Dictionary<string, T>()` (no comparer) throws from `GetAlternateLookup<ReadOnlySpan<char>>`
    ("The collection's comparer does not support the requested operation"); with
    `StringComparer.Ordinal`/`OrdinalIgnoreCase` it works. (Unconfirmed; may be fixed by the
    NUL-terminated string hashing change in 65139bc1.)
  - An Expression tree built from C# that captures a local or sets a field in an object
    initializer stops in `FieldInfo.GetFieldFromHandle` (unconfirmed).
  - `Type.GetType(string)` resolves only CoreLib names (`intrinsic_type_get_type`; other names
    return null), and `Assembly.GetType(string)` still reaches the `QCall` DLL error
    (unconfirmed).
- **Generic field declaring type:** `DeclaringType` on a field obtained from a constructed
  generic type returns the generic type definition rather than the constructed type. For example,
  `typeof(G<int>).GetField("Value").DeclaringType` is not reference-equal to `typeof(G<int>)`,
  unlike the CLR.
- **Delegate creation gaps:** `MethodInfo.CreateDelegate<T>(object target)` (a closed delegate)
  stops with "Reflection runtime type object missing index field", so the target appears to be read
  where the delegate type is expected; the static `Delegate.CreateDelegate(Type, MethodInfo)` and
  `(Type, object, MethodInfo)` overloads are missing from `DotnetRs.Delegate`. Open delegates via
  `MethodInfo.CreateDelegate(Type)` work. (The closed-delegate failure is unconfirmed; the missing
  static overloads are verified.)
- **SQLite connection-string keywords:** `new SqliteConnection("Data Source=:memory:")` throws
  `ArgumentException: Connection string keyword 'data source' is not supported` on the VM
  (`SqliteConnectionStringBuilder.GetIndex`); `dotnet` accepts it. This is the first wall for an EF Core
  SQLite rung and is not an Emit failure
  ([evidence](evidence/24-demand-survey.txt), probe 4; unconfirmed since).
- **Reflection member identity:** a property accessor is not the same object `GetMethod`
  returns for that method, and the CLR's `ReflectedType` behavior (distinct `MethodInfo`s for one
  method reached through different types) is not modeled
  ([plan 14](14-canonical-reflection-objects.md#remaining-limit)).
- **Reflection binding gaps:** `GetConstructor` pops and ignores its `Binder`,
  `CallingConventions` and `ParameterModifier` arguments, and `Type.InvokeMember` throws
  `NotSupportedException` ([plan 19](19-reflection-invoke-clr-parity.md)).
- **Array store checks:** `stelem` never throws `ArrayTypeMismatchException` for an incompatible
  reference element ([plan 18](18-managed-api-clr-gaps.md)).
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
- **Positive managed-pointer offsets beyond the compact range:** `ManagedPtr` uses signed 32-bit
  displacements, so positive offsets above `i32::MAX` are not representable. Negative offsets
  may be formed and carried, but access remains checked against the owner's extent; this does not
  permit out-of-range dereferences. See [the memory-safety contract](../GC_AND_MEMORY_SAFETY.md#managed-pointer-displacement-and-access).
- **Static field addresses (R03, p12):** `ldsflda; conv.u` writes and static payload alignment
  (`p12_ldsflda_convu_stind_static`, a suspected-UB diagnostic probe).
- **Instance field alignment (R03, p12):** `FieldStorage` uses `Vec<u8>`, which does not promise
  the alignment required by `get_field_mut_local`; callers of that path can trip its alignment
  assertion under Miri.
- **Unmanaged pointer serialization (R04):** the managed-pointer codec stores an `Unmanaged`
  origin as a bare integer address and re-forms the pointer from it, so no provenance survives a
  round trip; only the single-allocation round trip (p05) is checked.
- **Wide store into a narrow homed local (R11):** `p_wide_store_into_int_local` (an 8-byte
  `stind.i8` into a 4-byte homed local) is unverified: natively it fails the frame-home bounds
  check with an `AccessViolationException` the mock CoreLib cannot resolve, and its Miri outcome is
  unknown.
- **Native call boundary:** payload addresses, ABI returns, by-ref/out copy-back and pinned
  lifetime for P/Invoke (R06/R07). The current cross-arena raw-escape and by-value
  managed-pointer aggregate rejections stay until this is done.
- **Shared raw-memory access:** one protocol for competing plain, volatile, atomic, block and
  raw-alias accesses to shared storage (heap, statics) under `multithreading` (R09/R12).
  `FieldStorage`'s aligned byte-to-integer atomic path also discards pointer provenance when
  storing object references; a subsequent static-storage GC trace fails under Miri (found while
  validating `thread_static_gc_on_worker_exit`, which does not exercise that path).
- **Safe byte writes into reference slots:** `Vector::get_mut`, `FieldStorage::with_data_mut` and
  `ObjectRef::with_data_mut` (`crates/dotnet-value`) hand out `&mut [u8]` over storage the
  collector traces, so safe code can write forged pointer bytes; `ObjectPtr::with_data_mut` also
  skips the owner barrier (`ThreadSafeLock::borrow_mut_unbarriered`). Fold into R09/R12 and
  R10/R11.
- **Bulk-operation barriers and count width (R10/R11):** `Memmove`, `ClearWithoutReferences` and
  byte `Fill` already keep progress across safepoints, and `cpblk`/`initblk` run to completion by
  design ([plan 16](16-retire-legacy-continuations.md#bulk-operations)). Open: GC write barriers
  for bulk copies of reference slots (reference-containing `Span<T>.CopyTo` stays element-wise),
  and the general count width.
- **Retired-worker arena-ID capacity (R08):** arena IDs are monotonic and capped at 65,535 per process;
  they are not recycled because owner-keyed state and pointer/registration identities may outlive
  an individual execution. Plan 26 releases eligible empty worker arenas and OS threads, but does
  not alter the ID cap or release arenas that retain live objects.
- **String benchmark is code-placement sensitive in MT:** `string` moves by several percent with
  unchanged instruction counts (+7% and later −6% in plan 16), because its hot
  `IsNullOrWhiteSpace` loop shifts across cache-line boundaries as the binary changes. Layouts
  differ by 6–8% with identical instructions per iteration, and the slow layouts have twice the
  frontend stalls. Building with `-C llvm-args=-align-loops=64` gave the
  fast time on the two layouts tried; check more layouts and the `dispatch` bench before adding
  it to the bench profile. Until then, compare `perf stat` instructions before attributing a
  `string` change to a commit.
- **Static storage key aliasing:** static storage keys canonicalize through aliases recorded by
  `StaticStorageManager::init`, so cross-assembly `TypeRef`-versus-`TypeDef` unification depends
  on `init` having seen the spelling first (untested; [plan 16](16-retire-legacy-continuations.md#remaining-limits)).
- **Possibly write-only interpreter state:** `original_ip`/`original_stack_height` are saved on
  every instruction and around nested dispatch but may no longer be read. Removing them touches
  the layout-sensitive `step_normal` hot path, so it needs a benchmark.
- **Exit waits for retired-owner finalizers:** `Executor::run` waits while a retired worker is in
  `Finalizing` (`WorkerRegistry::foreground_quiescent`), so a finalizer that never returns there
  blocks `run`; the CLR does not wait for finalizers at exit
  ([plan 11](11-exception-control-flow.md#remaining-limit)).
- **No Loom model for the intern or canonical tables:** the shared-table models avoid strings
  because the interner is global ([plan 22](22-string-interning.md)).

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

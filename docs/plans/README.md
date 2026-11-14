# Plans

The project goal is a maintainable ECMA-335 runtime whose supported behavior and safety
boundaries are understandable. Prefer one working runtime path over more infrastructure,
audits or documents.

## Queue

In priority order. Each plan's own `Status:` line is the only record of its status.

| ID | Plan | Note |
|---|---|---|
| 21 | [Correctness backlog: byte-read bounds, pointer slots, feature forwarding, thread IDs and the timer flake](21-correctness-backlog.md) | Complete |
| 22 | [String interning](22-string-interning.md) | Not started; `ldstr`, `string.Empty` and `String.Intern` identity |
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

- **Static field reflection:** `DotnetRs.FieldInfo.SetValue` throws `NotImplementedException` for
  static fields and `GetValue` returns null (`crates/dotnet-intrinsics-reflection/src/fields.rs`).
  Needs static storage lookup, type-initializer triggering and access checks; surfaced by plan 21
  slice 4's pointer-slot fixture, which therefore covers instance pointer fields only.
- **Dependency major-version migrations:** defer the 12 exported notices without upgrading
  APIs: `gc-arena` in the workspace and both standalone fuzz manifests (3); workspace
  `parking_lot` and `lru` (2); benchmark `criterion` (1); `syn` in dotnet-macros,
  dotnet-macros-core, dotnet-vm build-dependencies and xtask (4); and `phf` plus `phf_codegen`
  in dotnet-vm (2). Resolve each family as deliberate API/safety/measurement work; MSRV
  compatibility alone is not evidence that an upgrade is safe. Why each is separate:
  - `gc-arena` 0.7: pointer representation and pacing (shallow bytes → pointer counts) change;
    needs a safety, pacing and performance review, plus an audit of 0.6.1's equal-alignment
    cast requirement. GC upgrades need Miri evidence, not just compilation.
  - `parking_lot` 0.13: per-acquisition raw guard state, fairness and recursive-lock changes
    affect the sync facade, lock order and GC locks; needs Loom evidence.
  - `syn` 3: AST/parser migration; validate emitted code, signature scans and slot consumers.
  - `phf`/`phf_codegen` 0.14: table and runtime formats must migrate together and pass the
    generation/dispatch probes.
  - `lru` 0.19: shared-iterator `Sync` requirements need trait review (types and VM caches).
  - `criterion` 0.8: several API generations from 0.5; needs smoke runs and a measurement
    comparability check.

  The full 2026-11 evidence (changelog and release links) is in the deleted inspection ledger,
  `git show 0f85ff25^:REVIEW.md` (finding F-DEP-002).
- **Managed resource lookup cost:** after plan 15 removed the `SR` resource-string intrinsic, the
  `exception_messages` benchmark (cold first `ResourceManager` load plus 127 lookups, fresh VM
  per iteration) went from about 3.4 ms to about 121 ms (~35x). Accepted by the maintainer for
  correctness (CLR-matching messages, no VM-side decoder). Fresh-VM first lookups account for
  substantial cost and repeated exception/message lookups also cost time; profile (samply) and
  optimize. The warm case includes exception handling rather than isolating dictionary lookup.
  Numbers and commands are in
  [plan 15](15-corelib-task-and-thread-pool.md).
- **Dynamic code without Reflection.Emit:** the VM reports `IsDynamicCodeSupported` true (EF Core
  treats false as NativeAOT) and `IsDynamicCodeCompiled` false (plan 18 slice 7). Code that checks
  only `IsDynamicCodeSupported` and then emits IL fails; two such paths are patched per call site
  in `crates/dotnet-vm/src/stack/call_ops_impl.rs` (`LambdaExpression.Compile()` redirected to
  the interpreter overload, Expression's `ForceAllowDynamicCode` stubbed). Either implement
  `DynamicMethod` or find a single switch-level answer before adding more call-site patches.
- **Access violation in `RuntimeType.GetNativeTypeHandle`:** reached from `DynamicMethod.Init`
  (`get_IsActualInterface`) when Newtonsoft used Reflection.Emit; managed code should never
  produce an `AccessViolationException`. No longer reached by any rung
  ([evidence](evidence/18-array-casts-host-rungs.txt)).
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
  cctor, span equality slow path and Expression.Compile routes remain non-inline. Non-inline
  intrinsics use a registered fallback or return `NotImplemented` when absent. Plan 16 retired every other
  legacy continuation (reflection return, monitor retry, static-init retry, yield-retry
  intrinsics, `VmContinuation::RetryInstruction`). Substring retains
  its measured valid-small fast path ([plan 13](13-async-intrinsic-api.md#substring-fast-path-measurement)).
- **Single-threaded finalizer identity:** without `multithreading`, finalizers run on the
  interrupted managed context and share its thread identity, including monitor and initializer ownership.
- **Negative and wide managed-pointer offsets:** `ManagedPtr` stores a compact unsigned offset,
  so byref arithmetic that goes below the base or past the compact range (for example
  `Unsafe.Subtract`, or `Unsafe.Add` with a negative count) fails with `NotImplemented`
  (`managed_offset_error` in `crates/dotnet-value/src/stack_value.rs`).
- **Static field addresses:** `ldsflda; conv.u` writes and static payload alignment
  (`p12_ldsflda_convu_stind_static`).
- **Instance field alignment:** `FieldStorage` uses `Vec<u8>`, which does not promise the
  alignment required by `get_field_mut_local`; exception stack-trace writes can trip its
  alignment assertion under Miri.
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
- **`Span<byte>.Clear()` is very slow on large spans:** clearing 3 MiB takes about 42 s and never
  reaches the `SpanHelpers.ClearWithoutReferences` intrinsic, so it runs as interpreted IL
  (found in [plan 16](16-retire-legacy-continuations.md) slice 6; present before it). Find the
  path `Clear` takes in this CoreLib and route it to the intrinsic.
- **String benchmark is code-placement sensitive in MT:** `string` moved +7% in plan 16 slice 6
  (and +33%, then −8%, in slice 3) with unchanged instruction counts, because its hot
  `IsNullOrWhiteSpace` loop shifts across cache-line boundaries as the binary changes. Compare
  `perf stat` instructions before attributing a `string` change to a commit; consider aligning
  hot intrinsic loops if this keeps masking real changes.
- **Other missing or incorrect runtime behavior:**
  - Empty strings have no shared identity: `string.Empty` allocates on every load
    (`intrinsic_field_string_empty`), so `ReferenceEquals(string.Empty, string.Empty)` and
    `ReferenceEquals(string.Empty, "")` are false (the CLR prints true for both), and APIs that
    return `string.Empty` for empty results cannot do so.
  - Buffered stdout lost on an internal-error exit: not reproduced. The VM has no stdout buffer
    of its own (`Console` writes reach `write(2)` through the native shim, and the CLI flushes
    stdio on every exit path), so a repro (a fixture that writes, then fails with `Internal VM
    error`) is needed first.
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

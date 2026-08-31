# Plan 09 — Whole-codebase legibility review

**Review date:** 2026-08-30
**Gate:** every active Phase-4 row reaches its objective gate or moves to a
separately tracked successor plan without duplicate scope.
**Status:** in progress — review accepted 2026-08-30; priorities 1's P/Invoke
last-error migration, 2's queue-bookkeeping repair, 3's workspace-wide Plan-01
citation extension, and 4's Plan-03 atomic refactor are complete; remaining
engineering work is not started.
**Depends on:** mixed; each Phase-4 row states its own prerequisites. Plan 08
remains parked.
**Scope:** every member of the root Cargo workspace, the VM's internal module
boundaries, the managed fixture suite, in-crate tests, fuzz targets, and
benchmarks.
**Disposition:** accepted planning document. It preserves the review-date
findings and proposes the remaining gated work; the explicitly marked
priorities 1–4 record completed follow-up work. It does not replace
[`README.md`](README.md).

## 1. Summary

The repository does **not** read as indiscriminate “vibe-coded slop.” Its hardest
complexity is usually earning its keep. The stop-the-world protocol, cross-arena
leases and generation stamps, write barriers, exception search/unwind state
machine, support-slot ABI generation, descriptor ownership, and generated
instruction/intrinsic dispatch all have identifiable correctness or performance
reasons. Their local safety arguments are generally unusually explicit. Large
files such as `dotnet-runtime-memory/src/access.rs`,
`dotnet-vm/src/gc/coordinator.rs`, and `dotnet-vm/src/stack/call_ops_impl.rs` are
not automatically decomposition failures: each is still a recognizable choke
point for one protocol.

The accreted quality is concentrated at **boundaries and unfinished
extractions**, not uniformly inside algorithms. The most serious example is a
process-global `static mut` P/Invoke last-error slot whose callers do not
actually establish the documented serialization premise. The clearest
encapsulation failures are `GenericLookup`'s public, hash-invalidating fields
and `dotnet-vm-data`'s duplicate knowledge of the private three-word
`ManagedPtr` encoding. The clearest policy failure is that plan 01 and
`check_doc_drift.sh` enforce predicate citations in only four crates even
though `CONTRIBUTING.md` states the policy for every unsafe site: 223
`// SAFETY:` comments in other `src/` trees had no registry citation at review
time. The expanded drift suite now mechanically covers every package-owned Rust
target and verifies the same bidirectional registry policy across the workspace.

There is also a substantial difference between the conceptual and physical
architecture. `dotnet-vm-ops` is described and drawn like a foundational trait
crate over `dotnet-vm-data`, but it directly imports the concrete assembly
loader and tracer as well as types, values, utilities, and VM data. It is really
the runtime's **ports/contracts layer**. That shape is defensible—it breaks
crate cycles and lets intrinsic crates compile below `dotnet-vm`—but the name,
header comment, and abbreviated dependency diagram make the boundary harder to
discover than it needs to be. The many single-implementation host traits are
also buying crate-DAG separation, while their claimed testing benefit is mostly
unrealized: the resolver, exception, VM-ops, delegate, reflection, threading,
and unsafe-intrinsic crates have no direct Rust unit tests.

The three highest-value engineering moves are:

1. eliminate the process-global P/Invoke last-error invariant by making the
   state thread-scoped and prove isolation with a concurrent fixture;
2. extend completed plan 01's citation/drift mechanism to the whole workspace,
   then execute existing plan 03 so atomic width consistency becomes a type
   property rather than another cited premise; and
3. restore ownership of representation invariants: only `dotnet-value` should
   encode serialized managed pointers, and `GenericLookup` must not expose
   mutation that invalidates its memoized hash (the latter should become an
   explicit first step of plan 05).

While promoting this review, the queue received a small bookkeeping repair.
Plan 01's own file said **complete**, with measured cited-site counts, while the
queue table still said **Not started**. The table now agrees, plan 08 has the
same canonical status-header form as the other plans, and the blocking
doc-drift gate checks every numbered file against the queue. This was status
drift, not a new plan-01 finding. The review used the individual plan files and
current code as the source of truth:

| Plan | State observed in this review | Treatment here |
| --- | --- | --- |
| 01 | **Complete** in `01-layer-invariant-specs.md`; README reconciled during plan-09 promotion | Extend its enforcement to non-core crates; do not redo the completed four-crate pass |
| 02 | **Not started** | Still the primary assurance/test-infrastructure item |
| 03 | **Not started** | Still a high-value, bounded invariant-elimination refactor |
| 04 | **Not started**, blocked on 01 predicate names | Retain; use the completed predicate registry and the proposed workspace extension |
| 05 | **Not started** | Retain and add the public-field hash hole as its first gate |
| 06 | **Not started**, blocked on 01 and 02 | Retain, but remove rather than register the P/Invoke last-error assumption |
| 07 | **Not started** | Retain; run it before increasing differential-oracle coverage |
| 08 | **Parked**; implementation complete but strict-provenance gate unmet after an explicit owner-directed deferral | Do not reopen. The managed-pointer ownership proposal below does not add or imply a strict-provenance gate |

No proof assistant, assurance DSL, ghost-token layer, or new deductive verifier
is proposed. The rejection rationale in `ASSURANCE_BACKGROUND.md` remains
sound.

## 2. Phase 1 — component survey

### Survey basis

The review followed the root workspace membership rather than assuming the
crate list in prose was complete. Production and test Rust is roughly 80k
lines, heavily skewed toward `dotnet-vm`, `dotnet-value`, reflection,
assemblies, types, utilities, and the two runtime-service crates. `cargo tree
--workspace --edges normal --depth 1` supplied the physical dependency view.
Public surface below means Rust `pub` surface; most of these packages are
workspace implementation crates, but none currently declares `publish =
false`, so “public for cross-crate composition” and “intended external API” are
not mechanically distinguished.

No `TODO`, `FIXME`, `todo!`, or `unimplemented!` marker was found in production
Rust. That is not the same as no unfinished behavior: there are explicit
`NotImplemented` returns, compatibility fallbacks, unused error variants, and
no-op extension seams described below.

### `dotnet-vm`: internal sub-survey

`dotnet-vm` is the composition root and interpreter, not merely an instruction
loop. Its name and graph position are accurate. Its crate-level public API is
much wider than the normal embedding entry point (`Executor`): it re-exports
most stack/operation traits and exposes `context`, `dispatch`, `gc`, `layout`,
`resolution`, `resolver`, `state`, `statics`, `sync`, and `threading`. Much of
that visibility exists for extracted crates, integration tests, and benchmarks
rather than as a designed stable API. The result is workable inside the
workspace but gives a new reader several equally plausible entry points.

| VM area | Actual responsibility and structure | Assessment and concrete seams |
| --- | --- | --- |
| `executor.rs`, `dispatch/`, `instructions/` | Owns executor lifecycle, the fetch/dispatch loop, generated handler registry, safe-point polling, and CIL handlers grouped by instruction family. | Instruction modules are easy to predict and code generation removes large handwritten dispatch tables. `ExecutionEngine::step_batch_instruction` currently ignores its budget and always returns one normal step; it is a residual optimization seam, not an abstraction paying rent. |
| `stack/` and `dotnet-vm-data` integration | `VesContext` adapts the shared stack/frame data model to VM operations. `call_ops_impl.rs` owns unified call, virtual, tail, `jmp`, intrinsic, and no-body dispatch. `raw_memory_ops_impl` bridges VM origins to `dotnet-runtime-memory`. | `call_ops_impl.rs` is long but cohesive and correctness-heavy; the previously accepted `call_ops` dispatch duplication is not re-flagged. `context_ops.rs` is a 997-line adapter hub for base, delegate, string, span, threading, unsafe, statics, exception, and P/Invoke ports, while reflection adapters already live separately. Splitting adapters by the subsystem they connect would improve navigation without changing the trait design. |
| `state.rs`, `cache.rs`, `statics.rs` | Owns shared/global versus arena-local state, cache stores and counters, reflection registries, application-context switches, static initialization, and its wait graph. | The shared/local division is clear and justified by multithreading. Many fields are public to satisfy cross-crate adapters, which makes state mutation less obviously mediated. Cache configuration enters here through `dotnet_value::string::parse_env_bool`, an inappropriate dependency direction for a generic configuration parser. |
| `resolver/`, `resolution.rs`, `layout.rs` | VM-owned adapters around `dotnet-runtime-resolver`; layout free functions forward into the resolver-owned factory. | The forwarding is deliberate cycle control, and the docs explain it. The names `resolution`, `resolver`, runtime-resolver, assembly resolution, and value layout nevertheless create a cold-navigation tax; an ownership map is more valuable than another refactor. |
| `gc/`, `threading/`, `sync/` | Owns the STW coordinator, collection-session RAII, arena command completion, managed thread lifecycle, safe-point handshakes, sync blocks, and single-/multi-thread implementations. | This is deliberate protocol complexity. Guards (`CollectionSession`, `GcCycleGuard`, `CommandCompletionGuard`, `StopTheWorldGuard`), generation checks, and negative lock-order assertions make failure cleanup and ordering legible. The major missing evidence is systematic interleaving exploration, already plan 02. One stale threading-doc sentence says GC may move objects even though the collector and assurance model are explicitly non-moving. |
| `intrinsics/` | Owns VM-local GC, object, metadata, CPU, diagnostics, exception, text, and app-context handlers plus generated registry glue; domain groups extracted to `dotnet-intrinsics-*` sit beside it. | The split criterion is not obvious from names alone: some BCL behavior is local because it needs VM internals, while other behavior is in independent crates. Generated dispatch is justified. The seven `BCL-dynamic layout probe — see REVIEW.md` comments across VM/resolver/reflection point at a deleted document and therefore no longer explain their assumption. |
| Error and continuation flow | Uses `StepResult` for interpreter control, `VmError` for host/runtime failure, and managed exception state for CLI exceptions. | The boundary is documented and mostly consistent. `dotnet-vm/src/error.rs` is only a re-export façade for errors actually owned by `dotnet-types`, which is a hidden architectural role of that supposedly type-focused crate. |

Two VM-level findings are more than style:

- `VesContext` reads and writes `dotnet_pinvoke::LAST_ERROR` directly. The
  safety comment says callers serialize access, but managed executions can run
  on multiple OS threads and there is no lock or thread-local storage on this
  path. The premise is not established at the call site.
- `dotnet-vm-data::EvaluationStack` must repair stack-origin managed pointers
  after a `Vec` move. That behavior is necessary, but value-type fields are
  repaired by reconstructing `word0`, `word1`, and `word2` with literal masks,
  shifts, tags, and XOR checksum copied from `dotnet-value::pointer::serde`.
  This crosses the representation boundary the extraction was supposed to
  create.

### Major non-VM crates

#### `dotnet-value`

This crate is the runtime representation layer: stack values, objects, field
storage, physical layouts, CLR strings, managed/unmanaged pointers, and
CTS/CLI scalar conversion. The name is accurate, but “value” undersells how
much raw-memory representation and pointer serialization policy it owns. Its
public surface is correspondingly broad (`StackValue`, object internals,
layout managers and their fields, pointer origins/info/resolvers, storage, and
string macros). Cross-crate construction explains much of that openness, but
it means representation invariants must be defended conventionally.

The pointer/origin/serialization code is complex for good reasons: stack and
static pointers must be rebound to live bases, heap handles require GC
branding, cross-arena origins require leases, and unmanaged origins genuinely
encode addresses. Plan 08's implementation is visible in these boundaries and
should not be reopened merely because the strict-provenance gate is parked.
The defect found here is narrower: encoding is not solely owned here because
`dotnet-vm-data` duplicates it.

`GenericLookup` actually lives in `dotnet-types`, but it is embedded throughout
value layouts and object descriptors. Its two argument fields are public even
though its `Hash` implementation uses a private eager hash. The documentation
warns consumers not to assign the public fields directly; Rust cannot enforce
that warning. Current production code uses the setters, so this is a latent API
correctness hole rather than evidence of an active corrupt key. Plan 05 will
replace the representation, but privatization should precede or be part of
that plan.

Concrete internal duplication is otherwise limited. Three independent
`LazyLock<bool>` definitions parse `DOTNET_TRACE_GC_PTR_READ` in `layout.rs`,
`storage.rs`, and `object/types.rs`. The previously accepted `PointerOrigin`
matching duplication remains justified by different read/write/trace
operations and is not proposed for wholesale deduplication.

Unsafe code is concentrated and locally commented. All 153 `// SAFETY:`
comments under this crate's `src/` cite the predicate registry. The local
fuzzers cover pointer offset, pointer serde, and raw atomic memory, although two
remain known-red/advisory as already recorded in `docs/FUZZING.md` and plan 02.

#### `dotnet-intrinsics-reflection`

This is a real subsystem, not a thin handler list: it constructs runtime
type/method/field/property objects, implements member discovery and binding
filters, materializes custom attributes, performs invocation marshalling, and
maintains host-facing reflection registries. The name and graph position are
right. Public handlers and host traits are necessary for generated cross-crate
dispatch.

The top-level files mostly follow managed concepts, but
`types/type_members.rs` has become a 1,764-line catch-all containing assembly
names, custom-attribute materialization, `Type.GetMethods`/constructors/fields,
member filtering, and property selection. Method behavior is consequently
split between `methods.rs` (MethodBase/RuntimeMethodInfo/invocation) and
`types/type_members.rs` (method lookup through `Type`). A new reader looking
for `GetMethod` in `methods.rs` will be wrong. Custom attributes deserve their
own owner, and type-member selection deserves a focused module.

Reflection uses several narrow host traits, all implemented only by
`VesContext`/`ResolutionContext`; there are no direct Rust unit tests in this
crate. The abstraction buys crate separation but has not yet bought fake-host
tests. Its four local safety comments were predicate-cited by the completed
workspace-wide gate extension. The seven deleted-`REVIEW.md` references include
two reflection sites.

#### `dotnet-assemblies`

The crate owns much more than parsing assemblies: metadata resolution and
caches, framework/app/NuGet probing from `runtimeconfig.json` and `deps.json`,
binding redirects/version policy, generated support-assembly construction, and
the support-slot ABI contract. Its graph position beneath resolver/VM is
correct, but its name undersells the host-policy half. `host.rs`, `loader.rs`,
`resolution.rs`, `support.rs`, and `support_contract.rs` are recognizable
owners; the loader/resolution/runtime-resolver naming boundary still needs an
architecture map.

The support-slot generation and validation may look elaborate, but it replaces
stringly typed layout knowledge with one dense ID contract and CI audit; this
is deliberate complexity. The crate has substantial tests for host probing,
loader behavior, support contracts, validation, versions, and drop/lifetime
behavior. Fifteen `src/` safety comments have good local prose but no registry
predicate citation. `AssemblyLoadError` is defined in `dotnet-types`, not here;
`error.rs` only re-exports it.

#### `dotnet-types`

This crate owns metadata-backed type/method/field descriptors, concrete generic
specialization, type identity/assignability/variance, runtime type forms, and
well-known type names. Those responsibilities match its name. `TypeComparer`
and `generics.rs` are large because ECMA identity, generic substitution,
variance, resolution failure policy, and recursion guards are genuinely
multi-dimensional. They remain cohesive and have targeted unit tests.

The public `GenericLookup` fields are the major API flaw described above.
Plan 05 already targets the larger mutable-key/clone problem, so this review
extends rather than duplicates that plan. A second boundary smell is
`error.rs`: it owns assembly loading, execution, memory, P/Invoke, pointer
serde, intrinsic, and aggregate VM errors in addition to type-resolution
errors. The workspace is consistent about `thiserror`, but the consistency is
achieved by making `dotnet-types` a hidden common-error crate. `MemoryError`
and `VmError::Memory` have no production constructor; the separately named
`MemoryAccessError` is the used path.

Twelve safety comments are outside plan 01's enforced crate set. The private
`TypeIndex` transmute is especially notable: source calls it the “permanent
approach,” while plan 06 explicitly says the upstream layout dependency should
be fixed and trusted only temporarily. That contradiction should be resolved
inside plan 06 rather than by creating a neighboring plan.

#### `dotnet-utils`

This is a genuine foundational utility crate, but it contains three substantial
runtime domains: typed indices/offsets, atomic raw access, and GC/concurrency
support (arena registry, cross-arena leases, thread-safe lock, synchronization
compatibility, and lock-order types). Its generic name hides that criticality.
Module ownership is nevertheless clear.

The nine atomic width ladders are the known plan-03 item and are not a new
finding. Cross-arena helper duplication marked as backward compatibility and
the accepted SIMD-origin patterns are not re-flagged. All 67 source safety
comments cite registry predicates. Cross-arena lease/generation bookkeeping
and the lock-order type graph are deliberate safety complexity; plan 02's loom
work is the appropriate next evidence, not simplification.

#### `dotnet-runtime-memory`

This crate is the memory-service choke point for bounds/layout validation,
raw/atomic access, write barriers, local versus cross-arena ownership, heap
registration, handles, finalization, and weak handles. Its name and position
are accurate. `RawMemoryAccess` intentionally centralizes all write-barrier and
validation paths; splitting `access.rs` merely because it is 1,942 lines would
risk duplicating those checks. The current file has recognizable phases and
about 400 lines of co-located tests.

`MemoryOps` exists in both `dotnet-vm-ops` (allocation/boxing port) and this
crate (an extension that exposes the heap accessor), which is semantically
defensible but naming-hostile. Plan 03 owns the repeated atomic width dispatch.
All 104 source safety comments cite predicates. The documented cross-arena
weak-reference limitation remains real and unqueued: the global fixed-point
coordinator handles strong resurrection but cannot correctly zero weak handles
across arenas.

#### `dotnet-runtime-resolver`

This crate owns semantic runtime resolution: normalization, generic and
virtual/static-constrained method resolution, value construction, type
properties, and physical layout computation/caching. `dotnet-assemblies`
resolves metadata identities and host paths; this crate interprets those
identities for execution. The distinction is reasonable after it is known but
not obvious from the two names.

`factory.rs`, `layout.rs`, `methods.rs`, and `types.rs` form good internal
owners. The public surface is mostly a generic service plus a broad set of
cache/layout/context adapter traits, all ultimately backed by the VM. There are
no direct tests despite 2.7k lines of behavioral logic. Eight of nine safety
comments lack registry citations. It also contains one of the stale
`REVIEW.md` layout-probe references.

#### `dotnet-pinvoke`

The crate cleanly owns native-library policy/loading, libffi call construction,
argument/return marshalling, write-back buffers, pinning, and fuzz-time denial.
That matches its name. `call.rs` is large but the ABI and temporary-lifetime
complexity is mostly justified; `call_types.rs`, `marshal.rs`, `loader.rs`, and
`sandbox.rs` already separate supporting concerns. Integer narrowing and
value-type return alignment have direct tests.

The public `LAST_ERROR: static mut i32` is neither justified complexity nor an
acceptable simplification. It exports a race-capable representation, contradicts
the managed per-thread semantics of `Marshal.Get/SetLastPInvokeError`, and
requires an external premise no caller establishes. Its thirty-one safety comments were predicate-cited by the completed
workspace-wide gate extension. Existing managed coverage exercises five Linux
`libc`/`libm` shapes but not last-error isolation, strings, arrays, callbacks,
calling-convention variation, or non-Linux ABIs.

#### `dotnet-intrinsics-span` and `dotnet-intrinsics-string`

These crates own exactly the Span/ReadOnlySpan and String BCL fast paths their
names imply. Conversion/search files are large because they bridge managed
layouts, GC-safe borrowing, managed pointers, and SIMD fallbacks. Shared SIMD
and scalar paths are intentionally duplicated where documented by the prior
dedup pass and are not re-flagged.

The common argument and borrow idiom is less coherent: `with_string!` and
`with_vector!` are exported from `dotnet-value` as control-flow macros that
reach into VM operation methods, while `dotnet-vm-ops::intrinsic_args` offers a
function-based null/type policy used mainly by core arrays and delegates.
String/span/unsafe/threading handlers still hand-roll many equivalent pops,
matches, null cases, and type errors. This is an incomplete abstraction
adoption, not evidence that all argument handling can be one generic macro.

Span has two direct Rust tests and 24 safety comments now cited by the
workspace-wide gate. String has six direct tests and 19 likewise-cited safety
comments. The generic environment parser
and three duplicated GC-pointer trace flags belong outside string/value
subdomains.

### Remaining crates

| Crate | Actual role and API assessment | Structure, duplication, unsafe, and residue |
| --- | --- | --- |
| `dotnet-intrinsics-core` | Array, math, and conservative hardware-intrinsic handlers. “Core” is broad but the graph position and public generated-handler surface are appropriate. | Clear three-module split and two direct SIMD tests. It is one of only two users of the shared intrinsic argument-policy helpers; no notable unsafe-policy gap. |
| `dotnet-intrinsics-delegates` | Delegate construction/combine/remove/equality and multicast invocation state. Public host traits and `try_delegate_dispatch` connect generated dispatch to the VM. | Cohesive three-module structure. Its one safety comment is now cited, and coverage is otherwise entirely through 14 managed fixtures and VM integration. Accepted dispatch duplication is not re-flagged. |
| `dotnet-intrinsics-threading` | Interlocked, volatile, monitor, and thread BCL handlers. Public monitor/stack-slot host ports are implemented only by `VesContext`. | Files map cleanly to concepts. All thirty safety comments are now cited. `Monitor.Wait`, `Pulse`, and `PulseAll` return explicit not-implemented errors and have no fixtures; this is compatibility scope, not dead code. |
| `dotnet-intrinsics-unsafe` | `Unsafe.*`, `Buffer`, and `Marshal` handlers over raw-memory and layout ports. | Module split is sensible. All twenty-three safety comments are now cited despite dense raw-memory behavior. `#[allow(unused_variables)]` mostly reflects the generated intrinsic signature contract rather than dead computation. |
| `dotnet-vm-data` | Shared execution data: step results, continuations, method/frame/evaluation stacks, and exception-state records. It is mutable data, not passive DTOs. | The name is accurate but most fields are public, so state invariants are convention-based. Five direct layout/stack tests, six now-cited safety comments. `normalize_reserve_target(x) -> x` is a dead extension seam. Its module doc still implies exception logic is in VM, while that logic now lives in `dotnet-exceptions`. The duplicated managed-pointer encoder is its serious boundary violation. |
| `dotnet-vm-ops` | Cross-crate runtime ports, intrinsic argument helpers, prepared calls, and trait aliases. | The API is intentionally all-public for downstream intrinsic crates, but “foundational operations” is misleading because it imports concrete `AssemblyLoader` and `Tracer`. There are no direct tests. The generated trait aliases avoid repetitive composite impls and are justified; the fine-grained host ecosystem should be evaluated by whether it gains direct tests. |
| `dotnet-exceptions` | Parses ECMA exception regions and executes the two-pass search/filter/unwind state machine using VM ports and VM-data state. | Small, cohesive, accurately named, and appropriately extracted. Its two safety comments are now cited; all behavior is exercised indirectly. The state machine's complexity is specified, not accidental. |
| `dotnet-simd` | Six portable byte operations with architecture-specific implementations and scalar fallback. | Compact API and five tests. All thirty-nine safety comments are now cited. Architecture-specific duplication was explicitly accepted by the prior dedup pass and remains justified by instruction-set differences. |
| `dotnet-metrics` | Runtime/cache/GC instrumentation, snapshots, active-metrics TLS forwarding, and benchmark-only counters. | The name is accurate; one 1,453-line file obscures models versus counters versus snapshots, but behavior remains straightforward. Feature-gated real/no-op method pairs are repetitive yet transparent; a macro may make them less legible. Nine tests and nine cited safety comments. Low structural priority. |
| `dotnet-tracer` | Structured tracing configuration plus bounded asynchronous delivery/flushing. | Cohesive public `Tracer`/sink/span API and five tests. Backpressure/drop behavior and a flusher thread are deliberate. The undocumented legacy `DOTNET_RS_TRACE_LEVEL` path should enter the configuration inventory. |
| `dotnet-cli` | CLI parsing, host startup, process exit, and the main integration harness. | Small production API with no production unsafe block. Tests reach deeply into VM internals and therefore help explain the VM's broad public surface; their unsafe uses are platform/test glue and the compile-only cross-arena probe. The harness oracle flaw is existing plan 07. |
| `dotnet-macros-core` | Shared parsers and expansion models for intrinsic/instruction signatures and trait aliases. | Correctly prevents the proc macros and VM build script from independently interpreting the mini-syntax. Thirteen tests; no unsafe. Complexity is build-time consistency, not runtime abstraction. |
| `dotnet-macros` | Proc-macro façade for intrinsic fields/methods, instructions, and trait aliases. | Small and appropriately thin over macros-core, with four tests. Source scanning by VM build code is unusual but fail-closed and documented; handwritten dispatch boilerplate would be worse. |
| `dotnet-build-tools` | Deterministic build-script file discovery, cache hashing, Cargo target-path derivation, and support-slot definition parsing. | Accurate, cohesive, and heavily tested for its size. No unsafe. |
| `dotnet-benchmarks` | Eighteen managed workload definitions and Criterion harnesses plus focused metadata, cold-start, pointer-serde, and primitive benches. | Broad public harness API is appropriate for bench binaries. It is performance evidence, not an independent correctness oracle; most workloads assert only exit status. No direct `#[test]` suite. |
| `xtask` | Fixture build/output/cache commands, CI feature matrices, and support-slot verification. | One 800-line binary with tests. The command families are still small enough that splitting is optional; it is not a priority. |

### Unsafe-predicate coverage by crate

The following counts are `// SAFETY:` comments, not counts of unsafe
expressions. The workspace-wide gate now discovers every Cargo workspace package
beneath `crates/` and scans all of its Rust targets (including package-owned
benchmarks and fuzz targets, excluding `target/`). Plan 01 intentionally gated
only its four core crates; that completed historical scope remains unchanged.

| Group | Cited / total | Finding |
| --- | ---: | --- |
| `dotnet-value`, `dotnet-runtime-memory`, `dotnet-utils`, `dotnet-vm` | 482 / 482 | Completed plan-01 core crates, including eight package-owned fuzz comments; do not redo |
| Other production workspace crates | 221 / 221 | All production `src/` comments cite an applicable named predicate |
| `dotnet-benchmarks` package-owned targets | 6 / 6 | All benchmark comments are covered by the same policy |
| **All discovered package-owned Rust targets** | **709 / 709** | `check_doc_drift.sh` mechanically verifies missing, undefined, and orphan citations bidirectionally |

The largest formerly uncovered groups were SIMD (39), P/Invoke (31), threading
intrinsics (30), span (24), unsafe intrinsics (23), and string (19). Their
citations retain local witnesses; architecture intrinsic and P/Invoke ABI sites
use the narrowly scoped predicates introduced for those facts.

## 3. Phase 2 — architectural findings

### Physical graph versus conceptual layering

The broad composition direction is healthy: CLI and benchmarks sit on VM; VM
composes extracted services and intrinsic crates; value/types/utils are low;
proc-macro implementation is split from the proc-macro façade. There are no
cycles hidden by the crate split. The abbreviated graph in `ARCHITECTURE.md`,
however, is a containment sketch rather than the actual dependency hierarchy.
The most important omitted edges are architectural, not incidental:

| Node | Important direct internal dependencies | Consequence |
| --- | --- | --- |
| `dotnet-value` | types, utils | Values are metadata-aware, not a metadata-independent scalar layer. |
| `dotnet-assemblies` | types, utils, value | Loading includes runtime descriptors and support-value construction. |
| `dotnet-vm-data` | types, utils, value | “Data” embeds full runtime values and descriptors. |
| `dotnet-vm-ops` | assemblies, tracer, types, utils, value, VM data, macros | This is a ports/contracts layer with concrete host types, not merely foundational traits over VM data. Its own header still says several traits remain in VM even though they are now defined here. |
| intrinsic crates | macros, types, value, VM data, VM ops; some also utils/SIMD/delegates | The extraction successfully prevents dependencies on `dotnet-vm`, at the price of public port surfaces and adapter boilerplate. |
| runtime memory | types, utils, value, VM ops | Allocation and raw-memory interfaces are mutually conceptual even though the Cargo DAG is acyclic. The duplicate `MemoryOps` name exposes this seam. |
| runtime resolver | assemblies, types, utils, value | It is the semantic layer above metadata resolution, not a sibling foundation. |
| tracer | metrics, utils | Tracing is not actually standalone in the strict dependency sense described by the docs. |

This divergence mostly means the **documentation abstraction is stale**, not
that the Cargo DAG should be flattened. A generated or manually complete
internal-edge table plus a “ports layer” label would make the actual design
legible. Moving `AssemblyLoader` and `Tracer` behind additional traits merely
to make `vm-ops` look lower would add indirection without a second
implementation. If compile times or reuse later require a truly foundational
ops crate, that should be justified with measurements first.

### Cross-crate repetition and workspace idiom

1. **Error style is consistent but ownership is not.** Runtime code uses
   `thiserror` and the documented `StepResult`/`VmError`/managed-exception
   split. Manual errors appear mainly in dependency-light build tooling. The
   problem is that almost every domain error lives in `dotnet-types`, forcing a
   type-system crate to be the de facto common-contract crate. Extracting an
   error crate immediately would be high-churn; the first step should be to
   document this role and remove the unused `MemoryError` branch. A later
   ports-layer change can move errors if it eliminates dependencies rather
   than only renaming imports.
2. **Configuration is independently invented.** Production string literals
   expose at least cache, GC threshold, safe-point, tracing, frame-limit,
   versioning, string-interning, write-barrier, and deadlock-diagnostic knobs.
   Parsing ranges from a reusable permissive boolean parser in `value::string`
   to exact `"1"`, nonzero/zero, presence-only, and panic-on-invalid numeric
   behavior. Ten production/benchmark variables have no docs entry, including
   `DOTNET_GC_THRESHOLD_BYTES`, `DOTNET_SAFE_POINT_POLL_INTERVAL`,
   `DOTNET_STRICT_VERSIONING`, both string-interner knobs, and
   `DOTNET_WB_FLUSH_THRESHOLD`. Three files separately latch the same
   `DOTNET_TRACE_GC_PTR_READ` flag.
3. **Intrinsic argument policy is only partly shared.** `vm-ops` contains
   `ArgPolicy`, object extraction, null handling, and type-mismatch helpers.
   Core arrays and delegates use them; most string/span/unsafe/threading and
   reflection handlers still spell their own variants. These cases are not all
   semantically identical, but today the differences are implicit in control
   flow rather than named as policy.
4. **Macro use is coherent.** Instruction/intrinsic marker macros and generated
   dispatch are used consistently across the workspace. `macros-core` is the
   correct shared parser. There is no large pocket of handwritten registration
   that should obviously use those macros instead.
5. **Caching patterns differ for real reasons, but configuration does not show
   those reasons.** `DashMap`, sharded stores, TLS LRU/front caches,
   `OnceLock`, and string interning serve different ownership/contention
   profiles. Plan 05 addresses descriptor-key cost. The missing artifact is an
   ownership/lifetime/configuration inventory, not one universal cache trait.

### Boundary intelligibility: where a cold reader looks and is wrong

| Concern | Plausible first location | Actual owner(s) |
| --- | --- | --- |
| VM operation traits | `dotnet-vm` or a low-level `vm-ops` over data only | Split between concrete-type-heavy `dotnet-vm-ops`, VM extensions in `vm/stack/ops.rs`, and per-intrinsic host traits |
| Error types | The crate reporting the error | Nearly all in `dotnet-types/src/error.rs`; several crates have re-export-only `error.rs` files |
| `Type.GetMethod` / property lookup | reflection `methods.rs` / a property module | `reflection/types/type_members.rs` |
| Method invocation marshalling | reflection `methods.rs` | Correctly there, separate from method selection in `type_members.rs` |
| Layout | `dotnet-value::layout` | Data model in value, computation in runtime-resolver, forwarding/cache adapters in VM |
| Type resolution | `dotnet-types` | Metadata identity in assemblies, semantic/runtime resolution in runtime-resolver, execution context in VM |
| P/Invoke last error | P/Invoke call state or thread state | Storage in `dotnet-pinvoke`, managed intrinsic in intrinsics-unsafe, direct access adapter in VM |
| Generic env parsing | utilities or CLI/config module | Public helper in `dotnet-value::string` |
| Exception behavior | `dotnet-exceptions` | Behavior there, state/data in VM data, managed object creation and integration in VM |
| Host probing/NuGet policy | CLI/host crate | `dotnet-assemblies::host` and loader |

These are documentation and module-ownership problems before they are crate
count problems. The repository has already undergone multiple extractions; a
new wave of crates without eliminating edges would likely make navigation
worse.

### Complexity that earns its keep versus complexity that does not

| Classification | Area | Judgment |
| --- | --- | --- |
| Deliberate | STW GC, cross-arena leases/generations, write barriers, safepoint exclusion, lock-order DAG | Cross-thread liveness and Rust aliasing require the guards and state transitions; simplifying them would discard correctness information. Plan 02 should add falsifiers. |
| Deliberate | Exception search/filter/unwind state | ECMA-335 requires two phases, nested filters, finally/fault execution, and continuation across managed frames. The state machine is the simpler honest model. |
| Deliberate | Support-slot generator/validator | A generated dense ABI plus loader validation removes scattered field-name/layout assumptions and is CI-audited. |
| Deliberate | Generated instruction/intrinsic dispatch and signature parser | It produces monomorphic hot dispatch while keeping signature syntax single-sourced. Build-time complexity replaces error-prone handwritten registries. |
| Deliberate | Managed-pointer origin/serde model | Stack/static rebasing, GC branding, cross-arena leasing, and genuine unmanaged addresses have different provenance rules. Plan 08 already paid this complexity cost. |
| Deliberate | `TypeComparer`, generic resolution, and call dispatch | Variance, generic substitution, metadata scopes, virtual/default-interface dispatch, tail calls, and ECMA identity rules are actual problem dimensions. Large cohesive functions/modules are preferable to false genericity here. |
| Mixed | VM-ops and per-intrinsic host traits | The trait split breaks crate cycles and narrows handler dependencies. Fine-grained single-implementation traits and ~1.6k lines of VM adapter code have not yet delivered direct fake-host tests, so some granularity is accreted. |
| Accreted | Public mutable `GenericLookup` arguments beside a private cached hash | The representation asks callers to preserve an invariant Rust could enforce. Plan 05 should close it. |
| Accreted | Managed-pointer bit layout copied into VM data | Two crates can silently disagree about tags, masks, offset mirrors, or checksum. The serializer should own rebasing. |
| Accreted | Process-global `static mut` last-error slot | Simplicity is unsafe and semantically wrong under managed multithreading. The correct move is slightly more state, not a stronger comment. |
| Accreted | Scattered environment parsing and three identical trace-flag latches | No runtime requirement demands inconsistent parsing or domain-misplaced helpers. |
| Accreted | `type_members.rs`, `context_ops.rs`, no-op reserve/batch seams, unused `MemoryError` | These are artifacts of growth or anticipated optimizations, not ECMA requirements. Remove or split only along the concrete boundaries named here. |

### Documentation undersell and oversell

**Undersold or missing:**

- `ARCHITECTURE.md`'s dependency tree omits most direct internal edges and does
  not name `dotnet-vm-ops` as the ports/contracts layer.
- `dotnet-assemblies` documentation lists host behavior, but the crate name and
  graph do not communicate that framework/NuGet probing and version policy live
  there.
- There is no one runtime-configuration inventory, so defaults, accepted
  values, latching time, and whether a knob is production/test/benchmark-only
  must be rediscovered from code.
- The docs describe host seams as decoupling/testability infrastructure but do
  not state that most have exactly one implementation and no direct consumer
  tests.
- The cross-arena weak-reference limitation **is** honestly documented in
  `GC_AND_MEMORY_SAFETY.md`; what is missing is a queued disposition.

**Oversold, stale, or contradictory:**

- The plans README said plan 01 was not started while the plan file and code
  showed its four-crate gate complete. Plan-09 promotion repaired the row and
  made queue/file status agreement a blocking drift check.
- The workspace contribution policy reads as universal, while CI requires
  predicate citations only in four crates. A green doc-drift check therefore
  oversells workspace-wide enforcement.
- `dotnet-vm-ops/src/ops.rs` says `StackOps`, `ResolutionOps`, `ReflectionOps`,
  `LoaderOps`, `StaticsOps`, `VesInternals`, and related traits remain in VM;
  they are defined in that very file. Its claim to depend only on lower-level
  types/value/utils also omits assemblies, tracer, and VM data.
- `THREADING_AND_SYNCHRONIZATION.md` says the GC can safely move objects without
  breaking sync blocks; the runtime and assurance model require a non-moving
  collector today.
- Seven source comments cite `REVIEW.md §4 (F-SCOPE-001)`, but no such file
  exists. Their BCL-layout assumption should move into the subsystem docs and,
  where assumed, plan 06's register.
- The `TypeIndex` transmute source comment calls the layout dependency
  permanent while plan 06 calls it a temporary upstream trust entry to remove.
  Plan 06 should settle the policy explicitly.

## 4. Phase 3 — coverage gap map

### Current suite shape

`dotnet-cli/tests/fixtures` contains 177 managed C# fixtures. The build script
discovers every `.cs` file, compiles it, derives the expected `u8` exit code
from the filename suffix, and generates a Rust test. A single debug fixture is
compiled ad hoc. Seven selected fixtures run under both stock `dotnet` and
`dotnet-rs` and compare exact stdout and exit status. Plans 02 and 07 already
track the differential-count ratchet and ambiguous exit-code oracle; neither is
new here.

| Managed fixture category | Count | Surface demonstrated |
| --- | ---: | --- |
| basic | 19 | startup, control flow, console/error messages, spans/strings, JSON smoke, hardware capability probes |
| reflection | 18 | type/member queries, attributes, invoke/byref write-through, runtime handles, arrays/byref types |
| exceptions | 16 | filters, nested filters/finally, leave/rethrow/unwind, stack traces, unhandled paths |
| delegates | 14 | static/instance/virtual/variance/multicast/combine/remove and buffer reuse |
| LINQ | 10 | arrays/lists/iterators, ordering/grouping/set operations/laziness |
| arithmetic | 9 | integer/float/overflow/math behavior |
| async | 9 | completed/faulted Task and ValueTask, one suspension/deferred completion path |
| GC | 9 | finalization, resurrection, weak handles, runtime-handle survival, descriptor tracing |
| threading | 9 | interlocked, volatile, monitor timeout, cross-arena/simple threading |
| interfaces | 8 | variance, MethodImpl, default/static constrained interface dispatch |
| unsafe | 7 | managed-pointer/Unsafe operations and GC safety of spans/ref structs |
| iterators | 7 | yield/break/disposal/finally/exception paths |
| conversions | 6 | checked and numeric conversions |
| fields | 6 | static/instance/property/enum behavior |
| P/Invoke | 5 | Linux libc/libm integer, long, double, and one returned struct |
| statics | 5 | cctor trigger/failure/cycles including a multithreaded cycle |
| structs | 5 | layout, constrained calls, arrays, misalignment |
| arrays | 4 | vectors, literal init, empty indices, one multidimensional case |
| generics | 4 | basic generics, constraints, one method-generic/static-field combination |
| strings | 3 | length, operations, implicit span |
| expressions | 2 | interpreter-backed expression compilation |
| memory / pointers / root | 3 total | nullable boxing, ref-struct stress, one root fixture |

The Rust unit-test distribution is uneven. VM (100), assemblies (51), utils
(49), value (47), and types (36) contain substantial local suites. Runtime
memory has 15. In contrast, runtime resolver, VM ops, exceptions, delegates,
reflection, threading intrinsics, and unsafe intrinsics have zero direct unit
tests; core/span/string intrinsics have only 2/2/6. This matters because the
extracted crates advertise narrow host traits that should make direct tests
possible.

There are four fuzz targets. Only `fuzz_raw_memory_access` corpus replay and
the value Miri leg are blocking in current CI; the remaining fuzz/Miri and
Valgrind workflows are advisory. This is exactly the existing plan-02 scope.
The 18 managed benchmark workloads add realistic hot paths—including EF Core
InMemory—but assert only exit status and performance/metrics, not broad
semantic equivalence.

The completed Newtonsoft.Json and EF Core InMemory host-runner rungs are
positive compatibility evidence and are **not** gaps. The map below starts
beyond them.

### Coverage categories that remain weak

| Category | What is covered | Gap category for a later triage pass |
| --- | --- | --- |
| BCL breadth beyond completed host rungs | LINQ, System.Text.Json DOM smoke, Task/ValueTask basics, expressions, reflection, strings/spans | No file/directory/stream IO, networking/sockets/HTTP, regex/XML/globalization breadth, real scheduler/timer/cancellation behavior, Reflection.Emit, or native-backed EF provider. `COMPATIBILITY.md` already honestly names several of these; reopening userland breadth requires a fresh scope decision, not pretending the old rungs are absent. |
| Generic instantiation | Basic constraints, variance/default-interface dispatch, a static-field/method-generic case, reflection MakeGenericMethod | Sparse coverage of nested/open constructed types, recursive/cyclic constraints, cross-assembly instantiations, multiple closed generic statics under concurrency, generic EH/delegates, and high-arity/state-space interactions. Plan 05 changes descriptor identity and needs a selected subset as regression evidence. |
| GC and threading interleavings | Single-arena finalization/resurrection/weak handle, repeated multi-arena fixtures, volatile/interlocked/monitor timeout, substantial Rust stress tests | No systematic scheduling model (plan 02); no cross-arena weak-handle semantics; limited finalizer/weak/reference races; multi-arena tests often rerun the same `cache_test_0`/`static_ref_42` payload with different thread counts rather than distinct protocols. |
| Exception and unwind | Managed fixtures cover the main ECMA paths well | No direct state-machine suite in `dotnet-exceptions`; malformed/overlapping EH metadata and transition invariants are mostly reached only through fuzz/integration. Unhandled status remains oracle-ambiguous until plan 07. |
| Delegate and reflection | Strong managed smoke breadth, including multicast ordering and byref reflection invoke | No direct fake-host tests despite extracted ports; limited binder/BindingFlags matrix, static reflection field get/set (source contains explicit fallback/not-implemented behavior), cross-assembly member identity, concurrent registry construction, and reflection/delegate interaction across more generic shapes. Reflection.Emit remains out of scope. |
| P/Invoke | Five Linux system-library calls and several Rust tests of temporary buffers/returns | No per-thread last-error semantics, Windows/macOS ABI legs, calling-convention matrix, bool/char/string/array/out/ref marshalling breadth, callbacks/reverse P/Invoke, native error/write-back failures, or CoreCLR differential oracle. The current `static mut` makes concurrent coverage unsafe rather than merely absent. |
| Monitor/thread APIs | Enter/Exit/TryEnter and volatile/interlocked paths | `Wait`, `Pulse`, and `PulseAll` are explicitly unimplemented and untested; interrupt/abort limitations are already documented. Real asynchronous scheduling is not represented by completed-Task fixtures. |
| Host/loading | Framework-dependent app path, deps/runtimeconfig parsing, NuGet probing logic, Newtonsoft and EF InMemory | Multiple load contexts, roll-forward/version-policy combinations, native asset RID variation, and application isolation remain outside the validated surface. Single load context is an explicit planned deviation, not an accidental omission. |
| Platform/configuration matrix | CI feature matrix covers default, multithreading, validation, and fuzzing configurations | Managed P/Invoke fixtures are Unix-specific; many production environment knobs have no invalid-value/default tests or docs contract; benchmark knobs are not part of a configuration registry. |

### Tests coupled to implementation accidents

Most C# fixtures test managed observable behavior and are appropriately black
box. A small set of Rust integration tests instead asserts implementation
structure:

- `test_cache_observability` requires assembly cache sizes to become nonzero.
- delegate and static-constrained tests assert internal cache hit/miss counters,
  not just dispatch semantics.
- feature-configuration tests mostly assert that a manager/field/variant
  exists or can be touched. `test_multithreading_cross_arena_value` constructs
  an address-only fake `ObjectPtr` solely to prove an enum variant compiles.
- VM-data and value layout-size tests intentionally pin memory footprints; they
  are valid performance/regression sentinels but should not be interpreted as
  ECMA behavior tests.

These tests should not necessarily be deleted. They should be labeled and
located as implementation/performance contract tests so a representation
refactor does not appear to break CLI compatibility. The cache assertions in
particular belong beside cache/resolver tests once those extracted crates have
direct harnesses.

## 5. Phase 4 — prioritized backlog

The list below is one integrated queue ordered by prerequisites and then by
value per unit effort. Existing plans are cited as existing work, not renamed
as new findings. “Gate” always means a repository-observable condition; no row
uses “reviewer is satisfied” as completion.

| Priority | Proposed change and why | Category | Objective gate | Effort / risk / dependencies | Relation to plans 01–08 |
| ---: | --- | --- | --- | --- | --- |
| 1 | **Completed 2026-08-31 — eliminate the process-global P/Invoke last-error slot.** `ArenaLocalState` now owns an initialized `pinvoke_last_error: i32`, and Marshal Get/Set access only the current executor's `self.local` cell. This removes the shared mutable-static/data-race premise rather than documenting it more strongly. | complexity-justification | **Met.** No mutable static stores the runtime cache; `F11.PInvokeLastErrorArenaLocal` records the factual arena-local invariant. The normal `pinvoke_last_error_isolation` managed-Thread fixture is `#[cfg(feature = "multithreading")]`, and CI's blocking test matrix includes that feature. This migration concerns the runtime cache only, not native `errno`/`GetLastError` capture. | **S–M / high correctness risk, low migration breadth. Completed.** No prerequisite. The later ABI matrix remains priority 14. | **New, now complete.** Do not add the eliminated premise to plan 06. |
| 2 | **Completed 2026-08-30 — repair queue status drift and check it mechanically.** The README now reports plan 01 as complete, plan 08 uses the canonical status-header form, and the blocking doc-drift job compares every queue row with its plan file. This keeps dependency and priority discussions anchored to one state. | intelligibility/structure | `docs/plans/README.md` and all numbered plan files report the same canonical status; CI fails on a deliberately introduced mismatch. **Met during plan-09 promotion.** | **XS / low. Completed.** No prerequisite. Documentation/tooling only. | **New queue-maintenance finding, now complete;** it did not reopen or change any plan gate. |
| 3 | **Completed 2026-08-31 — extend plan 01's predicate-citation gate across the workspace.** The completed four-crate Plan-01 work remains intact; 220 retained formerly uncited production comments and six package-owned benchmark comments now carry predicates whose local witnesses establish the cited claim, while three unprovable environment-mutation sites were eliminated. `check_doc_drift.sh` mechanically discovers package-owned Rust targets and has a blocking negative-drift harness. | intelligibility/structure | **Met.** Every discovered package-owned `// SAFETY:` comment is cited (709 / 709); every cited predicate is registered and every registry predicate is cited. The top-level gate and independent negative harness pass, as do the queried xtask feature-matrix configurations, including `multithreading,validation-all` and `fuzzing`. | **M / medium review risk, completed.** No prerequisite; supplies names to 04/06. | **Workspace-wide extension of completed plan 01.** It widens the policy without changing Plan 01's completed historical four-crate scope. |
| 4 | **Completed 2026-08-31 — width-generic atomic access was completed by [plan 03](03-width-generic-atomics.md).** Its sealed `W1`/`W2`/`W4`/`W8` markers and one dynamic bridge replace the nine width ladders, making an inconsistent width/representation pair unrepresentable at the typed API boundary. | complexity-justification | **Met in plan 03.** The plan's completion evidence records the sealed width-marker implementation, dynamic CTS-size bridge, and passing typed atomic tests across all four widths and both feature configurations. | **M / medium, completed.** No prerequisite. No atomic implementation is re-executed by this review. | **Existing plan 03, complete and unchanged.** This status reconciliation neither reopens nor duplicates its scope. |
| 5 | **Make `dotnet-value` the sole owner of managed-pointer serialization and rebasing.** Add an encoding/rebase operation that `EvaluationStack` can call instead of reconstructing Stack tag bits, slot masks, packed offsets, and checksums. The stack must still repair pointers; it should not know their wire format. | code quality (repetition / abstraction / maintainability) | No production file outside `dotnet-value/src/pointer/` contains managed-pointer tag/mask/shift/checksum encoding literals; VM-data's value-type fixup calls the value-layer API; pointer serde/fixup/fuzz regression suites cover stack offsets before and after stack reallocation. | **S–M / high representation risk**, so preserve the existing serialized format. No prerequisite. | **New and compatible with parked plan 08.** It refines ownership of the completed redesign and does not reopen strict-provenance testing. |
| 6 | **Close the `GenericLookup` mutation hole, then run plan 05.** Privatize `type_generics` and `method_generics` immediately (read-only accessors are sufficient for all current external reads) so no caller can desynchronize `Hash` from equality. Treat this as plan 05's first safety/correctness step before replacing cloned structural keys with interned IDs. | code quality (abstraction / maintainability) | Zero external direct field accesses; the argument slices are private and replaceable only through hash-refreshing constructors/setters; hash/equality mutation regression tests pass; then plan 05's original gates reach zero mutable-key allows and zero production key-clone counters. | **S** for privatization, **L / medium-high** for full interning. No hard prerequisite. | **Extends existing plan 05;** does not supersede its identity/performance work. |
| 7 | **Run plan 07 before expanding differential coverage.** Reserve unambiguous harness outcomes and add exit-code-only comparison first; otherwise a larger differential corpus inherits an oracle known to conflate managed assertion failures, unhandled exceptions, and setup/executor failures. This is a sequencing refinement, not disagreement with plan 07's technical scope. | test-coverage-prep | The exact plan-07 gate: harness outcomes use a reserved high code band and opt-in exit-code-only CoreCLR comparison exists. Plan-02/04 differential floors consume that mode. | **S / low.** No implementation prerequisite. Make it a practical prerequisite for plan 02 instrument 5 and plan 04's differential ratchet. | **Existing plan 07.** README calls it independent; this review agrees technically but recommends sequencing it before differential expansion. |
| 8 | **Execute the falsifier portfolio.** The strong local concurrency protocols currently rely on stress tests and reasoning, while three fuzz targets, non-value Miri, and Valgrind remain advisory. Implement loom/STW first, then fix/promote the known-red fuzz targets, add the scoped pure-value harnesses, guard-off leg, and differential floor. | complexity-justification | The exact plan-02 gate: blocking loom STW leg, all four fuzz targets blocking, Kani harnesses for F3/F4/F9, and guard-off evidence; differential floor added after priority 7. | **L / high**, naturally incremental. No hard prerequisite for loom; differential instrument depends on priority 7. Plan 03 may retire an F4 harness as intended. | **Existing plan 02.** This review confirms it is the highest-value broad assurance item. |
| 9 | **Create one runtime-configuration contract and owner.** Inventory production, test, and benchmark knobs with default, parser, allowed values, latching time, and owning subsystem. Move the generic boolean parser out of `value::string`, consolidate the three GC-pointer trace latches, and make inconsistent invalid-value handling explicit rather than accidental. This is a registry/config module, not a universal settings object passed through every hot path. | intelligibility/structure | A drift check accounts for every `DOTNET_*`/`DOTNET_RS_*` literal in Rust as production, test, build, or benchmark configuration; each production row documents owner/default/accepted values/latching; no generic env parser is exported from the string-value module; `DOTNET_TRACE_GC_PTR_READ` is parsed once. | **M / low-medium.** No prerequisite. Preserve externally used names and behavior unless a row explicitly declares a migration. | **Genuinely new.** Complements, but does not overlap, plan 05's cache representation work. |
| 10 | **Document the actual ports/contracts and ownership graph before moving more code.** Update the architecture diagram with real direct internal edges and label VM ops as the ports layer; add a concise concern-to-owner map for metadata resolution, semantic resolution, layout data/computation, errors, exceptions, and host probing. Correct stale VM-ops/VM-data headers, the moving-GC sentence, and deleted `REVIEW.md` references. Do not introduce a new crate solely for diagram purity. | intelligibility/structure | `cargo metadata`'s internal normal edges are all represented in a checked/generated architecture artifact; no source/doc text claims listed vm-ops traits live elsewhere; no `REVIEW.md` reference remains; moving/non-moving GC statements agree; each boundary named above has exactly one documented behavior owner. | **S–M / low.** No prerequisite, but perform before any ports/error crate refactor. | **New documentation/ownership item.** The assumed BCL-layout probes should feed plan 06 where appropriate. |
| 11 | **Finish adoption of named intrinsic argument policies.** Inventory object/null/type extraction in every intrinsic crate, express genuine semantic variants as named `ArgPolicy`-like choices, and route equivalent cases through shared functions. Avoid one control-flow macro that hides borrow lifetimes; the goal is explicit policy and uniform managed error behavior. | code quality (repetition / maintainability) | An inventory maps every object-consuming intrinsic parameter to a named null/type policy; equivalent extraction paths use the shared helper; tests demonstrate each policy's null, wrong-type, and valid behavior; no duplicate helper with the same policy remains in another intrinsic crate. | **M / medium behavior risk** because null and type errors are managed-observable. Priority 7 improves the oracle; priority 12 supplies direct harnesses. | **Genuinely new.** Accepted SIMD, PointerOrigin, and call-dispatch duplication stay out of scope. |
| 12 | **Realign the two worst module catch-alls.** Split reflection custom-attribute materialization, type-member lookup/binding, and property selection out of `types/type_members.rs`; split VM intrinsic host adapters out of `stack/context_ops.rs` by connected subsystem while retaining one discoverable adapter index. This follows existing conceptual boundaries and changes no crate graph. | intelligibility/structure | `types/type_members.rs` no longer owns both attribute materialization and member/property overload selection (or is deleted); `context_ops.rs` contains only common/base context operations and links to per-subsystem adapters; generated dispatch paths and public handler signatures are unchanged; the full existing feature test matrix passes. | **M / low semantic, medium merge risk.** Prefer after priority 10 names the owners. | **Genuinely new.** It is structural cleanup, not another extraction plan. |
| 13 | **Make extracted behavior crates independently testable—or reduce their ports.** Use the existing host traits to build small reusable fake contexts for resolver, exception, delegate, reflection, threading, and unsafe-intrinsic behavior. Where a micro-trait cannot support a direct test and only forwards one `VesContext` method, consolidate it into the nearest meaningful port. This makes abstraction granularity answerable by evidence. | test-coverage-prep | Every extracted crate with behavioral logic and a host/context port has at least one direct non-VM suite covering success and error/control paths, or the unused test seam has been removed/consolidated; zero-test crate count for runtime-resolver, exceptions, delegates, reflection, threading, and unsafe intrinsics is zero. | **L / medium.** Depends on priority 10's boundary map; priorities 11/12 can use the harnesses. | **Genuinely new.** Supplies evidence around plans 02/04 rather than replacing them. |
| 14 | **Add a portable P/Invoke ABI coverage matrix after fixing state.** Define categories for scalar widths/signs, floating point, structs/alignment, byref/out write-back, strings/arrays, failure paths, calling conventions, platform ABIs, callbacks if supported, and last-error isolation. Use a repository native helper where system libraries are not a stable oracle and compare eligible cases with stock .NET. | test-coverage-prep | A checked manifest maps every supported marshalling category to a managed/native fixture and every unsupported category to an explicit compatibility entry; Linux and every CI-supported non-Linux target run their applicable legs; eligible fixtures use the plan-07 differential mode; last-error concurrency is blocking. | **L / high platform risk.** Blocked on priority 1; uses priority 7. | **Genuinely new.** Its trust/deviation results feed plan 06's existing FFI-ABI rows. |
| 15 | **Resolve the documented cross-arena weak-reference limitation.** The current simple model is insufficient: per-arena finalization can clear local weak handles, but the global fixed-point collector does not track/clear weak references correctly across arenas. Specify short versus track-resurrection behavior at the coordinator boundary before implementation and add schedule-controlled evidence. | complexity-justification | Under `multithreading`, cross-arena `Weak` clears before finalization/resurrection and `WeakTrackResurrection` follows its documented lifetime across collection epochs; no strong root is introduced solely to implement weakness; the limitation is removed from `GC_AND_MEMORY_SAFETY.md`; blocking tests cover arena unregister/collection interaction. | **L–XL / high GC risk.** Depends on priority 8's STW/loom seam and should reuse its schedule control. | **New queued disposition for an already documented limitation.** Not a newly discovered limitation. |
| 16 | **Complete ECMA correspondence and the trust register after their prerequisites.** Build the clause-to-site index and differential floor, then register only irreducible assumptions with falsifiers and a CI ceiling. Reconcile the `TypeIndex` transmute's “permanent” source comment with plan 06's explicit upstream-fix policy; move the deleted `F-SCOPE-001` BCL-layout assumptions into a live artifact. | complexity-justification | Plan 04 and plan 06's existing gates both pass; every assumed registry predicate maps to a trust row/falsifier; no source cites deleted review IDs; the `TypeIndex` layout assumption is either eliminated upstream or consistently registered as temporary. | **L / medium.** Plan 04 depends on priority 3/plan 01; plan 06 depends on priority 3 and priority 8. Priority 10 supplies doc cleanup. | **Existing plans 04 and 06, extended only to reconcile concrete stale references.** |

### Items intentionally not queued by this review

- **Plan 08 remains parked.** Nothing above authorizes a strict-provenance run
  or changes its owner-directed disposition.
- **No blanket “split every large file” pass.** Runtime memory access, GC
  coordination, call dispatch, type comparison, and P/Invoke ABI code have
  cohesive reasons to remain centralized until a named responsibility can be
  extracted.
- **No universal cache/host/error trait framework.** The present differences
  often encode ownership and contention. Documentation and direct tests should
  precede more abstraction.
- **No automatic new compatibility campaign.** The Phase-3 gap map is input to
  an owner scoping decision. Completed Newtonsoft and EF InMemory rungs stay
  counted as completed, while networking, Reflection.Emit, real async
  scheduling, EF SQLite/native providers, and broad IO remain explicitly
  outside the validated surface.
- **No proof assistant or DSL.** Plans 01/02/04/06 remain the appropriately
  sized predicate/falsifier/trust mechanisms for this repository.

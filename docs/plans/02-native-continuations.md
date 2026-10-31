# Native continuations (async/await as VM effects) implementation plan

**Status:** Not started
**Prepared:** 2026-10-31
**Baseline:** `783c246b`
**Scope:** Effectful native intrinsics expressed as `'static` Rust futures polled by the executor:
vm-data/vm-ops surface, intrinsic ports, managed re-entry, exception boundaries, condition waits and
multithreaded enablement.

Replace ad-hoc `StepResult::Yield` + retry-from-scratch suspensions in selected intrinsics with native
continuations: an async body acquires VM access through `let mut vm = fx.vm().await?` (an RAII loan of
rooted views, never a bare `Gc`), stashes references that cross an `.await` in per-activation
`DynamicRootSet`s, and performs one effect per suspension (`SafePoint`, `CallManaged`, waits) that the
executor handles. Futures are polled inside the executor's existing mutation, first inline at spawn,
from a traced per-thread native stack. This is not an OS async runtime and not a .NET `Task`
implementation; awaiting external futures is a non-goal.

## Design record

All design decisions (D1–D12), rejected alternatives, the effect catalogue, exception and stash
models, the unsafe inventory and measured costs live in
[`experiments/async-effects/DESIGN.md`](../../experiments/async-effects/DESIGN.md). The executable
evidence is the layered prototype (`experiments/async-effects/layered/`, intrinsics depend only on
ops) and the [syntax gallery](../../experiments/async-effects/gallery/README.md). Do not re-open the
public API (`VmEffects` / `vm().await` / `VmAccess`) during implementation; the gates below are
required tests, not reasons to re-decide syntax.

Hard gates carried from the design (see its "Remaining risks and hard gates"):

1. **Foreign-arena lifetime / root publication.** MT native registrations stay disabled until plan 01
   M1e/M1g supply root publication and arena identity; no fabricated cross-arena brand or long-held
   `ArenaLease`.
2. **Real SEH integration.** Two-pass search, filters, finally ordering, `HandlerUnwinds` and suspended
   frame restoration need VM conformance tests (stage 2.5) before native work runs in those contexts.
3. **Cost and eligibility.** Non-suspending intrinsics gain zero poll/region/allocation work; the batched
   dispatch loop (`dispatch/mod.rs`) is unchanged. Prototype costs: `vm()` acquisition ≈ 6–9 ns,
   spawn + synchronous completion ≈ 51–66 ns, spawn + one suspension ≈ 74–95 ns. Real-adapter numbers
   from stage 1.5 set the budget for further migration.

## Dependencies

| Stage | Depends on |
|---|---|
| 1 — chunked string spike (ST only) | None |
| 2 — managed re-entry and exception boundaries | Stage 1 |
| 3 — condition effects and diagnostics | Stage 2 (finalizer waits run managed children) |
| 4 — multithreading and expansion | Stage 3; plan 01 M1e/M1g for 4.1 |

Plan 01 M5 ("retain owners, guards, operands and progress across chunk safepoints") may adopt the
stage 1 mechanism as its saved continuation for partial bulk operations; coordinate rather than
building a second resumption scheme.

## Stages

Each stage is a dependency-ordered vertical slice, sliced by crate order (vm-data → vm-ops →
intrinsic crates → vm → tests). Do not create semantic-axis tickets that change every crate before
any usable path exists. Do not put an unimplemented native entry in the generated table: introduce
its body as an unregistered helper, then enable the annotated shim when the VM adapter is present.
Old paths remain buildable in every stage, and `StepResult::Yield`/retry is never removed. Step IDs
`N.M` feed the orchestrated refactor workflow directly.

### Stage 1 — Minimal vertical spike: one chunked string operation

**1.1 — vm-data.** Add native IDs/header, per-frame roots data, call-site/stack-base/resume-IP
metadata and a distinct native-suspension result. Keep VmContinuation's old variants and
handler storage unchanged. No managed re-entry or wait registrations yet.

**1.2 — vm-ops.** Add concrete VmEffects, async `vm()` acquisition, VmAccess RAII loan,
rooted string view, internal Region primitive, root cells and typed string handle,
SafePoint-only handler request, Collect wrapper and NativeSpawnOps. Keep the erased slot
private; add registry predicate/citations/size assertions and compile-fail tests with real
lock/scope types. Provide a restricted adapter contract only for string allocation and stop
checks; enforce the 32-acquisition native quantum and keep guard Drop free of VM access.
Do not expose VesInternals. No real VM dependency is introduced into intrinsic crates.

**1.3 — string intrinsic crate.** Add unregistered substring async body and explicit shim
helper, preserving overload/null/range behavior, chunk size and pop trace accounting. Use
`fx.vm().await?` scopes and typed views for validation/copy/result creation; no direct
`fx.enter` calls in migrated bodies. Retain
legacy registration for MT and for all other handlers. Add body tests against a mock host.

**1.4 — vm.** Implement adapter from fresh VesContext/GCHandle, native frame stack in
ThreadContext, inline first poll, effect return to existing executor handshake, completion
helper with exactly-once push/IP advancement, and terminal abort/budget cleanup. Enable
this one ST registration; add guarded transition diagnostics.

**1.5 — vm integration tests/benchmarks.** Force a stop request after *each* copied chunk;
assert exact output, copied-count = length, bounded completion, progress under repeated
requests, no duplicate argument consumption and no unsafe borrow while parked. Include
first-poll-ready, null/range errors, zero-length, guard/view escape negatives, forgotten
access loans, post-teardown rejection, and abort/fuzz-budget teardown. Verify a loop with no
GC request yields after at most 32 acquisitions and can be cancelled before another scope. Run pinned
Miri on the bridge and real relevant VM unsafe gates per CI.md; measure real adapter and
spawn budget plus dispatch baseline. This completes the first slice before stage 2 starts.

### Stage 2 — Managed re-entry and exception boundaries

**2.1 — vm-data.** Add managed return-to-native destinations, logical native boundary data,
Capture/Transparent propagation and `UnwindTarget::Native`. Carry stable linkage through
frame suspension/recycling. Add traced exception mailbox payload types.

**2.2 — vm-ops, then dotnet-exceptions.** Add owned call plans, result/Thrown rehoming and
capture/transparent request semantics. Extend search, section-exit and unwind completion
for native boundaries; preserve ambient context and existing nested-filter rules. Unit-test
first/second-pass ordering with a context mock before connecting intrinsic registrations.

**2.3 — reflection/delegate intrinsic crates.** Move only required reflection host declarations
to vm-ops with reexports; narrow marshaling helper bounds. Add unregistered ports, explicitly
Transparent for reflection and multicast, preserving existing filter timing. Preserve byref box identity and
array-slot write-through, result boxing, constructor receiver roots and single-target fast
path. Do not accept general byref/TypedReference returns or argument layouts.

**2.4 — vm.** Connect managed frame push/return, exception search/unwind, cctor wrapping,
trace capture and native retirement. Rehome handles before dropping roots. Apply eligibility
before any operands are consumed; keep active-filter/handler contexts legacy initially.

**2.5 — conformance gate.** Test catch inside child, catch inside Rust Capture, native `?`
propagation, transparent outer filters *before* child finally, Capture outer filters *after*
child finally, exception from finally replacing prior exception, cctor failures, nested
managed/native calls, reflection byref writes on success/throw, reference-type byref
assignment, constructor/void/value/reference returns and trace preservation. Then test native
work invoked inside filters/finally, including suspended-frame restoration and HandlerUnwinds.
Do not enable those contexts before their tests pass. Fix or explicitly exclude F-LIFE-004
cases; no claim that native continuation work repairs it automatically.

### Stage 3 — Condition effects and diagnostics

**3.1 — vm-data.** Add owned static-init keys, wait reason/generation/deadline data and native
progress metadata. No VM sync-block type crosses into vm-data/vm-ops.

**3.2 — vm-ops, tracer/metrics.** Add WaitFinalizers/MonitorAcquire/WaitStaticInit, precise
unit/bool outcomes, checked heap-location descriptors, and structured native transition events.
Keep Sleep/Join deferred. Add no callback that borrows VesContext across wait.

**3.3 — threading/reflection/string intrinsic crates.** Add explicit new entry helpers only
for eligible overloads. ReliableEnter/ref-bool, raw arbitrary byrefs and single-target
delegates remain legacy. Keep absolute timeout in each activation, not thread-local state.

**3.4 — vm.** Implement ready probes and owned wait registrations; service finalizer child
frames; preserve static graph cycle/recursion policy and cleanup; complete monitor acquisition
once. Add separate native transition ring and state dumps. MT native registrations remain
disabled pending stage 4; this stage's effect/handler definitions must still compile there.

**3.5 — wait tests.** Exercise predicate-before-register races, spurious notifications,
timeout across repeated GC stops, abort during wait, cancellation after wake, static cycle
handling and owner-failure publication, finalizers which call managed/native code, and
self-finalizer wait policy. ST never reaches a Condvar wait; retain unsupported ST thread
behavior. These require actual VM tests, not merely the deterministic mock in this experiment.

### Stage 4 — Multithreading and controlled expansion

**4.1 — lower GC/value/util crates, only through [plan 01](01-pointer-provenance.md).** Establish
M1e/M1g-compatible foreign root publication, owner teardown/generation identity and
incremental barrier semantics. Add local root-cell→foreign ObjectRef retention tests under
coordinated GC, owner exit, ID reuse, cancellation and forced stops. Keep leases scoped to
access, never across parked waits. This is a dependency gate, not a side repair hidden in
async intrinsic work.

**4.2 — vm-data / vm-ops.** Adopt the proven identity/root protocol without changing VmEffects
syntax; update traced cells/handles if required, and add validation-mode assertions. Check
native frame teardown while outstanding foreign roots exist.

**4.3 — intrinsic crates.** Enable eligible native registrations for MT only after the lower
protocol passes. Preserve legacy selection where supported managed signatures contain
unrepresentable byrefs or transparent-handler contexts remain unproven.

**4.4 — vm.** Run actual coordinated waits/STW with native activations, feature combinations
`multithreading`, `memory-validation`, `fuzzing`, `bench-instrumentation`, and the relevant
CI-native/Miri/fuzz replay gates from CONTRIBUTING and CI.md. Run paired performance/memory
benchmarks from BENCHMARK_WORKFLOW.md, including JSON/EF workloads and tracing overhead.

**4.5 — further migration review.** Migrate additional chunked loops only with a checked
owner/offset model and progress tests. Extend stable stack/static/byref support only after
the provenance plan supplies it. No phase removes StepResult::Yield/retry support, and no
phase requires external async I/O or .NET Task integration.

## Execution and reporting

Each step runs `check.sh --fast` across the three feature configurations and
`scripts/check_doc_drift.sh`; new `unsafe` follows the `F*` citation scheme in
[SAFETY_CITATION_AUDIT](../SAFETY_CITATION_AUDIT.md) with registry rows added alongside the code.
Track stage progress here and update the [queue](README.md) when the overall status changes.

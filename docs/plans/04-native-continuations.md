# Native continuations: resumable Substring

**Status:** Complete (initial continuation slice; runtime path superseded by plan 08)

Retained copy progress, rooting, managed finalizer execution and dispatch refusal
controls pass. Resource counters and process-heap profiling are implemented;
the maintainer accepted the measured large-call cost.

**Outcome:** Both `System.String.Substring` overloads retain copy progress across real
single-threaded GC safepoints, consume operands once and copy each requested UTF-16
code unit once.

This document records the first Substring continuation implementation and its evidence,
not the current runtime wiring. The continuation was removed when plan 08 moved Substring
to `substring_async` and the native activation stack. Plan 08 describes the current path,
including finalizer segment isolation.

## Direction

Start with an explicit continuation to prove scheduling, rooting and completion.
The intended destination remains an async/await-compatible effects system that makes
instructions and intrinsics simpler to implement and reduces unnecessary data shuffling.
The explicit continuation is the first proof, not a permanent general effects API.

The archived experiment at `archive/async-effects-2026-10` contains candidate APIs,
not an implementation to restore wholesale. Its design starts at
`experiments/async-effects/DESIGN.md` in that tag. This work does not implement .NET
Task scheduling or an OS async runtime.

## Problem

`intrinsic_string_substring` in `dotnet-intrinsics-string/src/search.rs` peeks its
operands, allocates a vector and copies in 64K-character chunks. On `Yield` it drops
that vector and offset, so retry repeats the work. The single-threaded stop predicate
does not observe `needs_full_collect`; tests must request real executor collection,
rather than substitute a boolean for collection.

## Scope and entry

Enable the continuation without `multithreading`, for ordinary non-tail `call` and
`callvirt`, when the requested result exceeds 64K UTF-16 code units. Empty and
single-chunk results stay synchronous. Indirect calls, reflection, delegates, tail
calls, engine API calls and multithreaded execution retain the existing path.

Keep both registered signatures and one annotated handler. Add a narrow string-host
hook offering validated work to the VM. Declining the offer leaves operands intact.
Set eligibility explicitly in ordinary instruction-call dispatch; the resolved method
alone is insufficient to establish the return contract. Do not depend on the scanner
understanding feature gates or register duplicate handlers.

Validate receiver and signed ranges before installing state or consuming operands.
Use subtraction-based bounds checks; preserve exception types and parameter names,
with CLR comparisons. Share validation and bounded-copy helpers between paths.

## Retained state and execution

- Add one optional, traced native activation to thread execution state, separate from
  `VmContinuation::HandlerUnwinds`. This finite operation performs no managed re-entry.
- Retain the source `ObjectRef`, start, length, owned `Vec<u16>`, copied offset and
  originating caller/call-site identity. Derive tracing. Retain no heap slices,
  borrow guards or raw pointers.
- Allocate the buffer once. After validation, commit the activation and consume the
  receiver and arguments once, before copying.
- Copy at most 64K UTF-16 code units per executor turn, including the initial turn.
  If incomplete, return `Yield` unconditionally to bound abort and budget latency.
- Resume pending native work at engine entry before further IL. Leave the caller IP
  at the call instruction while pending; never restore its operand snapshot or use
  the instruction-retry marker. Poll outside the ordinary batched instruction loop.
- On completion, move the buffer into the result string, push one result, advance
  the caller IP once and clear the activation. Caller/call-site mismatch clears the
  activation and produces an internal error.
- End every source borrow before yielding. Observe the real single-threaded full-GC
  request without consuming it; acknowledgment belongs to executor collection.
- The initial continuation implementation retained queued finalizers while native work
  was pending and deferred their frames. Plan 08 superseded this scheduling with isolated
  finalizer segments that can run while a native activation is parked. Preserve ambient
  handler/filter and suspended handler-unwind state.
- Clear the activation on terminal abort, budget exhaustion, VM failure and executor
  teardown, including early returns from `Executor::run`. Add no lifetime erasure,
  poll-local context slot or new unsafe code.

## Slice and validation

Deliver one enabled vertical slice containing entry, retained state, executor
resumption, cleanup and tests. Do not merge preparatory infrastructure on its own.

- Compare both overloads with CLR: empty/full/partial ranges, null receiver, negative
  and overflowing arguments, chunk boundaries, surrogate pairs and unpaired surrogates.
- Use a deterministic test hook to request actual collection after every incomplete
  chunk. Assert collection before resumption, exact output, monotonic progress, one
  buffer allocation and exactly the requested number of copied code units.
- Root the source only through the activation and collect repeatedly. Use a weak
  witness to verify collection eligibility after completion and cancellation.
- Without GC pressure, verify every incomplete chunk returns to the executor. Abort
  or exhaust the budget between chunks; assert no further copying and state cleanup.
- Cover pending finalizers, catch/finally/filter contexts, teardown while suspended,
  back-to-back calls and result-stack shape. Preserve handler-unwind state.
- Test fallback call shapes and multithreaded execution: neither installs an activation.
- Run focused tests, then `./check.sh --fast` once for the coherent implementation.
  Register the rooting/resumption control in the existing pinned Miri manifests and
  run Tree Borrows, including the fallback configuration. Run doc drift checks.

## Performance and done conditions

Record a pre-implementation baseline. Run paired release comparisons for empty/small
and multi-chunk Substring, ordinary dispatch and the existing string workload. Report
time, copied units, buffer allocations and peak retained bytes. Measure forced
collection separately from ordinary throughput.

Investigate repeatable regressions above 1% in unaffected dispatch and direct-path
controls. Report scheduling overhead alongside progress preservation. Ask the
maintainer to accept any unresolved regression.

Done when correctness, rooting, cancellation and fallback checks pass; performance
evidence is recorded; and eligible calls use the continuation by default. The later
async/await effects redesign remains a separate outcome using this scheduler contract
and its behavior tests.

## Initial implementation validation

The initial implementation used a boxed `CallState` continuation for eligible calls;
it retained a traced source, output buffer, offset and caller identity separately from
handler unwind state. Plan 08 later removed that continuation in favor of
`substring_async` and native activations. The measurements and tests below describe the
initial implementation at their recorded commits, not the current runtime path. No
unsafe lifetime erasure or managed re-entry was added. Native tests
cover real collection between chunks, exact progress, stable buffer storage, one
completion, weak-source survival/reclamation, abort/budget cleanup, call-site mismatch,
queue deferral and teardown. p44 (shard 44, batch 3) passed pinned Tree Borrows Miri
without features and with multithreading; the latter checks refusal with intact operands.
These bounded controls are not a complete soundness proof.

Both managed fixtures have CLR oracles for UTF-16/ranges/handler contexts and
reflection/delegate fallback. Range throws populate `ArgumentException._paramName`;
all seven checked names match CLR. The delegate exception/retry failure was repaired
in `2b239a5f` by discarding backing evaluation operands on catch entry. The enabled
regression then passed all three fast configurations (632/744/759 tests respectively).
The older audit's parameter-name and delegate-retry failures are resolved.

The maintainer accepted the repeatable large-call throughput regression on
2026-11-05 (+4.30% mean across four pairs), considering the threshold tuning and
the value of this scheduler contract for testing and deploying the future effects
system. No repeatable >1% unaffected-control regression was established. All
completion conditions are satisfied.

## Completion controls

`pending_finalizer_runs_in_separate_segment_during_copy` queues a real object while native
work is parked, runs its virtual managed `Finalize` method in an isolated segment and returns
to the parked caller with the expected result. This deterministic scheduler control exercises
the plan 08 segment-isolation behavior, rather than a CLR finalizer timing comparison.

`substring_dispatch_admits_only_inline_calls` executes both overloads through inline, tail
and engine-API dispatch; only the inline path is admitted. The runtime has no registered
`calli` handler: `indirect_opcode_refuses_without_consuming_operands` executes normal and
tail-prefixed `calli` through the production registry, verifies `NotImplemented`, and verifies
intact operands, caller IP and absent activation. Supporting indirect calls remains outside this
slice; generic dispatch is not claimed as proof of an indirect-call fallback.

The existing `bench-instrumentation` feature now enables per-thread substring
counters at buffer allocation and actual chunk append boundaries on both paths.
For each overload and dispatch entry, a 131,089-unit call measures 131,089 copied
units, one buffer allocation, zero reallocations and a largest copy-buffer capacity
of 262,178 bytes. The forced-collection control measures the same totals after two
collections; abort after one chunk measures exactly 65,536 copied units. These
counters exclude inputs, result objects, allocator overhead and GC bookkeeping;
`max_buffer_bytes` is not presented as heap-wide peak memory.

Reproduce with `cargo test -p dotnet-vm --no-default-features --features
bench-instrumentation --lib native_substring_tests -- --nocapture`. The feature is
disabled for throughput comparisons. The ignored release control
`measure_whole_copy_with_and_without_collection` reports admission-through-result
latency with and without two collections, excluding input/caller construction.

Process heap profiling uses Valgrind Massif on each release `substring` benchmark
executable in `--test` mode (`--time-unit=B --max-snapshots=100`). Baseline and
candidate use byte-identical compiled small/large fixtures. Peak live allocated
heap was 238,036,434 bytes baseline and 239,091,826 bytes candidate; allocator
bookkeeping at those snapshots was 8,513,166 and 8,770,062 bytes respectively.
These are whole-process diagnostic peaks including the loader, Criterion harness
and unreclaimed guest allocations, not isolated per-call retained-memory costs.

## Final measurements and acceptance

Raw Criterion samples, mean estimates, resource-counter output, process-heap snapshot
series, fixture hashes and source hashes are retained in [04-evidence.json](04-evidence.json).
Historical baseline `43663231` was extracted into a separate source tree; only the
Substring benchmark declaration, entrypoint and fixtures were copied into that tree.
Both sides used the same toolchain, `bench-fat`, no default features and byte-identical
compiled fixtures. Project checks finished before these four CPU-0-pinned pairs.
Launch order alternated baseline/candidate and candidate/baseline. Each run used ten
Criterion samples, one-second warmup and four-second measurement target.

| Workload | Pair 1 | Pair 2 | Pair 3 | Pair 4 | Mean pair change |
|---|---:|---:|---:|---:|---:|
| Empty/small Substring | +1.16% | -0.30% | +0.21% | -0.39% | +0.17% |
| Multi-chunk Substring | +1.62% | +6.76% | +5.84% | +2.98% | +4.30% |
| Ordinary dispatch | -0.64% | -0.54% | -0.71% | +0.79% | -0.28% |
| String workload | -2.15% | -1.51% | -1.75% | -2.48% | -1.97% |

These are changes between run mean point estimates, not paired confidence intervals.
The empty/small first-pair slowdown did not repeat in either launch order; no repeatable
>1% regression was established in unaffected controls. The large-call regression did
repeat. That workload requests 12,000,000 units across 80 calls; each now returns to
the executor twice before completion (160 additional scheduler turns). Both paths
use the same bounded-copy helper and exact-capacity buffer; the new path retains
progress and roots while returning through executor checks between chunks. The average absolute delta is 0.524 ms per workload run (about 3.28 microseconds
per additional turn if attributed entirely to scheduling). This
explains a concrete source of extra work, but does not isolate every contributor to
the measured delta. The maintainer explicitly accepted this measured cost on
2026-11-05; the raw measurements and their limitations remain recorded here.

A separate isolated optimized native-control run measures admission through result
publication for 131,089 units: median 199.748 microseconds without collection and
196.417 microseconds with two real collections (30 samples each). Input and caller
construction are excluded equally. The modes run consecutively, so their difference
is not a reliable estimate of collection overhead; these are diagnostic latencies,
not a claim that collection improves performance or a throughput comparison with
the historical retry path.

Build benchmark executables in separate baseline/candidate target directories:
`cargo bench --profile bench-fat -p dotnet-benchmarks --no-default-features
--bench substring --bench end_to_end --no-run`. Copy the compiled fixture output
between their `bench-fat/dotnet-bench-fixtures` directories and verify DLL hashes.
Run each generated executable with `taskset -c 0`, setting a distinct `CRITERION_HOME`
per run, and arguments `--bench --sample-size 10 --warm-up-time 1
--measurement-time 4 --noplot`; select each Substring case individually and
`dispatch`/`string` individually for `end_to_end`. Set `NuGetAudit=false` during
fixture preparation to avoid unavailable vulnerability-service requests.

Build the native diagnostic with `cargo test -p dotnet-vm --profile bench-fat
--no-default-features --lib measure_whole_copy_with_and_without_collection --no-run`,
then pin its generated executable and select
`executor::native_substring_tests::measure_whole_copy_with_and_without_collection
--exact --ignored --nocapture --test-threads=1`.

Validation: the fast matrix passed 633/745/760 tests; focused instrumentation controls
passed 11 tests (two manual timings ignored); string helper tests passed seven tests;
instrumented Clippy with warnings denied and doc drift checks passed. Pinned p44
Tree Borrows controls pass in both no-features and multithreading configurations.
Exposed-provenance warnings still limit Miri detection. For p44 run
`bash scripts/pointer_provenance.sh controls --configuration CONFIG --shard 44`.

## Ask the maintainer only if

- A supported call needs managed re-entry or multithreaded continuation support.
- The slice requires new unsafe lifetime erasure or a weaker rooting guarantee.
- Performance investigation leaves a repeatable regression requiring acceptance.
- Integration expands into multiple preparatory modules rather than one enabled slice.

## Lessons from the abandoned attempt

| Issue found | Resolution to evaluate |
|---|---|
| Registration scanner ignored `cfg` and deduplicated IDs | Select the chosen signatures explicitly before arguments are consumed; leave unrelated registrations alone. |
| ST thread-manager stop predicate was a stub | Use the real heap collection request; test against real collections, not mock flags. |
| Safepoint acknowledgment needed branded state | Restore roots, leave the suspending mutation, run outer checks, then acknowledge in the next mutation. |
| Finalizers disturbed a parked caller | The initial continuation deferred finalizer frames; plan 08 later added isolated segments so finalizers can run while native work is parked. |
| Ordinary and indirect calls have different return contracts | Fall back to the legacy path before consuming operands for any call shape not yet implemented. |
| Polling `Ready` depends on completion handling | Build them together. |
| Prototype costs excluded real context/root work | Measure the real path, including retained allocations. |

## Safety requirements for any design

- Futures are `'static` and hold no unconfined GC-branded values or heap borrows. Anything
  carried across suspension is a traced root.
- Nothing held while parked prevents collection (no lock, lease or GC access scope).
- Operands are consumed once; progress, result and IP ownership are unambiguous.
- Cancellation, failure, return and teardown release futures, buffers and roots.

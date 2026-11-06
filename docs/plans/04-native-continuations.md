# Native continuations: resumable Substring

**Status:** In progress (finalizer, fallback coverage and performance evidence remain)

The [completion audit](../ci-evidence/2026-11-05-plans03-04-completion-audit.md)
records the historical parameter-name failure and remaining acceptance gaps. The
parameter-name repair is recorded in the implementation evidence. Retained copy
progress and rooting controls pass; the complete exit conditions are not satisfied.
**Outcome:** Both `System.String.Substring` overloads retain copy progress across real
single-threaded GC safepoints, consume operands once and copy each requested UTF-16
code unit once.

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
- Retain queued finalizers while native work is pending, but defer starting their
  frames. Resume finalizer processing after completion and before the next guest
  instruction. Preserve ambient handler/filter and suspended handler-unwind state.
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

Validation and measured costs are recorded in the
[implementation evidence](../ci-evidence/2026-11-05-substring-continuation.md) and
[performance evidence](../perf-evidence/2026-11-05-substring-continuation.md).

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
| Finalizers disturbed a parked caller | For a finite ST operation, keep queued finalizer work rooted and defer new finalizer frames until the call finishes. |
| Ordinary and indirect calls have different return contracts | Fall back to the legacy path before consuming operands for any call shape not yet implemented. |
| Polling `Ready` depends on completion handling | Build them together. |
| Prototype costs excluded real context/root work | Measure the real path, including retained allocations. |

## Safety requirements for any design

- Futures are `'static` and hold no unconfined GC-branded values or heap borrows. Anything
  carried across suspension is a traced root.
- Nothing held while parked prevents collection (no lock, lease or GC access scope).
- Operands are consumed once; progress, result and IP ownership are unambiguous.
- Cancellation, failure, return and teardown release futures, buffers and roots.

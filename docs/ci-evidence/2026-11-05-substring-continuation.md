# Resumable Substring implementation evidence

The explicit single-threaded Substring slice is enabled for ordinary non-tail call
instructions and both overloads when the result exceeds 65,536 UTF-16 code units.
Short calls and unsupported call shapes retain the synchronous path. The selected
scope is complete; an async/await effects API remains future work.

## Implementation and ownership

`CallState` groups the existing argument scratch buffer, admission flag and optional
boxed `NativeSubstringActivation` behind one `VesContext` reference. Admission is
explicit in ordinary call/callvirt dispatch. Generated intrinsic IDs restrict it to
the two existing Substring registrations without allocating a declaring-type name.
The activation traces its source, owns its output buffer and retains offset and
caller checks separately from exception-handler unwind state. It consumes operands
once and copies at most one chunk per executor turn. A completed resume publishes
one result and advances the caller IP once. Incomplete turns yield even without GC
pressure. Finalizer queues remain rooted but cannot push frames while copying;
terminal outcomes clear the activation, including budget exhaustion.

No new unsafe runtime code, lifetime erasure, external async runtime or managed
re-entry was introduced. The final boxed representation replaced a larger inline
state experiment after reviewing control measurements; timings are in the linked
[performance record](../perf-evidence/2026-11-05-substring-continuation.md).

## Final validation

- `./check.sh --fast` passed on the final representation: 627 passed / 7 skipped
  without features; 739 / 5 with multithreading; 754 / 7 with multithreading and
  validation-all. This includes all-target clippy with warnings denied and enabled
  CLI fixtures. Local log: `/tmp/substring-fast-final.log`.
- The focused native tests additionally passed with `--no-default-features --features
  fuzzing`, covering the otherwise feature-gated exhausted-budget cleanup path.
- Pinned Tree Borrows Miri passed `pointer_provenance_tests::p44_native_substring_gc_resume`
  with no features and with multithreading. The former forces real collection before
  resumption; the latter verifies refusal with intact operands. Logs:
  `/tmp/substring-miri-st-final.log`, `/tmp/substring-miri-mt-final.log`.
- Pointer runner contract tests passed (21); the static batch guard covers 44
  singleton controls in four batches. The new control is shard 44, batch 3,
  including the hosted matrix. Formatting, whitespace and doc drift checks passed.
- `substring_chunks_42` and `substring_fallback_42` pass in the VM and return 42
  under CLR. They cover overloads, chunk boundaries, raw UTF-16 including unpaired
  surrogates, consecutive calls, handler/filter contexts, invalid ranges and nulls,
  plus delegate and reflection fallbacks.

The native tests verify exact progress across repeated real full collections, one
completion and stack result, stable reserved buffer storage, source survival and
later weak-reference reclamation, abort/budget cleanup, mismatched call-site refusal,
finalizer queue deferral, teardown and direct-vs-synchronous dispatch admission.
The finalizer queue test uses a non-finalizable queued object to exercise retention
and drainage; it does not claim a new test of managed finalizer invocation itself.

The ignored manual measurement test is deliberately excluded from routine checks.
Its optimized execution passed separately; the performance record gives its interval,
resource observations and reproduction command. Local logs are volatile; this report
preserves the validation outcomes and the source retains the controls.

## Miri reproduction

```sh
MIRIFLAGS='-Zmiri-tree-borrows -Zmiri-disable-isolation -Zmiri-ignore-leaks' cargo +nightly-2026-05-27 miri test -p dotnet-vm --lib --no-default-features pointer_provenance_tests::p44_native_substring_gc_resume -- --exact --test-threads=1
MIRIFLAGS='-Zmiri-tree-borrows -Zmiri-disable-isolation -Zmiri-ignore-leaks' cargo +nightly-2026-05-27 miri test -p dotnet-vm --lib --no-default-features --features multithreading pointer_provenance_tests::p44_native_substring_gc_resume -- --exact --test-threads=1
```

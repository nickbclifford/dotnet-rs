# Executor teardown ownership and panic safety

**Status:** Not started
**Prepared:** 2026-11-01; revised 2026-11-02
**Original baseline:** `18cf9f03`
**Outcome:** Dropping or extracting an Executor never destroys another Executor's arena or
frees storage still protected by an in-flight lease, including during unwind.

This is the next substantial runtime repair after the CI feedback work in [the queue](README.md).
It closes a concrete part of plan 01 R08. The prior ST native refactor is parked and is not a
prerequisite. No recovery of historical test sources is needed to begin.

## Established evidence

- **Extracted TLS collision, observed.** After `extract_arena` empties `THREAD_ARENA`, a second
  Executor can install its Box. Dropping the older Executor can take the successor's Box.
  A non-dereferencing drop witness observed destruction while a valid lease was held.
  `executor::executor_teardown_recreation_tests::recreation_extracted_executor_tls_ownership`
  is the independent, currently ignored failing regression. No-features uses
  `recreation_executor_single_arena_cleanup_control`; it has no TLS extraction backend.
- **Drain panic safety, source-derived.** `teardown_arena` owns a Box across
  `ArenaRegistration::unregister_and_drain`; `drain_leases` invokes `tracing::warn!` and
  the self-held-lease branch calls `eprintln!` before `mem::forget`. A panic may free the Box
  before access drains. The claim that stderr cannot unwind is incorrect. This failure path
  needs a controlled regression; it is not an observed UAF result.
- **Ownership surface, source-derived.** Executor is Send and THREAD_ARENA is publicly mutable.
  Moving an Executor changes which TLS cell its methods/drop use. A mutable public slot can
  bypass the ownership protocol.

Cross-arena runs `ee84ff60` (8.6) and `0aa1627a` (8.8) record the collision and subsequent
questions. Earlier 5.3–5.5 before-results were reconstructed on `cdb7b642`; they are retrospective
native evidence, not contemporaneous or Miri results. The ordinary teardown repairs remain useful.

## One coherent ownership change

Keep an explicit owner identity independent of whether the TLS Box has been extracted. Reject
successor creation until the owning Executor drops; a separately owned extracted guard may outlive
it, and must never take a successor's slot. Validate guard-first, Executor-first and repeated
extraction lifecycles. Retain the owner-qualified registration through drain.

Default to internal TLS and a thread-affine Executor. Audit workspace consumers and explain the
public Rust API consequence in review. Escalate only if an identified supported consumer requires
cross-thread transfer or public TLS mutation; routine implementation details need no renewed
approval. A supported transfer alternative must establish an explicit ownership protocol.

Establish arena retention before any operation that can unwind. A retention guard that leaks
on an incomplete drain is preferable to freeing leased memory. Logging must not be the safety
boundary. Do not hold registry/coordinator locks or TLS borrows during drain, and do not hold a
lease across safepoint parking. A same-thread held lease must neither deadlock nor free storage.

Include the supporting registry, guard and API edits needed for this outcome in the same work.
Do not split a newly exposed part of this ownership invariant into another approval ticket.

## Completion and stopping conditions

- The collision regression is enabled and passes, with no replacement pending-repair ignore.
- Native drop-order witnesses cover extraction, real successor creation/teardown and held/released
  leases; panic injection establishes retention through interrupted drain.
- Workers are released and joined on every test exit. Applicable MT ownership/teardown tests pass
  pinned Miri; ST cleanup remains a control, not claimed cross-arena coverage.
- Run the relevant GC/registry/concurrency checks and one fast integration pass under
  [CI policy](../CI.md#test-hierarchy-and-migration). Full affected suites are required if the
  implementation changes the wider collection protocol. Do not repeat them per edit.
- Reconcile non-panicking teardown and public ownership claims in code and owning documentation.
  Explain any bounded lexical-budget changes under CONTRIBUTING's normal review rules.

Stop when this ownership invariant and its regressions are complete. The real-VM collection
fixture's foreign-brand concern remains separate plan 01 acceptance work: an approval cannot
establish brand validity. Leases do not establish object roots, ranges, synchronization or FFI
lifetimes. Do not claim universal R08 closure from teardown tests alone.

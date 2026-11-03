# Executor teardown ownership and panic safety

**Status:** Complete — scoped ownership outcome accepted
**Prepared:** 2026-11-01; revised 2026-11-02
**Original baseline:** `18cf9f03`
**Outcome:** Dropping or extracting an Executor never destroys another Executor's arena or
frees storage still protected by an in-flight lease, including during unwind.

This repair targets a concrete part of plan 01 R08; broader R08 acceptance remains open.
The prior ST native refactor is parked and is not a prerequisite.

## Baseline and current evidence

At the original baseline, an extracted Box left the TLS slot available to a successor; the
older Executor could take that successor's Box. A non-dereferencing witness observed destruction
while a lease was held. Drain also held a normally-droppable Box across fallible warning and
self-held-lease diagnostics; this was a source-derived panic risk, not an observed UAF. Executor
transfer and public TLS mutation could bypass ownership. Cross-arena runs `ee84ff60` (8.6) and
`0aa1627a` (8.8) record the collision and questions; reconstructed `cdb7b642` before-results
are retrospective native evidence, not contemporaneous or Miri results.

The repair now reserves a private exact-identity MT slot until owner Drop, gives extracted
storage to a thread-affine guard, and arms retention before cleanup. Exact-registration drain is
required for destruction; incomplete drain or unwind leaks. ST uses a direct Box. See the
[arena lifecycle contract](../GC_AND_MEMORY_SAFETY.md#per-thread-arenas) for the public Rust API
break and limits. The collision is enabled. Native ST, MT and MT-validation teardown regressions
passed, including drop order and injected panic cases; worker cleanup and registry controls passed.
Final acceptance on runtime source `b97b950412be5a1fe28af3be387566483b6343a4` passed all
36 guarded manifest Miri singletons (7 ST, 28 MT permissive and one isolated strict-core),
focused GC/concurrency and Loom, executor corpus replay, workspace consumers and the three-mode
fast integration pass. Permissive Miri ignores leaks; the strict-core result is not whole-VM
strict-provenance evidence. The pre-finalizer session log in branch history
(`git show 6148da3c:AGENT_MEMORY.md`, if that history is retained) records counts, timeouts,
timings and the initial fixture-path setup failure separately.

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
- Reconcile teardown panic/retention and public ownership claims in code and owning documentation; do not promise non-panicking Drop.
  Explain any bounded lexical-budget changes under CONTRIBUTING's normal review rules.

Stop when this ownership invariant and its regressions are complete. The real-VM collection
fixture's foreign-brand concern remains separate plan 01 acceptance work: an approval cannot
establish brand validity. Leases do not establish object roots, ranges, synchronization or FFI
lifetimes. Do not claim universal R08 closure from teardown tests alone.

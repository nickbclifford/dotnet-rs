# Native continuations: parked design and redesign boundary

**Status:** Parked — prior implementation unmerged; redesign not started
**Revised:** 2026-11-02
**Scope:** A possible mechanism for preserving native-operation progress across VM safepoints.
This concerns Rust async as VM effects, not .NET Task scheduling or an OS async runtime.

The maintainer is retiring the current native-continuation refactor and will select a proper
redesign separately. Main has the experiment and design evidence, not the native runtime
implementation or its benchmark changes. No stage-1 closeout or migration from the old checklist
is required to begin the CI feedback pivot. Further native work is not automatically authorized
by this plan.

## Evidence retained, without an API freeze

The [design record](../../experiments/async-effects/DESIGN.md), layered prototype and
[gallery](../../experiments/async-effects/gallery/README.md) demonstrate a tested model using
`VmEffects`, `fx.vm().await?`, rooted access loans, per-activation roots and a private poll slot.
They do not establish real VM SEH, cross-arena behavior or production performance.

The unmerged `supervised/native-continuations-stage1` work reached commit `faecc22f` and has
unfinished benchmark changes. It provides implementation lessons, not behavior shipped on main
or a requirement to retain that architecture. Refactor records may be removed by the maintainer;
this plan does not depend on their availability. Do not infer that a checked step or merged
planning commit establishes runtime acceptance.

The former API selection, crate-by-crate checklist, exact cfg locations and approval ceremony
are no longer binding on the redesign. Reuse mechanisms that earn their complexity. Changing
an internal API, combining dependent work or correcting a prototype assumption is an ordinary
engineering choice; material compatibility/scope/safety tradeoffs still require a decision under
[CONTRIBUTING](../../CONTRIBUTING.md#engineering-decisions-and-plan-authority).

## The next useful native outcome, if selected

Choose one real operation whose progress is currently lost on Yield. Substring remains a
useful candidate: repeated GC requests must not repeatedly copy its prefix. Before implementation,
name that operation, the observable improvement and a bounded completion criterion. Include the
smallest complete adapter/scheduler/result/cleanup path rather than disconnected layers or
another comprehensive framework plan.

Default to ST first and conservative legacy fallback for unsupported calls. Keep small
synchronous operations direct when there is no useful progress to retain. Managed re-entry,
exception boundaries, waits and MT enablement are separate possible outcomes, not prerequisites
for a finite copy. Preserve existing supported guest behavior and legacy Yield paths unless a
specific replacement has equivalent semantic evidence.

## Lessons for the redesign

| Integration issue found in the retired effort | Default technical resolution to evaluate against the new path |
|---|---|
| Registration scanner ignores cfg and deduplicates IDs | Make selection explicit for the chosen signatures before argument commitment; preserve unrelated registrations, including existing AsRef duplicates. |
| ST thread-manager stop predicate is a stub | Use the actual heap collection request plus borrow exclusion; prove tests service real collections rather than only mock flags. |
| Safepoint acknowledgment needs branded state | Restore roots, exit the suspending mutation, run outer abort/budget/collection checks, then acknowledge in the next entry mutation. No static reply-ticket abstraction is inherently required. |
| Finalizers can disturb a parked caller | For a finite ST operation, keep queued work rooted, defer new automatic finalizer frames while it owns the call, then drain. Verify caller/result ownership and eventual drain. |
| Ordinary and engine/indirect calls have different return contracts | Select conservative legacy fallback before consuming operands until the corresponding completion/exception behavior is implemented and tested. |
| Polling Ready depends on completion | Implement and review them together; checklist boundaries do not require permission to add supporting code. |
| Experimental costs exclude real context/root work | Measure comparable real paths and retained allocations. Keep instrumentation out of paired ordinary-dispatch measurements. |
| Lexical ceilings can turn a location into an approval request | Prefer narrow gates and established abstractions; justify bounded deltas in normal review. A count is not a safety proof. |

These are reusable engineering findings, not a demand to reproduce all former decisions.
The design's detailed future effect catalogue remains historical candidate behavior.

## Safety and behavior that a selected implementation must establish

- Static futures cannot retain unconfined GC-branded values or heap borrows. References carried
  across suspension need correctly traced roots, with separate owner/generation guarantees.
- No lock, lease or real GC access scope prevents collection while the operation is parked.
  Any erased poll-local access is exclusive, thread-affine and reset on normal, panic and
  cancellation paths. A different mechanism needs an equally clear ownership contract.
- Operands are consumed once, progress survives suspension, and result/IP ownership is unambiguous.
  Unsupported call shapes fall back before commitment; errors preserve managed versus host meaning.
- Polling is bounded so outer abort/budget/collection checks run. The old 32-acquisition quantum
  is a tested candidate, not a reason to seek approval for internal tuning. Test the chosen bound.
- Cancellation, failure, return and teardown release futures, buffers and roots, including when
  Executor itself is retained. A passing destructor-only test is insufficient.
- MT adoption requires accepted plan 01/03 ownership/publication and native-specific coordinated
  GC evidence. The unresolved foreign-brand fixture cannot be made valid by maintainer approval.

## Validation and stopping conditions

Follow the [CI hierarchy](../CI.md#test-hierarchy-and-migration): focused checks during iteration,
then one relevant integration pass. Use native large/chunked workloads for progress, tiny Miri
controls for unsafe boundaries and intended-error compile-fail tests for confinement. Select
legacy/registration/feature controls and replay/Loom when affected; broad shared memory changes
need full affected suites before acceptance. There is no automatic per-step broad advisory run.

For a selected copy operation, completion means exact output under a real request after each
chunk, each element copied once, bounded polls and stop latency, no duplicate operand/result
handling, roots surviving collection and cancellation before further work. Measure transition,
allocation and retained-state cost against the direct path. Historical 100 ns shim / 10 ns
transition targets were hypotheses to investigate, not proof of real adapter cost. Explain
material regressions and seek an acceptance decision only when an unavoidable tradeoff remains.

Stop when the selected behavior, ownership and cost are understandable and verified. Record
limits once; do not automatically start the next effect, consume the whole gallery, recover
missing historical artifacts or add an assurance ladder. Until that native outcome is selected,
this plan stays parked and [the project queue](README.md) prioritizes CI feedback and Executor
ownership.

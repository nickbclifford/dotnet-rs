# Consolidate tests around intended contracts

**Status:** Not started
**Outcome:** The actionable test-quality findings from [27a](27a-test-audit.md) are resolved, with
intended behavior and safety coverage preserved and verified through the relevant production paths.

## Problem

27a identifies tests that provide weak evidence, freeze incidental implementation choices, duplicate
other tests without adding failure sensitivity, or enforce arbitrary suite-size constraints. This
plan implements its conclusions. Initial audit candidates are not authorization to delete specific
tests without examining their contracts.

## Scope

Rewrite, merge and remove tests, fixtures and test-only helpers according to the audit ledger.
Add focused replacement coverage where necessary to preserve the intended guarantees. Update
selectors, manifests and owning documentation as part of the same change when affected.

Preserve meaningful internal-contract tests, safety predicates, feature coverage and execution
guards. Infrastructure changes are limited to those directly needed by an audited finding.
New test frameworks, broad harness redesign and a suite-size target are outside scope. Small
directly related production fixes include regression coverage; larger defects are tracked
separately under the same boundary as 27a.

## Design

Use the shared `27-test-audit-ledger.md` produced by 27a. Each cleanup change links its findings and
states which contract remains protected, which assertions change, and how verification establishes
the result. Prefer a direct behavioral test at the smallest boundary that can demonstrate the
contract. Retain broader integration or managed-runtime comparisons when they detect failures the
smaller test cannot. Do not replace every unit test with an end-to-end fixture.

Before deleting or merging a test, account for its distinct failure modes, edge cases and feature
configurations. Introduce and enable needed replacements in the same change. Remove helpers,
fixtures and instrumentation only when no remaining contract needs them. Do not change production
behavior merely to satisfy a brittle assertion, or weaken expected results to make a failure pass.

Eliminate arbitrary test-count limits identified by the audit. Preserve detection of missing tests
or incomplete runs; prefer named cases or counts derived from the authoritative selection where
practical. Do not mechanically increase thresholds, remove missing-case guards, or introduce a
replacement quota. A larger or smaller resulting suite is acceptable.

## Slices

### 1. Apply the audit handoff in ordered batches

The [audit ledger](27-test-audit-ledger.md) and its linked partition records define the concrete
symbols, owning files, proposed changes, retained coverage and verification commands. The batches
below cover every actionable finding. A batch is a work grouping; split it into coherent changes
by owning subsystem, leaving tests enabled after each change. This is remediation of inspected
findings, not another open-ended audit. Prioritize defects in the evidence before removing overlap.

| Order | Batch | Required evidence |
|---|---|---|
| 1 | Harness/discovery false passes: CLI-3, CLI-20–CLI-23, TOOLS-5 | Propagate teardown failures and actual execution failure, preserve missing-artifact policy, bound subprocesses and observe real tool invocations; retain partial-output/error controls |
| 2 | Fuzz reachability and independent oracles: VM-2–VM-3, LIB-2, LIB-22, TOOLS-6 | Requested instructions really execute; named seeds decode as intended; unmanaged pointer inputs and benchmark decode outcomes are checked independently; keep replay accounting |
| 3 | Managed behavior assertions: CLI-1, CLI-5–CLI-14, CLI-16, CLI-18 | Assert the actual exception/trace, arithmetic IL, sequence, target effect, finalizer event and ABI/initializer result; failures cannot return success; ILOffset cannot cancel its own error |
| 4 | Coordinated concurrency and fixture isolation: CLI-17, CLI-19, VM-9, VI-2, VI-5, LIB-10, LIB-14, LIB-16, LIB-21 | Establish required writer/reader, wait/acquisition and teardown checkpoints; preserve terminal outcomes; use isolated paths/IDs and truly misaligned storage |
| 5 | Runtime-unit and provenance witnesses: VM-5–VM-6, VM-8, VI-1, VP-1–VP-4, LIB-9, LIB-11, LIB-17–LIB-18, LIB-20, LIB-23–LIB-25 | Require completion/non-null/value-bearing results, use valid allocation extents/CIL, observe actual growth and intended errors, call production write-back/access, and reflect configuration in discovery |
| 6 | Registration/source guards: VM-4, VM-7, VI-4, LIB-1, LIB-3, LIB-7 | Replace incidental spelling with real dispatch/transport/filter evidence; retain justified structural guards and fail on incomplete scans |
| 7 | Contract tolerance and redundant models: CLI-2, CLI-4, CLI-15, VI-3, LIB-5, LIB-8, LIB-12, LIB-15, LIB-19, TOOLS-7 | Preserve behavioral/safety assertions while relaxing incidental event counts, diagnostic prose, hash inequality and footprint snapshots; check compile-fail reasons; retain one justified positive/negative model and use an overlap-guaranteeing API for overlap workloads |
| 8 | Count/registry maintenance: TOOLS-1–TOOLS-4, LIB-4, LIB-13 | Permit fully registered additions without unrelated counter edits; preserve named required identities, old ABI IDs, uniqueness, discovery and exactly-once execution checks |

LIB-6 is a separate production-policy investigation, tracked in the plans README. Preserve its
test until the resolution-error/type-identity policy is decided; change implementation and test
together under that decision. Do not silently delete the test or declare the underlying behavior
correct. This dependency does not block unrelated cleanup, but prevents closing that finding.

Within batch 7, retain each exact size requirement justified by ABI, serialization or safety;
use a documented footprint bound only where that is the actual performance requirement. Retain
internal lifecycle/transport tests throughout. Missing coverage calls for a focused replacement,
not a blanket move to end-to-end fixtures.

### 2. Implement and verify each populated batch

Apply the smallest coherent cleanup, run focused tests and the affected configurations from
[CI.md](../CI.md), and update each finding with its resolution and verification. Where a finding
included an executable counterexample or deliberate fault, check that the replacement detects it.
Distinguish actual runs from inspection and unavailable checks. Run relevant Miri/Loom/provenance
lanes when changing the tests or unsafe/concurrent paths they cover; a native pass alone does not
replace those checks. Remove deliberate faults before completing a batch.

### 3. Reconcile coverage and close

Check that changed tests remain discoverable, enabled and selected in the required configurations;
update affected manifests/guards alongside test names. Run the repository's prescribed checks for
the combined changes, following CI.md, and the affected deep lanes. Reconcile every actionable
finding with its final disposition and remove obsolete fixtures/helpers/docs. Add brief durable
test-writing guidance to CONTRIBUTING.md based on the accepted criteria in 27a; do not create a
new policy framework, numerical budget or lint suite to enforce subjective judgments.

## Done when

Every actionable audit finding is resolved with verification, or refuted with evidence in the
ledger. Larger production defects are explicitly tracked separately rather than hidden by weakened
tests; any that prevent verification remain blockers. Intended contract coverage survives each
deletion/merge, changed tests run in their required configurations, and relevant checks pass.
Unavailable required checks are reported as outstanding. No suite-size cap or reduction target
is introduced, and no finding is closed merely by raising a counter.

## Ask the maintainer only if

A finding exposes an unresolved requirement, a change would alter supported guest behavior/public
compatibility or weaken safety, or remediation needs a larger scope. Rewriting, merging or removing
tests on evidenced audit findings is already within scope.

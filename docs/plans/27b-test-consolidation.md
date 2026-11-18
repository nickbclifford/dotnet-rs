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

### 1. Turn the completed audit into concrete cleanup batches

Execution depends on 27a's completed handoff. Its final slice fills in finding references, owning
files, intended changes and verification for the batches below. They are an ordering template,
not a second audit or a finding-independent mandate to change every category. Omit empty batches;
split substantial batches by subsystem, leaving usable tests enabled after every increment.

| Order | Batch | Required evidence |
|---|---|---|
| 1 | False confidence: circular or vacuous checks, disconnected replicas and bypassed production paths | The replacement reaches production and rejects the identified incorrect behavior |
| 2 | Incidental structure: overspecific assertions, source scans and exact instrumentation counts | Intended internal/guest contracts remain checked while the identified valid refactor is allowed |
| 3 | Redundancy and test-count constraints | Distinct cases/configurations remain covered; missing-case detection remains effective |

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

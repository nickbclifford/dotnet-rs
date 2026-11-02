# Project focus and plan queue

Build a maintainable ECMA-335 runtime whose supported behavior, safety boundaries and
completion criteria are understandable. Prefer a completed runtime path over more milestones,
audits or artifacts. These priorities were revised with the maintainer on 2026-11-02.

## Next useful work

**Shorten the feedback loop first.** Implement the [CI hierarchy and migration](../CI.md#test-hierarchy-and-migration):
fast core checks get runner capacity first; affected code selects deeper checks; full sweeps
run on a schedule and before accepting broad architectural changes. Move source-policy scans
out of Miri and remove duplicate validation without dropping runtime regressions. The workflows
have not yet been changed to implement this policy.

The maintainer is retiring `supervised/native-continuations-stage1` for a proper redesign.
Its implementation and unfinished benchmarks are not merged into main and are not prerequisites
for the pivot. [Plan 02](02-native-continuations.md) is parked: preserve useful historical
findings, but do not resume the old checklist, require its closeout or freeze its API choices.
After CI feedback work, Executor/arena ownership in plan 03 is the next substantial runtime
repair, followed by stable byref homes under a common access protocol in plan 01. Native interop
remains a separate bounded repair with its current fail-closed exclusions retained.

| ID | Plan | Priority | Status | Dependencies |
|---|---|---|---|---|
| 01 | [Pointer/provenance model](01-pointer-provenance.md) | After ownership: stable homes and common access | In progress — scoped repairs retained; M1e/M1g acceptance open | Plan 03 for R08; storage, shared access and affected copy/barrier paths migrate together |
| 02 | [Native continuations (async/await as VM effects)](02-native-continuations.md) | Parked for redesign after CI feedback | Parked — prior implementation unmerged; redesign not started | Select one useful behavior before implementation; MT requires accepted cross-arena ownership/publication |
| 03 | [Executor teardown ownership and panic safety](03-executor-teardown-ownership.md) | Next substantial runtime repair | Not started | No historical-artifact recovery prerequisite; blocks R08/M1e acceptance of plan 01 |

## How work proceeds

The numbered plan owns the runtime outcome, boundaries and stopping conditions.
[CONTRIBUTING.md](../../CONTRIBUTING.md#engineering-decisions-and-plan-authority) owns agent
discretion and escalation; [CI.md](../CI.md#test-hierarchy-and-migration) owns validation selection.
Research, checklists and memory are supporting evidence, not competing sources of authority.
Later maintainer decisions supersede earlier proposals; passing checks do not resolve an
unproven safety contract.

An assignment covers the smallest complete behavior, including necessary supporting edits.
Agents resolve routine integration choices, correct stale prose, combine dependent steps and
explain consequential adaptations in review. Ask only about supported behavior, compatibility,
scope or unresolved safety tradeoffs. Do not create a new prerequisite ticket for each such
adaptation or repeat a settled approval.

Run focused checks during iteration, then one relevant integration pass. Documentation-only
changes need documentation checks. Missing historical sources remain explicit evidence limits;
fresh, labelled regressions can establish the current behavior. Further assurance research and
new async consumers require an outcome that justifies them.

Keep the status here synchronized with the plan's `Status` header. Validate with
`bash scripts/check_plan_queue.sh`.

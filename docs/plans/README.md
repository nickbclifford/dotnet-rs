# Project plan queue

This queue tracks implementation work: plan 01 is synthesized from the pointer/provenance reviews,
plan 02 from the async-effects design in `experiments/async-effects/`,
plan 03 from the deferred branch-8 Executor teardown findings.
The numbered plan owns its detailed scope, dependencies, evidence, and completion gates.
Historical assurance-plan links elsewhere in the documentation refer to plans removed before this queue was created.

| ID | Plan | Priority | Status | Dependencies |
|---|---|---|---|---|
| 01 | [Pointer/provenance model](01-pointer-provenance.md) | First | In progress — scoped M1b pointer semantics, M1c stable localloc storage and M1d owner/guard work complete; M1e/M1g implementation progressed, acceptance open; partial M0 (VM probe harness and controls) | None; internal milestone dependencies are in the plan |
| 02 | [Native continuations (async/await as VM effects)](02-native-continuations.md) | Second | Not started | Stage 4 (multithreading) needs plan 01 M1e/M1g; stages 1–3 none |
| 03 | [Executor teardown ownership and panic safety](03-executor-teardown-ownership.md) | Third | Not started | None; blocks R08/M1e acceptance of plan 01 |

Keep the status here synchronized with the plan's `Status` header. Validate with
`bash scripts/check_plan_queue.sh`.

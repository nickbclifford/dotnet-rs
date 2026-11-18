# Audit test contracts and evidence

**Status:** Complete
**Outcome:** Every project-owned test family has been inspected, and an audit ledger identifies
which tests protect intended contracts and which need changes, with concrete evidence and an
actionable handoff to [27b](27b-test-consolidation.md).

The [audit ledger](27-test-audit-ledger.md) links completed source reviews, findings and verification
limits. Plan 27b contains the concrete cleanup batches. Test and production code were unchanged.

## Problem

A passing suite can preserve implementation details, compare an implementation with itself, or
exercise a test-only reconstruction while the production path is broken. Test-count ceilings can
also turn additions into routine threshold increases without improving the suite.

Initial inspection found candidates, not confirmed defects:

- `intrinsics_codegen_guard_tests` matches generated handler calls and recomputes hashed symbols.
- `multicast_selection_precedes_operand_commitment` combines guest results with exact trace counts
  and source-string exclusions in `integration_tests_impl/fixtures.rs`.
- `arena_lifetime_transport` scans source for particular pointer conversions and buffer types.
- `loom_workers` compiles the production registry; inspect whether its surrounding model establishes
  conditions that production is actually responsible for establishing.

These examples seed the review; they do not bound it. Infrastructure is generally satisfactory;
the primary question is what the tests demonstrate.

## Scope

Inspect all project-owned Rust unit/integration tests, doctests and compile-fail probes; C#/IL
fixtures and host-project assertions; Python/shell and build/code-generation tests; Loom models;
Miri/provenance controls; and fuzz targets, properties and regression replay. Include ignored,
feature-gated and diagnostic tests, test helpers, and benchmark correctness assertions. Inspect
corpus selection and replay semantics; individual opaque corpus blobs need review only where
their origin or claimed regression matters. Exclude dependencies and generated artifacts as
independent review units; review their project-owned generators and consumers.

Read runner configuration enough to establish discovery, feature coverage, skipping and result
interpretation. Infrastructure redesign, new testing frameworks, comprehensive new feature
coverage, and performance optimization are outside scope.

Audit changes belong in the ledger. Test consolidation and routine rewrites belong in 27b.
A small production defect found directly on the inspected path may be fixed with a focused
regression test and the relevant checks. Larger defects go in the plans README's "Not yet planned"
section, linked from the finding; do not expand this audit into a runtime rewrite.

## Design

Review each test against these questions:

1. **Contract:** What intended behavior or invariant does it protect, and why is that requirement
   valid? Guest behavior, supported internal APIs, soundness, lifecycle/resource guarantees and
   justified performance requirements all qualify. Existing implementation alone is not authority.
2. **Oracle:** Where does the expected answer come from? Check reasoned examples, specification
   requirements, reference-runtime comparisons and independently justified properties. Shared
   helpers are acceptable when they do not reproduce the disputed computation. Round trips can
   establish consistency, but cannot alone establish conformance.
3. **Production connection:** Does the test reach the relevant production path? Does setup inject
   the desired state, duplicate the algorithm, supply missing synchronization, or bypass the
   validation or transition whose correctness the test claims to establish?
4. **Sensitivity:** What plausible incorrect behavior would fail it? Look for vacuous passes,
   swallowed errors, unchecked results, unreachable assertions, permissive output normalization,
   and tests that only check instrumentation announcing success.
5. **Change tolerance:** Would an implementation change preserving the contract fail it? Justify
   exact layouts, counts, schedules, messages, source spelling and call sequences individually.
   Internal tests are welcome when they protect an explainable requirement.
6. **Distinct value:** Which failure modes does it add beyond nearby tests? Similar inputs or
   overlapping layers are not sufficient grounds for deletion. Examine nondeterministic timing,
   shared global state and order dependence where they affect the credibility of assertions.

Source checks and generated-output assertions can protect genuine structural or generation
contracts; they must not be represented as execution evidence. Compile-fail probes must fail for
the intended reason. Loom and fuzz findings must state the model/input limits and their connection
to production. Use ECMA-335 sections where applicable and the existing invariant registry for
safety requirements; distinguish specified behavior from deliberately unsupported/invalid inputs.

**No test-count quotas:** do not impose ceilings, floors or reduction percentages on suite size.
Distinguish arbitrary caps from guards that detect zero matches, missing named cases, unexpected
ignores or incomplete execution. Retain useful execution guards; identify manually maintained
counts that can instead derive from an authoritative case set without losing coverage checks.
Resource timeouts and bounded model exploration are separate concerns, not test-count quotas.

## Slices

### 1. Establish review coverage

Create `27-test-audit-ledger.md` beside these plans when execution begins. Record the baseline
revision, test source locations, discovery/generation routes, configurations and review progress.
Inventory test families by crate/subsystem and test form, including disabled and manually invoked
paths. Enumerate the test symbols/cases covered by each review entry, or reference a reproducible
discovery list. Group common cases to keep the ledger small, but inspect their assertions and setup;
a file-level skim or a sample is not a completed review. Counts are informational only.

### 2. Inspect contracts and challenge doubtful evidence

Work through the inventory, prioritizing the seed candidates, self-derived expectations,
test-only models, count guards, and safety/lifecycle tests. Review each family against the questions
above. Trace helpers and production callers as needed. For doubtful claims, use a focused run,
reference comparison or temporary local fault injection to distinguish a real weakness from a
superficial smell. Restore deliberate faults; do not introduce a mutation framework or require a
mutation for every test. Separate observed results from inspection conclusions and unverified
hypotheses. Existing baseline failures are recorded rather than silently accepted as passes.

For each actionable finding record the test symbols, intended contract and its basis, concrete
weakness, a bug it would miss or valid change it would reject, recommended disposition, retained
coverage/replacement needed, and relevant verification commands. Use dispositions such as keep,
rewrite, merge, remove or investigate; keep entries need only a brief rationale per coherent family.
Rank actionable findings by false confidence and impact, not line count or ease of deletion.

### 3. Close the audit and hand off

Reconcile the inventory with the checkout, including changes since the baseline. Resolve
"investigate" entries into supported conclusions or explicit blockers; do not present uncertainty
as a confirmed deletion candidate. Add the ordered cleanup batches and finding references to 27b
using the actual findings. Record any production fixes and separately tracked defects. Keep the
ledger as the shared record rather than copying it into both plans.

## Done when

Every inventoried test family has a completed review with traceable case coverage, and every
actionable finding has evidence, a disposition and a destination. Any incomplete review is an
explicit blocker to declaring the audit complete. 27b contains concrete cleanup batches and
verification requirements. Completion does not depend on finding a minimum number of problems,
reducing the suite, or running every expensive lane when inspection does not require it.

## Ask the maintainer only if

The intended contract cannot be resolved from the agreed scope, specification or existing
requirements; a fix would change supported guest behavior/public compatibility or weaken a safety
requirement; or a larger change is needed to complete the agreed audit. Routine test dispositions
and directly related small production fixes do not require another approval.

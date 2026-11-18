# Audit test contracts and evidence

**Status:** Complete
**Outcome:** Every project-owned test family was inspected, and the
[audit ledger](27-test-audit-ledger.md) identifies which tests protect intended contracts and which
needed changes, with evidence and a handoff to [27b](27b-test-consolidation.md).

## Scope

All project-owned Rust unit and integration tests, doctests and compile-fail probes; C#/IL fixtures
and host-project assertions; Python, shell and build/code-generation tests; Loom models; Miri and
provenance controls; and fuzz targets, properties and replay, including ignored, feature-gated and
diagnostic tests, helpers and benchmark correctness assertions. Dependencies and generated
artifacts were reviewed only through their project-owned generators and consumers. Test and
production code were unchanged by the audit itself.

## Review criteria

These criteria are durable; the short form is in [CONTRIBUTING](../../CONTRIBUTING.md#tests).

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

## Outcome

Findings CLI-1–23, LIB-1–25, TOOLS-1–7, VM-1–9, VI-1–5 and VP-1–4 were recorded with their
contract basis, weakness, counterexample and disposition; their final dispositions are in the
cleanup records listed in the [ledger](27-test-audit-ledger.md). The suspected panic swallowing in
the fuzz executor target was refuted (libfuzzer-sys installs an aborting panic hook), so VM-1 is
not actionable.

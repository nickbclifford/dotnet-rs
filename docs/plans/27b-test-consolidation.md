# Consolidate tests around intended contracts

**Status:** Complete
**Outcome:** The actionable test-quality findings from [27a](27a-test-audit.md) are resolved, with
intended behavior and safety coverage preserved and verified through the relevant production paths.

## What changed

Tests, fixtures and test-only helpers were rewritten, merged or removed according to the audit, in
eight batches: harness and discovery false passes; fuzz reachability and independent oracles;
managed behavior assertions; coordinated concurrency and fixture isolation; runtime-unit and
provenance witnesses; registration and source guards; contract tolerance and redundant models; and
count/registry maintenance. Arbitrary test-count limits were replaced by counts derived from named
required selections, without losing missing-case and incomplete-run detection. The per-finding
dispositions are in the cleanup records linked from the [ledger](27-test-audit-ledger.md).

**LIB-6 policy.** A type-resolution failure is a typed error, distinct from a successful comparison
that yields `false`: concrete type identity, assignability, subclass/interface and variance checks
return `Result<bool, TypeResolutionError>`, and failures propagate to `VmError::TypeResolution`.

**Production fixes found by strengthened tests:** element-stride overflow before array bounds
handling (now a managed `IndexOutOfRangeException`; p19 became an enabled pointer control) and the
wrong managed exception from `Type.MakeGenericType` (now `ArgumentException` for constraint
violations, with resolution failures as host errors).

## Open items

Tracked in the plans README, "Not yet planned": negative `GC.Collect` generation validation, and
the legacy boolean metadata classifiers and optional reflection probes. The cleanup records state
the remaining model, stress-scheduling and representative-overload coverage limits.

Durable test-writing guidance is in [CONTRIBUTING](../../CONTRIBUTING.md#tests).

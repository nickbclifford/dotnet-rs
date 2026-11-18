# Test audit ledger

Baseline: `8431c1ab` (the committed 27a/27b plans). Review criteria and completion requirements
are defined in [27a](27a-test-audit.md). Counts, if recorded, describe discovery rather than limits.

## Coverage and ownership

| Partition | Locations | Review record | State |
|---|---|---|---|
| VM | `crates/dotnet-vm/**`, including its fuzz workspace and models | [VM review](27-audit/vm.md), [provenance](27-audit/vm-provenance.md), [inline](27-audit/vm-inline.md) | Inspection complete |
| CLI and managed fixtures | `crates/dotnet-cli/**` | [CLI review](27-audit/cli.md) | Inspection complete |
| Libraries | Other crates except benchmarks, including value fuzzing and memory Loom models | [Libraries review](27-audit/libraries.md) | Inspection complete |
| Tools and benchmarks | `scripts/**`, `xtask/**`, workflow test wiring, `crates/dotnet-benchmarks/**`, root test/check entrypoints | [Tools review](27-audit/tools.md) | Inspection complete |

The three delegated partitions used GPT-6.1-Terra with low reasoning effort. The CLI agent also
reviewed VM provenance after its first partition; the coordinator reviewed tools, benchmarks and
remaining inline VM tests and reconciled the handoff. Findings distinguish inspection from runs.

Source discovery and exact case coverage are in each report, [library inventory](27-audit/libraries-inventory.md),
[tools inventory](27-audit/tools-inventory.md) and [VM inline inventory](27-audit/vm-inline-inventory.md).
The VM and CLI reports include reproducible discovery queries and complete named-file/directory
review groups. Coverage includes ignored/manual and feature/target/debug cases, doctests,
compile-time guards, local test macros, fixture builders, Loom models and fuzz target/replay
semantics. Opaque corpus blobs were not individually decoded; VM-3 records the seed-format issue.
No tracked root `tests/**` sources or additional workspace test families were found outside these
partitions. Dependencies/generated artifacts are outside the agreed audit scope.

## Findings and dispositions

The partition records supply the specific symbols, contract basis, failure example, recommendation,
replacement/retained coverage and verification for each finding. [27b](27b-test-consolidation.md#1-apply-the-audit-handoff-in-ordered-batches)
maps every actionable finding to an ordered cleanup batch; all remediation is outstanding.
The detailed keep entries record useful internal, safety and guest contracts, including their
model/fixture limitations. These are source-inspection dispositions, not claims that the full
suite passed or that every plausible implementation mutation was tried.

The initial panic-swallowing suspicion was refuted: the pinned libfuzzer-sys initialization installs
an aborting panic hook. VM-1 is not an actionable finding. The substantive VM fuzz findings concern
execution reachability and the mismatch between seed formats and Arbitrary input decoding.

LIB-6 establishes that a regression codifies unjustified equal-error type identity; production
guest reachability and the replacement policy remain a separately tracked investigation in the
plans README. CLI-11's documented negative-generation GC divergence is also separately tracked.
Neither is silently resolved by deleting a test, and neither is presented as a newly reproduced
guest bug. No other review uncertainty remains as an uninspected test-family blocker.

## Verification and handoff

Observed checks:

- `python3 -m unittest discover -s scripts/tests -p 'test_*.py'`: 95 passed.
- `cargo test -p xtask --offline`: 12 passed.
- `bash scripts/check_doc_drift.sh`: 59 passed; the injected negative controls also passed.
- Relative document links resolve, every actionable finding maps to 27b or its explicit
  production-policy dependency, and `git diff --check` passes.
- Temporary pointer-runner copy: an additive control in both catalogs failed at the fixed total;
  deriving that total from the named registry admitted it. No production script changed.
- Temporary advisory manifest: a valid extra named selector failed the fixed shard range.
- Minimal temporary rustdoc probe on pinned stable 1.95: `compile_fail,E0133` passed for an
  unrelated E0425 error; compilable code failed. LIB-15 records the resulting diagnostic gap.
  [Rustdoc documents error-number checking as nightly functionality](https://doc.rust-lang.org/rustdoc/unstable-features.html#error-numbers-for-compile-fail-doctests).
- ECMA-335 III.3.30 and III.3.47 were checked through the connected specification text for
  cpblk overlap and localloc stack requirements (TOOLS-7 and VP-2).

All other findings rest on source inspection. The full native/managed feature matrix, deep
Miri/Loom/fuzz lanes and destructive clean-build probes were not run; 27a does not require them
without relevant code changes. The proposed commands and counterexamples belong to 27b's
implementation verification. Passing existing tests does not refute test-quality findings.

`git diff --name-only 8431c1ab -- crates scripts xtask .github check.sh` was empty at reconciliation:
test and production sources match the baseline; changes are audit documents and plan handoff only.
Plan 27a is complete; plan 27b remains not started.

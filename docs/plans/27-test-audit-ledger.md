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
maps every actionable finding to an ordered cleanup batch. Implementation and verification are
recorded in the cleanup records: [CLI](27-audit/cleanup-cli.md),
[libraries](27-audit/cleanup-libraries.md), [VM](27-audit/cleanup-vm.md), and
[tools and benchmarks](27-audit/cleanup-tools.md).
The detailed keep entries record useful internal, safety and guest contracts, including their
model/fixture limitations. These are source-inspection dispositions, not claims that the full
suite passed or that every plausible implementation mutation was tried.

The initial panic-swallowing suspicion was refuted: the pinned libfuzzer-sys initialization installs
an aborting panic hook. VM-1 is not an actionable finding. The substantive VM fuzz findings concern
execution reachability and the mismatch between seed formats and Arbitrary input decoding.

LIB-6 establishes that a regression codifies unjustified equal-error type identity. The maintainer
clarified that resolution failure must propagate as a typed VM error; 27b includes this production
correction and its regression coverage. CLI-11's documented negative-generation GC divergence is
separately tracked in the plans README. Neither is silently resolved by deleting a test, and neither
is presented as a newly reproduced guest bug. No other review uncertainty remains as an uninspected
test-family blocker.

## Audit verification and handoff

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

At the audit handoff, test and production sources matched the baseline; its changes were audit
documents and plan handoff only. Subsequent 27b implementation and verification are recorded in
the linked cleanup records.

## Combined cleanup verification

The partition cleanup records map every actionable finding to its replacement or retained
contract and record focused/deep checks. Combined verification:

- `CARGO_NET_OFFLINE=true CARGO_BUILD_JOBS=4 ./check.sh --fast` passes in full:
  no features (1,170 tests), multithreading (1,516), and multithreading with validation-all
  (1,531), including Clippy with warnings denied and the final managed fixtures.
- Isolated CLI fuzzing nextest and Clippy with warnings denied pass. Feature configurations
  use separate target directories when concurrent so subprocess executables cannot be replaced
  by another feature build.
- Python script suite and workflow command-expansion guards pass, including registered-growth,
  missing-selection, wrong-selector and ignored/failing-execution controls.
- The native-effects and FieldType doctests and diagnostic checkers pass with no features and
  multithreading; their positive controls compile.
- Documentation drift checks and their injected negative controls pass.
- The real clean-clippy invocation probe passes in an isolated target directory; its synthetic
  controls distinguish quiet tool invocation, harmless output text and unrelated command failure.
- Value pointer arithmetic passes the actual workflow Miri selection and named execution guards;
  the new live-allocation offset control passes separately with strict provenance. Library transport,
  atomic/access and changed VM pointer evidence is recorded in the partition reports.
- Loom STW and production shared-table/finalizer models pass.
- Advisory tail and non-tail call depth tests pass as guarded Miri singletons with no features
  and multithreading. Provenance lints cover the workspace feature configurations and both fuzz
  workspaces.
- All named fuzz-harness Miri cases pass with exact-method instruction observation. Their
  disjoint guarded groups cover actual Miri discovery without duplicates or omissions. A missing
  `Enum`/`ValueType` mock contract was fixed, the constructor control was added to native mock
  coverage, and its helper/localloc group passed on rerun. Native fuzz-harness tests and Clippy
  with warnings denied pass after the final observer/mock changes.
- Managed-pointer serialization/cache benchmark preflight passes. Both overlap workloads build
  and return success on the CLR and through the VM benchmark harness.
- Executor replay verifies the exact staged corpus and execution census. The legacy renames retain
  identical bytes; named semantic entries are additions. Value corpus replay evidence is in the
  library record. Address instrumentation remains enabled; LeakSanitizer is disabled only for
  these local runs because it cannot operate under the sandbox's ptrace restriction.

The final cyclic-constraint refinement also passes its focused validation-feature test: the
resolver budget detects runaway recursion without pinning the incidental successful lookup count.
Formatting, staged and unstaged whitespace checks, and cleanup document links pass.

Failed/interrupted iterations are not counted as passes. Known smoke/model limits remain
documented rather than being promoted into claims of scheduling coverage or CLR parity. The
unchanged full clean-build probe set and unrelated optional configurations were not rerun; the
changed invocation guard and affected feature/deep lanes were exercised directly.

# CI and Local Validation

Locally, run focused tests and `./check.sh --fast` once per coherent change. CI selects deep
Miri/fuzz lanes from changed paths; run a deep lane locally when changing unsafe code it covers or
to debug its failure.

The workflow YAML under `.github/workflows/` is the source of truth for what runs. This document
explains the structure and how to reproduce a job locally; it does not restate test lists or
counts.

## Test hierarchy and migration

CI has three layers. Each answers a different question, and a later layer does not replace an
earlier one.

| Layer | When it runs | What it covers |
|---|---|---|
| Fast core | Every push and PR to `main`/`master` | Source policy and doc lint, format, provenance lints, the Loom STW model, clippy and native tests over the shared feature matrix, build-script probes, fixtures, and the library Miri jobs (`miri-value`, `miri-frame-blocks`, `miri-owner-retention`). The `fast-core` job aggregates them. |
| Selected deep | Push and PR, after `fast-core` succeeds, only for lanes `select-deep` chooses | VM Miri (`miri-cross-arena`, pointer controls) and `fuzz-corpus-replay`. The `deep-safety` job aggregates them. |
| Full sweep | `ci.yml` nightly cron and `workflow_dispatch` with `full: true`; the advisory workflows on their own schedules | Every deep lane, the pointer controls as singletons, plus the advisory Miri, fuzz and Valgrind workflows. |

`select-deep` runs `scripts/ci_select.py` over the changed paths (PR: `base...head`; push:
`before..head`) and publishes per-lane booleans, the fuzz target matrix, `pointer_mode` and a
reason for each lane in the job summary. The path families and crate dependency closures live in
that script; `scripts/tests/test_ci_select.py` recomputes the closures from the `Cargo.toml`
files and fails on drift.

### Selection and merge policy

- Ambiguity selects more. An unrecognized path, a missing base, a git error, an empty diff or a
  selector failure selects every lane (`mode=full`), and `select-deep` publishes full-sweep
  fallback outputs before running the selector.
- A deep job is skipped only when its lane output is explicitly `false`; a missing output runs it.
- `deep-safety` passes a lane that was not selected and is `skipped`; a selected lane must be
  `success`. If `fast-core` did not succeed, `deep-safety` reports the deep layer as not executed,
  which is distinct from a deep job that ran and failed.
- A zero-match filter, an unexpected ignore, a timeout or a failed setup fails the job. The
  guards in `scripts/ci_diagnostics.py`, `scripts/pointer_provenance.sh` and
  `scripts/miri_advisory.py` enforce exact pass counts.
- Full sweeps supplement selection. Do not defer an unsafe change's relevant deep lane to the
  nightly run.
- "Blocking" below means the job fails the workflow. Which checks are required for merging is a
  branch-protection setting.

## Workflows

| File | Trigger | Blocking | Purpose |
|---|---|---|---|
| `ci.yml` | push/PR to `main`/`master`; nightly 06:00 UTC full sweep; `workflow_dispatch` (`full`, default true) | Yes | Fast core and selected deep layers |
| `miri.yml` | daily 02:00 UTC; `workflow_dispatch` | No | Advisory Miri: `dotnet-utils` suite, assembly and VM singleton manifest |
| `fuzz.yml` | daily 00:00 UTC; `workflow_dispatch` | No | Duration-based fuzz campaigns and crash minimization |
| `valgrind.yml` | daily 04:00 UTC; `workflow_dispatch` | No | Native leak and uninitialized-memory checks on curated integration tests |
| `self-hosted-probe.yml` | `workflow_dispatch` | No | One pointer batch on the self-hosted runner, reporting wall time and memory peak |

## Jobs and their sources of truth

| Job | Source of truth |
|---|---|
| `doc-lint` | Its steps in `ci.yml`: the two ceiling scripts, `xtask verify-slots`, the pointer, selector and aggregate script tests, `pointer_provenance.sh verify-batches`, `scripts/check_doc_drift.sh`, and the intra-doc-link check |
| `format`, `clippy`, `test` | `cargo fmt`; `SHARED_FEATURE_MATRIX` in `xtask/src/main.rs` (`cargo run -p xtask -- matrix test-features`). The `test` job's `multithreading` leg also runs the hang-probe filters listed in its YAML step. |
| `provenance-lints` | `scripts/check_provenance_lints.sh` |
| `loom` | `crates/dotnet-vm/tests/loom_stw.rs` |
| `build-script-regression` | `scripts/check_build_script_regressions.sh` |
| `build-fixtures` | `xtask fixtures` (output path and cache key) |
| `miri-value`, `miri-frame-blocks`, `miri-owner-retention` | Selectors and `--require` counts in the job steps in `ci.yml` |
| `miri-cross-arena` | The job's `include` list in `ci.yml`; `scripts/tests/test_ci_cross_arena_manifest.py` checks it |
| `miri-pointer-batches`, `miri-pointer-controls` | `expected_controls` in `scripts/pointer_provenance.sh` (`bash scripts/pointer_provenance.sh list`) |
| `fuzz-corpus-replay` | Tracked corpora and `scripts/replay_fuzz_corpus.py`; see [FUZZING.md](FUZZING.md#blocking-corpus-regression) |
| `select-deep`, `fast-core`, `deep-safety` | `scripts/ci_select.py` and the aggregate steps in `ci.yml`; `scripts/tests/test_ci_aggregates.py` exercises them |
| `miri.yml` advisory shards | `scripts/miri_advisory_manifest.json`, driven by `scripts/miri_advisory.py` |
| `valgrind.yml` | The test list in its steps; `valgrind.supp` |

## Budgets

`timeout-minutes` values are caps sized from hosted samples, not expected runtimes. Jobs that use
`scripts/ci_diagnostics.py` set `CI_DIAG_DEADLINE` to the job timeout minus three minutes so
diagnostics can still upload. Dated measurements are in [ci-evidence/](ci-evidence/).

| Job | Timeout | Rationale |
|---|---|---|
| `select-deep`, `fast-core`, `deep-safety` | 5 min | Git diff and result aggregation only |
| `loom`, `clippy`, `miri-value`, `miri-frame-blocks`, `miri-owner-retention`, `fuzz-corpus-replay` | 15 min | Small library suites or `-runs=0` replays |
| `build-script-regression` | 20 min | Several clean builds |
| `provenance-lints`, `test` | 30 min | Whole-workspace builds per configuration |
| `miri-cross-arena` | 30 min library, 45 min VM | One VM Miri process per job, 1800 s per invocation |
| `miri-pointer-controls` | 45 min | One VM Miri process per control |
| `miri-pointer-batches` | 60 min | One support-library parse plus 8–9 controls in sequence |
| `miri.yml` advisory shards | 70 min | Cold-cache assembly and VM singletons |

## Blocking `miri-pointer-controls` lane

The pointer lane runs the enabled VM pointer controls under Miri in no-features and
multithreading configurations. It is not a full VM Miri suite.

- `pointer_mode=batched` (selected push/PR runs): `miri-pointer-batches` runs each batch as one
  Miri process that calls its controls in sequence and prints `batch-control ok <name>` lines.
  Batches are `pointer_provenance_tests::batches::batch_N`; they skip p13 under
  `memory-validation`.
- `pointer_mode=singleton` (every full sweep, including selector fallback):
  `miri-pointer-controls` runs one Miri process per control shard.
- If a batch fails, the singleton jobs re-run to attribute it. The lane still fails: a batch
  failure with all singletons passing indicates an interaction between controls or a flake.

Batch membership is authored once, in the third column of `expected_controls` in
`scripts/pointer_provenance.sh`. `verify-batches` checks the Rust batch lists against it, and
`verify-controls` checks native discovery, so an `#[ignore]`d control fails before Miri runs.
Six ignored probes are diagnostic-only and never run in CI.

## Running checks locally

### Toolchains and Miri flags

The stable toolchain is set in `rust-toolchain.toml`. Miri, fuzz targets and provenance lints use
the nightly pinned in the workflows, **`nightly-2026-05-27`**. The commands below and in
[FUZZING.md](FUZZING.md) use it through `$NIGHTLY`:

```bash
NIGHTLY=nightly-2026-05-27
rustup toolchain install "$NIGHTLY" --component miri
export MIRIFLAGS="-Zmiri-tree-borrows -Zmiri-disable-isolation -Zmiri-ignore-leaks"
```

Every Miri job uses these flags. The one exception is the isolated strict-core cross-arena
singleton, which appends `-Zmiri-strict-provenance`.

### `check.sh`

`./check.sh` defaults to `--fast` (`CHECK_FULL=1` or `--full` selects full mode). Both modes run
`xtask verify-slots`, build fixtures once, run clippy and `cargo nextest run` per feature
configuration, then the two ceiling scripts.

- `--fast`: three configurations (no features, `multithreading`, `multithreading,validation-all`).
- `--full`: the whole shared feature matrix, plus the build-script probes and the
  `bench-instrumentation`, `heap-diagnostics` and `deadlock-diagnostics` guards.

Neither mode runs Miri, Loom, doc lint or fuzz replay, and neither selects by changed paths.

### Miri

Reproduce a library job with its selector from `ci.yml`, for example:

```bash
cargo +"$NIGHTLY" miri test -p dotnet-value -- --test-threads=1
cargo +"$NIGHTLY" miri test -p dotnet-vm-data --lib --no-default-features frame_alloc:: -- --test-threads=1
```

Reproduce one cross-arena row with its selector from the `miri-cross-arena` `include` list:

```bash
cargo +"$NIGHTLY" miri test -p dotnet-vm --lib --no-default-features --features multithreading \
  SELECTOR -- --exact --test-threads=1
```

Reproduce one advisory shard with the same guard as CI:

```bash
python3 scripts/miri_advisory.py execute --configuration no-features --shard V01
```

### Pointer lane

`scripts/pointer_provenance.sh` pins the toolchain and flags itself. Add `--dry-run` to print
commands without running Cargo.

```bash
bash scripts/pointer_provenance.sh list
bash scripts/pointer_provenance.sh verify-controls --configuration no-features
bash scripts/pointer_provenance.sh batch --configuration multithreading --batch 2
bash scripts/pointer_provenance.sh controls --configuration no-features --shard 1
```

Ignored probes run only through the diagnostic entrypoint, one probe per process. Known-UB probes
must not run natively; `--lane native` is permitted only for p19.

```bash
bash scripts/pointer_provenance.sh diagnostic --configuration no-features --probe p19_ldelem_negative_index
```

### Loom

```bash
DOTNET_SKIP_BUILD=1 RUSTFLAGS="--cfg loom" \
  cargo nextest run -p dotnet-vm --test loom_stw --no-default-features
```

Loom is a rustc `cfg`, not a Cargo feature. The model is exhaustive at three threads (the
collector and two mutators).

### Source policy and docs

```bash
bash scripts/check_mt_cfg_ceiling.sh
bash scripts/check_std_sync_ceiling.sh
cargo run -p xtask -- verify-slots
bash scripts/check_doc_drift.sh
DOTNET_SKIP_BUILD=1 RUSTDOCFLAGS="-D rustdoc::broken_intra_doc_links" cargo doc --no-deps --no-default-features
bash scripts/check_build_script_regressions.sh
for t in scripts/tests/test_*.py; do python3 "$t"; done
```

The ceiling scripts are lexical regression checks. When a change has to raise a ceiling, explain
the boundary and the delta in that change. When the count falls, lower the ceiling.

### Fixtures and build environment

```bash
cargo run -p xtask -- fixtures build             # writes .fixtures_hash next to the fixtures
DOTNET_USE_PREBUILT_FIXTURES=1 cargo nextest run
cargo run -p xtask -- fixtures build --output-dir /tmp/fx
DOTNET_FIXTURES_BASE=/tmp/fx DOTNET_USE_PREBUILT_FIXTURES=1 cargo nextest run
cargo run -p xtask -- fixtures output-dir [--profile P] [--target T]
cargo run -p xtask -- fixtures cache-key --profile debug
```

| Variable | Effect |
|---|---|
| `DOTNET_SKIP_BUILD=1` | `dotnet-assemblies` and `dotnet-cli` build scripts skip dotnet restore/build and use stub or metadata-only paths. Auto-enabled under `cargo clippy`. |
| `DOTNET_USE_PREBUILT_FIXTURES=1` | `dotnet-cli/build.rs` never builds fixtures. It validates `.fixtures_hash` and fails on stale or missing artifacts. Takes priority over the other variables. |
| `DOTNET_FIXTURES_BASE=/path` | Fixture artifacts live at this path instead of `target/<profile>/dotnet-fixtures`. |
| `DOTNET_TEST_FILTER=<substring>` | Generate fixture tests only for matching fixture paths. |
| `DOTNET_TEST_TIMEOUT_SECS` | Per-fixture-test timeout used by the integration tests. |

### Valgrind

```bash
cargo test -p dotnet-cli --test integration_tests --no-run --no-default-features
BINARY=$(ls -t target/debug/deps/integration_tests-* | grep -v '\.d$' | head -1)
valgrind --suppressions=valgrind.supp --error-exitcode=1 --leak-check=full \
  --show-leak-kinds=all -s "$BINARY" --test-threads=1 --exact TEST
```

Take `TEST` and the feature legs from `valgrind.yml`.

### Hosted runs

Dispatch only when authorized: `gh workflow run ci.yml --ref "$BRANCH"` (likewise `miri.yml`,
`fuzz.yml`, `valgrind.yml`). `gh run rerun RUN --job JOB` repeats a job on that run's original
SHA. A new push to the same ref cancels the older `ci.yml` run. Advisory success alone does not
show that tests ran; check the job logs.

## Policy notes

### Strict provenance

No Miri lane runs `-Zmiri-strict-provenance` except the isolated cross-arena transport-core
singleton. The VM still exposes and reconstructs provenance for unmanaged addresses. The
`provenance-lints` job makes every such cast explicit (`expose_provenance` /
`with_exposed_provenance*`); it is a source-level check, not Miri evidence.

### `valgrind.supp`

Suppress only false positives whose allocation stack originates outside `dotnet-rs` crates (Rust
runtime, test harness, external dependencies). Never suppress a leak that originates in a
`dotnet-rs` crate. Run with `-s`, inspect the stack, and give each suppression a comment naming
its source.

### Self-hosted runners

`miri-cross-arena`, `miri-pointer-batches` and `miri-pointer-controls` in `ci.yml`, and
`advisory-discovery` and `advisory-shards` in `miri.yml`, select the labels
`[self-hosted, linux, x64, miri]` only when all of these hold:

- the repository is `nickbclifford/dotnet-rs`;
- the ref is `main` or `master`;
- the event is `push`, `schedule` or `workflow_dispatch`;
- `vars.SELF_HOSTED_MIRI == 'on'`.

Otherwise the job runs on `ubuntu-latest`. The routing expression only schedules jobs. The runner's
own admission gate rejects PR and feature-branch jobs. Self-hosted jobs skip toolchain, cache and
setup steps. A `dotnet-rs-check-pins` step fails the job if the image's pinned toolchains differ
from the checkout. Budgets stay sized for hosted runners, and `CI_DIAG_CACHE` records the runner
so self-hosted timings are not mixed with hosted samples. The host and image live in the separate
`dotnet-rs-runner` repository.

## See also

- [Validation Features](VALIDATION_FEATURES.md)
- [Fuzzing](FUZZING.md)
- [Benchmark Workflow](BENCHMARK_WORKFLOW.md)
- [CI evidence](ci-evidence/)

# CI Architecture and Local Validation

This document describes the CI workflows for `dotnet-rs`, why they are split, and how to run equivalent checks locally.

## Design Principle

Deterministic correctness checks are blocking, including the `dotnet-value` Miri leg and the `fuzz_raw_memory_access` corpus replay. Other instrumentation-heavy checks (the remaining fuzzing and Miri legs, and Valgrind) are informative and non-blocking.

## Workflow Table

| Workflow file                    | Toolchain | Blocking?                | Trigger                        | Purpose                              |
|----------------------------------|-----------|--------------------------|--------------------------------|--------------------------------------|
| `.github/workflows/ci.yml`       | stable / pinned nightly | Yes | push/PR to `main`/`master`     | Source policy, format, clippy, test, `miri-value`, and `fuzz-raw-memory-access` gates |
| `.github/workflows/fuzz.yml`     | nightly   | No (`continue-on-error`) | push/PR to `main` + daily cron | Fuzzing coverage                     |
| `.github/workflows/miri.yml`     | nightly-2026-05-27 | No (`continue-on-error`) | push/PR to `main` + daily cron | UB and memory-safety checks          |
| `.github/workflows/valgrind.yml` | stable    | No (`continue-on-error`) | push/PR to `main` + daily cron | Leak and uninitialized-memory checks |

## `ci.yml` — Blocking Correctness Gate

`ci.yml` runs the main test suite across a matrix of feature combinations to ensure that validation logic does not regress. See [Validation Features](VALIDATION_FEATURES.md) for a detailed breakdown of what each feature validates.

Jobs:

1. `doc-lint` (Documentation and Source Policy Checks): enforces the multithreading cfg-occurrence budget with `scripts/check_mt_cfg_ceiling.sh`, runs `scripts/check_doc_drift.sh` (doc-to-code drift detector), and runs a broken intra-doc-link check (`RUSTDOCFLAGS="-D rustdoc::broken_intra_doc_links" cargo doc --no-deps --no-default-features`, with `DOTNET_SKIP_BUILD=1`)
2. `format`: `cargo fmt --all -- --check`
3. `matrix-definitions`: resolves the clippy/test feature matrices from `xtask`; the `clippy` and `test` jobs depend on it
4. `clippy`: feature matrix resolved from `xtask`
   - source of truth: `cargo run --quiet -p xtask -- matrix clippy-features --format json`
   - CI sets `DOTNET_SKIP_BUILD=1` for this job as an explicit analysis-only guardrail
5. `build-script-regression`: targeted probes for build-script skip/env/rerun invalidation behavior
6. `build-fixtures`: uses `xtask` to resolve fixture output path and compute the fixture cache key from the same input set used by `dotnet-cli/build.rs` (`.cs` fixtures, fixture `.csproj` files, and shared MSBuild/NuGet config candidates)
7. `test`: feature matrix resolved from `xtask` (`cargo run --quiet -p xtask -- matrix test-features --format json`); its `multithreading` leg also runs the hang-probe integration-test step
8. `miri-value`: pinned-nightly, blocking Miri test suite for `dotnet-value`
9. `fuzz-raw-memory-access`: pinned-nightly, blocking replay of the committed `fuzz_raw_memory_access` corpus (`-runs=0`) with `cargo-fuzz` 0.13.1

Hang probes use tighter timeouts and run these filters individually:

- `integration_tests_impl::fixtures::test_allocation_pressure`
- `integration_tests_impl::fixtures::test_gc_coordinator`
- `integration_tests_impl::fixtures::test_multiple_arenas`
- `integration_tests_impl::fixtures::test_stw_stress`

### Feature Ownership (Post-Extraction)

Feature forwarding in `dotnet-vm` is intentionally limited to crates that actually gate source code on that feature:

| Feature | Crates Owning `#[cfg(feature = ...)]` Code |
|---|---|
| `multithreading` | `dotnet-vm`, `dotnet-runtime-resolver`, `dotnet-runtime-memory`, `dotnet-pinvoke`, `dotnet-tracer`, `dotnet-utils`, `dotnet-value` |
| `memory-validation` | `dotnet-vm`, `dotnet-runtime-memory`, `dotnet-utils`, `dotnet-value` |
| `fuzzing` | `dotnet-vm`, `dotnet-pinvoke`, `dotnet-types`, `dotnet-utils`, `dotnet-value` |

## Local Equivalent

### Build/Fixture Environment Contract

These env vars define the build-script behavior for analysis-only and fixture workflows:

| Variable | Used by | Effect |
|---|---|---|
| `DOTNET_SKIP_BUILD=1` | `dotnet-assemblies/build.rs`, `dotnet-cli/build.rs` | Skip dotnet restore/build and use stub/metadata-only paths for analysis commands (`dotnet-cli` applies this in non-prebuilt mode). |
| `DOTNET_USE_PREBUILT_FIXTURES=1` | `dotnet-cli/build.rs` | Do not run dotnet restore/build for fixtures; require prebuilt fixture tree and validate `.fixtures_hash`. |
| `DOTNET_FIXTURES_BASE=/path` | `dotnet-cli/build.rs` | Read/write fixture artifacts at this path instead of the default Cargo-derived path. |
| `DOTNET_TEST_FILTER=<substring>` | `dotnet-cli/build.rs` | Restrict generated fixture tests to matching fixture paths. |

Precedence in `dotnet-cli/build.rs`:

1. `DOTNET_USE_PREBUILT_FIXTURES=1` takes priority; build.rs validates prebuilt artifacts/hash and never runs dotnet build/restore in that mode.
2. Otherwise build.rs compares fixture hash and only runs dotnet build/restore when artifacts are stale and skip mode is not active.
3. `cargo clippy` auto-enables skip behavior in both dotnet build scripts via `clippy-driver` wrapper detection (`RUSTC_WORKSPACE_WRAPPER`/`RUSTC_WRAPPER`/`CLIPPY_ARGS`).

### Building Fixtures Locally

If you are working on the VM and don't want to wait for .NET compilation during every `cargo test` run, you can build the fixtures once:

```bash
cargo run -p xtask -- fixtures build
```

The convenience script is a thin wrapper over the same xtask entrypoint:

```bash
./scripts/build_fixtures.sh
```

Then run tests using the prebuilt fixtures:

```bash
DOTNET_USE_PREBUILT_FIXTURES=1 cargo test
```

`xtask fixtures build` now writes `.fixtures_hash` next to the compiled fixture tree. When
`DOTNET_USE_PREBUILT_FIXTURES=1` is set, `dotnet-cli/build.rs` validates that hash against
current fixture inputs and fails fast on stale, missing, or incomplete artifacts (including
missing DLLs).

You can also specify a custom output directory:

```bash
cargo run -p xtask -- fixtures build --output-dir /tmp/my-fixtures
DOTNET_FIXTURES_BASE=/tmp/my-fixtures DOTNET_USE_PREBUILT_FIXTURES=1 cargo test
```

Or use Cargo-style output conventions (matching `dotnet-cli/build.rs` path expectations):

```bash
# Default convention:
cargo run -p xtask -- fixtures output-dir
# -> target/debug/dotnet-fixtures

# Non-default profile:
cargo run -p xtask -- fixtures output-dir --profile bench-fat
# -> target/bench-fat/dotnet-fixtures

# Explicit target triple:
cargo run -p xtask -- fixtures output-dir --target x86_64-unknown-linux-gnu
# -> target/x86_64-unknown-linux-gnu/debug/dotnet-fixtures
```

For cache debugging, you can print the same fixture cache key input digest used by CI:

```bash
cargo run -p xtask -- fixtures cache-key --profile debug
```

Historical `crates/dotnet-cli/tests/bin` and `crates/dotnet-cli/tests/obj` directories are local/legacy MSBuild artifacts and are intentionally ignored (via root `.gitignore` `bin/` and `obj/` patterns). They are not used by current Cargo build-script output paths, which are target-tree based (`target/<profile>/dotnet-fixtures` and `target/.../build/.../out`).

### Analysis-Only Commands (`cargo clippy`)

`crates/dotnet-assemblies/build.rs` and `crates/dotnet-cli/build.rs` skip dotnet restore/build work during `cargo clippy` by detecting `clippy-driver` through Cargo wrapper env (`RUSTC_WORKSPACE_WRAPPER`/`RUSTC_WRAPPER`).

If you need to force skip behavior outside clippy (for example a local `cargo check` run), set:

```bash
DOTNET_SKIP_BUILD=1 cargo check --workspace --all-targets
```

### Full Check Matrix

```bash
# Format
cargo fmt --all -- --check

# Clippy matrix (source of truth from xtask)
readarray -t CLIPPY_FEATURES < <(
  cargo run --quiet -p xtask -- matrix clippy-features --format lines
)
for features in "${CLIPPY_FEATURES[@]}"; do
  if [ -z "$features" ]; then
    cargo clippy --all-targets --no-default-features -- -D warnings
  else
    cargo clippy --all-targets --no-default-features --features "$features" -- -D warnings
  fi
done

# Test matrix (source of truth from xtask)
readarray -t TEST_FEATURES < <(
  cargo run --quiet -p xtask -- matrix test-features --format lines
)
for features in "${TEST_FEATURES[@]}"; do
  if [ -z "$features" ]; then
    DOTNET_TEST_TIMEOUT_SECS=180 cargo test --no-default-features -- --nocapture --test-threads=1
  elif [[ "$features" == *"multithreading"* ]]; then
    DOTNET_TEST_TIMEOUT_SECS=180 cargo test --no-default-features --features "$features" -- --nocapture
  else
    DOTNET_TEST_TIMEOUT_SECS=180 cargo test --no-default-features --features "$features" -- --nocapture --test-threads=1
  fi
done

# Hang probes (multithreading only)
for TEST in \
    "integration_tests_impl::fixtures::test_allocation_pressure" \
    "integration_tests_impl::fixtures::test_gc_coordinator" \
    "integration_tests_impl::fixtures::test_multiple_arenas" \
    "integration_tests_impl::fixtures::test_stw_stress"; do
  DOTNET_TEST_TIMEOUT_SECS=60 timeout 300 \
    cargo test --no-default-features --features multithreading \
      -p dotnet-cli --test integration_tests "$TEST" \
      -- --test-threads=1 --nocapture
done
```

Or run:

```bash
bash check.sh
```

### Build-Script Regression Probes

The repository includes a dedicated probe script that validates three contracts:

1. clean `cargo clippy --workspace --all-targets` does not invoke dotnet/MSBuild work.
2. toggling `DOTNET_SKIP_BUILD` invalidates `dotnet-assemblies` output (`support.dll` stub -> rebuilt non-empty DLL).
3. `dotnet-vm` directory-level `rerun-if-changed` retriggers on add/remove under `src/intrinsics`.

Run locally with:

```bash
bash scripts/check_build_script_regressions.sh
```

### Documentation Drift Check

The `doc-lint` job runs the doc-to-code drift detector and the broken intra-doc-link check.
Run locally with:

```bash
bash scripts/check_doc_drift.sh
DOTNET_SKIP_BUILD=1 RUSTDOCFLAGS="-D rustdoc::broken_intra_doc_links" \
  cargo doc --no-deps --no-default-features
```

### Multithreading cfg-Occurrence Budget

The blocking `doc-lint` job and `check.sh` both run a ratcheted source-policy check that prevents new `feature = "multithreading"` forks from silently accumulating. The current ceiling lives in the script and is lowered whenever the canonical count drops.

Run it locally with:

```bash
bash scripts/check_mt_cfg_ceiling.sh
```

The blocking `miri-value` and `fuzz-raw-memory-access` jobs above are also available as local commands in their respective sections below. The latter replays the committed corpus only; exploratory fuzzing remains advisory in `fuzz.yml`.

## `fuzz-raw-memory-access` — Blocking Corpus Replay

The gate pins `nightly-2026-05-27` and `cargo-fuzz` 0.13.1, then replays the tracked
`crates/dotnet-value/fuzz/corpus/fuzz_raw_memory_access` inputs without generating new cases:

```bash
cd crates/dotnet-value
cargo +nightly-2026-05-27 fuzz run fuzz_raw_memory_access -- -runs=0
```

The duration-based targets in `fuzz.yml`, including a second exploratory run of
`fuzz_raw_memory_access`, remain advisory.

## `miri.yml` — Non-Blocking UB Checks

`miri.yml` remains an advisory matrix. Its `dotnet-value` entry provides additional scheduled coverage; the required `dotnet-value` Miri gate is the separate `miri-value` job in `ci.yml`.

The workflow runs per crate (`fail-fast: false`):

| Crate               | Args                                                                    | Notes                                                           |
|---------------------|-------------------------------------------------------------------------|-----------------------------------------------------------------|
| `dotnet-value`      | `-- --test-threads=1`                                                   | Full crate test suite under Miri                                |
| `dotnet-utils`      | `--no-default-features -- --test-threads=1`                             | Utility tests without default features                          |
| `dotnet-assemblies` | `-- --test-threads=1`                                                   | Filesystem-dependent tests are conditionally skipped under Miri |
| `dotnet-vm`         | `--no-default-features -- --test-threads=1 jmp_tests tail_calls fault_tests` | Targeted VM unsafe-gate suite without multithreading            |
| `dotnet-vm`         | `--no-default-features --features multithreading -- --test-threads=1 jmp_tests tail_calls fault_tests` | Same conservative VM unsafe-gate filters with multithreading paths enabled |

The two `dotnet-vm` entries deliberately use the same `jmp_tests`, `tail_calls`, and
`fault_tests` filter set. This keeps the Miri workload bounded while running the second entry
with the VM's `multithreading` configuration enabled. That entry compiles the cross-arena and
stop-the-world paths, but the filtered suite and local time box have not established that every
such path is dynamically exercised. The filters are conservative because the transitive
`parking_lot` stack has known Miri limitations. The separate `dotnet-utils` matrix leg remains
advisory along with the other entries.

The workflow sets `MIRIFLAGS="-Zmiri-tree-borrows -Zmiri-disable-isolation -Zmiri-ignore-leaks"`.
Strict provenance is currently infeasible for `dotnet-vm` because dependency-level
integer-to-pointer casts are reached during assembly parsing before VM unsafe
sites execute.

### Strict-provenance deferral

No current Miri job adds `-Zmiri-strict-provenance`. Plan 01's Phase 9 CI work
was explicitly closed by owner-directed deferral on 2026-08-05; this is an
accepted missing gate, not implied coverage. In particular:

- `dotnet-value` remains known-red locally at the atomic GC-handle
  `gc_handle_from_addr` storage boundary, with continuation triage also reaching
  serialized `ObjectRef::read_unchecked` reconstruction.
- `dotnet-runtime-memory` passed its pinned-nightly strict-provenance baseline,
  but it has never had an entry in this advisory matrix and was not added solely
  to satisfy the plan's stale gate wording.
- both `dotnet-vm` entries retain their existing flags and filters unchanged.

Reopening strict-provenance CI requires explicit authorization and a green local
run for each intended leg. Do not enable a known-red matrix entry or infer strict
coverage from the ordinary `miri-value` job in `ci.yml`.

The workflow pins `nightly-2026-05-27`, which locally reports
`rustc 1.98.0-nightly (d1fc603d1 2026-05-26)`. Both documented `dotnet-vm`
unsafe-gate commands were attempted with that toolchain. The multithreading
command completed `fault_tests::tests::test_fault_handler_executed_on_exception`,
but, like the no-feature command, did not complete within five minutes after
starting the second `fault_tests` test and emitting the known dependency
integer-to-pointer-cast warnings. Consequently, this pin records attempted
coverage rather than an established passing result for either targeted suite.
The matrix job therefore remains explicitly non-blocking with `continue-on-error: true`.

Local commands:

```bash
rustup toolchain install nightly-2026-05-27
rustup component add miri --toolchain nightly-2026-05-27

MIRIFLAGS="-Zmiri-tree-borrows -Zmiri-disable-isolation -Zmiri-ignore-leaks" \
cargo +nightly-2026-05-27 miri test -p dotnet-value -- --test-threads=1
MIRIFLAGS="-Zmiri-tree-borrows -Zmiri-disable-isolation -Zmiri-ignore-leaks" \
cargo +nightly-2026-05-27 miri test -p dotnet-assemblies -- --test-threads=1
MIRIFLAGS="-Zmiri-tree-borrows -Zmiri-disable-isolation -Zmiri-ignore-leaks" \
cargo +nightly-2026-05-27 miri test -p dotnet-vm --no-default-features -- --test-threads=1 jmp_tests tail_calls fault_tests
MIRIFLAGS="-Zmiri-tree-borrows -Zmiri-disable-isolation -Zmiri-ignore-leaks" \
cargo +nightly-2026-05-27 miri test -p dotnet-vm --no-default-features --features multithreading -- --test-threads=1 jmp_tests tail_calls fault_tests
```

## `valgrind.yml` — Non-Blocking Leak/Uninit Checks

The workflow builds `dotnet-cli` integration tests across two legs — first `--no-default-features`,
then `--no-default-features --features multithreading` — and runs a curated subset of integration
tests under Valgrind on each leg.

Local commands:

```bash
sudo apt-get install -y valgrind libc6-dbg

# --- No-features leg ---
cargo test -p dotnet-cli --test integration_tests \
  --no-run --no-default-features

BINARY=$(ls -t target/debug/deps/integration_tests-* | grep -v '\.d$' | head -1)
echo "Binary: $BINARY"

for TEST in \
  integration_tests_impl::fixtures::hello_world \
  integration_tests_impl::fixtures::memory_nullable_boxing_42; do
  timeout 600s valgrind \
    --suppressions=valgrind.supp \
    --error-exitcode=1 \
    --leak-check=full \
    --show-leak-kinds=all \
    -s \
    "$BINARY" \
    --test-threads=1 \
    --nocapture \
    --exact \
    "$TEST"
done

# --- Multithreading leg ---
cargo test -p dotnet-cli --test integration_tests \
  --no-run --no-default-features --features multithreading

BINARY=$(ls -t target/debug/deps/integration_tests-* | grep -v '\.d$' | head -1)
echo "Binary: $BINARY"

for TEST in \
  integration_tests_impl::fixtures::threading_monitor_try_enter_timeout_42 \
  integration_tests_impl::fixtures::threading_monitor_try_enter_timeout_single_42; do
  timeout 600s valgrind \
    --suppressions=valgrind.supp \
    --error-exitcode=1 \
    --leak-check=full \
    --show-leak-kinds=all \
    -s \
    "$BINARY" \
    --test-threads=1 \
    --nocapture \
    --exact \
    "$TEST"
done
```

## `valgrind.supp` Suppression Policy

`valgrind.supp` should suppress only known false positives from the Rust runtime/test harness or external dependencies.

When adding a suppression:

1. Run Valgrind with `-s` and inspect the allocation stack.
2. Add suppression only if the allocation does not originate in `dotnet-rs` code.
3. Add a brief comment in `valgrind.supp` describing the source and date.
4. Keep the workflow subset table in this document up to date.

Never suppress leaks that originate in `dotnet-rs` crates.

## Valgrind Subset in CI

| Build leg                              | Test                                                                              |
|----------------------------------------|-----------------------------------------------------------------------------------|
| no features                            | `integration_tests_impl::fixtures::hello_world`                                   |
| no features                            | `integration_tests_impl::fixtures::memory_nullable_boxing_42`                     |
| `multithreading`                       | `integration_tests_impl::fixtures::threading_monitor_try_enter_timeout_42`        |
| `multithreading`                       | `integration_tests_impl::fixtures::threading_monitor_try_enter_timeout_single_42` |

## See Also

- [Validation Features](VALIDATION_FEATURES.md)
- [Fuzzing](FUZZING.md)
- [Benchmark Workflow](BENCHMARK_WORKFLOW.md)
- [Threading and Synchronization](THREADING_AND_SYNCHRONIZATION.md)
- [GC and Memory Safety](GC_AND_MEMORY_SAFETY.md)

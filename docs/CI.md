# CI Architecture and Local Validation

This document describes the CI workflows for `dotnet-rs`, why they are split, and how to run equivalent checks locally.

## Design Principle

Deterministic correctness checks are blocking. Instrumentation-heavy checks (fuzzing, Miri, Valgrind) are informative and non-blocking.

## Workflow Table

| Workflow file                    | Toolchain | Blocking?                | Trigger                        | Purpose                              |
|----------------------------------|-----------|--------------------------|--------------------------------|--------------------------------------|
| `.github/workflows/ci.yml`       | stable    | Yes                      | push/PR to `main`/`master`     | Format, clippy, and test gates       |
| `.github/workflows/fuzz.yml`     | nightly   | No (`continue-on-error`) | push/PR to `main` + daily cron | Fuzzing coverage                     |
| `.github/workflows/miri.yml`     | nightly   | No (`continue-on-error`) | push/PR to `main` + daily cron | UB and memory-safety checks          |
| `.github/workflows/valgrind.yml` | stable    | No (`continue-on-error`) | push/PR to `main` + daily cron | Leak and uninitialized-memory checks |

## `ci.yml` — Blocking Correctness Gate

`ci.yml` runs the main test suite across a matrix of feature combinations to ensure that validation logic does not regress. See [Validation Features](VALIDATION_FEATURES.md) for a detailed breakdown of what each feature validates.

Jobs:

1. `format`: `cargo fmt --all -- --check`
2. `clippy`: feature matrix resolved from `xtask`
   - source of truth: `cargo run --quiet -p xtask -- matrix clippy-features --format json`
   - CI sets `DOTNET_SKIP_BUILD=1` for this job as an explicit analysis-only guardrail
3. `build-script-regression`: targeted probes for build-script skip/env/rerun invalidation behavior
4. `build-fixtures`: uses `xtask` to resolve fixture output path and compute the fixture cache key from the same input set used by `dotnet-cli/build.rs` (`.cs` fixtures, fixture `.csproj` files, and shared MSBuild/NuGet config candidates)
5. `test`: feature matrix resolved from `xtask` (`cargo run --quiet -p xtask -- matrix test-features --format json`)
6. `hang-probe integration tests`: run only on the `multithreading` test leg

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

## `miri.yml` — Non-Blocking UB Checks

The workflow runs per crate (`fail-fast: false`):

| Crate               | Args                                | Notes                                                           |
|---------------------|-------------------------------------|-----------------------------------------------------------------|
| `dotnet-value`      | `-- --test-threads=1`               | Full crate test suite under Miri                                |
| `dotnet-assemblies` | `-- --test-threads=1`               | Filesystem-dependent tests are conditionally skipped under Miri |
| `dotnet-vm`         | `-- --test-threads=1 --skip stress` | `stress_*` tests are skipped due to runtime cost under Miri     |

Local commands:

```bash
rustup toolchain install nightly
rustup component add miri --toolchain nightly

cargo +nightly miri test -p dotnet-value -- --test-threads=1
cargo +nightly miri test -p dotnet-assemblies -- --test-threads=1
cargo +nightly miri test -p dotnet-vm -- --test-threads=1 --skip stress
```

## `valgrind.yml` — Non-Blocking Leak/Uninit Checks

The workflow builds `dotnet-cli` integration tests with:

```bash
--no-default-features --features multithreading
```

It then runs a curated subset of integration tests under Valgrind.

Local commands:

```bash
sudo apt-get install -y valgrind libc6-dbg

cargo test -p dotnet-cli --test integration_tests \
  --no-run --no-default-features --features multithreading

BINARY=$(ls target/debug/deps/integration_tests-* | grep -v '\.d$' | head -1)
echo "Binary: $BINARY"

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
  integration_tests_impl::fixtures::hello_world

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
  integration_tests_impl::fixtures::threading_monitor_try_enter_timeout_single_42
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

| Test                                                                              |
|-----------------------------------------------------------------------------------|
| `integration_tests_impl::fixtures::hello_world`                                   |
| `integration_tests_impl::fixtures::threading_monitor_try_enter_timeout_single_42` |

## See Also

- [`VALIDATION_FEATURES.md`](VALIDATION_FEATURES.md)
- [`FUZZING.md`](FUZZING.md)
- [`BENCHMARK_WORKFLOW.md`](BENCHMARK_WORKFLOW.md)
- [`THREADING_AND_SYNCHRONIZATION.md`](THREADING_AND_SYNCHRONIZATION.md)
- [`GC_AND_MEMORY_SAFETY.md`](GC_AND_MEMORY_SAFETY.md)

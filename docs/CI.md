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
2. `clippy`: feature matrix
   - `--no-default-features`
   - `--no-default-features --features multithreading`
   - `--no-default-features --features generic-constraint-validation`
   - `--no-default-features --features memory-validation`
   - `--no-default-features --features metadata-validation`
   - `--no-default-features --features multithreading,memory-validation`
   - `--no-default-features --features multithreading,validation-all`
   - `--no-default-features --features fuzzing`
3. `test`: same matrix except the `fuzzing` leg
4. `hang-probe integration tests`: run only on the `multithreading` test leg

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

### Building Fixtures Locally

If you are working on the VM and don't want to wait for .NET compilation during every `cargo test` run, you can build the fixtures once:

```bash
./scripts/build_fixtures.sh
```

Then run tests using the prebuilt fixtures:

```bash
DOTNET_USE_PREBUILT_FIXTURES=1 cargo test
```

You can also specify a custom output directory:

```bash
./scripts/build_fixtures.sh --output-dir /tmp/my-fixtures
DOTNET_FIXTURES_BASE=/tmp/my-fixtures DOTNET_USE_PREBUILT_FIXTURES=1 cargo test
```

### Full Check Matrix

```bash
# Format
cargo fmt --all -- --check

# Clippy matrix
cargo clippy --all-targets --no-default-features -- -D warnings
cargo clippy --all-targets --no-default-features --features multithreading -- -D warnings
cargo clippy --all-targets --no-default-features --features generic-constraint-validation -- -D warnings
cargo clippy --all-targets --no-default-features --features memory-validation -- -D warnings
cargo clippy --all-targets --no-default-features --features metadata-validation -- -D warnings
cargo clippy --all-targets --no-default-features --features multithreading,memory-validation -- -D warnings
cargo clippy --all-targets --no-default-features --features multithreading,validation-all -- -D warnings
cargo clippy --all-targets --no-default-features --features fuzzing -- -D warnings

# Test matrix
DOTNET_TEST_TIMEOUT_SECS=180 cargo test --no-default-features -- --nocapture --test-threads=1
DOTNET_TEST_TIMEOUT_SECS=180 cargo test --no-default-features --features multithreading -- --nocapture
DOTNET_TEST_TIMEOUT_SECS=180 cargo test --no-default-features --features generic-constraint-validation -- --nocapture --test-threads=1
DOTNET_TEST_TIMEOUT_SECS=180 cargo test --no-default-features --features memory-validation -- --nocapture --test-threads=1
DOTNET_TEST_TIMEOUT_SECS=180 cargo test --no-default-features --features metadata-validation -- --nocapture --test-threads=1
DOTNET_TEST_TIMEOUT_SECS=180 cargo test --no-default-features --features multithreading,memory-validation -- --nocapture
DOTNET_TEST_TIMEOUT_SECS=180 cargo test --no-default-features --features multithreading,validation-all -- --nocapture

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
- [`THREADING_AND_SYNCHRONIZATION.md`](THREADING_AND_SYNCHRONIZATION.md)
- [`GC_AND_MEMORY_SAFETY.md`](GC_AND_MEMORY_SAFETY.md)

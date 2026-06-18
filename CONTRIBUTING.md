# Contributing to dotnet-rs

Thanks for contributing to `dotnet-rs`.

## Prerequisites

Before running local builds or tests, install:

- **Rust (stable toolchain)**
- **.NET SDK** (required for fixture compilation and build-script paths that invoke dotnet/MSBuild)

## Run the project checks

For the standard local validation pass used by contributors, run:

```bash
bash check.sh
```

## Build test fixtures

Some test paths rely on managed fixtures. Build them with:

```bash
cargo run -p xtask -- fixtures build
```

## Miri policy for `dotnet-vm` unsafe-gate signoff

For `dotnet-vm` changes that add or modify `unsafe`, the accepted local signoff invocation is:

```bash
MIRIFLAGS="-Zmiri-tree-borrows -Zmiri-disable-isolation -Zmiri-ignore-leaks" \
cargo +nightly miri test -p dotnet-vm --no-default-features -- --test-threads=1 jmp_tests tail_calls fault_tests
```

Why this is the project-standard invocation right now:
- `-Zmiri-strict-provenance` currently fails before VM execution due dependency-level integer-to-pointer casts reached during assembly parsing (`bitvec`/`dotnetdll`, plus rayon worker-thread paths via `crossbeam-epoch`).
- `-Zmiri-disable-isolation` is needed for host filesystem syscalls used by the test harness.
- `-Zmiri-ignore-leaks` is needed for non-joined background worker threads that are not VM-unsafe regressions.

Until upstream dependency behavior changes, strict-provenance is treated as infeasible for `dotnet-vm` unsafe-gate signoff.

## Documentation drift check

If your change touches docs, run the doc drift gate and rustdoc link check:

```bash
bash scripts/check_doc_drift.sh
DOTNET_SKIP_BUILD=1 cargo doc --no-deps --no-default-features
```

## Feature flag matrix reference

Feature combinations and validation ownership are documented in:

- [`docs/VALIDATION_FEATURES.md`](docs/VALIDATION_FEATURES.md)

For a subsystem overview, see [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md). For broader CI/local validation guidance, see [`docs/CI.md`](docs/CI.md). The project overview and quick-start flow live in [`README.md`](README.md).

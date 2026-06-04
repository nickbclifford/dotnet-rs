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

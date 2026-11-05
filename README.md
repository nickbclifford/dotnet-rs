# dotnet-rs

`dotnet-rs` is an experimental Rust implementation of the ECMA-335 Virtual Execution System (VES) for loading and executing .NET assemblies. It is an active research/runtime project focused on correctness, architecture, and validation workflows, and is **not production-ready**.

## Quick start

### Prerequisites

- Rust, at the version pinned in [`rust-toolchain.toml`](rust-toolchain.toml) (selected
  automatically by rustup)
- .NET SDK installed locally (provides the runtime assemblies and supports fixture/build-script paths)

### Build the CLI

```bash
cargo build -p dotnet-cli --bin dotnet-rs
```

### Run a managed assembly (`.dll`)

1. Find your installed .NET runtime directory (example command):

   ```bash
   dotnet --list-runtimes | grep Microsoft.NETCore.App
   ```

2. Run the interpreter:

   ```bash
   cargo run -p dotnet-cli --bin dotnet-rs -- \
     -a /path/to/Microsoft.NETCore.App/<version> \
     /path/to/YourApp.dll
   ```

## Workspace layout

This repository is a Rust workspace: crates under `crates/` plus the `xtask/` automation crate.

- `crates/` — runtime, VM, intrinsic, tooling, and CLI crates
- `docs/` — architecture and subsystem design documentation
- `scripts/` — CI and maintenance scripts (including doc drift checks)
- `xtask/` — build/test automation helpers

For a subsystem-level overview, see [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md). For contributor setup and validation commands, see [`CONTRIBUTING.md`](CONTRIBUTING.md).

## CI and contributing

See [`CONTRIBUTING.md`](CONTRIBUTING.md) for how to build, test and make changes, and
[`docs/CI.md`](docs/CI.md) for what CI runs. Planned work is in
[`docs/plans/README.md`](docs/plans/README.md).

## Design documentation

Start with:

- [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md)
- [`docs/GC_AND_MEMORY_SAFETY.md`](docs/GC_AND_MEMORY_SAFETY.md)
- [`docs/EXCEPTION_HANDLING.md`](docs/EXCEPTION_HANDLING.md)
- [`docs/THREADING_AND_SYNCHRONIZATION.md`](docs/THREADING_AND_SYNCHRONIZATION.md)
- [`docs/COMPATIBILITY.md`](docs/COMPATIBILITY.md)

See the `docs/` directory for additional deep dives.

## Feature flags and validation matrix

Feature and validation flag behavior is documented in [`docs/VALIDATION_FEATURES.md`](docs/VALIDATION_FEATURES.md).

# dotnet-rs

`dotnet-rs` is an experimental Rust implementation of the ECMA-335 Virtual Execution System (VES) for loading and executing .NET assemblies. It is an active research/runtime project focused on correctness, architecture, and validation workflows, and is **not production-ready**. It runs a Newtonsoft.Json round-trip application and an EF Core InMemory application with output matching `dotnet`, and supports `DynamicMethod` but not `TypeBuilder`; see [`docs/COMPATIBILITY.md`](docs/COMPATIBILITY.md).

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

For a framework-dependent app, pass the `.dll`. The CLI reads `YourApp.runtimeconfig.json` (and
`deps.json`) next to it and locates the shared framework, honoring `DOTNET_ROOT`:

```bash
cargo run -p dotnet-cli --bin dotnet-rs -- /path/to/YourApp.dll
```

To choose the runtime assembly directory explicitly, pass `-a/--assemblies`:

```bash
dotnet --list-runtimes | grep Microsoft.NETCore.App
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
- `tests/` — out-of-workspace probe projects (`ef-probe`)
- `.github/workflows/` — CI workflows (see [`docs/CI.md`](docs/CI.md))

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
- [`docs/INVARIANT_REGISTRY.md`](docs/INVARIANT_REGISTRY.md)
- [`docs/pointer-provenance/README.md`](docs/pointer-provenance/README.md)

See the `docs/` directory for additional deep dives.

## Feature flags and validation matrix

Feature and validation flag behavior is documented in [`docs/VALIDATION_FEATURES.md`](docs/VALIDATION_FEATURES.md).

# Contributing to dotnet-rs

Thanks for contributing to `dotnet-rs`.

## Prerequisites

Before running local builds or tests, install:

- **Rust 1.95.0**, selected by the repository's `rust-toolchain.toml` and
  including the `clippy` and `rustfmt` components. Rustup activates this pin
  automatically when commands are run from the repository.
- **.NET SDK** (required for fixture compilation and build-script paths that invoke dotnet/MSBuild)

## Unsafe code policy

Unsafe code must make the invariant that justifies it reviewable at the point
where it is relied on. Follow these rules when adding or changing unsafe code:

- Put a `// SAFETY:` comment immediately above every `unsafe { ... }` block.
  The comment must explain the invariant that makes the operation valid; it
  must not merely restate the operation.
- Do not use a `where` clause as proof for `unsafe impl Send` or `unsafe impl
  Sync`: a bound only makes the implementation conditional. Use an
  unconditional implementation when it is valid (retaining any required
  feature `cfg`), document its SAFETY invariant, and add an
  `assert_impl_all!` guard in a `#[cfg(test)]` block to mechanically check the
  intended traits.
- An `unsafe impl Collect` must trace every contained `Gc<'gc, _>` handle. For
  types that are `'static` and contain no GC handles, use `static_collect!`.
  For a non-`'static` type with no GC handles, write
  `const NEEDS_TRACE: bool = false` in its `Collect` implementation.
- A layout transmute must have a co-located size assertion and a `SAFETY:`
  comment that explicitly explains the representation and validity reasoning.

The workspace enforces this policy through `[workspace.lints.clippy]` in the
root `Cargo.toml`: `undocumented_unsafe_blocks = "deny"` and
`multiple_unsafe_ops_per_block = "deny"`. Member crates inherit these lints;
keep unsafe operations individually proved rather than weakening the workspace
configuration.

## Panic-vs-Result policy

When adding or changing runtime behavior, classify failure paths into two categories:

1. **VM invariant violation (bug / impossible state):** use `panic!`, `unreachable!`, `debug_assert!`, or `expect()` with a specific message.
   - These are conditions that should never be reachable if VM internals are correct.
   - Do **not** route these through `VmError`.
2. **Runtime/metadata failure (input/environment driven):** return `VmError` via `Result` / `StepResult::Error`.
   - This includes metadata resolution failures, invalid CIL, null/memory access errors, P/Invoke load/symbol failures, and other recoverable host-side execution failures.

### Host errors vs. managed exceptions

`VmError` and related enums are **host-side Rust errors**.

Managed `.NET` exceptions (`ManagedException`, `ExceptionState`, SEH search/unwind flow) are a separate mechanism described in [`docs/EXCEPTION_HANDLING.md`](docs/EXCEPTION_HANDLING.md).

Keep this distinction intact: host errors are not managed exceptions. The `.cctor` wrapping path (host error -> managed `System.TypeInitializationException`) is a bridge between layers, not a reason to merge the two models.

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
cargo +nightly-2026-05-27 miri test -p dotnet-vm --no-default-features -- --test-threads=1 jmp_tests tail_calls fault_tests
```

Why this is the project-standard invocation right now:
- `-Zmiri-strict-provenance` currently fails before VM execution due dependency-level integer-to-pointer casts reached during assembly parsing (`bitvec`/`dotnetdll`, plus rayon worker-thread paths via `crossbeam-epoch`).
- `-Zmiri-disable-isolation` is needed for host filesystem syscalls used by the test harness.
- `-Zmiri-ignore-leaks` is needed for non-joined background worker threads that are not VM-unsafe regressions.

Until upstream dependency behavior changes, strict-provenance is treated as infeasible for `dotnet-vm` unsafe-gate signoff.
The Miri-only nightly is pinned separately because `rust-toolchain.toml` selects
stable by default. As recorded in [`docs/CI.md`](docs/CI.md), the targeted VM
suite did not complete within the validation time box on this nightly, so the
pin records the attempted toolchain rather than a known-passing Miri result.

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

# Benchmark Workflow

This document defines local benchmark execution paths for `dotnet-rs`, including an optional two-pass PGO flow for benchmark binaries.

## Standard Benchmark Profiles

Use workspace benchmark profiles from the root `Cargo.toml`:

- `bench-thin`: faster turnaround (`lto = "thin"`)
- `bench-fat`: stronger optimization baseline (`lto = "fat"`, `codegen-units = 1`)

Recommended quick validation command:

```bash
cargo bench --profile bench-fat -p dotnet-benchmarks --bench end_to_end -- --sample-size 10
```

## Optional Two-Pass PGO Workflow

Use `scripts/bench_pgo.sh` to run a scripted two-pass PGO flow:

1. **Generate profile data** with `-Cprofile-generate=<dir>` by running the selected benchmark.
2. **Merge** `.profraw` files into a single `.profdata` using `llvm-profdata`.
3. **Use merged profile** with `-Cprofile-use=<file>` for a validation test build and a second benchmark run.

### Default Runbook

From repo root:

```bash
rustup component add llvm-tools-preview
./scripts/bench_pgo.sh
```

Default behavior:

- Bench target: `-p dotnet-benchmarks --bench end_to_end`
- Cargo profile: `bench-fat`
- Criterion args: `--sample-size 10`
- PGO target dir: `target/pgo-bench/`
- Includes a PGO-use validation build/test step:
  - `cargo test -p dotnet-benchmarks --no-run`

### Common Variants

Use `bench-thin` instead:

```bash
./scripts/bench_pgo.sh --profile bench-thin
```

Increase sample size:

```bash
./scripts/bench_pgo.sh --sample-size 20
```

Forward extra Criterion arguments after `--`:

```bash
./scripts/bench_pgo.sh -- --measurement-time 5
```

Skip the PGO-use test build step:

```bash
./scripts/bench_pgo.sh --no-tests
```

### Notes

- `scripts/bench_pgo.sh` expects toolchain-matched `llvm-profdata` from `llvm-tools-preview`.
- The script stores generated/merged artifacts under the selected `--target-dir` and rewrites stale profile files for repeatable runs.
- Keep this PGO path optional; baseline phase comparisons still use standard `bench-thin`/`bench-fat` commands.

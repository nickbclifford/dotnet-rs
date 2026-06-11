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

## Optional Perf/Flamegraph Traces

Use `scripts/profile_perf.sh` on Linux to capture `perf.data` for focused
runtime analysis. The script builds the selected target first, then profiles the
test or benchmark executable directly so build work does not pollute the trace.

Both modes build with the dedicated `profiling` Cargo profile (inherits
`bench-fat`, adds `debug = "full"` and `strip = false`) and force frame pointers
via `RUSTFLAGS=-Cforce-frame-pointers=yes`. This guarantees that perf and
external flamegraph explorers can unwind stacks and resolve symbols — including
inlined frames — on optimized code.

Captures use `--call-graph fp` (frame pointer unwinding) by default. `dwarf`
mode looks appealing for deep stacks but silently drops all frames on the default
`kernel.perf_event_paranoid=2` / `perf_event_mlock_kb=516` kernel configuration.
Since the profiling profile forces frame pointers into every frame, `fp` mode is
the reliable choice. Pass `--call-graph dwarf,16384` explicitly if you need
inlined-frame resolution and have tuned the kernel accordingly.

### System.Text.Json Fixture Trace

The CLI integration fixture is useful when you want to inspect the exact
correctness test for `System.Text.Json`:

```bash
./scripts/profile_perf.sh fixture --name system_text_json
```

Default behavior:

- Builds `dotnet-cli` integration tests (`profiling` profile) with `DOTNET_TEST_FILTER=system_text_json`.
- Discovers the generated `integration_tests-*` executable from Cargo JSON output.
- Runs only `integration_tests_impl::fixtures::basic_system_text_json_42`.
- Writes artifacts under `target/perf-traces/fixture-system_text_json/<timestamp>/`.

### JSON Benchmark Trace

The Criterion benchmark is a better steady-state target for flamegraph analysis:

```bash
./scripts/profile_perf.sh bench --name json --sample-size 10
```

Default behavior:

- Builds `dotnet-benchmarks` `end_to_end` with the `profiling` profile.
- Profiles only the benchmark process, not compilation.
- Runs Criterion with the `json` filter.
- Writes artifacts under `target/perf-traces/bench-json/<timestamp>/`.

### Perf Trace Artifacts

Each run writes:

- `perf.data`: raw perf profile for external flamegraph explorers.
- `perf.script`: text dump from `perf script -F +pid`, ready to load directly in
  the [Firefox Profiler](https://profiler.firefox.com) (drag the file into the UI).
- `stacks.folded` and `flamegraph.svg`: optional outputs when Inferno or Brendan
  Gregg flamegraph tools are installed.
- `command.txt`: run metadata and the discovered executable path.

Common options:

```bash
./scripts/profile_perf.sh bench --name json --frequency 499 --call-graph fp
./scripts/profile_perf.sh fixture --name system_text_json --features validation-all
./scripts/profile_perf.sh bench --name json -- --measurement-time 5
```

If `perf record` fails with a permissions error, check the local
`kernel.perf_event_paranoid` setting or run the script in an environment where
perf events are permitted.

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

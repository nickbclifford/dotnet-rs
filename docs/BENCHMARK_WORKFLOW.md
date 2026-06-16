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

## Metadata-Load Parallelism (`RAYON_NUM_THREADS`)

dotnetdll uses rayon to parallelize per-assembly metadata decoding. On many-core machines the
fork/join and work-stealing overhead can dominate actual decode work. `dotnet-cli` caps the global
rayon pool at `min(available_parallelism, 4)` by default — measured optimum on a 24-core machine
(§ measured results below). Override with `RAYON_NUM_THREADS` to test other values.

**Measured thread-count sweep (24-core, `load_framework_set/lazy`):**

| threads | 1 | 2 | 4 (default) | 8 | 24 |
|---------|---|---|-------------|---|----|
| parse time | 34.5 ms | 30.6 ms | **28.1 ms** | 28.6 ms | 32.5 ms |

All-cores default is ~12% slower than 4–8; single-thread is ~20% slower (corlib is large enough to
benefit from a few threads).

### Validate the thread-cap default (P1 regression guard)

The `cold_start/load_dominated` bench forces loading the full framework working set while executing
almost nothing. Cold − warm ≈ pure metadata-load cost, making it the tightest regression signal:

```bash
# Baseline: default cap (min(parallelism, 4))
cargo bench -p dotnet-benchmarks --bench cold_start -- 'load_dominated/lazy'

# Compare against all-cores (should be ~12% slower on many-core hosts)
RAYON_NUM_THREADS=24 cargo bench -p dotnet-benchmarks --bench cold_start -- 'load_dominated/lazy'

# Confirm single-thread penalty (should be ~20–30% slower)
RAYON_NUM_THREADS=1 cargo bench -p dotnet-benchmarks --bench cold_start -- 'load_dominated/lazy'
```

### Raw parse thread sweep

Isolates the dotnetdll decode pipeline (no I/O, no execution):

```bash
# Unset = production default (capped at 4 by dotnet-cli init)
cargo bench -p dotnet-benchmarks --bench metadata_load -- --warm-up-time 2 --measurement-time 4

# Sweep a specific thread count and filter to the load-sensitive case
RAYON_NUM_THREADS=4  cargo bench -p dotnet-benchmarks --bench metadata_load -- --warm-up-time 1 --measurement-time 3 'load_framework_set/lazy'
RAYON_NUM_THREADS=8  cargo bench -p dotnet-benchmarks --bench metadata_load -- --warm-up-time 1 --measurement-time 3 'load_framework_set/lazy'
RAYON_NUM_THREADS=24 cargo bench -p dotnet-benchmarks --bench metadata_load -- --warm-up-time 1 --measurement-time 3 'load_framework_set/lazy'
```

Note: `metadata_load` does **not** respect the `dotnet-cli` pool cap — it calls `Resolution::parse`
directly. Use `RAYON_NUM_THREADS` explicitly to control thread count in that bench.

## Resolving `[unknown]` frames in perf traces

Three categories of `[unknown]` appear in `perf script` output; each has a different cause and fix:

### 1. Libc internals (~25% of samples in a typical dotnet-rs trace)

**Cause:** libc is shipped without frame pointers, so `--call-graph fp` stops at the
libc boundary. The leaf frame shows as `[unknown]` from `/usr/lib/libc.so.6`. These are
nearly always allocator internals (`_int_malloc`, `malloc_consolidate`, `_int_free`)
called from Rust's `Vec` growth path (`alloc::raw_vec::finish_grow → realloc`).

**Fix:** Use DWARF call-graph mode, which walks `.eh_frame`/`.debug_frame` instead of
frame pointers. Requires `kernel.perf_event_mlock_kb ≥ 8192` (default is 516).

One-time setup (survives until reboot):
```bash
sudo sysctl kernel.perf_event_mlock_kb=65536
```

Permanent (create `/etc/sysctl.d/99-perf.conf`):
```
kernel.perf_event_mlock_kb = 65536
```

Once set, `profile_perf.sh` auto-detects the higher budget and switches to
`--call-graph dwarf,8192`. You can also pass it explicitly:
```bash
./scripts/profile_perf.sh fixture --name system_text_json --call-graph dwarf,8192
```

**Workaround without sudo:** `perf report` already uses DWARF and resolves these frames
correctly from the existing `perf.data` — use it for analysis even when `perf.script`
shows `[unknown]`.

### 2. PMU interrupt-skid frames (~7% of samples)

**Cause:** The hardware PMU counter fires the overflow interrupt during a user→kernel
transition (syscall entry / interrupt return). The CPU records the current instruction
pointer which lands just inside kernel text (`ffffffffa…`). These appear as
`[unknown] ([unknown])` because perf cannot associate the address with any DSO or symbol.

**Fix:** Requires PEBS (Precise Event-Based Sampling), an Intel hardware feature that
latches the exact retired instruction rather than the interrupted instruction. Not
configurable from software without hardware support. On supported CPUs:
```bash
./scripts/profile_perf.sh fixture --name system_text_json \
  --call-graph dwarf,8192 -e cpu/cycles/ppp  # 'ppp' = PEBS precise level 3
```

**Reality:** ~7% of main-thread cycles are in syscalls (mmap/mprotect from Vec growth
hitting the allocator). This is expected allocation pressure; the skid frames are an
artefact of measuring it with a regular PMU, not a real cost to fix.

## Optional Perf/Flamegraph Traces

Use `scripts/profile_perf.sh` on Linux to capture `perf.data` for focused
runtime analysis. The script builds the selected target first, then profiles the
test or benchmark executable directly so build work does not pollute the trace.
Defaults are tuned for denser captures (`--frequency 3997`, bench
`--sample-size 30`) so flamegraphs have finer resolution.

Both modes build with the dedicated `profiling` Cargo profile (inherits
`bench-fat`, adds `debug = "full"` and `strip = false`) and force frame pointers
via `RUSTFLAGS=-Cforce-frame-pointers=yes`. This guarantees that perf and
external flamegraph explorers can unwind stacks and resolve symbols — including
inlined frames — on optimized code.

The script defaults to `call-graph auto`, which picks `dwarf,8192` when
`kernel.perf_event_mlock_kb ≥ 8192` and falls back to `fp` otherwise. With `fp`,
all dotnet-rs frames unwind correctly (compiled with `-Cforce-frame-pointers=yes`)
but libc internals appear as `[unknown]`; see the "Resolving `[unknown]` frames"
section above for setup. Pass `--call-graph fp` to force frame-pointer mode even
when mlock_kb is large.

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
./scripts/profile_perf.sh bench --name json --sample-size 30
```

Default behavior:

- Builds `dotnet-benchmarks` `end_to_end` with the `profiling` profile.
- Profiles only the benchmark process, not compilation.
- Runs Criterion with the `json` filter.
- Writes artifacts under `target/perf-traces/bench-json/<timestamp>/`.

### Perf Trace Artifacts

Each run writes:

- `perf.data`: raw perf profile for external flamegraph explorers.
- `perf.script`: text dump from `perf script -F +pid --no-inline`, ready to load
  directly in the [Firefox Profiler](https://profiler.firefox.com) (drag the file into the UI).
- `perf.inline.script`: inline-expanded text dump from `perf script -F +pid`,
  used as the source for folded stacks to capture more frame detail.
- `stacks.folded` and `flamegraph.svg`: optional outputs (generated from
  `perf.inline.script`) when Inferno or Brendan Gregg flamegraph tools are installed.
- `command.txt`: run metadata and the discovered executable path.

Common options:

```bash
./scripts/profile_perf.sh bench --name json --frequency 3997 --call-graph fp
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

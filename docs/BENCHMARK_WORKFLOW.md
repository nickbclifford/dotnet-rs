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

## Pick the right target: `bench`, not `fixture`

**For flamegraphs, profile the `bench` target, not the `fixture`.** A fixture is a one-shot
correctness test that starts, runs for ~50 ms, and exits — the capture is dominated by process
startup (`ld.so`), the dynamic linker, and teardown, with only a handful of usable samples of
actual interpreter work. DWARF mode further caps the rate at 299 Hz, so a 50 ms run yields
~14 usable samples. The Criterion `bench` runs the workload in a steady-state loop for seconds,
giving thousands of dense, unwindable samples.

`profile_perf.sh` prints a `[quality]` summary after every capture (also saved to
`quality.txt`) and **warns** when a trace is too short, idle-dominated, or polluted by another
process. Use `fixture` mode only to inspect a specific correctness test, and expect the warning.

## Subprocess pollution (the dominant cause of "empty frames")

The benchmark harness lazily runs a one-time `dotnet build` to compile each fixture `.cs` into a
DLL on its first iteration. If that build lands inside `perf record`, the entire MSBuild / Roslyn
(`VBCSCompiler`) / CoreCLR process tree is captured — dozens of managed threads that DWARF cannot
unwind — and it can be **70%+ of the samples**, drowning the real signal and showing as empty or
`[unknown]` stacks.

`profile_perf.sh` prevents this automatically: it does an **untraced pre-warm run** before
recording, so the fixture DLL is built to its on-disk cache and the traced run finds it fresh and
never shells out. (`--no-prewarm` opts out.) The `[quality]` line reports `foreign-process=N%` and
warns if a subprocess slipped into the trace anyway.

## Resolving `[unknown]` frames in perf traces

Three categories of `[unknown]` appear in `perf script` output; each has a different cause:

### 1. Libc internal symbol names — resolved automatically

**Cause:** libc's internal functions (`_int_malloc`, `__memmove_avx512_unaligned_erms`,
`__libc_malloc2`, …) live only in the debuginfo `.symtab`, not in the stripped on-disk
`.dynsym`. perf never pulls these names from debuginfod — **not in `perf script`, and not even in
`perf report`** (a common misconception: `perf report` resolves *exported* libc symbols like
`malloc`/`free`, but internal ones still show as raw offsets). This is independent of frame-pointer
vs DWARF unwinding.

**Fix (automatic):** `profile_perf.sh` post-processes with `scripts/resolve_perf_syms.py`, which
re-runs `perf script` with `+dsoff` (DSO-relative offsets), maps each DSO to its build-id via
`perf buildid-list`, fetches the debug file with `debuginfod-find`, and resolves offsets with
`addr2line`/`eu-addr2line`. Resolved names replace `[unknown]` in both `perf.script` and the
folded stacks. It is best-effort: if `python3`, `debuginfod-find`, or `addr2line` is missing, the
raw output is passed through unchanged. (`DEBUGINFOD_URLS` defaults to
`https://debuginfod.archlinux.org`.)

Separately, if you want libc's *Rust callers* preserved when a sample's leaf is inside libc (e.g.
"who called `malloc`"), pass `--call-graph dwarf,8192` — frame-pointer mode stops at the libc
boundary. This is opt-in (fp is the default; see "Call graph" above) and trades away most of the
sample density. DWARF requires `kernel.perf_event_mlock_kb ≥ 8192` (default is 516):

```bash
sudo sysctl kernel.perf_event_mlock_kb=65536          # until reboot
# or permanently in /etc/sysctl.d/99-perf.conf:  kernel.perf_event_mlock_kb = 65536
```

(The samply backend sidesteps this entirely — it keeps libc callers *and* stays dense.)

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

Use `scripts/profile_perf.sh` on Linux to capture a profile for focused runtime analysis. The
script builds the selected target first, then profiles the test or benchmark executable directly
so build work does not pollute the trace.

Both modes build with the dedicated `profiling` Cargo profile (inherits `bench-fat`, adds
`debug = "full"` and `strip = false`) and force frame pointers via
`RUSTFLAGS=-Cforce-frame-pointers=yes`, so every dotnet-rs frame — including inlined ones —
unwinds on optimized code.

### Backend: samply (default) or perf

`--backend auto` (the default) picks **samply** when it is installed and
`kernel.perf_event_paranoid ≤ 1`, otherwise the **perf** path. On a dense `bench json` both now
capture ~44k clean main-thread samples; samply additionally symbolicates libc/system libraries
itself and serves the Firefox Profiler natively (see the samply section below). Force a backend
with `--backend perf` / `--backend samply`.

### Call graph: fp (default) vs dwarf — perf backend

The perf path defaults to **`fp`** (frame-pointer) unwinding. This is the right default: our
frames all unwind (forced frame pointers), fp samples are tiny so the ring buffer never overflows
even on a dense benchmark (measured **0% empty stacks** at 3997 Hz), and libc/system leaf names
are recovered in post-processing. **DWARF is *not* the default** — its 8 KiB-per-sample stack
dumps force a low rate and drop most stacks on a real workload (measured **80% empty / 87%
unresolved** on `bench json`). Use `--call-graph dwarf,8192` only when you specifically need the
call tree *through* libc (e.g. who-called-`malloc`), and accept the density hit; it needs
`kernel.perf_event_mlock_kb ≥ 8192` (`sudo sysctl kernel.perf_event_mlock_kb=65536`).

### System.Text.Json Fixture Trace

The CLI integration fixture is useful only to **inspect** the exact correctness test for
`System.Text.Json` — it is a one-shot run and a poor flamegraph target (expect the low-sample
`[quality]` warning; use the bench for flamegraphs):

```bash
./scripts/profile_perf.sh fixture --name system_text_json
```

Default behavior:

- Builds `dotnet-cli` integration tests (`profiling` profile) with `DOTNET_TEST_FILTER=system_text_json`.
- Discovers the generated `integration_tests-*` executable from Cargo JSON output.
- Runs only `integration_tests_impl::fixtures::basic_system_text_json_42`.
- Writes artifacts under `target/perf-traces/fixture-system_text_json/<timestamp>/`.

### JSON Benchmark Trace

The Criterion benchmark is the **recommended** steady-state target for flamegraph analysis:

```bash
./scripts/profile_perf.sh bench --name json --sample-size 30
```

Default behavior:

- Builds `dotnet-benchmarks` `end_to_end` with the `profiling` profile.
- Pre-warms untraced so the fixture's `dotnet build` stays out of the trace.
- Profiles only the benchmark process, not compilation.
- Runs Criterion with the `json` filter.
- Writes artifacts under `target/perf-traces/bench-json/<timestamp>/`.

### Perf Trace Artifacts

Each run writes:

- `perf.data`: raw perf profile for external flamegraph explorers.
- `perf.script`: symbol-resolved text dump (`perf script -F +pid,+dsoff --no-inline` piped
  through `resolve_perf_syms.py`), ready to load directly in the
  [Firefox Profiler](https://profiler.firefox.com) (drag the file into the UI). System-library
  internals (libc/ld.so) are resolved to real names rather than `[unknown]`.
- `perf.inline.script`: inline-expanded, also symbol-resolved; source for the folded stacks.
- `stacks.folded` and `flamegraph.svg`: generated from `perf.inline.script` via Inferno (or
  Brendan Gregg tools). Install with `cargo install inferno` if absent.
- `quality.txt`: the `[quality]` summary (sample count, period-weighted unresolved-leaf %,
  foreign-process %, and any warnings).
- `command.txt`: run metadata and the discovered executable path.

Common options:

```bash
./scripts/profile_perf.sh bench --name json --frequency 3997 --call-graph fp
./scripts/profile_perf.sh fixture --name system_text_json --features validation-all
./scripts/profile_perf.sh bench --name json -- --measurement-time 5
./scripts/profile_perf.sh bench --name json --no-prewarm   # opt out of the warm-up run
```

If `perf record` fails with a permissions error, check the local
`kernel.perf_event_paranoid` setting or run the script in an environment where
perf events are permitted.

## Alternative: samply (native Firefox Profiler backend)

[`samply`](https://github.com/mstange/samply) (`cargo install samply`) is a Rust sampling profiler
that records and serves the Firefox Profiler UI directly, and symbolicates system libraries from
debuginfod out of the box — so it needs neither `perf script` conversion nor `resolve_perf_syms.py`.
It is a good cross-check against the perf path.

```bash
# build first (reuse the profiling profile + frame pointers), then record the bench binary:
RUSTFLAGS=-Cforce-frame-pointers=yes cargo bench --profile profiling -p dotnet-benchmarks \
  --bench end_to_end --no-run
EXE=$(ls -t target/profiling/deps/end_to_end-* | grep -v '\.d$' | head -1)
# pre-build the fixture untraced (same reason as profile_perf.sh's pre-warm), then record:
"$EXE" json --warm-up-time 0.3 --measurement-time 0.3 --sample-size 10 >/dev/null 2>&1
samply record -- "$EXE" --bench json --measurement-time 4
```

Caveat: samply needs `kernel.perf_event_paranoid ≤ 1` (the perf path here works at the default
`2`). One-time: `echo 1 | sudo tee /proc/sys/kernel/perf_event_paranoid`. The same
fixture-pre-build and target-choice guidance applies (record the bench, not the fixture).

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
- Keep this PGO path optional; ordinary timing comparisons still use standard `bench-thin`/`bench-fat` commands.

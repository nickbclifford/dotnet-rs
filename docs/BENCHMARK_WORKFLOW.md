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

The end-to-end suite includes both focused runtime loops and heavier framework workloads:

- `json_dom`: repeated `System.Text.Json` DOM parse and property traversal.
- `linq_pipeline`: filtering, projection, grouping, ordering, and aggregation over an in-memory dataset.
- `ef_inmemory`: EF Core InMemory model discovery, change tracking, save, and translated query execution.

Run one case by passing its name after `--`:

```bash
cargo bench --profile bench-fat -p dotnet-benchmarks --bench end_to_end -- ef_inmemory --sample-size 10
```

Most fixtures are single source files. `ef_inmemory` is intentionally a self-contained project
with a pinned `Microsoft.EntityFrameworkCore.InMemory` package reference. The benchmark harness
builds it into the normal fixture cache and uses its `.runtimeconfig.json`/`.deps.json` files to
register package assemblies with the loader.

### Release Overflow-Checks Policy

The root `[profile.release]` enables `overflow-checks = true` as a correctness backstop; the
setting also reaches `bench-fat` through Cargo's bench/release profile inheritance. A 30-sample
`dispatch` measurement compared this policy with Cargo's unchecked release default. The unchecked
baseline median was **392.09 ms** (95% CI **390.86–393.41 ms**) and the checked median was
**387.87 ms** (95% CI **386.94–388.84 ms**). Criterion reported
`[-0.4271% +0.0373% +0.5034%]` with `p = 0.88`, so it detected no performance change in this
workload. This result supports the checked-release policy; it is a recorded workload-specific
measurement, not a claim that overflow checks are universally free.

The related newtype arithmetic and underflow policy is documented in `CONTRIBUTING.md`.

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
  --backend perf --call-graph dwarf,8192 --event cpu/cycles/ppp  # 'ppp' = PEBS precise level 3
```

**Reality:** ~7% of main-thread cycles are in syscalls (mmap/mprotect from Vec growth
hitting the allocator). This is expected allocation pressure; the skid frames are an
artefact of measuring it with a regular PMU, not a real cost to fix.

## Optional Firefox Profiler Traces

Use `scripts/profile_perf.sh` on Linux to capture a profile for focused runtime analysis. The
script builds the selected target first, then profiles the test or benchmark executable directly
so build work does not pollute the trace.

The primary output is a Firefox-Profiler-importable trace, not the generated SVG. For the perf
backend, drag `perf.script` into [profiler.firefox.com](https://profiler.firefox.com); it keeps
the captured instruction addresses, process/thread identity, and resolved symbols. The SVG is a
secondary quick-look export for tools that prefer folded stacks.

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

Every capture writes a `command.txt` manifest and `record.log`. The manifest records the selected
backend, recorder version, input event, sampling rate, call-graph mode, Cargo configuration,
kernel/CPU details, and Firefox Profiler artifact path. Keep this file with any trace shared for
comparison; `auto` can select a different backend on another host. Samply manifests mark the
perf-only event and call-graph fields as not applicable.

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

### Trace Artifacts

Each run writes:

- `perf.data` (perf backend): raw perf profile for external tooling.
- `perf.script`: symbol-resolved text dump (`perf script -F +pid,+dsoff --no-inline` piped
  through `resolve_perf_syms.py`), the primary perf artifact for
  [Firefox Profiler](https://profiler.firefox.com). System-library internals (libc/ld.so) are
  resolved to real names rather than `[unknown]`.
- `perf.inline.script` (perf backend): inline-expanded form used only to generate folded stacks.
- `profile.json.gz` (samply backend): self-contained Firefox Profiler profile; open it with
  `samply load profile.json.gz` to serve symbols from the capture machine.
- `stacks.folded` and `flamegraph.svg`: generated from `perf.inline.script` via Inferno (or
  Brendan Gregg tools). These are secondary quick-look exports; their width is labeled with the
  selected perf event rather than the generic “samples”. Install with `cargo install inferno` if
  absent.
- `quality.txt`: for perf, sample count plus period-weighted unresolved-leaf, foreign-process,
  and single-frame percentages, with the busiest process/thread identities. For samply, it records
  the capture mode and points at the Firefox Profiler artifact; perf-only percentages are not
  fabricated.
- `command.txt`: the trace manifest, including the discovered executable path, host/kernel and
  recorder data, and the artifact intended for Firefox Profiler import.
- `record.log`: unfiltered recorder output, retained to diagnose recorder warnings or sample loss.

Common options:

```bash
./scripts/profile_perf.sh bench --name json --frequency 3997 --call-graph fp
./scripts/profile_perf.sh bench --name json --backend perf --event cycles
./scripts/profile_perf.sh bench --name ef_inmemory --sample-size 10
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

## `drop_top` comparison (2026-07-31)

After replacing discard-only `EvalStackOps::pop_multiple` calls with `drop_top`, removing
`peek_multiple`, and making `pop_args` allocation-free (the unused helper was subsequently deleted
in the `supervised/pop-args` goal), the full `end_to_end` target was compared with the
`before-drop-top` Criterion baseline:

```bash
cargo run -p xtask -- fixtures build
DOTNET_USE_PREBUILT_FIXTURES=1 cargo bench --profile bench-fat -p dotnet-benchmarks \
  --bench end_to_end -- --baseline before-drop-top --sample-size 10
```

Both the saved baseline and comparison use Criterion's minimum 10-sample configuration. The table
records Criterion's 95% confidence interval for relative time change; negative values are faster.

| case | relative change (95% CI) | Criterion outcome |
|---|---:|---|
| `json` | −2.2948% to −1.9736% | improved |
| `arithmetic` | −1.3363% to −0.7045% | within noise threshold |
| `gc` | −0.6922% to +0.4698% | no change detected |
| `alloc_throughput` | +4.8959% to +11.842% | regressed |
| `gc_cross_arena` | −4.1923% to +1.5019% | no change detected |
| `dispatch` | −0.7660% to +0.0199% | no change detected |
| `generics` | −1.2225% to +0.6136% | no change detected |
| `stack` | −1.3225% to −0.6865% | within noise threshold |
| `span` | −2.2999% to −1.6383% | improved |
| `span_equality` | −4.5346% to −2.7866% | improved |
| `memory` | −5.0361% to −2.0085% | improved |
| `unsafe_buffer` | −4.7595% to −2.0134% | improved |
| `string` | +2.4821% to +3.6921% | regressed |
| `reflection` | −3.0075% to −1.9820% | improved |

This low-sample end-to-end measurement is not a direct count of the eliminated Rust `Vec`
allocations and must not be treated as a causal explanation for individual workload deltas. The
comparison completes successfully across every benchmark case; it reports mixed workload-level
timing changes, including regressions for `alloc_throughput` and `string` that should be repeated
under a controlled environment before drawing a performance conclusion.

## `before-dead-api-sweep` comparison (2026-08-01)

After the dead-API consolidation and atomic allocation-sweep work, the full `end_to_end` target
was compared with the `before-dead-api-sweep` Criterion baseline:

```bash
DOTNET_USE_PREBUILT_FIXTURES=1 cargo bench --profile bench-fat -p dotnet-benchmarks \
  --bench end_to_end -- --baseline before-dead-api-sweep --sample-size 10
```

Both the saved baseline and comparison use Criterion's minimum 10-sample configuration. The table
records Criterion's 95% confidence interval for relative time change; positive values are slower.

| case | relative change (95% CI) | Criterion outcome |
|---|---:|---|
| `json` | +9.7828% to +11.142% | regressed |
| `arithmetic` | +27.452% to +28.421% | regressed |
| `gc` | +10.746% to +11.401% | regressed |
| `alloc_throughput` | +7.9997% to +9.1688% | regressed |
| `gc_cross_arena` | +10.234% to +15.987% | regressed |
| `dispatch` | +13.712% to +15.766% | regressed |
| `generics` | +7.5938% to +8.7815% | regressed |
| `stack` | +20.003% to +20.690% | regressed |
| `span` | +13.270% to +13.599% | regressed |
| `span_equality` | +10.393% to +11.985% | regressed |
| `memory` | +4.9911% to +8.0939% | regressed |
| `unsafe_buffer` | +10.246% to +10.771% | regressed |
| `string` | +16.590% to +17.399% | regressed |
| `reflection` | +8.5142% to +9.1745% | regressed |

Criterion reported a statistically significant regression for every case in this run; it reported
no improved, within-noise-threshold, or no-change result. That uniform direction includes workloads
unrelated to the changed evaluation-stack and atomic-field paths, and does not match the localized
signal expected from this refactor. The pattern therefore points to a systematic run-to-run
environmental or build-configuration difference rather than supporting attribution to these code
changes.

These low-sample end-to-end timing deltas are workload-level observations, not a direct count of
allocations or proof that any specific change caused a result. A controlled-environment repeat is
required before drawing a causal performance conclusion.

## Static-constrained resolver-cache comparison (2026-08-07)

`linq_pipeline` was captured twice with the same `profiling` configuration, Linux `perf` backend,
`cycles` event, frame-pointer call graph, 3997 Hz sample rate, and 30 Criterion samples. The
fixture was pre-warmed before each capture. The cache-off run used
`DOTNET_STATIC_CONSTRAINED_CACHE=0`; enabled is the default.

| mode | Criterion estimate (95% CI) | trace samples | static-constrained resolver, inclusive cycle weight |
|---|---:|---:|---:|
| enabled | 514.44 ms (513.70–515.24 ms) | 77,682 | 0.07% |
| disabled | 616.30 ms (604.19–628.89 ms) | 92,780 | 17.39% |

Criterion compared the disabled capture with the enabled baseline as **+19.80%** (95% CI
+17.46% to +22.33%, `p < 0.05`). Both traces reported zero empty stacks and zero foreign-process
cycle weight, so this is a localized, clean signal: the resolver-level cache is a measurable win
for this constrained-dispatch workload.

The `bench-instrumentation` counter run recorded six cached metadata entries, 12,144 hits, and six
misses (99.95% hit rate) for the final `linq_pipeline` execution; cache-disabled mode correctly
recorded zero hits, misses, and entries. Its 12,150 cache-key clones show the remaining per-call
key cost explicitly. Broad descriptor/generic and `Arc` leaf costs stayed approximately flat in
absolute sampled cycle weight between the two runs, so this result supports avoiding repeated
resolver traversal, not a claim that the cache removes all descriptor churn.

### JSON/EF generality follow-up after vector tracing fix (2026-08-07)

The earlier `cache-generality/*` JSON and EF artifacts are rejected.  Each stopped in warm-up, so
their sample counts and cycle weights are not comparison data.

The cache-independent correctness failure was a GC tracing omission in `dotnet-value::Vector::trace`.
Its direct `ObjectRef` vector path traced elements, but reference-bearing value-type elements could
also serialize ordinary object references.  Those references were not traced across collection,
which corrupted the arrays used by JSON and EF.  The fixed implementation calls
`LayoutManager::trace` for every reference-bearing non-`ObjectRef` vector element; reference-free
elements still return immediately.

Post-fix captures use the same pre-warmed `profiling` configuration as the LINQ pair: Linux `perf`,
`cycles`, frame-pointer call graph, and 3997 Hz.  `DOTNET_STATIC_CONSTRAINED_SHADOW=1` was set for
every run, and cache-off also set `DOTNET_STATIC_CONSTRAINED_CACHE=0`.  All four completed without a
managed exception, VM error, or shadow mismatch:

| case | mode | Criterion estimate (95% CI) | trace samples | trace quality | static-constrained resolver, inclusive cycle weight |
|---|---|---:|---:|---|---:|
| `json_dom` | enabled | 247.62 ms (245.38–250.12 ms) | 36,882 | 0 empty, 0.0% foreign | 0.492% |
| `json_dom` | disabled | 240.03 ms (239.47–240.72 ms) | 45,535 | 0 empty, 0.0% foreign | 0.507% |
| `ef_inmemory` | enabled | 1.9342 s (1.9250–1.9435 s) | 257,151 | 0 empty, 0.0% foreign | 1.789% |
| `ef_inmemory` | disabled | 1.9518 s (1.9346–1.9683 s) | 259,907 | 0 empty, 0.0% foreign | 1.737% |

EF's disabled run reported no timing change (`-0.07%` to `+1.88%`, `p = 0.08`).  The JSON captures
use the retained 10-sample enabled capture and a new 30-sample disabled capture, so their differing
Criterion estimates are not a controlled throughput claim.  More importantly, the resolver's
inclusive cycle share is effectively unchanged in both workloads.  Shadow mode deliberately runs
the uncached resolver on a cache hit to compare the selected metadata, so these are correctness and
cost-location captures, not a measurement of the production cache-hit fast path.

The leading named leaves are also stable: `drop_in_place<MethodDescription>` is 6.79%/6.98%
(JSON enabled/disabled) and 6.06%/6.18% (EF); `VesContext::current_context` is 6.42%/6.93% and
5.33%/5.11%; and loader method/type lookup stays near 4–5%.  No consistent, material
metadata-arena `Arc` clone/drop cost appears in both valid pairs, so this result does not justify
an Arc optimization.  Valid artifacts are exclusively under
`target/perf-traces/cache-generality-fixed/{json-dom,ef-inmemory}-{on,off}`.

The valid cache-enabled `linq_pipeline` trace still showed metadata-arena `Arc` clone/drop as the
largest leaves (12.84%/11.51%). Rather than add another broad cache or alter virtual dispatch, the
next change is deliberately limited to `VesContext::init_locals`: it resolves local declarations
and initializes default values but never uses reflection owner access, so its temporary resolver
context now borrows the method and carries no method/type reflection owners. This removes the
otherwise unnecessary descriptor clones at the two frame-entry call sites.

The first like-for-like enabled follow-up captured 76,399 samples with zero empty or foreign
process stacks and a 507.98 ms median, compared with 77,682 samples and a 514.44 ms median in the
earlier enabled capture. Its `init_locals` exclusive cycle share was 0.10%, down from 0.15%; the
dominant metadata-arena leaves remained essentially unchanged (12.89%/11.40%). This is a
directional single-run check, not a causal performance claim; repeat a controlled unprofiled
comparison after restoring `json_dom` and `ef_inmemory` correctness.

Reproduce the performance captures with distinct output directories:

```bash
./scripts/profile_perf.sh bench --name linq_pipeline --backend perf --sample-size 30 \
  --output-dir target/perf-traces/linq-cache-on
DOTNET_STATIC_CONSTRAINED_CACHE=0 ./scripts/profile_perf.sh bench --name linq_pipeline \
  --backend perf --sample-size 30 --output-dir target/perf-traces/linq-cache-off
```

## Controlled unprofiled vector-tracing comparison (2026-08-10)

This closes the requested follow-up after `0509cf71`, which fixed missing
`LayoutManager::trace` calls for reference-bearing value-type elements in
`Vector::trace`.  The primary intended targets were `json_dom` and `ef_inmemory`.
The baseline was run from a detached worktree at `5b1efde2` (the revision immediately
before the fix), then the comparison was run at current HEAD.  Both used the same
`target/bench-fat/dotnet-bench-fixtures` cache, Criterion's 10-sample minimum, and no
`perf`, Samply, or profiling wrapper:

```bash
# In a detached worktree at 5b1efde2, with CARGO_TARGET_DIR pointing at this checkout's target/
DOTNET_USE_PREBUILT_FIXTURES=1 cargo bench --profile bench-fat -p dotnet-benchmarks \
  --bench end_to_end -- --sample-size 10 --save-baseline before-vector-trace-fix

# At current HEAD, using that same target directory and prebuilt fixture cache
DOTNET_USE_PREBUILT_FIXTURES=1 cargo bench --profile bench-fat -p dotnet-benchmarks \
  --bench end_to_end -- --baseline before-vector-trace-fix --sample-size 10
```

The table records Criterion's relative-time mean 95% confidence interval; negative
values are faster.  `5b1efde2`'s `end_to_end` suite contains only the first 14 cases.
It does not register `json_dom`, `linq_pipeline`, or `ef_inmemory`, so Criterion has no
baseline sample from which to compute their relative change or outcome.  In particular,
this controlled run cannot support a before/after throughput claim for the two primary
targets; a baseline from a pre-fix revision that includes them is required for that.

| case | relative change (95% CI) | Criterion outcome |
|---|---:|---|
| `json` | −1.5320% to −1.1185% | improved |
| `arithmetic` | −0.2542% to +0.5992% | no change detected |
| `gc` | −0.2182% to +0.9189% | no change detected |
| `alloc_throughput` | −0.2879% to +0.5695% | no change detected |
| `gc_cross_arena` | −4.9245% to +1.1844% | no change detected |
| `dispatch` | −0.7639% to +0.6262% | no change detected |
| `generics` | −0.8233% to +0.1750% | no change detected |
| `stack` | −0.0297% to +0.8053% | no change detected |
| `span` | −0.5409% to −0.0231% | within noise threshold |
| `span_equality` | −0.5628% to +0.0227% | no change detected |
| `memory` | −0.6467% to +0.0238% | no change detected |
| `unsafe_buffer` | +0.0449% to +0.5565% | no change detected |
| `string` | −0.9545% to +0.0103% | no change detected |
| `reflection` | −1.1392% to −0.5076% | within noise threshold |
| `json_dom` | baseline unavailable (not in `5b1efde2` suite) | not compared |
| `linq_pipeline` | baseline unavailable (not in `5b1efde2` suite) | not compared |
| `ef_inmemory` | baseline unavailable (not in `5b1efde2` suite) | not compared |

The matched cases show no uniform regression pattern like the
`before-dead-api-sweep` run: one case improved, two changes remained within Criterion's
noise threshold, and the other eleven reported no change.  That is neither evidence of
a whole-suite environmental slowdown nor a causal performance conclusion about the
vector-tracing fix.  Most importantly, the absent `json_dom` and `ef_inmemory` baseline
rows mean this particular experiment cannot answer the intended localized question.

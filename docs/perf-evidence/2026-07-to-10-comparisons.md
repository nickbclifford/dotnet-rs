# Benchmark comparisons, 2026-07 to 2026-10

Dated Criterion comparisons recorded when specific runtime changes landed. These are historical
measurements, not current baselines or open work items. Use the runbook in
[BENCHMARK_WORKFLOW.md](../BENCHMARK_WORKFLOW.md) to produce new ones.

## `drop_top` comparison (2026-07-31)

After replacing discard-only `EvalStackOps::pop_multiple` calls with `drop_top`, removing
`peek_multiple`, and making `pop_args` allocation-free (the unused helper was later deleted), the full `end_to_end` target was compared with the
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

## Heap-access comparison (2026-10-28)

The heap-access changes were compared with the matching pre-runtime Criterion baselines using
`bench-fat`, 10 samples, `DOTNET_USE_PREBUILT_FIXTURES=1`, stable `rustc 1.95.0`, no PGO, and the
same AMD Ryzen 9 9900X / Linux x86_64 host with the `powersave` governor. Negative relative changes
are faster; each slash-separated triple is the lower bound, point estimate, and upper bound of the
Criterion 95% confidence interval. End-to-end case comparisons use `m1a-before` for default features
and `m1a-before-nofeatures` for `--no-default-features`.

```bash
DOTNET_USE_PREBUILT_FIXTURES=1 cargo bench --profile bench-fat -p dotnet-benchmarks \
  --bench end_to_end -- '^(memory|unsafe_buffer|span|span_equality|dispatch|heap_access)$' \
  --sample-size 10 --baseline m1a-before
DOTNET_USE_PREBUILT_FIXTURES=1 cargo bench --profile bench-fat -p dotnet-benchmarks \
  --no-default-features --bench end_to_end \
  -- '^(memory|unsafe_buffer|span|span_equality|dispatch|heap_access)$' \
  --sample-size 10 --baseline m1a-before-nofeatures
DOTNET_USE_PREBUILT_FIXTURES=1 cargo bench --profile bench-fat -p dotnet-benchmarks \
  --bench managed_ptr_serde -- 'ManagedPtr' --sample-size 10 --baseline m1a-before
```

| Default-feature `end_to_end` case | Relative change (95% CI) | Criterion outcome |
|---|---:|---|
| `dispatch` | −4.1700% / −3.8969% / −3.6724% | improved (`p < 0.05`) |
| `span` | −0.1697% / +0.0725% / +0.3130% | no change (`p = 0.58`); rerun: −0.2647% / −0.0456% / +0.1571% (`p = 0.70`) |
| `span_equality` | −0.7323% / −0.5674% / −0.3992% | within Criterion noise threshold (`p < 0.05`) |
| `memory` | −0.2260% / −0.0444% / +0.1523% | no change (`p = 0.67`); rerun: −0.8807% / −0.4330% / −0.0728% (`p = 0.06`, two severe low outliers) |
| `heap_access` | −1.9662% / −1.6883% / −1.3862% | improved (`p < 0.05`); new fixture's first pre/post comparison |
| `unsafe_buffer` | −0.0919% / +0.0881% / +0.2757% | no change (`p = 0.38`); rerun: −1.8543% / −1.6976% / −1.5340% (improved, one mild low outlier) |

| No-default-features `end_to_end` case (`m1a-before-nofeatures`) | Relative change (95% CI) | Criterion outcome |
|---|---:|---|
| `dispatch` | final +2.7320% / +3.1776% / +3.7114%; earlier +2.9017% / +3.1312% / +3.3564% | regressed (`p < 0.05`) in both; first incomplete run +4.6829% / +5.1742% / +5.6604% |
| `span` | final +5.3591% / +6.0160% / +6.6081%; earlier +2.7269% / +2.9259% / +3.1313%; hot rerun +3.4456% / +3.8331% / +4.3122% | regressed (`p < 0.05`) in all runs; outliers varied |
| `span_equality` | final +3.2253% / +3.9696% / +4.7995%; earlier +1.9721% / +2.6453% / +3.4223% | regressed (`p < 0.05`); first incomplete run +4.3664% / +4.7823% / +5.1955% |
| `memory` | final +2.0553% / +2.8038% / +3.5176%; earlier +2.3509% / +3.2281% / +4.1255%; hot rerun +2.0012% / +2.4456% / +2.9294% | regressed (`p < 0.05`) in all runs |
| `heap_access` | final +6.6197% / +6.8289% / +7.0380%; initial single-case compare +4.1803% / +4.7371% / +5.3239% | regressed (`p < 0.05`) in both; pre-runtime baseline reconstructed with the benchmark-only fixture/registration diff |
| `unsafe_buffer` | final +3.6961% / +4.2270% / +4.6760%; earlier +6.5345% / +7.7659% / +8.7303%; hot rerun +3.1147% / +3.6599% / +4.1315% | regressed (`p < 0.05`) in all runs |

The default-feature `managed_ptr_serde` comparisons used the matching `m1a-before` baseline. Its
15 benchmark IDs reported:

| `ManagedPtr` case | Relative change (95% CI) | Criterion outcome |
|---|---:|---|
| `write/unmanaged` | −0.4807% / −0.3555% / −0.2141% | within noise threshold |
| `write/stack` | −0.7536% / −0.5556% / −0.3556% | within noise threshold |
| `write/heap` | −0.8875% / −0.7789% / −0.6486% | within noise threshold |
| `write/static` | −1.2042% / −0.9032% / −0.6105% | within noise threshold |
| `write/cross_arena` | −0.5853% / −0.3764% / −0.1867% | within noise threshold |
| `write/transient` | −1.2875% / −0.9795% / −0.6709% | within noise threshold |
| `read_resolved_unchecked/unmanaged` | −13.226% / −12.858% / −12.554% | improved (`p < 0.05`) |
| `read_resolved_unchecked/stack` | −12.598% / −12.477% / −12.336% | improved (`p < 0.05`) |
| `read_resolved_unchecked/heap` | −9.3582% / −9.2229% / −9.0755% | improved (`p < 0.05`) |
| `read_resolved_unchecked/static` | −16.293% / −16.042% / −15.757% | improved (`p < 0.05`) |
| `read_resolved_unchecked/cross_arena` | −1.2816% / −1.1308% / −0.9754% | within noise threshold |
| `read_resolved_unchecked/transient` | first +2.0178% / +2.1703% / +2.3399%; rerun −1.2258% / −1.1366% / −1.0539% | regression then improvement; inconsistent direction, treat as unstable |
| `read_resolved_with_heap_cache_unchecked/heap_hit` | −1.8826% / −1.7993% / −1.7069% | improved (`p < 0.05`) |
| `read_resolved_with_heap_cache_unchecked/heap_miss` | −15.764% / −2.5626% / +12.180%; rerun −16.692% / −1.6837% / +13.571% | no change (`p = 0.75`, then `p = 0.84`); broad intervals/outliers |
| `read_resolved_with_heap_cache_unchecked/heap_mixed_hit_miss_pair` | −13.422% / −0.6534% / +16.737%; rerun −14.446% / −0.5107% / +16.883% | no change (`p = 0.94`, then `p = 0.95`); broad intervals/outliers |

The pre-runtime default baselines for existing end-to-end cases and `managed_ptr_serde` were recorded
at commit `968f66fe11a28bb8633304034a36aee4febcb1c7`. The default `heap_access` baseline was captured
at base commit `62d944eb41f405810e27df3368f9ba535fb805a2` plus the then-uncommitted benchmark-only
fixture and registration; this new case has no earlier history. The no-features `heap_access` baseline
was independently captured from that same pre-runtime base plus the benchmark-only diff and stored
under the separate `m1a-before-nofeatures` name. `managed_ptr_serde` requires `multithreading`, so no
no-features baseline exists for that target.

The default-feature `span` and `memory` cases were neutral on rerun, `unsafe_buffer` was neutral on the
full run and faster on rerun, and `heap_access` improved by about 1.7%. In contrast, all six no-features
end-to-end cases repeatedly had positive (slower) relative-change intervals; hot reruns remained
above-noise but varied in magnitude. These aggregate measurements do not isolate cause: the checked
extent/access work could contribute to some hot paths, but `dispatch` also regressed without features
while improving with defaults. The powersave governor and feature-dependent code generation are
possible confounders, not established explanations. Preserve this no-features performance signal for
later review; no optimization was attempted. These are native Criterion measurements, not Miri evidence.

## Pointer-semantics comparison (2026-10-29)

The planned finished-tree comparison covered the same six `end_to_end` cases in both feature sets,
against the unchanged-tree `m1b-before` / `m1b-before-nofeatures` baselines and, while present, the
retained `m1a-before` / `m1a-before-nofeatures` baselines. All 24 case/baseline estimate files were
available locally, but **the comparison was not measured**: the host was not sufficiently idle. The
AMD Ryzen 9 9900X host had 24 logical CPUs, all using the `powersave` governor; observed load averages
were 1.37/1.03/1.36 and later 0.91/0.97/1.32. A separate Vite/Playwright process was active in the
first snapshot and unrelated browser/desktop activity remained in later snapshots (despite 88–95%
CPU idle in `vmstat`). No medians, relative-change 95% intervals, p-values, or Criterion outcomes
were obtained for the finished pointer-semantics tree.

These were the intended **native Criterion** commands; none was run because of the host activity:

```bash
DOTNET_USE_PREBUILT_FIXTURES=1 cargo bench --profile bench-fat -p dotnet-benchmarks \
  --bench end_to_end -- '^(memory|unsafe_buffer|span|span_equality|dispatch|heap_access)$' \
  --sample-size 10 --baseline m1b-before
DOTNET_USE_PREBUILT_FIXTURES=1 cargo bench --profile bench-fat -p dotnet-benchmarks \
  --no-default-features --bench end_to_end \
  -- '^(memory|unsafe_buffer|span|span_equality|dispatch|heap_access)$' \
  --sample-size 10 --baseline m1b-before-nofeatures
```

The retained `m1a-before` and `m1a-before-nofeatures` comparisons were also not run. Local
`target/criterion` baselines are volatile. This result is a statement of missing performance
measurement, not a correctness result; Criterion measurements are native and are not Miri evidence.

At the time of this record the heap-access no-features regressions were unaddressed; no
optimization was attempted. The final heap-access runs recorded positive relative-change
95% intervals for all six no-features cases (`dispatch` +3.1776%, `span` +6.0160%, `span_equality`
+3.9696%, `memory` +2.8038%, `heap_access` +6.8289%, `unsafe_buffer` +4.2270% point estimates;
each `p < 0.05`). Reproduce the no-features comparison, when its local baseline remains available,
with:

```bash
DOTNET_USE_PREBUILT_FIXTURES=1 cargo bench --profile bench-fat -p dotnet-benchmarks \
  --no-default-features --bench end_to_end \
  -- '^(memory|unsafe_buffer|span|span_equality|dispatch|heap_access)$' \
  --sample-size 10 --baseline m1a-before-nofeatures
```

The prior [heap-access report](#heap-access-comparison-2026-10-28) retains all six intervals,
repeat-run details, and the default-feature comparison command. This performance signal remains
separate from correctness and does not establish a cause.

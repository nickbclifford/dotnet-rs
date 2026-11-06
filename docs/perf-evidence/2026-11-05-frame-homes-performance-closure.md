# Stable frame homes: performance closure (2026-11-05)

**Disposition: complete, with accepted primitive cost.** The maintainer requested a
reasonable correct source repair first and explicitly allowed accepting the measured
cost if that effort did not produce a better maintainable architecture. The bounded
source experiments below did not meet the agreed performance bar. Production runtime
source remains exactly `938254189cd031eb3fb8c482d0bbe4f59b7d0967`.

## Experiments

Each variant started independently from the committed repair source in an isolated
archive. No runtime changes were applied to the production checkout.

1. Add `#[inline]` to the thin `EvaluationStack::push` forwarding method.
2. Keep the existing trace guard and extract enabled PUSH/POP formatting and emission
   into cold, non-inlined helpers, preserving event ordering.
3. Check capacity before push, call a cold helper to `reserve(1)` only when full,
   then use ordinary safe `Vec::push`.

No variant introduced unsafe code, binary patches, barriers, padding or fixups.
The preexisting binary-patch experiments were diagnostic only and were not used here.
No combination was attempted: none of these individual variants qualified across
both configurations. This exhausts the bounded source candidates; it does not prove
that every possible stack representation or compiler transformation is slower.

## Results

Each comparison uses the saved unmodified repair binary as control, not the original
pre-home baseline. Rust 1.95.0, locked offline dependencies, `bench-fat`, CPU 2,
`RAYON_NUM_THREADS=4`, disabled ASLR and equal-length executable aliases were used.
Runs were serial control–variant–variant–control, 30 samples, 3-second warmup and
5-second measurement target. Builds finished before timing. No profiler ran during
timing. Benchmark bodies and fixture sources were unchanged. All 24 timing runs
completed successfully. Statistical comparisons loaded the saved samples without
collecting new timings; intervals are Criterion bootstrap 95% confidence intervals.

| Variant | Configuration | Pair | Relative mean time [95% CI] |
|---|---|---:|---:|
| inline | nofeatures | 1 | +1.25% [+0.99%, +1.50%] |
| inline | nofeatures | 2 | +1.21% [+0.91%, +1.54%] |
| inline | default | 1 | +0.87% [+0.39%, +1.39%] |
| inline | default | 2 | +0.28% [-0.40%, +0.86%] |
| tracing | nofeatures | 1 | -0.68% [-1.29%, -0.06%] |
| tracing | nofeatures | 2 | -0.53% [-1.17%, +0.04%] |
| tracing | default | 1 | -1.02% [-1.53%, -0.48%] |
| tracing | default | 2 | -1.31% [-1.83%, -0.77%] |
| reserve | nofeatures | 1 | +6.70% [+6.37%, +7.01%] |
| reserve | nofeatures | 2 | +7.08% [+6.79%, +7.36%] |
| reserve | default | 1 | +5.96% [+5.42%, +6.51%] |
| reserve | default | 2 | +5.87% [+5.38%, +6.39%] |

The inline variant did not improve the control. Trace extraction gave small gains,
with no-default gains below the 1% acceptance threshold. Capacity-growth extraction
was slower in both configurations. Control drift within every block was below 1%,
so the planned drift-triggered repeat was unnecessary. No variant achieved repeatable
improvement beyond 1% in both configurations. Complete optimized disassembly is saved
for each variant, but no successful source repair was established and no particular
instruction sequence is claimed as a fix.

## Final validation and acceptance

The [complete measurement campaign](2026-11-05-stable-frame-homes-measurement.md)
remains the final runtime comparison against pre-home baseline
`0bae37275891576cf76d7daf4ab552229805d5b8`: all 18 measured end-to-end cases improved
in both feature configurations and both comparisons. The verified address-free guest
arithmetic and stack controls improved. These results do not establish performance
for all address-free guest methods.

The [control investigation](2026-11-05-frame-homes-control-investigation.md) establishes
a genuine generated stack-copy cost in the isolated Rust control, approximately 4%
in its stable original comparison; its magnitude varies with launch conditions.
That cost is explicitly accepted for this implementation. Vector results retain
unresolved launch/placement sensitivity, including reversed and inconclusive results;
no vector tracing algorithm regression or universal placement explanation was proven.
The original unfavorable measurements remain part of the record.

No production source repair survived, so the existing successful fast checks and
native/Miri pointer controls for the enum-home repair remain applicable. Production
`crates/`, Cargo manifests and lockfile match `93825418`; no unchanged correctness or
full end-to-end matrix was rerun for this documentation-only closure. Plan 02's
original correctness requirements remain satisfied, and slice 5 closes under the
maintainer's accepted-cost exception. This accepts the documented primitive cost,
not future correctness failures or newly discovered guest regressions.

## Reproduction artifacts

`target/plan01-slice5/closure/` contains `run.py` (exact source transformations,
build commands and launch order), `analyze.py`, build/timing/comparison logs, all
24 Criterion sample sets, `summary.json`, `comparisons.json`, six binaries and their
SHA-256 hashes, and optimized disassemblies. Existing baseline fixture hashes and
host details are in the linked measurement record and measurement manifest. These
local artifacts are volatile; this report preserves the conclusions and intervals.

No commit or push was performed as part of this phase.

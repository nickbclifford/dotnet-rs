> **Subsequent disposition:** Plan 01 is complete under the maintainer’s accepted-cost
> exception. See the [performance closure](2026-11-05-frame-homes-performance-closure.md).
> Statements below describing the plan as open record its status at investigation time.

# Frame-home control investigation (2026-11-05)

The no-home Rust stack control has a reproducible generated-code cost. A diagnostic
change to its copy path recovers the observed ~4% slowdown. Vector tracing has no
consistent directional cost across launch/placement experiments; the earlier +8%
result does not establish a vector algorithm regression. Neither result justifies
restoring pointer fixups. No production source or benchmark changes were made.

Plan 01 remains in progress. A maintainable Rust source repair for the stack copy
sequence has not been established; the maintainer's report-and-pause policy still
applies before changing runtime code.

## Reproduction and controls

The inputs are the saved `bench-fat` binaries from `0bae37275891576cf76d7daf4ab552229805d5b8`
and `938254189cd031eb3fb8c482d0bbe4f59b7d0967`. Both measured benchmark sources are
unchanged. The host and compiler match the [measurement report](2026-11-05-stable-frame-homes-measurement.md).
Every comparison below uses 30 samples, 3-second warmup, 5-second measurement target,
CPU 2, `RAYON_NUM_THREADS=4`, no profiler during timing, and Criterion's saved-sample
comparison with 95% confidence intervals and the standard 1% noise threshold.

A new original-binary ABBA pass also disabled ASLR with `setarch x86_64 -R` and used
equal-length executable paths and result-directory names. Default stack was
13.396/13.388 ns baseline and 13.877/13.898 ns candidate. No-features stack was
13.394/13.431 ns baseline and 15.721/15.713 ns candidate. The changed absolute
candidate cost under this launch configuration is itself evidence of sensitivity;
it must not be merged with prior samples as one stable percentage estimate.

Subsequent stack and vector experiments were serial, with ASLR disabled, equal-length
aliases and fixed CPU affinity. Variants ran in forward then reverse order. All raw
samples and comparisons are preserved; no timing was performed during compilation
or statistical analysis. Diagnostic patched binaries passed Criterion test mode
before timing, and timed runs completed successfully. No guest workload regression
was newly observed; the earlier full guest matrix remains the end-to-end evidence.

## Stack: generated copy path is the actionable lead

The steady-state benchmark has no guest frames, performs no home lookup and allocates
no home block. Frame-home bounds/serial checks cannot account for its per-iteration
cost. The optimized baseline's no-growth path copies the original stack-value source
to the evaluation-stack vector. The candidate instead reloads an intermediate stack
buffer that was just written. Both paths contain other copies, tracing checks and
arena bookkeeping: describing this as merely a larger instruction count is incomplete.

Three diagnostic variants were made without changing tracked sources:

- **Scratch fields:** a source archive of the candidate regained the two unused
  `Vec<NonNull<u8>>` scratch fields and empty initializers, with no fixup behavior.
  It did not recover performance, so removed scratch storage is not a sufficient
  explanation. These unused fields are not a proposed repair.
- **Growth guard:** another archive retained the candidate's layout but added a
  post-push capacity-growth condition whose body was `black_box(&self.stack)`.
  It generated an original-source no-growth copy, but did not improve this control.
  A different source alignment/register allocation and other generated code changed
  too; adding a redundant capacity check is not a validated repair.
- **Direct-copy diagnostic:** a copy of the original candidate binary bypassed the
  intermediate-buffer construction on the no-growth path and copied the 112-byte
  original source directly to the vector. It retained the original growth path and
  frame/pop processing. This recovered approximately the original measured gap.

| Comparison | Mean time | Relative time (95% CI) |
|---|---:|---:|
| Original baseline → candidate, first | 13.3484 → 13.9065 ns | +4.18% [+3.42%, +4.85%] |
| Original baseline → candidate, second | 13.3609 → 13.8949 ns | +4.00% [+3.21%, +4.68%] |
| Candidate → scratch fields, first | 13.9065 → 14.0224 ns | +0.83% [+0.40%, +1.20%] |
| Candidate → scratch fields, second | 13.8949 → 14.0706 ns | +1.26% [+0.94%, +1.56%] |
| Candidate → growth guard, first | 13.9065 → 14.0999 ns | +1.39% [+0.55%, +2.41%] |
| Candidate → growth guard, second | 13.8949 → 14.0558 ns | +1.16% [+0.44%, +2.04%] |
| Candidate → direct-copy diagnostic, first | 13.9065 → 13.2897 ns | -4.44% [-4.67%, -4.21%] |
| Candidate → direct-copy diagnostic, second | 13.8949 → 13.2791 ns | -4.43% [-4.67%, -4.17%] |

Criterion classified both original comparisons as regressions, both direct-copy
comparisons as improvements, and all scratch/growth-guard comparisons as changes
within the noise threshold.

A further matched binary experiment separated copy data flow from code placement.
Both variants execute the same first-stage stores, then the same instructions,
registers, control flow and code addresses. Only seven disp32 load-source operands
change (14 differing bytes in the complete binaries):

```text
staged:      heap-copy loads read rsp + 0x20  ... rsp + 0x80
independent: heap-copy loads read rsp + 0x170 ... rsp + 0x1d0
```

The first range is the freshly written intermediate buffer; the second is the
original value source. Both include the same valid scalar tag and value. The
source offsets also differ in cache-line position, so store forwarding is a likely
mechanism, not a uniquely proven CPU explanation. This experiment establishes that
selecting the intermediate rather than original source is a causal cost in this
loop; it does not quantify the contribution of every move in ordinary guest code.

| Comparison | Mean time | Relative time (95% CI) |
|---|---:|---:|
| Staged → independent source, first | 14.8664 → 13.5184 ns | -9.07% [-9.38%, -8.76%] |
| Staged → independent source, second | 14.9093 → 13.5269 ns | -9.27% [-9.58%, -8.99%] |

Both changes were classified as improvements. The experiment is intentionally a
compiler/data-flow diagnostic, not a production implementation. A safe source-level
change needs to reproduce the gain without unused storage, opaque barriers or
architecture-specific patches, and then pass correctness and end-to-end comparisons.
The source-level growth-guard failure shows that copying a superficial baseline
shape is insufficient.

## Vector: launch and placement sensitivity, no algorithm fix identified

The scalar vector trace source is unchanged, and the decoded executed loop and
`is_or_contains_refs` instructions match across revisions. Repeated original runs
now reverse the prior +8% result when launched through equal-length aliases with
ASLR disabled. Relocating the loop and helper to common addresses in diagnostic
copies removes the difference in both paired comparisons:

| Comparison | Mean time | Relative time (95% CI) |
|---|---:|---:|
| Original baseline → candidate, first | 1.3112 → 1.2929 ns | -1.40% [-1.46%, -1.34%] |
| Original baseline → candidate, second | 1.3121 → 1.2944 ns | -1.35% [-1.42%, -1.26%] |
| Matched placement baseline → candidate, first | 1.2905 → 1.2902 ns | -0.02% [-0.05%, +0.01%] |
| Matched placement baseline → candidate, second | 1.2927 → 1.2914 ns | -0.11% [-0.26%, +0.03%] |
| Alternate matched placement, first | 1.2929 → 1.2929 ns | -0.00% [-0.04%, +0.03%] |
| Alternate matched placement, second | 1.1808 → 1.2941 ns | +9.60% [+9.42%, +9.76%] |

The first original pairs were classified as improvements; the first matched
placements and first alternate pair showed no change. Importantly, the second
alternate pair reproduced a +9.60% regression despite matching code addresses:
its baseline shifted from 1.2929 to 1.1808 ns while candidate stayed about 1.293 ns.
Thus code address alone is not a complete explanation, and the initial apparent
placement result must not be promoted to a universal cause. Stack/data placement,
process initialization and microarchitectural state remain possible contributors.

Separate post-timing `perf stat` runs profiled each vector variant twice for five
seconds. User-cycle/instruction ratios were 0.17811/0.17822 for original baseline,
0.17512/0.17530 for original candidate, and 0.17519–0.17527 for the common-placement
copies. Average user cycles per task-clock time were 5.421–5.438 GHz, so these
specific profile pairs were not separated by a large average-frequency difference.
The counters cover the profile process, not exact per-iteration cycle counts, and
did not capture the occasional 1.18 ns state. They corroborate the selected placement
comparison, not a complete explanation of every outlier state.

Do not modify vector tracing logic to optimize an unchanged scalar fast path on
this evidence. Preserve both the original +8% measurements and the newer reversed,
matched and variable results; none replaces the others.

## Artifacts and disposition

All investigation artifacts are under `target/plan01-slice5/investigation/`:
`stable-results`, `ablation-results`, `dependency-results`, `comparisons`,
`comparison-summary.json`, the patch/placement manifests, source archives,
disassembly, scripts, launch logs and post-timing hardware counters. The manifests
record source and diagnostic binary SHA-256 hashes. These are volatile local files.

The diagnostic executables overwrite an unused benchmark function to hold a stub.
They may run only the exact benchmark filter recorded in their manifests; they are
not general runtime binaries and must not be substituted for acceptance builds.
The matched vector destinations were `0xeab00`/`0xeae00` (loop/helper), with an
alternate at `0xeab10`/`0xeae30`. The stack stub was at `0xe7140`, redirected from
`0xe74e2`, with the original growth path preserved. These offsets apply only to the
recorded x86-64 binary hashes. The matched stack variants have identical control
flow and differ only at the documented load-source operands.

No runtime source change, new performance claim about guest workloads, commit or
push was made. `git diff --check` passes. The next engineering task is a safe,
measured source-level improvement to the stack move path, or an explicit maintainer
acceptance of its documented cost. Plan 01 stays open pending that decision.

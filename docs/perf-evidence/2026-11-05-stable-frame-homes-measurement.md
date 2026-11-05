> **Subsequent disposition:** Plan 01 is complete under the maintainer’s accepted-cost
> exception. See the [performance closure](2026-11-05-frame-homes-performance-closure.md).
> Statements below describing the plan as open record its status at investigation time.

# Stable frame homes: slice 5 measurement (2026-11-05)

**Status: in progress.** Measurement and follow-up are complete, with unresolved
no-home control slowdowns and variability. Plan 01 remains open for maintainer review. The earlier [preflight](2026-11-05-stable-frame-homes.md) records the
separate EF correctness blocker fixed before this campaign.

The baseline is `0bae37275891576cf76d7daf4ab552229805d5b8`; the candidate is
`938254189cd031eb3fb8c482d0bbe4f59b7d0967`. All 1,156 tracked blobs in the baseline
source archive matched its Git tree; the candidate checkout was clean at that revision.
The two measured benchmark suites and their fixture source files are unchanged between
those revisions.

Both suites built and passed Criterion test-mode prewarming at both revisions, with
and without default features. The baseline fixture cache was copied to the candidate;
SHA-256 hashes of all fixture DLLs, dependency manifests and runtime configuration
files match. The build records contain no validation or benchmark instrumentation
features. Rust 1.95.0, locked offline dependencies, `bench-fat` and
`RAYON_NUM_THREADS=4` are used.

Timing ran serially baseline–candidate–candidate–baseline for each configuration,
with 30 samples, a 3-second warmup and a 5-second measurement target. Criterion extends
collection when 30 executions cannot fit in five seconds. Host activity and frequency
and temperature samples, per-run logs, raw Criterion samples, fixture hashes and the
local driver are under `target/plan01-slice5/`; these volatile artifacts do not replace
the completed report. Fixture hashes were checked again after timing.

Compiled fixture IL establishes that arithmetic `Program.Main` contains no calls or
local/argument address-taking, and stack `Main`, `RunRound` and `Mix` call only one
another and contain no `ldloca` or `ldarga`. The Rust primitive stack benchmark creates
no frame or home. Other workloads require classification of their callees as well as
the fixture IL; absence of address-taking in an entrypoint alone is insufficient.

## Initial matrix and controls

All 18 end-to-end cases improved in both comparisons in both configurations. The
address-free guest arithmetic and stack controls improved in all four comparisons;
these fixtures do not establish that every address-free method improves. Some
improvement magnitudes differ materially across the two pairs, especially JSON in
both configurations and default-feature EF and generics. Targeted repeats retained the improvement direction; default EF still varied
from -4.15% to -7.02%, so its exact effect size remains uncertain.

The no-default Rust primitive stack control slowed by 5.23% and 6.50% in the initial
pairs. A repeat on CPU 2 gave +4.18% (95% CI +3.91% to +4.50%) and +4.85%
(+3.88% to +5.77%); Criterion reported regression in both pairs. The default primitive
stack results were inconsistent: the initial first/second changes were +3.61% and
+0.84%, while the fixed-CPU repeat gave +4.88% and +0.30%; the fixed-CPU baseline
itself rose 4.80%. This benchmark pushes and pops directly in Rust, creates no guest
frame and exercises no home; it is a no-home control, not a guest method with
`ldloca` or `ldarga`. The additional blocks below showed mostly slower candidate results, but baseline
changes as large as 5% prevent a stable default effect estimate.

The Rust vector trace control is also home-free: default candidate was slower by
7.93% and 7.95%, with fixed-CPU changes +7.85% and +8.15%; no-default was faster by
3.33% and 4.04%, with fixed-CPU changes -3.39% and -3.54%. These are nanosecond-scale
Rust measurements, not address-taking costs. The source implementing the vector trace
path is unchanged between revisions; the code investigation below narrows but does not
identify the cause.

The fixture IL contains `ldloca` in heap access, JSON DOM and EF; prior EF diagnosis
also established `ldarga` in a called EF method. Other framework-heavy cases execute
code beyond the entrypoint, so they are mixed or unclassified. Do not infer that
their end-to-end changes are exclusively the cost of home allocation or of
address-free methods.

## Method and host

The original matrix ran from 18:30:52 to 19:53:42 UTC, sequentially on an AMD
Ryzen 9 9900X (12 cores, 24 logical CPUs), Linux 7.2.14, powersave governor, active
`amd_pstate`, boost enabled, and affinity 0–23. No CPU quota was set. The .NET SDK was
10.0.114 and the shared framework 10.0.14. The 10-second aggregate CPU samples
averaged 1.02 busy CPUs and peaked at 1.47; one-minute load ranged from 1.00 to 1.54.
Observed CPU Tctl ranged from 63.4°C to 85.4°C. Process lists are limited to the
benchmark's sandbox PID namespace; the aggregate CPU and load counters cover the host.

All 65 fixture DLL/dependency/runtime configuration files had identical hashes across
build directories and remained unchanged after timing. All 344 recorded shared
framework files had the same SHA-256 hashes after timing. Prewarm and timing logs
showed no fixture rebuild, and every saved run has 30 samples per case. Criterion
was configured for 3-second warmup and 5-second collection, extending collection
when an execution was too long. No profiler, PGO or diagnostic build was used for
these estimates. The fixed-CPU primitive repeats used the same binaries and settings,
restricted to CPU 2 (its sibling is CPU 14), with separate telemetry.

The tables use each run's mean time and bootstrap 95% CI. Changes are Criterion's
relative mean estimates from saved samples, candidate versus the matching baseline;
B1/C1 and B2/C2 are independent run comparisons, not paired individual samples.
Criterion's bootstrap t-test and its 1% noise threshold determine its stated outcome;
smaller changes are also examined. Reported p values are rounded by Criterion, so
`0.00` does not mean zero. Repeated samples within a run can share temporal effects;
these intervals alone do not account for host drift or multiple comparisons.

## default

| Case | B1 mean [95% CI] | C1 mean [95% CI] | C1/B1 change [95% CI]; Criterion | B2 mean [95% CI] | C2 mean [95% CI] | C2/B2 change [95% CI]; Criterion |
|---|---:|---:|---|---:|---:|---|
| runtime_primitives/eval_stack_push_i32_pop_safe | 13.356 [13.250, 13.470] ns | 13.838 [13.810, 13.864] ns | +3.61% [+2.72%, +4.47%]; Performance has regressed; p 0.00 < 0.05 | 13.918 [13.828, 14.000] ns | 14.036 [13.964, 14.115] ns | +0.84% [+0.06%, +1.69%]; No change in performance detected; p 0.05 > 0.05 |
| runtime_primitives/vector_trace_i32 | 1.198 [1.197, 1.199] ns | 1.293 [1.292, 1.293] ns | +7.93% [+7.86%, +8.01%]; Performance has regressed; p 0.00 < 0.05 | 1.196 [1.195, 1.198] ns | 1.291 [1.291, 1.292] ns | +7.95% [+7.76%, +8.09%]; Performance has regressed; p 0.00 < 0.05 |
| alloc_throughput | 393.736 [393.085, 394.676] ms | 349.641 [349.079, 350.500] ms | -11.20% [-11.46%, -10.91%]; Performance has improved; p 0.00 < 0.05 | 395.265 [394.672, 396.162] ms | 350.358 [349.726, 351.343] ms | -11.36% [-11.63%, -11.04%]; Performance has improved; p 0.00 < 0.05 |
| arithmetic | 389.607 [388.908, 390.314] ms | 239.058 [238.933, 239.200] ms | -38.64% [-38.76%, -38.53%]; Performance has improved; p 0.00 < 0.05 | 393.164 [392.995, 393.334] ms | 239.026 [238.834, 239.215] ms | -39.20% [-39.25%, -39.15%]; Performance has improved; p 0.00 < 0.05 |
| dispatch | 496.384 [494.281, 498.688] ms | 387.331 [385.243, 389.460] ms | -21.97% [-22.52%, -21.43%]; Performance has improved; p 0.00 < 0.05 | 499.843 [497.588, 502.217] ms | 392.674 [390.672, 394.775] ms | -21.44% [-21.96%, -20.84%]; Performance has improved; p 0.00 < 0.05 |
| ef_inmemory | 1.872 [1.867, 1.876] s | 1.766 [1.761, 1.770] s | -5.67% [-6.01%, -5.33%]; Performance has improved; p 0.00 < 0.05 | 1.868 [1.863, 1.875] s | 1.807 [1.802, 1.812] s | -3.28% [-3.70%, -2.89%]; Performance has improved; p 0.00 < 0.05 |
| gc | 297.032 [296.792, 297.279] ms | 254.774 [254.608, 254.949] ms | -14.23% [-14.32%, -14.14%]; Performance has improved; p 0.00 < 0.05 | 298.380 [298.199, 298.562] ms | 256.409 [256.239, 256.580] ms | -14.07% [-14.15%, -13.99%]; Performance has improved; p 0.00 < 0.05 |
| gc_cross_arena | 114.945 [114.655, 115.394] ms | 93.617 [93.566, 93.664] ms | -18.56% [-18.88%, -18.34%]; Performance has improved; p 0.00 < 0.05 | 115.141 [114.873, 115.591] ms | 93.596 [93.547, 93.648] ms | -18.71% [-19.04%, -18.51%]; Performance has improved; p 0.00 < 0.05 |
| generics | 10.711 [10.674, 10.751] s | 9.271 [9.262, 9.280] s | -13.45% [-13.78%, -13.14%]; Performance has improved; p 0.00 < 0.05 | 10.682 [10.646, 10.722] s | 9.455 [9.446, 9.465] s | -11.48% [-11.83%, -11.17%]; Performance has improved; p 0.00 < 0.05 |
| heap_access | 603.026 [602.259, 603.763] ms | 441.247 [440.876, 441.681] ms | -26.83% [-26.94%, -26.71%]; Performance has improved; p 0.00 < 0.05 | 602.949 [602.371, 603.504] ms | 442.221 [441.696, 442.805] ms | -26.66% [-26.77%, -26.55%]; Performance has improved; p 0.00 < 0.05 |
| json | 210.037 [209.781, 210.272] ms | 170.063 [169.848, 170.288] ms | -19.03% [-19.17%, -18.89%]; Performance has improved; p 0.00 < 0.05 | 199.407 [199.293, 199.529] ms | 170.357 [170.154, 170.578] ms | -14.57% [-14.68%, -14.44%]; Performance has improved; p 0.00 < 0.05 |
| json_dom | 267.452 [267.036, 267.934] ms | 241.888 [241.640, 242.142] ms | -9.56% [-9.74%, -9.39%]; Performance has improved; p 0.00 < 0.05 | 266.895 [266.427, 267.444] ms | 243.960 [243.699, 244.242] ms | -8.59% [-8.81%, -8.40%]; Performance has improved; p 0.00 < 0.05 |
| linq_pipeline | 538.419 [537.625, 539.550] ms | 510.424 [509.614, 511.612] ms | -5.20% [-5.46%, -4.94%]; Performance has improved; p 0.00 < 0.05 | 537.580 [536.869, 538.586] ms | 511.906 [511.082, 513.258] ms | -4.78% [-5.01%, -4.49%]; Performance has improved; p 0.00 < 0.05 |
| memory | 493.659 [492.761, 494.471] ms | 323.613 [323.106, 324.099] ms | -34.45% [-34.59%, -34.29%]; Performance has improved; p 0.00 < 0.05 | 496.140 [495.367, 496.906] ms | 325.534 [325.096, 325.976] ms | -34.39% [-34.52%, -34.25%]; Performance has improved; p 0.00 < 0.05 |
| reflection | 287.302 [287.158, 287.443] ms | 257.099 [256.907, 257.304] ms | -10.51% [-10.60%, -10.43%]; Performance has improved; p 0.00 < 0.05 | 287.319 [287.129, 287.533] ms | 257.230 [256.909, 257.730] ms | -10.47% [-10.60%, -10.30%]; Performance has improved; p 0.00 < 0.05 |
| span | 257.452 [256.925, 258.301] ms | 171.116 [170.685, 171.821] ms | -33.53% [-33.83%, -33.20%]; Performance has improved; p 0.00 < 0.05 | 259.704 [259.125, 260.582] ms | 172.651 [172.178, 173.461] ms | -33.52% [-33.84%, -33.12%]; Performance has improved; p 0.00 < 0.05 |
| span_equality | 286.055 [285.632, 286.454] ms | 203.697 [203.500, 203.910] ms | -28.79% [-28.92%, -28.66%]; Performance has improved; p 0.00 < 0.05 | 285.954 [285.736, 286.204] ms | 204.496 [204.328, 204.686] ms | -28.49% [-28.57%, -28.40%]; Performance has improved; p 0.00 < 0.05 |
| stack | 686.918 [685.677, 688.150] ms | 471.914 [470.855, 473.116] ms | -31.30% [-31.50%, -31.09%]; Performance has improved; p 0.00 < 0.05 | 689.658 [688.151, 691.301] ms | 476.312 [475.412, 477.332] ms | -30.94% [-31.15%, -30.72%]; Performance has improved; p 0.00 < 0.05 |
| string | 419.148 [416.269, 421.751] ms | 343.534 [341.366, 345.242] ms | -18.04% [-18.76%, -17.34%]; Performance has improved; p 0.00 < 0.05 | 416.354 [413.208, 419.404] ms | 344.531 [344.003, 344.946] ms | -17.25% [-17.87%, -16.61%]; Performance has improved; p 0.00 < 0.05 |
| unsafe_buffer | 377.768 [376.802, 378.924] ms | 247.829 [247.430, 248.213] ms | -34.40% [-34.62%, -34.20%]; Performance has improved; p 0.00 < 0.05 | 379.150 [378.600, 379.747] ms | 249.094 [248.888, 249.308] ms | -34.30% [-34.42%, -34.19%]; Performance has improved; p 0.00 < 0.05 |

## nofeatures

| Case | B1 mean [95% CI] | C1 mean [95% CI] | C1/B1 change [95% CI]; Criterion | B2 mean [95% CI] | C2 mean [95% CI] | C2/B2 change [95% CI]; Criterion |
|---|---:|---:|---|---:|---:|---|
| runtime_primitives/eval_stack_push_i32_pop_safe | 13.309 [13.279, 13.341] ns | 14.005 [13.950, 14.053] ns | +5.23% [+4.76%, +5.66%]; Performance has regressed; p 0.00 < 0.05 | 13.842 [13.758, 13.922] ns | 14.741 [14.629, 14.855] ns | +6.50% [+5.54%, +7.56%]; Performance has regressed; p 0.00 < 0.05 |
| runtime_primitives/vector_trace_i32 | 1.286 [1.286, 1.287] ns | 1.244 [1.242, 1.246] ns | -3.33% [-3.49%, -3.16%]; Performance has improved; p 0.00 < 0.05 | 1.287 [1.285, 1.289] ns | 1.235 [1.232, 1.238] ns | -4.04% [-4.31%, -3.77%]; Performance has improved; p 0.00 < 0.05 |
| alloc_throughput | 394.360 [393.749, 395.185] ms | 332.413 [331.790, 333.296] ms | -15.71% [-15.96%, -15.43%]; Performance has improved; p 0.00 < 0.05 | 392.371 [391.604, 393.437] ms | 331.531 [330.787, 332.467] ms | -15.51% [-15.80%, -15.22%]; Performance has improved; p 0.00 < 0.05 |
| arithmetic | 459.164 [458.514, 459.719] ms | 244.723 [242.913, 246.498] ms | -46.70% [-47.08%, -46.25%]; Performance has improved; p 0.00 < 0.05 | 453.111 [452.336, 453.969] ms | 238.851 [238.167, 239.476] ms | -47.29% [-47.46%, -47.13%]; Performance has improved; p 0.00 < 0.05 |
| dispatch | 521.404 [519.171, 523.805] ms | 363.196 [360.786, 365.736] ms | -30.34% [-30.93%, -29.81%]; Performance has improved; p 0.00 < 0.05 | 519.906 [518.120, 521.931] ms | 364.085 [362.038, 366.244] ms | -29.97% [-30.43%, -29.47%]; Performance has improved; p 0.00 < 0.05 |
| ef_inmemory | 1.813 [1.809, 1.817] s | 1.713 [1.705, 1.720] s | -5.54% [-5.99%, -5.10%]; Performance has improved; p 0.00 < 0.05 | 1.810 [1.805, 1.814] s | 1.699 [1.693, 1.705] s | -6.10% [-6.46%, -5.74%]; Performance has improved; p 0.00 < 0.05 |
| gc | 294.480 [294.278, 294.690] ms | 238.372 [238.170, 238.580] ms | -19.05% [-19.15%, -18.97%]; Performance has improved; p 0.00 < 0.05 | 294.284 [293.783, 294.806] ms | 237.059 [236.833, 237.280] ms | -19.45% [-19.61%, -19.29%]; Performance has improved; p 0.00 < 0.05 |
| gc_cross_arena | 117.452 [117.157, 117.888] ms | 85.224 [85.158, 85.298] ms | -27.44% [-27.71%, -27.24%]; Performance has improved; p 0.00 < 0.05 | 116.860 [116.617, 117.285] ms | 85.091 [85.008, 85.180] ms | -27.19% [-27.47%, -27.00%]; Performance has improved; p 0.00 < 0.05 |
| generics | 10.479 [10.433, 10.542] s | 8.606 [8.595, 8.617] s | -17.87% [-18.38%, -17.49%]; Performance has improved; p 0.00 < 0.05 | 10.456 [10.416, 10.500] s | 8.621 [8.610, 8.631] s | -17.55% [-17.91%, -17.22%]; Performance has improved; p 0.00 < 0.05 |
| heap_access | 642.118 [641.609, 642.670] ms | 417.954 [417.255, 418.669] ms | -34.91% [-35.02%, -34.79%]; Performance has improved; p 0.00 < 0.05 | 639.025 [638.139, 640.000] ms | 418.768 [417.992, 419.530] ms | -34.47% [-34.62%, -34.31%]; Performance has improved; p 0.00 < 0.05 |
| json | 212.182 [211.921, 212.451] ms | 172.372 [172.199, 172.551] ms | -18.76% [-18.89%, -18.63%]; Performance has improved; p 0.00 < 0.05 | 211.246 [211.058, 211.438] ms | 163.194 [163.039, 163.361] ms | -22.75% [-22.85%, -22.65%]; Performance has improved; p 0.00 < 0.05 |
| json_dom | 272.400 [271.531, 273.493] ms | 234.618 [234.104, 235.409] ms | -13.87% [-14.27%, -13.48%]; Performance has improved; p 0.00 < 0.05 | 269.616 [268.931, 270.580] ms | 234.266 [233.901, 234.750] ms | -13.11% [-13.45%, -12.82%]; Performance has improved; p 0.00 < 0.05 |
| linq_pipeline | 529.350 [526.595, 533.282] ms | 486.724 [486.155, 487.632] ms | -8.05% [-8.73%, -7.54%]; Performance has improved; p 0.00 < 0.05 | 525.456 [524.766, 526.403] ms | 484.323 [483.707, 485.294] ms | -7.83% [-8.03%, -7.59%]; Performance has improved; p 0.00 < 0.05 |
| memory | 554.320 [553.263, 555.393] ms | 301.095 [300.575, 301.684] ms | -45.68% [-45.82%, -45.55%]; Performance has improved; p 0.00 < 0.05 | 551.759 [551.069, 552.497] ms | 303.865 [302.855, 305.033] ms | -44.93% [-45.14%, -44.71%]; Performance has improved; p 0.00 < 0.05 |
| reflection | 285.241 [285.016, 285.468] ms | 240.553 [240.305, 240.803] ms | -15.67% [-15.78%, -15.56%]; Performance has improved; p 0.00 < 0.05 | 284.265 [283.913, 284.633] ms | 240.856 [240.601, 241.131] ms | -15.27% [-15.41%, -15.12%]; Performance has improved; p 0.00 < 0.05 |
| span | 288.747 [287.931, 289.790] ms | 157.705 [157.348, 158.226] ms | -45.38% [-45.62%, -45.14%]; Performance has improved; p 0.00 < 0.05 | 289.136 [288.275, 290.112] ms | 158.565 [158.115, 159.097] ms | -45.16% [-45.41%, -44.92%]; Performance has improved; p 0.00 < 0.05 |
| span_equality | 313.568 [313.137, 314.008] ms | 187.379 [187.019, 187.733] ms | -40.24% [-40.39%, -40.10%]; Performance has improved; p 0.00 < 0.05 | 312.643 [312.208, 313.092] ms | 187.961 [187.525, 188.386] ms | -39.88% [-40.05%, -39.73%]; Performance has improved; p 0.00 < 0.05 |
| stack | 792.283 [790.823, 793.961] ms | 471.777 [471.013, 472.538] ms | -40.45% [-40.62%, -40.31%]; Performance has improved; p 0.00 < 0.05 | 791.379 [790.858, 791.893] ms | 469.906 [469.192, 470.697] ms | -40.62% [-40.72%, -40.53%]; Performance has improved; p 0.00 < 0.05 |
| string | 460.762 [459.688, 461.735] ms | 346.185 [345.618, 346.734] ms | -24.87% [-25.07%, -24.66%]; Performance has improved; p 0.00 < 0.05 | 461.512 [459.444, 463.305] ms | 345.845 [345.232, 346.471] ms | -25.06% [-25.38%, -24.70%]; Performance has improved; p 0.00 < 0.05 |
| unsafe_buffer | 423.409 [422.735, 424.095] ms | 230.868 [230.296, 231.631] ms | -45.47% [-45.64%, -45.28%]; Performance has improved; p 0.00 < 0.05 | 420.723 [420.283, 421.207] ms | 233.256 [232.402, 234.194] ms | -44.56% [-44.75%, -44.32%]; Performance has improved; p 0.00 < 0.05 |


## Focused repeats

For the differing effect sizes, we repeated default JSON, EF and generics and
no-default JSON in baseline–candidate–candidate–baseline order. These were the
same 30-sample, 3-second warmup, 5-second target runs, restricted to physical CPUs
2–5, with `RAYON_NUM_THREADS=4`. All eight comparisons below were classified as
improvements by Criterion (`p < 0.05`); no profiler or diagnostic build was used.
The direction repeats, while default EF's two intervals remain separated. Aggregate
CPU activity during these repeats averaged 1.03 busy CPUs and peaked at 1.10.

| Configuration | Case | B1 mean [95% CI] | C1 mean [95% CI] | C1/B1 change [95% CI] | B2 mean [95% CI] | C2 mean [95% CI] | C2/B2 change [95% CI] |
|---|---|---:|---:|---|---:|---:|---|
| default | ef_inmemory | 1850.969 [1845.382, 1856.296] ms | 1774.210 [1768.157, 1779.882] ms | -4.15% [-4.56%, -3.71%] | 1855.128 [1849.632, 1860.102] ms | 1724.907 [1720.715, 1729.133] ms | -7.02% [-7.37%, -6.67%] |
| default | generics | 10638.267 [10598.498, 10682.078] ms | 9383.004 [9354.952, 9415.797] ms | -11.80% [-12.23%, -11.33%] | 10571.262 [10541.015, 10606.488] ms | 9384.694 [9334.939, 9453.277] ms | -11.22% [-11.78%, -10.55%] |
| default | json | 201.749 [201.603, 201.897] ms | 170.901 [170.687, 171.104] ms | -15.29% [-15.42%, -15.18%] | 199.840 [199.727, 199.966] ms | 170.246 [169.935, 170.589] ms | -14.81% [-14.98%, -14.61%] |
| nofeatures | json | 210.937 [210.705, 211.170] ms | 166.484 [166.299, 166.682] ms | -21.07% [-21.20%, -20.94%] | 210.982 [210.812, 211.155] ms | 164.963 [164.789, 165.147] ms | -21.81% [-21.92%, -21.70%] |

The fixed-CPU primitive repeat used CPU 2 and the same 30/3/5 settings. These
means differ from Criterion's displayed slope estimates for linearly sampled cases.
All changes below were classified by Criterion as improvements or regressions, except
the second default stack comparison, for which Criterion detected no change.

| Configuration and control | B1 mean [95% CI] | C1 mean [95% CI] | C1/B1 change [95% CI] | B2 mean [95% CI] | C2 mean [95% CI] | C2/B2 change [95% CI] |
|---|---:|---:|---|---:|---:|---|
| default runtime_primitives/eval_stack_push_i32_pop_safe | 13.310 [13.288, 13.331] ns | 13.960 [13.901, 14.015] ns | +4.88% [+4.41%, +5.32%] | 13.949 [13.860, 14.030] ns | 13.991 [13.935, 14.041] ns | +0.30% [-0.41%, +1.04%] |
| default runtime_primitives/vector_trace_i32 | 1.196 [1.195, 1.196] ns | 1.290 [1.289, 1.291] ns | +7.85% [+7.77%, +7.95%] | 1.195 [1.194, 1.197] ns | 1.293 [1.291, 1.295] ns | +8.15% [+7.93%, +8.40%] |
| nofeatures runtime_primitives/eval_stack_push_i32_pop_safe | 13.305 [13.287, 13.321] ns | 13.861 [13.831, 13.900] ns | +4.18% [+3.91%, +4.50%] | 13.363 [13.260, 13.475] ns | 14.011 [13.951, 14.065] ns | +4.85% [+3.88%, +5.77%] |
| nofeatures runtime_primitives/vector_trace_i32 | 1.285 [1.284, 1.287] ns | 1.242 [1.239, 1.244] ns | -3.39% [-3.58%, -3.19%] | 1.287 [1.286, 1.289] ns | 1.242 [1.239, 1.244] ns | -3.54% [-3.78%, -3.30%] |

To investigate default stack drift, three additional fixed-CPU ABBA blocks used the
same settings. Criterion reported no change for block 1's first pair and regression
for the other five. Baseline drift within blocks 1 and 2 was -5.27% and +4.72%,
respectively; in block 3 the baseline was stable and both comparisons showed a
slowdown. This makes the direction concerning but the overall default magnitude
unstable. Aggregate host activity during these blocks averaged 1.05 busy CPUs and
peaked at 1.26.

| Block | B1 mean [95% CI] | C1 mean [95% CI] | C1/B1 change [95% CI] | B2 mean [95% CI] | C2 mean [95% CI] | C2/B2 change [95% CI] |
|---|---:|---:|---|---:|---:|---|
| 1 | 13.980 [13.885, 14.066] ns | 13.997 [13.940, 14.049] ns | +0.13% [-0.59%, +0.90%] | 13.243 [13.173, 13.326] ns | 13.882 [13.857, 13.913] ns | +4.83% [+4.15%, +5.43%] |
| 2 | 13.288 [13.258, 13.316] ns | 13.843 [13.816, 13.868] ns | +4.18% [+3.88%, +4.48%] | 13.915 [13.830, 13.995] ns | 14.173 [14.089, 14.261] ns | +1.85% [+1.02%, +2.74%] |
| 3 | 13.308 [13.283, 13.330] ns | 14.169 [14.067, 14.274] ns | +6.47% [+5.66%, +7.31%] | 13.280 [13.255, 13.305] ns | 14.019 [13.975, 14.056] ns | +5.56% [+5.16%, +5.93%] |

Fixture and framework hashes were checked again after all repeats: all 65 fixture
files per checkout and all 344 recorded framework files matched their pre-timing hashes.

## Diagnosis and limits

`ilspycmd` disassembly of the actual baseline fixture assemblies confirms no local or
argument address-taking in arithmetic `Program.Main` or stack `Main`, `RunRound` and
`Mix`; these only call one another. Their end-to-end improvements are therefore strong
controls, not evidence that all address-free methods improved. Heap-access, JSON DOM
and EF fixture `Main` contain `ldloca`. Inspection of the EF dependency also confirms
`CoreEventId.MakeUpdateId` uses `ldarga` and a constrained virtual call. Other
framework callees were not exhaustively classified.

The primitive stack test has an empty guest frame stack. Its Rust `push`/`pop_safe`
path, arena mutation, tracing checks and value copies all contribute to the timing.
Between revisions, `EvaluationStack::push` removed the capacity-change fixup and the
frame/stack data layout and generated loop changed. Optimized no-default benchmark
disassembly shows different inlined stack loops (baseline 2,709 bytes, candidate
2,025 bytes), with copies of the 112-byte stack value on the hot path. Separate
post-timing `perf` samples localized execution to those inlined benchmark loops, but
sampling skid and the number of code changes do not identify which change caused
the slowdown. No performance estimate is taken from those diagnostic profiles.

For the default vector control, the optimized benchmark loop and its
`LayoutManager::is_or_contains_refs` callee have the same decoded instructions in
the two binaries apart from relocated addresses and cold panic references. Separate
post-timing samples landed in those same two functions. The functions have different
code addresses; code placement and host effects are possible explanations, not proven
causes. The opposite no-default result further limits any generalization.

Raw samples, comparisons, telemetry, disassembly and diagnostic profiles remain in
`target/plan01-slice5/`; they are local, volatile artifacts. The new report and plan
link are uncommitted. No runtime fix was made. Because the no-home stack/vector
slowdowns remain unexplained and default stack is variable, leave plan 01 in progress
and ask the maintainer how to proceed. No slowdown of an address-free guest method was
confirmed in the two direct guest controls.

## Subsequent control investigation

The [control investigation](2026-11-05-frame-homes-control-investigation.md) isolates
a generated stack-copy/data-flow cost with matched binary experiments, and records
new vector results whose direction reverses or disappears under controlled launch
and placement. An alternate matched placement still varies, so vector code address
alone is not a complete explanation. No production repair was made; plan 01 remains
open for a maintainable stack-copy improvement or maintainer acceptance of its cost.

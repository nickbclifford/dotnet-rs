# Phase 6 Profile Comparison (P6.S2)

Generated: 2026-04-17T23:43:18.791250+00:00

Baseline sources:
- `crates/dotnet-benchmarks/baselines/phase0/baseline.json` (stored historical baseline, default config workloads)
- `crates/dotnet-benchmarks/baselines/phase6/bench-thin.json` (current run, `--profile bench-thin`, sample-size 10)
- `crates/dotnet-benchmarks/baselines/phase6/bench-fat.json` (current run, `--profile bench-fat`, sample-size 10)

Interpretation: negative `fat-vs-thin` means `bench-fat` is faster; negative `vs phase0` means improved versus stored phase0 baseline.

## Phase 1-3 Core Runtime

| Workload | bench-thin (ms) | bench-fat (ms) | fat-vs-thin | thin vs phase0 | fat vs phase0 |
|---|---:|---:|---:|---:|---:|
| json | 286.54 | 281.63 | -1.71% | -8.19% | -9.76% |
| arithmetic | 475.23 | 466.46 | -1.85% | -18.77% | -20.27% |
| dispatch | 721.56 | 713.90 | -1.06% | -9.14% | -10.11% |
| generics | 18188.95 | 17489.29 | -3.85% | -3.02% | -6.75% |

Summary: `bench-fat` faster on 4/4 workloads, slower on 0/4.

## Phase 4 Iterator/GC/Reflection

| Workload | bench-thin (ms) | bench-fat (ms) | fat-vs-thin | thin vs phase0 | fat vs phase0 |
|---|---:|---:|---:|---:|---:|
| gc | 423.67 | 415.08 | -2.03% | -2.17% | -4.15% |
| gc_cross_arena | 166.39 | 158.15 | -4.95% | n/a | n/a |
| stack | 895.02 | 861.96 | -3.69% | n/a | n/a |
| reflection | 369.35 | 352.27 | -4.62% | n/a | n/a |

Summary: `bench-fat` faster on 4/4 workloads, slower on 0/4.

## Phase 5 SIMD/Intrinsic Paths

| Workload | bench-thin (ms) | bench-fat (ms) | fat-vs-thin | thin vs phase0 | fat vs phase0 |
|---|---:|---:|---:|---:|---:|
| span | 489.11 | 489.93 | +0.17% | n/a | n/a |
| span_equality | 596.15 | 592.61 | -0.59% | n/a | n/a |
| memory | 1206.01 | 1216.02 | +0.83% | n/a | n/a |
| unsafe_buffer | 808.57 | 810.34 | +0.22% | n/a | n/a |
| string | 552.04 | 580.15 | +5.09% | n/a | n/a |

Summary: `bench-fat` faster on 1/5 workloads, slower on 4/5.


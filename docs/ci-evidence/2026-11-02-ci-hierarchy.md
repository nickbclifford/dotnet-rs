# CI hierarchy evidence — 2026-11-02

Measurements and observations collected while introducing the fast-core / select-deep / full-sweep
layering described in [CI.md](../CI.md#test-hierarchy-and-migration). This is a dated record,
not current policy; numbers are single samples unless stated.

## Before the hierarchy

- The 2026-10-31 accepted [CI run](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237422062)
  took 68m03s and its [advisory Miri run](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237423847)
  83m03s (99 and 57 jobs at that SHA). Native test jobs started after 56–64 minutes although their
  dependencies finished in about a minute; individual pointer jobs took 7–20 minutes. CI plus Miri
  peaked at 20 simultaneous jobs. Together they used about 1,411 runner-minutes, including 876
  pointer and 473 advisory Miri runner-minutes. Per-phase timings are in
  [2026-10-31-acceptance.md](2026-10-31-acceptance.md).
- The 2026-11-02 [push CI](https://github.com/nickbclifford/dotnet-rs/actions/runs/41367145909)
  took 87m39s; value-Miri exhausted its budget in the workspace source scan after its library
  tests passed. Locally the scan alone took 807 s under Miri, against about 30 s for the rest of
  the `dotnet-value` Miri suite. The scan is now `cfg(not(miri))` and runs natively.
- Read-only inspection returned "Branch not protected" for `main` and no applicable rules.

## Selector payoff estimate

Over the last 100 non-merge commits on `main` (each against its parent), `scripts/ci_select.py`
would have run both Miri lanes for 67 (52 also all four fuzz targets, 15 only `fuzz_executor`)
and skipped all deep work for 33.

## Why VM Miri tests are slow

Under Miri (no-features, permissive Tree Borrows) `pointer_provenance_tests::p00_smoke` took 233 s
while a trivial `dotnet-vm` unit test took 3 s; `RAYON_NUM_THREADS=1` made no difference. Natively
every pointer control cost 74–80M user instructions; callgrind on p00 attributed about 94% to
`AssemblyLoader::new_bare` → `add_support_library` → `dotnetdll` `Resolution::parse` of the
embedded support DLL. `get_mock_loader()` is a `thread_local!`, libtest runs each test on a fresh
thread and each shard is its own Miri process, so every singleton re-parsed the support library.

After dotnetdll 0.4.0 (`ReadOptions::with_own_heap_values(true)` under Miri, via
`default_read_options()`), with the same flags: `p00_smoke` 233 s → 36 s; pointer `batch_2`
(single control, no-features) 240 s → 41 s, multithreading 44 s.

## Pointer batches (before dotnetdll 0.4.0)

Local, pinned nightly, permissive flags, four Miri processes concurrently, warm build cache.
All eight batches passed. Miri test-process wall time:

| Batch | no-features | multithreading |
|---|---|---|
| 1 (9 controls) | 543 s | 466 s |
| 2 (8 controls) | 551 s | 475 s |
| 3 (8 controls) | 539 s | 473 s |
| 4 (8 controls) | 555 s | 478 s |

A batch costs roughly one parse plus 35–40 s per additional control. A deliberately failing
batch (p11 injected into batch 2, not committed) stopped after p02 with exit 1 and named p11 as
the control in flight. Hosted runners were 1.3–4.5x slower than local for p00, which sized the
60-minute batch job and 3000 s execute step.

## Cross-arena split

Run 41495133436 hit the 45-minute deadline during `teardown-stress` in the multithreading job,
when the three `dotnet-vm` recreations still ran serially in one job:

| Recreation | Run 41484662885 (passed) | Run 41495133436 |
|---|---|---|
| teardown | 610 s | 745 s |
| collection | 596 s | 870 s |
| teardown-stress | 563 s | killed by the deadline |

Runner speed varied 25–45% between the runs. The 870 s sample was within 4% of the old 900 s
per-invocation cap, so VM invocations moved to 1800 s and one process per 45-minute job. The
`library` group's execute phase measured about 5 minutes hosted (about 4 of them in
`transport-structural`), hence its 30-minute job. One MT ownership singleton
(`recreation_owner_first_guard_does_not_touch_successor`) took 36.84 s test time locally.

## After the hierarchy

One [push CI run](https://github.com/nickbclifford/dotnet-rs/actions/runs/41555605366) at
`b9ffaf5c` succeeded: fast-core completed 5m40s after run creation and deep-safety 10m15s after.
A [manual advisory Miri run](https://github.com/nickbclifford/dotnet-rs/actions/runs/41568669566)
at the same SHA took 9m50s.

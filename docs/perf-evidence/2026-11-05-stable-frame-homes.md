# Stable frame homes: slice 5 preflight (2026-11-05)

Slice 5 is incomplete. The original candidate EF Core failure below was repaired
after this preflight; see the [repair record](../plans/03-frame-home-enum-representation.md).
No performance estimates or regression claims were obtained. Plan 02 remains in progress.

## Revisions and configuration

- Baseline: `0bae37275891576cf76d7daf4ab552229805d5b8`, main before slice 1.
- Candidate: `c743e4d0` (full revision and host details in the local manifest).
- Rust 1.95.0, .NET SDK 10.0.114, Linux x86_64, powersave governor.
- Locked offline Cargo builds, `bench-fat`, default features, `RAYON_NUM_THREADS=4`;
  no PGO, profiler, validation or benchmark instrumentation.
- Fresh independent build directories were required because the existing target
  cache retained build-script paths into a deleted `/tmp/byref-223-base` checkout.

Both benchmark targets compiled at both revisions. Baseline prewarming passed both
runtime primitives and all 18 end-to-end cases. Candidate prewarming passed both
primitives and the first 17 end-to-end cases, then failed at `ef_inmemory`.
NuGet emitted vulnerability-feed connectivity warnings, but fixture builds succeeded.

## Reproduced correctness blocker

Candidate EF execution fails with:

```text
Type resolution failed: Invalid layout: argument 0 does not match its parameter type: Memory access error: Type mismatch: Expected ValueType for Struct write
```

Copying the baseline's complete prebuilt fixture cache into the candidate's cache
and rerunning EF produces the same error. A fresh baseline EF rerun passes. This
rules out differing fixture binaries as the explanation for this reproduction.

The error originates in `VesContext::install_frame_homes`, when `store_home` copies
an addressed argument from its evaluation-stack slot into the declared home layout.
The field-layout writer rejects a value other than `StackValue::ValueType`. The
specific guest method and cause of the representation mismatch remain undiagnosed;
this observation does not establish which slice introduced it.

Reproduce after building and prewarming the targets:

```bash
RAYON_NUM_THREADS=4 target/plan01-slice5/baseline-build/bench-fat/deps/end_to_end-b83fb585a98f8aa1 --test '^ef_inmemory$'
RAYON_NUM_THREADS=4 target/plan01-slice5/candidate-build/bench-fat/deps/end_to_end-b83fb585a98f8aa1 --test '^ef_inmemory$'
```

Executable hashes are local build details; discover fresh executables from Cargo's
`--no-run` output on another host. The source archive is under
`target/plan01-slice5/baseline-src`. Build each checkout with a separate
`CARGO_TARGET_DIR`, `cargo bench --locked --offline --profile bench-fat
-p dotnet-benchmarks --bench runtime_primitives --bench end_to_end --no-run`.

## Artifacts and remaining work

Local artifacts are under `target/plan01-slice5/`: `manifest.json`, the reproduction
and build logs in `logs/`, and `run.py`, the attempted sequential measurement driver.
These target artifacts are volatile and are not committed evidence substitutes.
No timed samples were collected; the no-default-features comparison and IL
classification have not run. The driver stops on failed prewarming and must not be
used to claim a completed measurement campaign.

Resolve the EF correctness blocker before resuming the agreed full comparison:
both suites, both feature configurations, baseline–candidate–candidate–baseline,
30 samples, 3-second warmup and 5-second measurement. Verify fixture hashes,
classify address-free controls from compiled IL, preserve raw results and report
confidence intervals. Confirmed address-free slowdowns require maintainer review
before runtime fixes. No runtime change was made in this attempt.

## Validation of this documentation change

`git diff --check` passed. `./check.sh --fast` passed support-slot consumer
coverage, then remained in the .NET fixture-build stage without further output;
it was interrupted. The fast suite did not complete and no pass is claimed.
The focused baseline/candidate EF reproductions above completed. Slice-4 evidence
remains the prior deep-lane evidence, not validation of this newly found workload.

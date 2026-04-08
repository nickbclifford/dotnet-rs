# Agent Session Log
## Session: 2026-04-07T20:20:21-05:00
### Context
- Phase/Step: Architecture review (Phases 0-4)
- Goal: Produce a low-level performance review, phased checklist, and follow-up tooling scaffold.
### Actions Taken
- Read `AGENTS.md` and architecture/GC/threading/build-generation docs.
- Reviewed dispatch, stack, pointer/value/object layout, resolver cache, heap manager, GC coordinator, and sync code paths with line-level evidence.
- Collected release layout measurements for `StackValue`, `ManagedPtr`, `PointerOrigin`, `ObjectInner`, and related hot types.
- Collected synthetic evaluation-stack reallocation timing baselines.
- Collected runtime cache/GC metric sample using existing fixture DLL (`gc/cache_test_0`).
- Authored `REVIEW.md`, `CHECKLIST.md`, `AGENT_PROMPT.md`, and `copy_prompt.py`.
### Findings
- `StackValue` and `ManagedPtr` are large in release (`184` and `168` bytes), with additional growth under `memory-validation`.
- Evaluation stack growth still performs O(n) pointer fixup on each `Vec` reallocation.
- Shared global caches are unbounded `DashMap`s with clone-heavy keys and no bounded policy.
- `HeapManager` keeps `_all_objs` in a `BTreeMap` in normal construction path.
- Intrinsic key building still does per-call canonicalization and string normalization before PHF lookup.
- Existing metrics cover cache hit/miss, GC pause, and lock contention, but not dispatch/opcode counts or intrinsic signature counts.
- Benchmark harness requested in Phase 0 does not yet exist in workspace.
### Blockers
- none
### Next Steps
- Implement Phase 0 benchmark crate and instrumentation feature (`0a`, `0b`, `0c`) before performance refactors.
- Start with `1a` and `1b` only after baseline capture is reproducible.
- Use `copy_prompt.py` to generate per-step execution prompts for follow-up agents.
---
## Session: 2026-04-07T20:49:49-05:00
### Context
- Phase/Step: Phase 0 / Step 0a (Criterion benchmark harness)
- Goal: Add reproducible Criterion end-to-end benchmark crate and seed fixtures for five workload classes.
### Actions Taken
- Read `AGENTS.md`, architecture/build-generation docs, and step files in `CHECKLIST.md`.
- Added workspace member `crates/dotnet-benchmarks` in root `Cargo.toml`.
- Created `crates/dotnet-benchmarks` with feature forwarding aligned to `dotnet-cli`/`dotnet-vm` and Criterion bench target (`benches/end_to_end.rs`, `harness = false`).
- Implemented benchmark harness in `crates/dotnet-benchmarks/src/lib.rs`:
  - fixture specs and named workload IDs (`json`, `arithmetic`, `gc`, `dispatch`, `generics`),
  - fixture compilation via `dotnet build` + `SingleFile.csproj`,
  - output path under `target/<profile>/dotnet-bench-fixtures`,
  - in-process VM execution path using `AssemblyLoader` + `Executor`.
- Added five benchmark fixtures under `crates/dotnet-benchmarks/fixtures/{json,arithmetic,gc,dispatch,generics}`.
- Adjusted benchmark registration so fixture compilation is lazy per selected benchmark, preserving name-filter behavior.
- Updated `CHECKLIST.md` Step `0a` status/tasks to completed.
### Findings
- `System.Text.Json` and common string/parsing APIs trigger currently unimplemented runtime intrinsic paths in bench mode (`RuntimeHelpers.TryGetHashCode`, `UInt32.Log2`, `Unsafe.NullRef`).
- JSON fixture was reworked to a manual JSON-like parse workload using only supported primitives.
- Criterion artifacts were generated successfully at `target/criterion/json/**` and fixture DLLs at `target/release/dotnet-bench-fixtures/**`.
- Re-running `cargo bench -p dotnet-benchmarks -- json` produced an immediate Criterion delta against prior run:
  - prior: `json` time `[318.63 ms, 320.17 ms]`
  - latest: `json` time `[315.12 ms, 317.21 ms]`
  - reported change: about `-1.02%` (within noise threshold).
### Blockers
- none
### Next Steps
- Implement Step `0b` (`bench-instrumentation`) and expose instrumentation snapshots in benchmark outputs.
- Run full five-workload baseline capture for Step `0c` (default + no-default-features where applicable).
---
## Session: 2026-04-07T21:05:00-05:00
### Context
- Phase/Step: Phase 0 / Step 0b (Bench instrumentation feature)
- Goal: Add a feature-gated instrumentation path for benchmark-only metrics and emit serializable snapshots from benchmark runs.
### Actions Taken
- Read `AGENTS.md`, architecture/build-generation docs, `CHECKLIST.md`, and all step-relevant files (`dotnet-vm`, `dotnet-vm-data`, `dotnet-metrics`, benchmark harness files).
- Added feature plumbing:
  - `dotnet-metrics`: new `bench-instrumentation` feature.
  - `dotnet-vm-data`: optional `dotnet-metrics` dep + `bench-instrumentation` feature.
  - `dotnet-vm-ops`: forwarded `bench-instrumentation` to `dotnet-vm-data`.
  - `dotnet-vm`: new `bench-instrumentation` feature forwarding to metrics + vm-ops.
  - `dotnet-benchmarks`: forwarded `bench-instrumentation` to `dotnet-vm`.
- Extended `dotnet-metrics`:
  - added bench counters for eval-stack reallocations/fixup duration,
  - opcode category dispatch totals,
  - per-signature intrinsic call totals,
  - serializable `RuntimeMetricsSnapshot` (+ feature-gated bench snapshot payload),
  - thread-local `ActiveRuntimeMetricsGuard` and active-metrics recording helper.
- Instrumented runtime hot paths:
  - `EvaluationStack::push` now records reallocation + pointer-fixup duration under `bench-instrumentation`.
  - Dispatch loop records opcode category counters.
  - Intrinsic dispatch records per-signature call counters.
  - Executor establishes active-metrics scope and exposes `get_runtime_metrics_snapshot()`.
  - Shared state exposes `get_runtime_metrics_snapshot()`.
- Extended benchmark harness:
  - Added run result struct with runtime metrics snapshot.
  - Wrote JSON snapshots to `target/<profile>/dotnet-bench-metrics/<case>.json` once per benchmark case (after Criterion iteration loop).
- Updated `CHECKLIST.md` Step `0b` to completed with all atomic tasks checked.
### Findings
- Instrumentation benchmark artifacts are emitted at `target/release/dotnet-bench-metrics/json.json`.
- New counters observed (json workload, instrumentation enabled):
  - `eval_stack_reallocations`: `4`
  - `eval_stack_pointer_fixup_count`: `4`
  - `eval_stack_pointer_fixup_total_ns`: `591`
  - `opcode_dispatch_total`: `1,676,145`
  - non-zero opcode categories: `stack`, `flow`, `comparisons`, `calls`, `objects`, `memory`, `conversions`
  - `intrinsic_call_total`: `0` (expected for this fixture path)
- Benchmark deltas:
  - Default (same code, no instrumentation): `json` `[313.17 ms, 314.75 ms]`
  - With `bench-instrumentation`: `json` `[417.45 ms, 421.07 ms]`
  - Instrumented run vs non-instrumented run: roughly `+33%` slower.
  - Instrumented run vs prior Step 0a stored baseline (`[315.12 ms, 317.21 ms]`): roughly `+32%` to `+34%`.
### Blockers
- none
### Next Steps
- Proceed to Step `0c` to capture/persist full five-workload baselines (default + no-default-features where applicable), including the new instrumentation snapshot JSONs.
- Optionally tighten instrumentation overhead in follow-up (e.g., avoid per-instruction `format!("{:?}", instr)` in category classification) if runtime perturbation becomes too large for analysis.
---
## Session: 2026-04-07T22:30:00-05:00
### Context
- Phase/Step: Phase 0 / Step 0c (Baseline capture and persistence)
- Goal: Capture and persist reproducible Criterion baselines (default + no-default), persist instrumentation snapshots, and verify rerun delta reporting.
### Actions Taken
- Read `AGENTS.md`, `docs/ARCHITECTURE.md`, `docs/VALIDATION_FEATURES.md`, `CHECKLIST.md`, `REVIEW.md`, benchmark harness files, and prior `AGENT_MEMORY.md` entries.
- Added baseline automation script: `crates/dotnet-benchmarks/scripts/capture_baseline.py`.
  - Runs `cargo bench -p dotnet-benchmarks --bench end_to_end` per workload (`json`, `arithmetic`, `gc`, `dispatch`, `generics`) for:
    - default features,
    - `--no-default-features`,
    - `--features bench-instrumentation`.
  - Supports configurable Criterion timings (`--sample-size`, `--measurement-time`, `--warm-up-time`).
  - Parses Criterion `estimates.json` + `sample.json` to persist median/p95/mean.
  - Copies raw Criterion artifact trees into `crates/dotnet-benchmarks/baselines/phase0/criterion/<config>/<workload>/`.
  - Persists summary + instrumentation into `crates/dotnet-benchmarks/baselines/phase0/baseline.json`.
  - Prints delta vs previously stored baseline.
- Fixed script persistence behavior for partial reruns:
  - preserved prior config/workload entries on subset runs,
  - merged updates by workload rather than replacing full config tables,
  - preserved full `workloads` set in baseline metadata.
- Captured full five-workload baseline artifacts (default, no-default, instrumented) and persisted them under `crates/dotnet-benchmarks/baselines/phase0/`.
- Re-ran one benchmark (`json`) with the script and confirmed delta reporting against stored baseline.
- Updated `REVIEW.md` Phase 0 with baseline medians/p95, derived deltas, and instrumentation counters.
- Updated `CHECKLIST.md` Step `0c` status and all atomic tasks to completed.
- Ran required verification commands:
  - `cargo clippy --all-targets -- -D warnings`
  - `cargo test -- --nocapture`
### Findings
- Persisted baseline medians (default, ms):
  - `json` `312.11`, `arithmetic` `585.06`, `gc` `433.05`, `dispatch` `794.17`, `generics` `18754.46`.
- Persisted baseline medians (no-default, ms):
  - `json` `312.95`, `arithmetic` `574.24`, `gc` `428.88`, `dispatch` `764.01`, `generics` `17920.70`.
- Persisted baseline medians (instrumented, ms):
  - `json` `412.76`, `arithmetic` `1034.82`, `gc` `608.92`, `dispatch` `1200.72`, `generics` `23229.22`.
- Instrumentation overhead (median vs default):
  - `json +32.25%`, `arithmetic +76.87%`, `gc +40.61%`, `dispatch +51.19%`, `generics +23.86%`.
- Re-run delta verification (`json`) from script output:
  - `default`: median `+0.41%`, p95 `+0.96%`.
### Blockers
- none
### Next Steps
- Use `crates/dotnet-benchmarks/baselines/phase0/baseline.json` as the baseline reference for Step `1a` performance comparisons.
- If instrumentation perturbation blocks interpretation in later steps, reduce per-op instrumentation overhead (e.g., category/signature classification cost) while keeping counters functionally equivalent.
---

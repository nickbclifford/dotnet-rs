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
## Session: 2026-04-08T12:00:00-05:00
### Context
- Phase/Step: Phase 1 / Step 1a (StackValue footprint reduction)
- Goal: Reduce `StackValue` hot-path footprint via indirection for large byref variants, add compile-time size guards, and record benchmark impact.
### Actions Taken
- Read `AGENTS.md`, `docs/ARCHITECTURE.md`, `docs/GC_AND_MEMORY_SAFETY.md`, `CHECKLIST.md`, `REVIEW.md`, and all step-relevant files (`stack_value.rs`, `object/types.rs`, `pointer/mod.rs`, plus downstream call sites).
- Implemented boxed indirection for stack byref variants:
  - added `StackManagedPtr` wrapper (`Box<ManagedPtr>`) in `crates/dotnet-value/src/stack_value.rs`,
  - changed `StackValue::ManagedPtr` and `StackValue::TypedRef` to store `StackManagedPtr`,
  - updated constructors/arithmetic/pointer access paths to preserve semantics.
- Updated tracing/representation plumbing:
  - added `Collect` impl for `StackManagedPtr`,
  - updated `StackValue` GC tracing to trace wrapped pointers,
  - updated `CTSValue::into_stack` mappings for pointer/typedref values.
- Updated serialization-adjacent and byref consumers across workspace to construct/deconstruct the new `StackValue` representation (`.into()` / `.into_inner()` conversions) in resolver, runtime-memory, pinvoke, intrinsics, VM instruction handlers, and call setup.
- Added `crates/dotnet-value/tests/layout_sizes.rs`:
  - compile-time assertions (`const` asserts) for `size_of`/`align_of` of `StackValue`, `ManagedPtr`, and `PointerOrigin`,
  - config-aware expected sizes for debug/memory-validation vs optimized non-validation builds.
- Updated `CHECKLIST.md` Step `1a` status/tasks to completed.
- Ran required verification:
  - `cargo clippy --all-targets -- -D warnings`
  - `cargo test -- --nocapture`
  - `cargo clippy --all-targets --no-default-features -- -D warnings`
  - `cargo test --no-default-features -- --nocapture --test-threads=1`
- Ran step benchmarks before and after change:
  - before: `cargo bench -p dotnet-benchmarks --bench end_to_end -- arithmetic --sample-size 10 --measurement-time 5 --warm-up-time 1`
  - before: `cargo bench -p dotnet-benchmarks --bench end_to_end -- json --sample-size 10 --measurement-time 5 --warm-up-time 1`
  - after: same two commands.
### Findings
- Layout impact (64-bit):
  - `StackValue` release size reduced from `184` to `120` bytes (`-34.8%`),
  - `StackValue` debug/default size is `128` bytes,
  - `ManagedPtr` remained `168` (release) / `184` (debug or `memory-validation`),
  - `PointerOrigin` remained `120` (release) / `128` (debug or `memory-validation`).
- Benchmark impact (point estimates from Criterion run):
  - `arithmetic`: before `602.67 ms`, after `534.48 ms` (`-11.31%`).
  - `json`: before `337.19 ms`, after `311.44 ms` (`-7.64%`).
- Comparison vs Phase 0 stored baseline medians (`baseline.json`):
  - `arithmetic`: `585.06 ms` -> `534.48 ms` (`-8.64%`).
  - `json`: `312.11 ms` -> `311.44 ms` (`-0.21%`, effectively neutral/slightly improved).
### Blockers
- none
### Next Steps
- Proceed to Step `1b` (ManagedPtr compaction), now that `StackValue` size is reduced and guarded by compile-time assertions.
- As part of Step `1b`, remove the existing pre-existing release warning in `pointer/serde.rs` (`OBJECT_MAGIC` import) to keep bench runs warning-clean.
---
## Session: 2026-04-08T12:37:42-05:00
### Context
- Phase/Step: Phase 1 / Step 1b (ManagedPtr compaction)
- Goal: Compact `ManagedPtr`/`PointerOrigin` layout, preserve pointer serialization and `'gc` safety, and record pointer-heavy benchmark impact.
### Actions Taken
- Read `AGENTS.md`, `docs/ARCHITECTURE.md`, `docs/GC_AND_MEMORY_SAFETY.md`, `CHECKLIST.md`, `REVIEW.md`, and step-relevant files in `dotnet-value` and `dotnet-utils`.
- Added `ManagedByteOffset(u32)` in `crates/dotnet-utils/src/newtypes.rs` and re-exported it from `dotnet-utils`.
- Refactored `ManagedPtr` in `crates/dotnet-value/src/pointer/mod.rs`:
  - replaced `pinned: bool` with packed `flags: u8`,
  - stored compact `ManagedByteOffset` for managed origins,
  - kept unmanaged full-width correctness by falling back to `_value` for absolute addresses when offset exceeds `u32`,
  - updated constructors/accessors (`byte_offset`, `is_pinned`, `with_origin`, `with_stack_origin`, `offset`, etc.).
- Compacted `PointerOrigin` in `crates/dotnet-value/src/pointer/origin.rs`:
  - `Static(TypeDescription, GenericLookup)` -> `Static(Arc<StaticMetadata>)`,
  - `Transient(Object)` -> `Transient(Box<Object>)`,
  - added helper constructors/accessors (`new_static`, `new_transient`, `static_parts`).
- Updated pointer serialization paths in `crates/dotnet-value/src/pointer/serde.rs`:
  - switched to `self.byte_offset()` instead of raw stored field,
  - adjusted static origin round-trip to use `Arc<StaticMetadata>`,
  - fixed pre-existing warning by tightening `OBJECT_MAGIC` import cfg.
- Updated downstream call sites:
  - transient origin creators in `crates/dotnet-intrinsics-unsafe/src/lib.rs` and `crates/dotnet-vm/src/instructions/objects/mod.rs`,
  - static origin pattern matches in `crates/dotnet-vm/src/stack/raw_memory_ops_impl/mod.rs`.
- Updated tests and layout guards:
  - pointer static-origin assertions in `crates/dotnet-value/src/pointer/tests.rs`,
  - feature-aware size assertions in `crates/dotnet-value/tests/layout_sizes.rs`.
- Updated docs/status:
  - marked Step `1b` completed in `CHECKLIST.md`,
  - recorded Step `1b` implementation and measurements in `REVIEW.md`.
- Ran required verification:
  - `cargo clippy --all-targets -- -D warnings`
  - `cargo test -- --nocapture`
  - `cargo clippy --all-targets --no-default-features -- -D warnings`
  - `cargo test --no-default-features -- --nocapture --test-threads=1`
  - `cargo test -p dotnet-value -- --nocapture`
- Ran pointer-heavy benchmarks:
  - `cargo bench -p dotnet-benchmarks --bench end_to_end -- gc --sample-size 10 --measurement-time 5 --warm-up-time 1`
  - `cargo bench -p dotnet-benchmarks --bench end_to_end -- dispatch --sample-size 10 --measurement-time 5 --warm-up-time 1`
  - `cargo bench -p dotnet-benchmarks --bench end_to_end -- json --sample-size 10 --measurement-time 5 --warm-up-time 1`
### Findings
- Layout impact (64-bit):
  - default release: `ManagedPtr 168 -> 72` (`-57.14%`), `PointerOrigin 120 -> 24` (`-80.00%`),
  - default debug: `ManagedPtr 184 -> 80` (`-56.52%`), `PointerOrigin 128 -> 24` (`-81.25%`),
  - no-default: `ManagedPtr 72` (debug) / `64` (release), `PointerOrigin 16`.
- Benchmark medians vs Phase 0 baseline (`baseline.json`):
  - `gc`: `433.05 ms -> 441.66 ms` (`+1.99%`),
  - `dispatch`: `794.17 ms -> 741.26 ms` (`-6.66%`),
  - `json`: `312.11 ms -> 324.99 ms` (`+4.13%`), with Criterion run-level comparison reporting no significant change (`p=0.38`).
- Pointer serde and round-trip behavior remained passing in `dotnet-value` test suite.
### Blockers
- none
### Next Steps
- Proceed to Step `1c` (Object header compaction).
- If Step `1c` uses `json` as an indicator workload, re-run `json` with longer Criterion windows to reduce variance before attributing small regressions.
---
## Session: 2026-04-08T12:52:29-05:00
### Context
- Phase/Step: Phase 1 / Step 1c (Object header compaction)
- Goal: Compact `ObjectInner` header metadata while preserving cross-arena safety and validation semantics.
### Actions Taken
- Read `AGENTS.md`, `docs/ARCHITECTURE.md`, `docs/GC_AND_MEMORY_SAFETY.md`, `CHECKLIST.md`, `REVIEW.md`, and step-relevant files (`crates/dotnet-value/src/object/mod.rs`, `crates/dotnet-value/src/validation.rs`, `crates/dotnet-runtime-memory/src/heap.rs`, plus owner-id consumers in `dotnet-runtime-memory` and `dotnet-vm`).
- Refactored `ObjectInner` layout in `crates/dotnet-value/src/object/mod.rs`:
  - gated `magic` field to `any(feature = "memory-validation", debug_assertions)`,
  - gated `owner_id` field to `any(feature = "multithreading", feature = "memory-validation")`,
  - added `ObjectInner::new(storage, owner_id)` and `ObjectInner::owner_id()` accessors,
  - documented owner-id invariants and added `with_magic_for_tests` helper for corruption tests.
- Updated all direct owner/magic consumers to use accessors:
  - `crates/dotnet-value/src/object/mod.rs` (trace path, serialization read/write, tests),
  - `crates/dotnet-value/src/pointer/serde.rs`,
  - `crates/dotnet-runtime-memory/src/access.rs`,
  - `crates/dotnet-vm/src/stack/context.rs`.
- Added zero-cost compile-time guard in `crates/dotnet-value/src/validation.rs` asserting `ValidationTag` is zero-sized in non-validation release builds.
- Extended `crates/dotnet-value/tests/layout_sizes.rs` with `ObjectInner` size/alignment assertions (validation and non-validation configurations).
- Updated status/docs:
  - marked Step `1c` completed in `CHECKLIST.md`,
  - replaced REVIEW Step `1c` section with implemented changes and measurements.
- Ran required and step-relevant verification:
  - `cargo clippy --all-targets -- -D warnings`
  - `cargo test -- --nocapture`
  - `cargo clippy --all-targets --no-default-features -- -D warnings`
  - `cargo test --no-default-features -- --nocapture --test-threads=1`
  - targeted validation checks:
    - `cargo test -p dotnet-value --features memory-validation -- --nocapture`
    - `cargo test -p dotnet-value --no-default-features --features memory-validation -- --nocapture --test-threads=1`
- Ran allocation benchmark before/after:
  - before: `cargo bench -p dotnet-benchmarks --bench end_to_end -- gc --sample-size 10 --measurement-time 5 --warm-up-time 1`
  - after: same command post-change.
- Collected release object-layout probes (default, no-default, memory-validation) via temporary path-based `dotnet-value` size probe crates.
### Findings
- Object header/layout impact (64-bit release):
  - default: `ObjectInner 136`, `HeapStorage 128`, overhead `+8` (unchanged),
  - no-default: `ObjectInner 128`, `HeapStorage 128`, overhead `+0` (previously implied `+8`),
  - default + memory-validation: `ObjectInner 152`, `HeapStorage 136`, overhead `+16`.
- Per-object memory impact:
  - single-threaded non-validation release path saves `8` bytes per object (`136 -> 128`, `-5.88%`).
- Allocation benchmark (`gc`) point estimate:
  - before `450.04 ms`, after `445.56 ms` (`-0.99%`),
  - Criterion reported no statistically significant change (`p=0.11`).
- Baseline comparison (Phase 0 median):
  - `gc`: `433.05 ms -> 445.56 ms` (`+2.89%`) on this run.
- Cross-arena and validation behavior remained passing across default/no-default test suites and targeted memory-validation test runs.
### Blockers
- none
### Next Steps
- Proceed to Step `2a` (evaluation stack reallocation elimination).
- If allocation-pressure variance remains noisy, rerun `gc` with longer Criterion windows before attributing sub-2% deltas.
---
## Session: 2026-04-08T13:05:00-05:00
### Context
- Phase/Step: Phase 2 / Step 2a (Evaluation stack reallocation elimination)
- Goal: Thread IL `max_stack` into runtime method metadata, pre-reserve eval stack capacity on frame push, prototype segmented reserve behavior behind a feature flag, and validate stack-origin managed pointer correctness.
### Actions Taken
- Read `AGENTS.md`, `docs/ARCHITECTURE.md`, `docs/GC_AND_MEMORY_SAFETY.md`, `docs/THREADING_AND_SYNCHRONIZATION.md`, `CHECKLIST.md`, `REVIEW.md`, and all step-relevant files (`crates/dotnet-vm-data/src/lib.rs`, `crates/dotnet-vm-data/src/stack.rs`, `crates/dotnet-vm/src/lib.rs`, `crates/dotnet-vm/src/stack/call_ops_impl.rs`).
- Added `max_stack: usize` to `MethodInfo` in `crates/dotnet-vm-data/src/lib.rs`.
- Updated `build_method_info` in `crates/dotnet-vm/src/lib.rs` to populate:
  - `max_stack: body.header.maximum_stack_size` for methods with bodies,
  - `max_stack: 0` for body-less methods.
- Added `EvaluationStack::reserve_slots(total_slots)` in `crates/dotnet-vm-data/src/stack.rs`:
  - reserves capacity ahead of use,
  - applies `update_stack_pointers()` once when reserve reallocation occurs,
  - records bench instrumentation through the existing reallocation counter path.
- Refactored `EvaluationStack::push` to share a single reallocation-fixup helper.
- Added segmented prototype reserve behavior behind new feature `segmented-eval-stack-prototype`:
  - reserve target rounds to 64-slot boundaries under the feature.
- Wired feature forwarding through:
  - `crates/dotnet-vm-data/Cargo.toml`
  - `crates/dotnet-vm-ops/Cargo.toml`
  - `crates/dotnet-vm/Cargo.toml`
  - `crates/dotnet-cli/Cargo.toml`
  - `crates/dotnet-benchmarks/Cargo.toml`
- Updated frame setup in `crates/dotnet-vm/src/stack/call_ops_impl.rs`:
  - `call_frame` now reserves `locals_base + locals + method.max_stack` before local initialization,
  - `entrypoint_frame` now reserves `argument_base + args + locals + method.max_stack` before argument push.
- Updated docs/status:
  - marked Step `2a` complete with all atomic tasks in `CHECKLIST.md`,
  - replaced REVIEW Step `2a` section with implementation and measurements.
- Ran verification:
  - `cargo clippy --all-targets -- -D warnings`
  - `cargo test -- --nocapture`
  - `cargo clippy --all-targets --no-default-features -- -D warnings`
  - `cargo test --no-default-features -- --nocapture --test-threads=1`
- Ran targeted deep-stack/byref fixtures:
  - `cargo test -p dotnet-cli --test integration_tests -- --nocapture basic_span_stack_read_0`
  - `cargo test -p dotnet-cli --test integration_tests -- --nocapture basic_span_pinnable_stack_0`
  - `cargo test -p dotnet-cli --test integration_tests -- --nocapture pointers_ref_struct_stress_42`
  - `cargo test -p dotnet-cli --test integration_tests -- --nocapture unsafe_ref_struct_gc_safety_42`
- Benchmarked dispatch workload before/after and prototype:
  - pre-change instrumented: `cargo bench -p dotnet-benchmarks --bench end_to_end --features bench-instrumentation -- dispatch --sample-size 10 --measurement-time 5 --warm-up-time 1`
  - pre-change default: `cargo bench -p dotnet-benchmarks --bench end_to_end -- dispatch --sample-size 10 --measurement-time 5 --warm-up-time 1`
  - post-change instrumented/default: same commands
  - prototype instrumented/default: same commands with `--features segmented-eval-stack-prototype` (plus `bench-instrumentation` where applicable).
### Findings
- Dispatch runtime (default, Criterion median point estimate):
  - pre-change: `751.15 ms`
  - post-change: `725.98 ms`
  - delta: `-3.35%`.
- Dispatch vs Phase 0 baseline (`794.17 ms`): `-8.59%`.
- Bench instrumentation (`dispatch`) reallocation/fixup counters:
  - pre-change: `eval_stack_reallocations=3`, `eval_stack_pointer_fixup_total_ns=711`
  - post-change: `eval_stack_reallocations=2`, `eval_stack_pointer_fixup_total_ns=331`
  - delta: reallocations `-33.33%`, fixup time `-53.45%`.
- Segmented prototype (`segmented-eval-stack-prototype`) results:
  - default median: `726.27 ms` (`+0.04%` vs non-prototype post-change run, effectively neutral),
  - instrumented counters: `eval_stack_reallocations=1`, `eval_stack_pointer_fixup_total_ns=110`.
- Managed pointer stack-origin semantics remained correct in targeted stack/byref fixtures and full integration suites.
### Blockers
- none
### Next Steps
- Proceed to Step `2b` (HeapManager container modernization).
- For deeper Step `2a` follow-up, consider a true segmented backing store (not only chunked reserve policy) if further reallocation elimination is required.
---
## Session: 2026-04-08T13:33:14-05:00
### Context
- Phase/Step: Phase 2 / Step 2f rollback
- Goal: Revert Step `2f` integration complexity and return workspace behavior/status to Step `2a` baseline.
### Actions Taken
- Reverted `2f` code-path changes while preserving existing Step `2a` work.
- Restored `EvaluationStack` in `crates/dotnet-vm-data/src/stack.rs` to contiguous `Vec` backing plus the existing Step `2a` reserve policy and fixup behavior.
- Removed `2f`-specific VM call-site adaptations and restored prior stack access paths:
  - `crates/dotnet-vm/src/stack/mod.rs`
  - `crates/dotnet-vm/src/stack/stack_ops_impl.rs`
  - `crates/dotnet-vm/src/executor.rs`
- Reverted planning/status docs for `2f` back to not-started/proposed:
  - `CHECKLIST.md` Step `2f` status/tasks reset
  - `REVIEW.md` Step `2f` section restored to Evidence/Proposed Change/Expected Impact/Risk
  - removed the prior `2f` completion entry from `AGENT_MEMORY.md`.
- Ran verification:
  - `cargo clippy --all-targets -- -D warnings`
  - `cargo test -- --nocapture`
  - `cargo clippy --all-targets --no-default-features -- -D warnings`
  - `cargo test --no-default-features -- --nocapture --test-threads=1`
- Ran dispatch benchmark and instrumentation at the reverted state:
  - `cargo bench -p dotnet-benchmarks --bench end_to_end -- dispatch --sample-size 10 --measurement-time 5 --warm-up-time 1`
  - `cargo bench -p dotnet-benchmarks --bench end_to_end --features bench-instrumentation -- dispatch --sample-size 10 --measurement-time 5 --warm-up-time 1`
### Findings
- Reverted default dispatch median point estimate: `752.93 ms` for this run.
- Bench instrumentation at reverted state:
  - `eval_stack_reallocations=2`
  - `eval_stack_pointer_fixup_count=2`
  - `eval_stack_pointer_fixup_total_ns=520`
- Behavior and test parity returned to the prior contiguous stack model with Step `2a` reserve/fixup mechanics.
### Blockers
- none
### Next Steps
- Proceed from Step `2a` baseline to Step `2b` (HeapManager container modernization) or revisit `2f` later with a tighter complexity/perf budget.
---
## Session: 2026-04-08T14:10:00-05:00
### Context
- Phase/Step: Phase 2 / Step 2c (Cache capacity and contention optimization)
- Goal: Add optional bounded cache mode, add thread-local resolver front-caches (`method_info`, `vmt`, `hierarchy`), add clone/footprint instrumentation, benchmark dispatch+generics with/without front-cache, and validate correctness.
### Actions Taken
- Read required context before editing:
  - `AGENTS.md`
  - `docs/ARCHITECTURE.md`
  - `docs/TYPE_RESOLUTION_AND_CACHING.md`
  - `docs/THREADING_AND_SYNCHRONIZATION.md`
  - `docs/GC_AND_MEMORY_SAFETY.md`
  - `CHECKLIST.md`, `REVIEW.md`
  - Step-relevant source files listed in prompt.
- Implemented bounded cache policy and front-cache policy in `crates/dotnet-vm/src/state.rs`:
  - added env-driven cache policy:
    - `DOTNET_CACHE_LIMIT_METHOD_INFO`
    - `DOTNET_CACHE_LIMIT_VMT`
    - `DOTNET_CACHE_LIMIT_HIERARCHY`
    - `DOTNET_FRONT_CACHE_ENABLED`
    - `DOTNET_FRONT_CACHE_CAPACITY`
  - added bounded insertion helper with opportunistic eviction,
  - added thread-local `method_info` front-cache,
  - updated `GlobalCaches::get_method_info` to front-cache -> global-cache fallback,
  - added estimated cache byte accounting for all exported cache sizes,
  - updated runtime snapshot path to pass full `CacheSizes` into metrics snapshot.
- Added thread-local resolver front-cache plumbing in `crates/dotnet-vm/src/resolver/mod.rs`:
  - front-caches for `vmt` and `hierarchy`,
  - front-cache hit/miss metric recording,
  - clone-counter recording on clone-required paths,
  - bounded insertion routing via `GlobalCaches::set_vmt` and `GlobalCaches::set_hierarchy`.
- Refactored resolver cache adapter interfaces in `crates/dotnet-runtime-resolver/src/lib.rs`:
  - changed VMT/hierarchy cache methods to key-part signatures to avoid mandatory tuple construction at call sites,
  - added optional clone-counter hooks (`record_vmt_key_clones`, `record_hierarchy_key_clones`).
- Updated resolver call sites:
  - `crates/dotnet-runtime-resolver/src/methods.rs`
  - `crates/dotnet-runtime-resolver/src/types.rs`
- Extended instrumentation and snapshot payload in `crates/dotnet-metrics/src/lib.rs`:
  - added cache bytes to `CacheSizes`,
  - added bench fields for:
    - `cache_key_clone_total`
    - `cache_key_clones_by_cache`
    - `front_cache_hits_by_cache`
    - `front_cache_misses_by_cache`
    - `cache_memory_bytes_total`
    - `cache_memory_bytes_by_cache`
  - added no-op/non-feature-safe recording methods for new counters.
- Updated `crates/dotnet-tracer/src/lib.rs` `CacheSizes` dummy literal for new fields.
- Applied mitigation after benchmark results:
  - changed front-cache default to opt-in (`DOTNET_FRONT_CACHE_ENABLED` default `false`) to avoid always-on dispatch regression.
- Updated planning/status docs:
  - `CHECKLIST.md`: marked Step `2c` completed and all atomic tasks checked,
  - `REVIEW.md`: replaced Step `2c` proposed section with implementation + measured results + mitigation.
- Ran verification:
  - `cargo fmt`
  - `cargo clippy --all-targets -- -D warnings`
  - `cargo test -- --nocapture`
  - `cargo clippy --all-targets --no-default-features -- -D warnings`
  - `cargo test --no-default-features -- --nocapture --test-threads=1`
- Ran Step `2c` benchmarks:
  - non-instrumented:
    - `cargo bench -p dotnet-benchmarks --bench end_to_end -- dispatch --sample-size 10 --measurement-time 5 --warm-up-time 1`
    - `cargo bench -p dotnet-benchmarks --bench end_to_end -- generics --sample-size 10 --measurement-time 5 --warm-up-time 1`
    - `DOTNET_FRONT_CACHE_ENABLED=0 cargo bench -p dotnet-benchmarks --bench end_to_end -- dispatch --sample-size 10 --measurement-time 5 --warm-up-time 1`
    - `DOTNET_FRONT_CACHE_ENABLED=0 cargo bench -p dotnet-benchmarks --bench end_to_end -- generics --sample-size 10 --measurement-time 5 --warm-up-time 1`
  - instrumented:
    - `cargo bench -p dotnet-benchmarks --bench end_to_end --features bench-instrumentation -- dispatch --sample-size 10 --measurement-time 5 --warm-up-time 1`
    - `cargo bench -p dotnet-benchmarks --bench end_to_end --features bench-instrumentation -- generics --sample-size 10 --measurement-time 5 --warm-up-time 1`
    - `DOTNET_FRONT_CACHE_ENABLED=0 cargo bench -p dotnet-benchmarks --bench end_to_end --features bench-instrumentation -- dispatch --sample-size 10 --measurement-time 5 --warm-up-time 1`
    - `DOTNET_FRONT_CACHE_ENABLED=0 cargo bench -p dotnet-benchmarks --bench end_to_end --features bench-instrumentation -- generics --sample-size 10 --measurement-time 5 --warm-up-time 1`
- Preserved instrumentation snapshots for both modes under `/tmp/step2c_metrics/`.
### Findings
- Non-instrumented Criterion medians:
  - front-cache on (`DOTNET_FRONT_CACHE_ENABLED=1`):
    - `dispatch`: `759.91 ms`
    - `generics`: `18.908 s`
  - front-cache off (`DOTNET_FRONT_CACHE_ENABLED=0`):
    - `dispatch`: `736.79 ms`
    - `generics`: `19.589 s`
  - on vs off delta:
    - `dispatch`: `+3.14%` (regression)
    - `generics`: `-3.48%` (improvement)
- Instrumented paired medians:
  - front-cache on:
    - `dispatch`: `1.1522 s`
    - `generics`: `24.044 s`
  - front-cache off:
    - `dispatch`: `1.1542 s`
    - `generics`: `24.670 s`
  - on vs off delta:
    - `dispatch`: `-0.17%` (neutral)
    - `generics`: `-2.54%` (improvement)
- New instrumentation counters (snapshot highlights):
  - `dispatch`: key clones `64` (on) vs `1,000,040` (off); front hits `method_info=200,004`, `vmt=199,994`.
  - `generics`: key clones `78` (on) vs `22,760,030` (off); front hits `hierarchy=5,040,000`, `method_info=2,559,997`, `vmt=2,519,996`.
  - estimated cache memory bytes decreased slightly with front-cache-enabled runs in measured snapshots.
- Phase 0 baseline comparisons (`crates/dotnet-benchmarks/baselines/phase0/baseline.json`, default medians):
  - baseline `dispatch`: `794.17 ms`
    - front-cache on run: `759.91 ms` (`-4.31%`)
    - front-cache off run: `736.79 ms` (`-7.22%`)
  - baseline `generics`: `18.75446 s`
    - front-cache on run: `18.908 s` (`+0.82%`)
    - front-cache off run: `19.589 s` (`+4.45%`)
- Correctness remained stable via full default/no-default clippy+test matrices.
### Blockers
- none
### Next Steps
- Proceed to Step `2b` (HeapManager container modernization).
- Revisit front-cache policy per workload (for example per-cache toggles for `vmt`/`method_info`/`hierarchy`) if later multithreaded contention benchmarks justify enabling subsets by default.
---
## Session: 2026-04-08T15:35:00-05:00
### Context
- Phase/Step: Phase 2 / Step 2b (HeapManager container modernization)
- Goal: Modernize HeapManager containers for production performance by gating diagnostics-only tracking, replacing hot lookup structures, and reducing small-cardinality allocation overhead.
### Actions Taken
- Read required context before editing:
  - `AGENTS.md`
  - `docs/ARCHITECTURE.md`
  - `docs/GC_AND_MEMORY_SAFETY.md`
  - `docs/THREADING_AND_SYNCHRONIZATION.md`
  - step files and targets in `CHECKLIST.md`, `REVIEW.md`, and relevant source files.
- Captured pre-change benchmark snapshots:
  - `cargo bench -p dotnet-benchmarks --bench end_to_end -- gc --sample-size 10 --measurement-time 5 --warm-up-time 1`
  - `cargo bench -p dotnet-benchmarks --bench end_to_end --features bench-instrumentation -- gc --sample-size 10 --measurement-time 5 --warm-up-time 1`
- Implemented HeapManager modernization in `crates/dotnet-runtime-memory/src/heap.rs`:
  - added `HeapManager::new()` and `Default` impl,
  - introduced `heap-diagnostics` feature-gated `_all_objs` storage,
  - added production object registry (`Slab` + indexed buckets) for `find_object()`,
  - changed `finalization_queue`/`pending_finalization` to `SmallVec<[ObjectRef; 8]>`,
  - changed `pinned_objects` to `SmallVec<[ObjectRef; 8]>` with deduped `pin_object`/`unpin_object`,
  - added helper APIs: `register_object`, `snapshot_objects`, `live_object_count`.
- Updated VM call sites:
  - `crates/dotnet-vm/src/state.rs`: switched to `HeapManager::new()`.
  - `crates/dotnet-vm/src/stack/context.rs`: switched object registration and pin/unpin to HeapManager methods.
  - `crates/dotnet-vm/src/stack/mod.rs`: switched tracer heap/object-count reads to HeapManager methods.
  - `crates/dotnet-vm/src/intrinsics/gc.rs`: switched pinned GCHandle updates to HeapManager methods.
- Updated feature/dependency wiring:
  - `crates/dotnet-runtime-memory/Cargo.toml`: added `heap-diagnostics` feature and `smallvec`/`slab` deps.
  - `Cargo.toml` (workspace deps): added `smallvec`, `slab`.
  - `crates/dotnet-vm/Cargo.toml`: added `heap-diagnostics` forwarding.
  - `crates/dotnet-cli/Cargo.toml` and `crates/dotnet-benchmarks/Cargo.toml`: added `heap-diagnostics` forwarding.
- Updated planning/status docs:
  - `CHECKLIST.md`: marked Step `2b` completed with all atomic tasks checked.
  - `REVIEW.md`: replaced Step `2b` proposal section with implementation details and measured results.
- Ran verification:
  - `cargo fmt`
  - `cargo clippy --all-targets -- -D warnings`
  - `cargo test -- --nocapture`
  - `cargo clippy --all-targets --no-default-features -- -D warnings`
  - `cargo test --no-default-features -- --nocapture --test-threads=1`
- Captured post-change benchmark snapshots:
  - `cargo bench -p dotnet-benchmarks --bench end_to_end -- gc --sample-size 10 --measurement-time 5 --warm-up-time 1`
  - `cargo bench -p dotnet-benchmarks --bench end_to_end --features bench-instrumentation -- gc --sample-size 10 --measurement-time 5 --warm-up-time 1`
### Findings
- Criterion medians (`gc`) before vs after Step `2b`:
  - non-instrumented: `511.39 ms` -> `435.33 ms` (`-14.87%`)
  - instrumented: `721.84 ms` -> `629.71 ms` (`-12.76%`)
- Phase 0 baseline comparison:
  - baseline default `gc`: `433.05 ms`
    - post-step default run: `435.33 ms` (`+0.53%`)
  - baseline instrumented `gc`: `608.92 ms`
    - post-step instrumented run: `629.71 ms` (`+3.41%`)
- GC pause metrics (instrumented snapshot at `target/release/dotnet-bench-metrics/gc.json`):
  - post-step: `gc_pause_total_us=4941`, `gc_pause_count=169` (avg `29.24 us`)
  - Phase 0 instrumented baseline snapshot: `gc_pause_total_us=3647`, `gc_pause_count=253` (avg `14.42 us`)
- Correctness remained stable across default and no-default clippy/test matrices, including GC fixture coverage under `dotnet-cli` integration tests.
### Blockers
- none
### Next Steps
- Proceed to Step `2d` or `2e` per dependency plan; keep `heap-diagnostics` disabled by default for performance runs.
- If detailed heap snapshot tracing is needed during future debugging, enable `heap-diagnostics` explicitly and rerun targeted workloads.
---
## Session: 2026-04-08T17:43:08-05:00
### Context
- Phase/Step: Phase 2 / Step 2d (Instruction dispatch tuning)
- Goal: Add alternate jump-table instruction dispatch generation, add configurable safe-point polling interval, prototype a super-instruction pair behind a feature flag, benchmark arithmetic+virtual dispatch variants, and verify opcode parity.
### Actions Taken
- Read required context before editing:
  - `AGENTS.md`
  - `docs/ARCHITECTURE.md`
  - `docs/BUILD_TIME_CODE_GENERATION.md`
  - `docs/DELEGATES_AND_DISPATCH.md`
  - Step targets in `CHECKLIST.md` and `REVIEW.md`
  - Relevant files: `crates/dotnet-vm/build.rs`, `crates/dotnet-vm/src/dispatch/mod.rs`, `crates/dotnet-vm/src/dispatch/registry.rs`
- Implemented dispatch generation and selection:
  - `crates/dotnet-vm/build.rs`
    - added build-time duplicate-opcode registration guard,
    - retained monomorphic `dispatch_monomorphic` generation,
    - added generated opcode-indexed function-pointer dispatcher `dispatch_jump_table`,
    - generated per-opcode wrapper functions and shared unimplemented fallback.
  - `crates/dotnet-vm/src/dispatch/mod.rs`
    - `InstructionRegistry::dispatch` now selects generated backend by feature:
      - default: monomorphic match dispatcher,
      - `instruction-dispatch-jump-table`: jump-table dispatcher.
- Implemented safe-point polling configuration:
  - `crates/dotnet-vm/src/dispatch/mod.rs`
    - added `DOTNET_SAFE_POINT_POLL_INTERVAL` (`32|64|128`) via `OnceLock`,
    - invalid values warn and fall back to default `32`,
    - batch loop now uses configured interval while retaining batch size `128`.
- Implemented super-instruction prototype:
  - `crates/dotnet-vm/src/dispatch/mod.rs`
    - added `dispatch-super-instruction-prototype` fast path in batch execution,
    - fused pair: `LoadConstantInt32(1)` + `Add`,
    - kept per-instruction ring-buffer/tracing/opcode-metric accounting,
    - added fallback to normal stepping when pair preconditions are not met.
- Updated feature wiring:
  - `crates/dotnet-vm/Cargo.toml`
    - added `instruction-dispatch-jump-table`
    - added `dispatch-super-instruction-prototype`
  - `crates/dotnet-benchmarks/Cargo.toml`
    - added forwarding features for both flags.
- Updated generated-dispatch tests:
  - `crates/dotnet-vm/src/dispatch/registry.rs`
    - added assertions that jump-table dispatch artifacts are generated.
- Updated planning/status docs:
  - `CHECKLIST.md`: marked Step `2d` completed with all atomic tasks checked.
  - `REVIEW.md`: replaced Step `2d` proposal section with implementation, verification, measurements, and mitigation.
- Ran verification:
  - `cargo fmt`
  - `cargo clippy --all-targets -- -D warnings`
  - `cargo test -- --nocapture`
  - `cargo clippy --all-targets --no-default-features -- -D warnings`
  - `cargo test --no-default-features -- --nocapture --test-threads=1`
  - `cargo test -p dotnet-vm --features instruction-dispatch-jump-table -- --nocapture`
  - `cargo test -p dotnet-vm --features "instruction-dispatch-jump-table dispatch-super-instruction-prototype" -- --nocapture`
- Ran Step `2d` benchmark matrix (non-instrumented):
  - Match dispatch:
    - `DOTNET_SAFE_POINT_POLL_INTERVAL=32 cargo bench -p dotnet-benchmarks --bench end_to_end -- arithmetic --sample-size 10 --measurement-time 5 --warm-up-time 1`
    - `DOTNET_SAFE_POINT_POLL_INTERVAL=32 cargo bench -p dotnet-benchmarks --bench end_to_end -- dispatch --sample-size 10 --measurement-time 5 --warm-up-time 1`
    - `DOTNET_SAFE_POINT_POLL_INTERVAL=64 cargo bench -p dotnet-benchmarks --bench end_to_end -- arithmetic --sample-size 10 --measurement-time 5 --warm-up-time 1`
    - `DOTNET_SAFE_POINT_POLL_INTERVAL=64 cargo bench -p dotnet-benchmarks --bench end_to_end -- dispatch --sample-size 10 --measurement-time 5 --warm-up-time 1`
    - `DOTNET_SAFE_POINT_POLL_INTERVAL=128 cargo bench -p dotnet-benchmarks --bench end_to_end -- arithmetic --sample-size 10 --measurement-time 5 --warm-up-time 1`
    - `DOTNET_SAFE_POINT_POLL_INTERVAL=128 cargo bench -p dotnet-benchmarks --bench end_to_end -- dispatch --sample-size 10 --measurement-time 5 --warm-up-time 1`
  - Jump-table dispatch:
    - `DOTNET_SAFE_POINT_POLL_INTERVAL=32 cargo bench -p dotnet-benchmarks --features instruction-dispatch-jump-table --bench end_to_end -- arithmetic --sample-size 10 --measurement-time 5 --warm-up-time 1`
    - `DOTNET_SAFE_POINT_POLL_INTERVAL=32 cargo bench -p dotnet-benchmarks --features instruction-dispatch-jump-table --bench end_to_end -- dispatch --sample-size 10 --measurement-time 5 --warm-up-time 1`
    - `DOTNET_SAFE_POINT_POLL_INTERVAL=64 cargo bench -p dotnet-benchmarks --features instruction-dispatch-jump-table --bench end_to_end -- arithmetic --sample-size 10 --measurement-time 5 --warm-up-time 1`
    - `DOTNET_SAFE_POINT_POLL_INTERVAL=64 cargo bench -p dotnet-benchmarks --features instruction-dispatch-jump-table --bench end_to_end -- dispatch --sample-size 10 --measurement-time 5 --warm-up-time 1`
    - `DOTNET_SAFE_POINT_POLL_INTERVAL=128 cargo bench -p dotnet-benchmarks --features instruction-dispatch-jump-table --bench end_to_end -- arithmetic --sample-size 10 --measurement-time 5 --warm-up-time 1`
    - `DOTNET_SAFE_POINT_POLL_INTERVAL=128 cargo bench -p dotnet-benchmarks --features instruction-dispatch-jump-table --bench end_to_end -- dispatch --sample-size 10 --measurement-time 5 --warm-up-time 1`
  - Jump-table + super-instruction prototype:
    - `DOTNET_SAFE_POINT_POLL_INTERVAL=32 cargo bench -p dotnet-benchmarks --features "instruction-dispatch-jump-table dispatch-super-instruction-prototype" --bench end_to_end -- arithmetic --sample-size 10 --measurement-time 5 --warm-up-time 1`
    - `DOTNET_SAFE_POINT_POLL_INTERVAL=32 cargo bench -p dotnet-benchmarks --features "instruction-dispatch-jump-table dispatch-super-instruction-prototype" --bench end_to_end -- dispatch --sample-size 10 --measurement-time 5 --warm-up-time 1`
    - `DOTNET_SAFE_POINT_POLL_INTERVAL=64 cargo bench -p dotnet-benchmarks --features "instruction-dispatch-jump-table dispatch-super-instruction-prototype" --bench end_to_end -- arithmetic --sample-size 10 --measurement-time 5 --warm-up-time 1`
    - `DOTNET_SAFE_POINT_POLL_INTERVAL=64 cargo bench -p dotnet-benchmarks --features "instruction-dispatch-jump-table dispatch-super-instruction-prototype" --bench end_to_end -- dispatch --sample-size 10 --measurement-time 5 --warm-up-time 1`
### Findings
- Criterion medians (ms) by variant:
  - match, poll 32: `arithmetic=532.48`, `dispatch=767.04`
  - match, poll 64: `arithmetic=551.07`, `dispatch=768.06`
  - match, poll 128: `arithmetic=542.35`, `dispatch=759.97`
  - jump-table, poll 32: `arithmetic=553.00`, `dispatch=791.16`
  - jump-table, poll 64: `arithmetic=549.86`, `dispatch=770.73`
  - jump-table, poll 128: `arithmetic=557.45`, `dispatch=769.13`
  - jump-table+super, poll 32: `arithmetic=568.05`, `dispatch=787.32`
  - jump-table+super, poll 64: `arithmetic=579.16`, `dispatch=786.49`
- Deltas vs current default config (match, poll 32):
  - match poll 64: arithmetic `+3.49%`, dispatch `+0.13%`
  - match poll 128: arithmetic `+1.85%`, dispatch `-0.92%`
  - jump-table poll 32: arithmetic `+3.85%`, dispatch `+3.14%`
  - jump-table+super poll 32: arithmetic `+6.68%`, dispatch `+2.64%`
- Baseline (`phase0/results.default`) context:
  - baseline arithmetic median `585.0569755 ms` vs best observed here `532.48 ms` (`-8.99%`)
  - baseline dispatch median `794.1701455 ms` vs best observed here `759.97 ms` (`-4.31%`)
- Functional parity:
  - dispatch-focused and opcode behavior tests passed under default, jump-table, and jump-table+super feature configurations.
### Blockers
- none
### Next Steps
- Keep jump-table and super-instruction prototype features disabled by default due regressions in this benchmark environment.
- Proceed to Step `2e` (Intrinsic dispatch fast path), then revisit jump-table/super variants after 2e/4a changes to reassess interaction with call-heavy workloads.
---
## Session: 2026-04-08T18:06:09-05:00
### Context
- Phase/Step: Phase 2 / Step 2e (Intrinsic dispatch fast path)
- Goal: Cache intrinsic classification/lookup work on resolved methods, unify duplicate intrinsic dispatch checks into a canonical path, and measure impact on `json` + `generics` workloads.
- Outcome: Implementation was benchmarked, showed significant regression in local Criterion comparisons, then was manually reverted and Step `2e` was marked manually reverted in `CHECKLIST.md`.
### Actions Taken
- Read required context before editing:
  - `AGENTS.md`
  - `docs/ARCHITECTURE.md`
  - `docs/TYPE_RESOLUTION_AND_CACHING.md`
  - `docs/BUILD_TIME_CODE_GENERATION.md`
  - `docs/DELEGATES_AND_DISPATCH.md`
  - Step targets in `CHECKLIST.md` and `REVIEW.md`
  - Relevant files listed for Step `2e`
- Implemented Step `2e` candidate changes (later reverted):
  - added per-method intrinsic classification cache in `IntrinsicRegistry` to avoid repeated PHF key normalization + lookup per `MethodDescription`,
  - routed `ExecutionEngine::dispatch_method` through canonical `VesContext::dispatch_method` path to remove duplicate engine-side intrinsic dispatch logic,
  - updated resolver-side intrinsic compute path to use cached metadata lookup.
- Ran verification before revert:
  - `cargo fmt`
  - `cargo clippy --all-targets -- -D warnings`
  - `cargo test -- --nocapture`
- Ran Step `2e` benchmark workloads:
  - `cargo bench -p dotnet-benchmarks --bench end_to_end -- json --sample-size 10 --measurement-time 5 --warm-up-time 1`
  - `cargo bench -p dotnet-benchmarks --bench end_to_end -- generics --sample-size 10 --measurement-time 5 --warm-up-time 1`
  - `cargo bench -p dotnet-benchmarks --features bench-instrumentation --bench end_to_end -- json --sample-size 10 --measurement-time 5 --warm-up-time 1`
  - `cargo bench -p dotnet-benchmarks --features bench-instrumentation --bench end_to_end -- generics --sample-size 10 --measurement-time 5 --warm-up-time 1`
- Captured instrumentation snapshots at:
  - `target/release/dotnet-bench-metrics/json.json`
  - `target/release/dotnet-bench-metrics/generics.json`
- Manual rollback:
  - reverted the Step `2e` code changes,
  - marked Step `2e` as manually reverted in `CHECKLIST.md`.
### Findings
- Measured medians during the candidate run:
  - default `json`: `290.36 ms`
  - default `generics`: `17.705 s`
  - instrumented `json`: `382.54 ms`
  - instrumented `generics`: `21.530 s`
- Criterion local change output during candidate run reported regressions for instrumented workloads:
  - instrumented `json`: `+31.75%` (regression)
  - instrumented `generics`: `+21.60%` (regression)
- Instrumentation snapshots still reported `bench.intrinsic_call_total = 0` for `json` and `generics`; intrinsic cache counters remained low-cardinality (`intrinsic` cache size 0-3), indicating this candidate did not materially improve measured intrinsic-call hot counters in these workloads.
### Blockers
- none
### Next Steps
- Keep Step `2e` in reverted state.
- Re-scope Step `2e` around lower-risk deltas only (for example: dispatch-path deduplication without registry behavior change, or targeted microbench focused on intrinsic-heavy fixtures before broad workload runs).
---
## Session: 2026-04-08T18:22:58-05:00
### Context
- Phase/Step: Recovery planning after reverted Steps `2e` and `2f`
- Goal: Decide whether to re-attempt or defer `2e`/`2f`, update planning/docs to remove limbo state, and validate current workspace health.
### Actions Taken
- Read required files in full before edits:
  - `AGENTS.md`
  - `REVIEW.md`
  - `CHECKLIST.md`
  - `AGENT_MEMORY.md`
- Re-ran targeted benchmark slice (Criterion `--sample-size 10 --measurement-time 5 --warm-up-time 1`) on current reverted code:
  - `cargo bench -p dotnet-benchmarks --features bench-instrumentation --bench end_to_end -- dispatch --sample-size 10 --measurement-time 5 --warm-up-time 1`
  - `cargo bench -p dotnet-benchmarks --features "bench-instrumentation segmented-eval-stack-prototype" --bench end_to_end -- dispatch --sample-size 10 --measurement-time 5 --warm-up-time 1`
  - `cargo bench -p dotnet-benchmarks --features bench-instrumentation --bench end_to_end -- json --sample-size 10 --measurement-time 5 --warm-up-time 1`
  - `cargo bench -p dotnet-benchmarks --features bench-instrumentation --bench end_to_end -- generics --sample-size 10 --measurement-time 5 --warm-up-time 1`
- Collected instrumentation snapshots from `target/release/dotnet-bench-metrics/{dispatch,json,generics}.json`.
- Updated planning docs:
  - `CHECKLIST.md`
    - marked `2f` and `2e` as deliberately deferred with explicit rationale and reopen criteria,
    - updated `2e` task list to require intrinsic-heavy fixture baselining first,
    - updated `4a` dependency note to `2d, 3c` with `2e` optional for intrinsic-specific tuning.
  - `REVIEW.md`
    - replaced `2f` and `2e` proposed-only sections with defer decisions, rollback rationale, and current measurements,
    - updated dependency ordering and recommended sequence to move directly to Phase 3,
    - updated risk summary to mark `2e`/`2f` as deferred.
- Ran required verification matrix after edits:
  - `cargo clippy --all-targets -- -D warnings && cargo test -- --nocapture`
  - `cargo clippy --all-targets --no-default-features -- -D warnings`
  - `cargo test --no-default-features -- --nocapture --test-threads=1`
### Findings
- Step `2f` (segmented stack) remains low ROI relative to risk:
  - contiguous instrumented `dispatch`: `eval_stack_reallocations=2`, `eval_stack_pointer_fixup_total_ns=490`, median point estimate `1.1364 s`.
  - segmented prototype instrumented `dispatch`: `eval_stack_reallocations=1`, `eval_stack_pointer_fixup_total_ns=300`, median point estimate `1.1330 s`.
  - runtime delta (prototype vs contiguous): about `-0.30%` (Criterion: no significant change).
  - conclusion: remaining pointer-fixup time is negligible at workload scale; architectural segmentation is not justified now.
- Step `2e` (intrinsic fast path) lacks benchmark signal:
  - current instrumented `json` and `generics` still report `intrinsic_call_total=0`.
  - prior attempted implementation had already regressed instrumented workloads (`json +31.75%`, `generics +21.60%`) and was reverted.
  - conclusion: defer optimization work until intrinsic-heavy fixtures exist with non-zero intrinsic counters.
- Phase-0 baseline context for current instrumented reruns:
  - `dispatch`: `-5.36%` vs phase0 instrumented baseline median.
  - `json`: `-4.88%` vs phase0 instrumented baseline median.
  - `generics`: `-2.55%` vs phase0 instrumented baseline median.
- Verification status:
  - default and `--no-default-features` clippy/test matrices passed.
### Blockers
- none
### Next Steps
- Proceed to Phase 3 in sequence: `3a -> 3b -> 3c`.
- Keep `2e` and `2f` parked unless reopen criteria are met.
- Optional side track: add intrinsic-heavy benchmark fixture(s), baseline intrinsic counters, then re-test a lightweight `2e` candidate.
---
## Session: 2026-04-08T20:10:00-05:00
### Context
- Phase/Step: Phase 3 / Step 3a (Trace-cost and cross-arena profiling)
- Goal: Add bench-instrumented profiling for GC fixed-point behavior, trace-root timing, and high-cost layout traversal; benchmark allocation-pressure and capture STW quantiles.
### Actions Taken
- Read required context before edits:
  - `AGENTS.md`
  - `docs/ARCHITECTURE.md`
  - `docs/GC_AND_MEMORY_SAFETY.md`
  - `docs/THREADING_AND_SYNCHRONIZATION.md`
  - `CHECKLIST.md`
  - `REVIEW.md`
  - Relevant files listed for Step `3a`
- Implemented Step `3a` instrumentation changes:
  - `crates/dotnet-metrics/src/lib.rs`
    - extended `BenchInstrumentationSnapshot` with GC pause quantiles, fixed-point/cross-arena counters, trace-root timing maps, and layout-scan timing maps,
    - added `RuntimeMetrics` counters/maps and record methods for fixed-point iterations/cycles, trace-root timing, and layout-scan timing,
    - added active-runtime helper functions for new counters.
  - `crates/dotnet-vm/src/gc/coordinator.rs`
    - recorded fixed-point iteration number and per-iteration cross-arena object volume,
    - recorded per-cycle fixed-point iteration count.
  - `crates/dotnet-vm/src/threading/basic.rs`
    - timed major mark-phase root operations and recorded them via active metrics helpers.
  - `crates/dotnet-runtime-memory/src/access.rs`
    - timed high-cost layout traversal entry paths (`record_refs_recursive_with_recorder`, `record_refs_in_range_with_recorder`).
  - Feature propagation:
    - `crates/dotnet-runtime-memory/Cargo.toml`: added optional `dotnet-metrics` dependency and `bench-instrumentation` feature,
    - `crates/dotnet-vm/Cargo.toml`: forwarded `bench-instrumentation` to `dotnet-runtime-memory`.
- Updated planning/docs:
  - `CHECKLIST.md`: marked Step `3a` completed and all atomic tasks checked.
  - `REVIEW.md`: replaced Step `3a` proposed section with implemented measurements and verification results.
- Ran verification matrix:
  - `cargo fmt`
  - `cargo clippy --all-targets -- -D warnings`
  - `cargo test -- --nocapture`
  - `cargo clippy --all-targets --no-default-features -- -D warnings`
  - `cargo test --no-default-features -- --nocapture --test-threads=1`
- Ran benchmark workload:
  - `cargo bench -p dotnet-benchmarks --features bench-instrumentation --bench end_to_end -- gc --sample-size 10 --measurement-time 5 --warm-up-time 1`
- Captured instrumentation snapshot:
  - `target/release/dotnet-bench-metrics/gc.json`
### Findings
- Allocation-pressure benchmark (`gc`) post-change:
  - Criterion estimate: `598.04 ms` (CI `[595.89, 600.83]`).
- Phase-0 instrumented baseline median for `gc` (`crates/dotnet-benchmarks/baselines/phase0/baseline.json`, `results.instrumented_default.gc.median_ns`): `608.923 ms`.
- Delta vs phase-0 instrumented baseline median: `-1.79%` (faster).
- New STW quantiles (from `target/release/dotnet-bench-metrics/gc.json`):
  - `gc_pause_p50_us=28`, `gc_pause_p95_us=32`, `gc_pause_p99_us=34`, `gc_pause_max_us=34`.
- Fixed-point/cross-arena profile:
  - `gc_fixed_point_cycle_count=169`
  - `gc_fixed_point_iteration_total=169`
  - `gc_fixed_point_max_iterations_per_cycle=1`
  - `gc_fixed_point_cross_arena_objects_total=1012`
  - `gc_fixed_point_cross_arena_objects_max_per_iteration=8`
  - `gc_fixed_point_cross_arena_objects_by_iteration={"1":1012}`
- Trace root timing totals (ns):
  - dominant: `mark_all.finish_marking=568331`
  - next: `mark_all.harvest_cross_arena_refs=71149`
- Layout traversal timing maps were empty for this fixture (`layout_scan_count_by_path={}`, `layout_scan_total_ns_by_path={}`).
### Blockers
- none
### Next Steps
- Proceed to Step `3b` using Step `3a` metrics as guardrails:
  - prioritize changes that reduce allocation pressure without increasing `gc_pause_p95/p99`,
  - use trace-root and fixed-point counters to validate no regressions in cross-arena GC behavior.
---
## Session: 2026-04-08T18:52:05-05:00
### Context
- Phase/Step: Phase 3 / Step 3b (Allocation-rate reduction in runtime paths)
- Goal: Reduce allocation churn in call/local-init hot paths, add an optional string interning experiment, and validate JSON/generics benchmark impact against phase-0 baseline.
### Actions Taken
- Read required context before edits:
  - `AGENTS.md`
  - `docs/ARCHITECTURE.md`
  - `docs/GC_AND_MEMORY_SAFETY.md`
  - `docs/THREADING_AND_SYNCHRONIZATION.md`
  - `CHECKLIST.md`
  - `REVIEW.md`
  - Full contents of step files:
    - `crates/dotnet-vm/src/stack/context.rs`
    - `crates/dotnet-vm/src/stack/call_ops_impl.rs`
    - `crates/dotnet-vm-data/src/stack.rs`
    - `crates/dotnet-value/src/string.rs`
- Implemented Step `3b` runtime changes:
  - `crates/dotnet-vm/src/stack/context.rs`
    - rewrote `init_locals` to write local defaults directly to eval-stack slots,
    - added reusable call-argument buffer helpers (`pop_call_args_into_buffer`, `copy_slots_into_call_args_buffer`),
    - extended `VesContext` to carry a mutable reference to thread-local `call_args_buffer`.
  - `crates/dotnet-vm/src/stack/call_ops_impl.rs`
    - switched constructor/tail/jmp argument staging to thread-local reusable buffer,
    - updated frame setup paths to use direct local-slot initialization output.
  - `crates/dotnet-vm-data/src/stack.rs`
    - added reusable helpers (`pop_multiple_into`, `copy_slots_into`) for allocation-free slot staging in callers,
    - kept existing `pop_multiple` semantics by delegating to reusable helper.
  - `crates/dotnet-vm/src/stack/mod.rs`
    - added `ThreadContext.call_args_buffer` and wired it into `VesContext` construction.
  - `crates/dotnet-value/src/string.rs`
    - added optional env-gated interning experiment with bounded cache:
      - `DOTNET_STRING_INTERN_EXPERIMENT` enables,
      - `DOTNET_STRING_INTERN_MAX_ENTRIES` caps entries (default `4096`),
    - kept default non-interned storage path (`Owned(Vec<u16>)`) to avoid default-path overhead.
- Updated planning/docs:
  - `CHECKLIST.md`: marked Step `3b` complete and checked all atomic tasks.
  - `REVIEW.md`: replaced Step `3b` proposal text with implemented changes, benchmarks, counter deltas, and verification.
- Ran verification matrix:
  - `cargo fmt`
  - `cargo clippy --all-targets -- -D warnings`
  - `cargo test -- --nocapture`
  - `cargo clippy --all-targets --no-default-features -- -D warnings`
  - `cargo test --no-default-features -- --nocapture --test-threads=1`
- Ran Step `3b` benchmark workloads:
  - `cargo bench -p dotnet-benchmarks --features bench-instrumentation --bench end_to_end -- json --sample-size 10 --measurement-time 5 --warm-up-time 1`
  - `cargo bench -p dotnet-benchmarks --features bench-instrumentation --bench end_to_end -- generics --sample-size 10 --measurement-time 5 --warm-up-time 1`
- Captured instrumentation snapshots:
  - `target/release/dotnet-bench-metrics/json.json`
  - `target/release/dotnet-bench-metrics/generics.json`
### Findings
- Benchmark medians (Criterion point estimates):
  - `json`: `399.7157975 ms`
  - `generics`: `22.673594735 s`
- Phase-0 instrumented baseline medians (`crates/dotnet-benchmarks/baselines/phase0/baseline.json`):
  - `json`: `412.761822 ms`
  - `generics`: `23.2292185985 s`
- Delta vs phase-0 instrumented baseline medians:
  - `json`: `-3.16%`
  - `generics`: `-2.39%`
- Allocation/counter deltas vs phase-0 instrumented baseline:
  - `json`: `eval_stack_reallocations 4 -> 3` (`-25%`), `eval_stack_pointer_fixup_total_ns 601 -> 672` (`+11.8%`)
  - `generics`: `eval_stack_reallocations 4 -> 3` (`-25%`), `eval_stack_pointer_fixup_total_ns 841 -> 1213` (`+44.2%`)
- Intrinsic signal remains unchanged on these workloads:
  - `intrinsic_call_total=0` for both `json` and `generics`.
- Step `3a` guardrail metrics remained stable in these runs:
  - `json`: `gc_pause_p95_us=6`, `gc_pause_p99_us=7`
  - `generics`: `gc_pause_p95_us=7`, `gc_pause_p99_us=9`
### Blockers
- none
### Next Steps
- Proceed to Step `3c` (stack-frame locality/pooling), using current `json`/`generics` numbers as new guardrail references for frame-allocation changes.
- If string-heavy duplication workloads are added, run A/B with `DOTNET_STRING_INTERN_EXPERIMENT=1` to evaluate interning ROI separately from default-path performance.
## Session: 2026-04-08T22:35:00-05:00
### Context
- Phase/Step: Phase 3 / Step 3c (StackFrame pooling and locality)
- Goal: Reduce call-path allocator churn by improving `StackFrame` locality and introducing safe per-thread frame reuse.
### Actions Taken
- Read required context before edits:
  - `AGENTS.md`
  - `docs/ARCHITECTURE.md`
  - `docs/THREADING_AND_SYNCHRONIZATION.md`
  - `CHECKLIST.md`
  - `REVIEW.md`
  - Relevant stack files (`crates/dotnet-vm-data/src/stack.rs`, `crates/dotnet-vm/src/stack/mod.rs`, `crates/dotnet-vm/src/stack/call_ops_impl.rs`) plus `crates/dotnet-vm/src/stack/context.rs` for return/unwind lifecycle wiring.
- Implemented Step `3c` changes:
  - `crates/dotnet-vm-data/src/stack.rs`
    - replaced `StackFrame.exception_stack` and `StackFrame.pinned_locals` with `SmallVec`-backed types,
    - added `PinnedLocals`/`ExceptionStack` aliases and inline-capacity constants,
    - added frame reset/cleanup helpers (`reset_for_call`, `prepare_for_pool`),
    - added bounded per-thread frame pool in `FrameStack` (`pooled_frames`, `push_frame`, `recycle_frame`),
    - ensured pooled frames clear GC-bearing state before reuse.
  - `crates/dotnet-vm/src/stack/call_ops_impl.rs`
    - switched frame construction call sites to `frame_stack.push_frame(...)`,
    - recycled popped frames in tail-call and `jmp` replacement paths.
  - `crates/dotnet-vm/src/stack/context.rs`
    - switched local-pin tracking builder to `PinnedLocals` (`SmallVec`),
    - recycled frames in `return_frame` and `unwind_frame` after cleanup logic.
  - Re-export and dependency plumbing:
    - `crates/dotnet-vm-data/Cargo.toml`: added `smallvec` dependency,
    - `crates/dotnet-vm-data/src/lib.rs` and `crates/dotnet-vm-ops/src/lib.rs`: re-exported new stack types.
  - `crates/dotnet-metrics/src/lib.rs`
    - added bench counters: `frame_pool_hit_count`, `frame_pool_miss_count`, `frame_pool_recycle_count`,
    - added active-runtime helpers used from frame-pool paths.
- Updated planning/docs:
  - `CHECKLIST.md`: marked Step `3c` completed and checked all atomic tasks.
  - `REVIEW.md`: replaced Step `3c` proposal text with implemented details, metrics, and verification.
- Ran verification matrix:
  - `cargo fmt`
  - `cargo clippy --all-targets -- -D warnings`
  - `cargo test -- --nocapture`
  - `cargo clippy --all-targets --no-default-features -- -D warnings`
  - `cargo test --no-default-features -- --nocapture --test-threads=1`
- Ran Step `3c` benchmark workloads:
  - `cargo bench -p dotnet-benchmarks --features bench-instrumentation --bench end_to_end -- dispatch --sample-size 10 --measurement-time 5 --warm-up-time 1`
  - `cargo bench -p dotnet-benchmarks --features bench-instrumentation --bench end_to_end -- generics --sample-size 10 --measurement-time 5 --warm-up-time 1`
- Captured instrumentation snapshots:
  - `target/release/dotnet-bench-metrics/dispatch.json`
  - `target/release/dotnet-bench-metrics/generics.json`
### Findings
- Benchmark medians (criterion point estimates):
  - `dispatch`: `1.1201492835 s`
  - `generics`: `22.432503621 s`
- Phase-0 instrumented baseline medians (`crates/dotnet-benchmarks/baselines/phase0/baseline.json`):
  - `dispatch`: `1.2007179425 s`
  - `generics`: `23.2292185985 s`
- Delta vs phase-0 instrumented baseline medians:
  - `dispatch`: `-6.71%`
  - `generics`: `-3.43%`
- Frame-pool allocator-activity counters:
  - `dispatch`: `frame_pool_hit_count=200008`, `frame_pool_miss_count=3`, `frame_pool_recycle_count=200011` (hit ratio `99.9985%`)
  - `generics`: `frame_pool_hit_count=2560008`, `frame_pool_miss_count=4`, `frame_pool_recycle_count=2560012` (hit ratio `99.9998%`)
- Related guardrails/counters:
  - `eval_stack_reallocations`: `dispatch 3 -> 2`, `generics 4 -> 3` (vs phase-0 instrumented)
  - GC pause quantiles in this run stayed stable: `dispatch p95/p99 = 4/4 us`, `generics p95/p99 = 7/9 us`.
### Blockers
- none
### Next Steps
- Proceed to Step `4a` (hot-path inlining and monomorphization audit) using new `dispatch`/`generics` medians and frame-pool counters as the post-3c baseline.
- If additional call-specific ROI is needed, add a dedicated micro fixture for direct call/ret chains and compare against current pooled-frame counters.
---

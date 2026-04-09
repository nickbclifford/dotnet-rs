## Phase 0: Baseline and Instrumentation

### Step 0a: Criterion benchmark harness
- **Status**: [x] Completed
- **Files**: Cargo.toml, crates/dotnet-benchmarks/Cargo.toml, crates/dotnet-benchmarks/src/lib.rs, crates/dotnet-benchmarks/benches/end_to_end.rs, crates/dotnet-benchmarks/fixtures/json/JsonBenchmark_0.cs, crates/dotnet-benchmarks/fixtures/arithmetic/TightLoop_0.cs, crates/dotnet-benchmarks/fixtures/gc/AllocationPressure_0.cs, crates/dotnet-benchmarks/fixtures/dispatch/VirtualDispatchStress_0.cs, crates/dotnet-benchmarks/fixtures/generics/GenericsStress_0.cs
- **Depends on**: none
- **Risk**: medium
- **Tasks**:
  - [x] Add `crates/dotnet-benchmarks` to workspace members and create a benchmark crate with `criterion`.
  - [x] Implement benchmark fixture compilation logic that outputs DLLs under `target/<profile>/dotnet-bench-fixtures`.
  - [x] Add five independent criterion benches (JSON end-to-end, arithmetic loop, allocation pressure, virtual dispatch stress, generics stress).
  - [x] Add benchmark filtering by name so `cargo bench -p dotnet-benchmarks -- <name>` runs one workload.
  - [x] Verify: run `cargo bench -p dotnet-benchmarks -- json` and confirm criterion output files are created in `target/criterion`.

### Step 0b: Bench instrumentation feature
- **Status**: [x] Completed
- **Files**: crates/dotnet-vm/Cargo.toml, crates/dotnet-vm-data/src/stack.rs, crates/dotnet-vm/src/dispatch/mod.rs, crates/dotnet-vm/src/intrinsics/mod.rs, crates/dotnet-vm/src/executor.rs, crates/dotnet-metrics/src/lib.rs, crates/dotnet-vm/src/state.rs
- **Depends on**: 0a
- **Risk**: medium
- **Tasks**:
  - [x] Add a `bench-instrumentation` feature gate in `dotnet-vm` and plumb it to metrics code.
  - [x] Add counters/timers for evaluation stack reallocations and `update_stack_pointers` duration.
  - [x] Add opcode-category dispatch counters in the dispatch loop.
  - [x] Add per-signature intrinsic call counters in intrinsic dispatch.
  - [x] Extend `RuntimeMetrics` with serialization-friendly snapshot methods for benchmark collection.
  - [x] Verify: run one benchmark with `--features bench-instrumentation` and confirm all new counters are non-zero where expected.

### Step 0c: Baseline capture and persistence
- **Status**: [x] Completed
- **Files**: REVIEW.md, crates/dotnet-benchmarks/scripts/capture_baseline.rs (or .py), target/criterion/**
- **Depends on**: 0a, 0b
- **Risk**: low
- **Tasks**:
  - [x] Run all five benchmarks on default features and collect criterion JSON artifacts.
  - [x] Run all five benchmarks on `--no-default-features` where applicable and collect artifacts.
  - [x] Record baseline medians, p95, and instrumentation counters in `REVIEW.md` under Phase 0.
  - [x] Verify: re-run one benchmark and confirm the script reports delta against stored baseline.

## Phase 1: Data Layout and Type Size

### Step 1a: StackValue footprint reduction
- **Status**: [x] Completed
- **Files**: crates/dotnet-value/src/stack_value.rs, crates/dotnet-value/src/object/types.rs, crates/dotnet-value/src/pointer/mod.rs, crates/dotnet-value/tests/layout_sizes.rs
- **Depends on**: 0c
- **Risk**: high
- **Tasks**:
  - [x] Add compile-time assertions for `size_of` and `align_of` of `StackValue`, `ManagedPtr`, and `PointerOrigin`.
  - [x] Implement an indirection strategy for rare large `StackValue` variants (`ManagedPtr` and/or `TypedRef`) and keep common scalar/object variants inline.
  - [x] Update GC tracing and serialization logic for the new representation.
  - [x] Run criterion arithmetic benchmark and JSON benchmark before/after and record deltas.
  - [x] Verify: `cargo test -p dotnet-value -- --nocapture` passes and size assertions match expected target values.

### Step 1b: ManagedPtr compaction
- **Status**: [x] Completed
- **Files**: crates/dotnet-value/src/pointer/mod.rs, crates/dotnet-value/src/pointer/origin.rs, crates/dotnet-utils/src/newtypes.rs, crates/dotnet-value/src/pointer/serde.rs
- **Depends on**: 1a
- **Risk**: high
- **Tasks**:
  - [x] Replace `pinned: bool` storage with packed flags and update accessor methods.
  - [x] Evaluate and implement a narrower managed-object offset representation where safe (retain unmanaged fallback correctness).
  - [x] Reduce `PointerOrigin` hot-path payload size by moving cold metadata out of the main enum when feasible.
  - [x] Re-run pointer-heavy fixtures and record size and runtime deltas.
  - [x] Verify: run `cargo test -p dotnet-value -- --nocapture` and ensure pointer serde tests still pass.

### Step 1c: Object header compaction
- **Status**: [x] Completed
- **Files**: crates/dotnet-value/src/object/mod.rs, crates/dotnet-value/src/validation.rs, crates/dotnet-runtime-memory/src/heap.rs
- **Depends on**: 0c
- **Risk**: medium
- **Tasks**:
  - [x] Make validation-only object header state zero-cost in non-validation release builds without removing feature functionality.
  - [x] Audit `owner_id` consumers and document required invariants for any packing/externalization.
  - [x] Add size assertions for `ObjectInner` under validation and non-validation builds.
  - [x] Run allocation-pressure benchmark before/after and record per-object memory impact.
  - [x] Verify: run default and no-default test suites and confirm no cross-arena validation regressions.

## Phase 2: Algorithm and Data Structure

### Step 2a: Evaluation stack reallocation elimination
- **Status**: [x] Completed
- **Files**: crates/dotnet-vm-data/src/lib.rs, crates/dotnet-vm-data/src/stack.rs, crates/dotnet-vm/src/lib.rs, crates/dotnet-vm/src/stack/call_ops_impl.rs
- **Depends on**: 1a, 1b
- **Risk**: high
- **Tasks**:
  - [x] Add `max_stack` metadata to `MethodInfo` from IL header.
  - [x] Reserve evaluation stack capacity from `max_stack` at frame push time.
  - [x] Implement and benchmark a segmented stack prototype behind a feature flag.
  - [x] Remove or minimize eager full-stack pointer fixup on capacity growth.
  - [x] Verify: run deep-stack fixtures and confirm managed pointer semantics remain correct.

### Step 2f: Segmented stack architecture integration
- **Status**: [~] Deliberately deferred (low ROI after measurement)
- **Files**: crates/dotnet-vm-data/src/stack.rs, crates/dotnet-vm-data/src/lib.rs, crates/dotnet-vm/src/stack/mod.rs, crates/dotnet-vm/src/stack/context.rs, crates/dotnet-vm/src/stack/ops.rs, crates/dotnet-vm/src/stack/call_ops_impl.rs, crates/dotnet-vm/src/stack/exception_ops_impl.rs
- **Depends on**: 2a
- **Risk**: high
- **Decision**: Keep contiguous `Vec` backend from Step `2a`; do not pursue true segmented backend in the current phase.
- **Rationale**:
  - Current instrumented `dispatch`: `eval_stack_reallocations=2`, `eval_stack_pointer_fixup_total_ns=490`.
  - Segmented prototype (`segmented-eval-stack-prototype`): `eval_stack_reallocations=1`, `eval_stack_pointer_fixup_total_ns=300`, runtime delta `-0.30%` vs contiguous (no meaningful throughput gain).
  - Full segmented integration previously broke suspend/restore and unwind/truncate assumptions and materially increased complexity risk.
- **Reopen criteria**:
  - New evidence of fixup-driven bottlenecks (for example, intrinsic/deep-stack workloads with non-trivial fixup time share), or
  - A low-complexity design that preserves current contiguous stack invariants at call/exception boundaries.
- **Tasks**:
  - [ ] Replace prototype reserve-only behavior with a real segmented evaluation-stack backend that guarantees stable stack-slot addresses across growth.
  - [ ] Introduce a clear stack-storage abstraction (backend trait or equivalent) so `VesContext` and stack ops do not depend on raw `Vec` internals.
  - [ ] Remove full-stack pointer fixup from normal growth path in segmented mode; keep contiguous mode as fallback behind feature gate.
  - [ ] Wire suspend/restore, truncate, `set_slot_at`, and exception unwind paths to segment-aware indexing and addressing.
  - [ ] Run dispatch/deep-stack/byref benchmarks for contiguous vs segmented backends and record tradeoffs in `REVIEW.md`.
  - [ ] Verify: run full default and no-default suites plus stack/byref fixtures for parity.

### Step 2b: HeapManager container modernization
- **Status**: [x] Completed
- **Files**: crates/dotnet-runtime-memory/src/heap.rs, crates/dotnet-vm/src/state.rs, crates/dotnet-runtime-memory/Cargo.toml
- **Depends on**: 0c
- **Risk**: medium
- **Tasks**:
  - [x] Gate `_all_objs` behind a diagnostics feature and disable it by default for release performance runs.
  - [x] Replace `BTreeMap` object lookup path with an O(1) indexed structure or slab for production mode.
  - [x] Evaluate `SmallVec` for finalization queues and optimize pinned-object tracking for small cardinalities.
  - [x] Re-run allocation-pressure benchmark and compare GC pause and throughput.
  - [x] Verify: run GC fixture suite (`tests/fixtures/gc/*.cs`) and confirm behavior remains unchanged.

### Step 2c: Cache capacity and contention optimization
- **Status**: [x] Completed
- **Files**: crates/dotnet-vm/src/state.rs, crates/dotnet-vm/src/resolver/mod.rs, crates/dotnet-runtime-resolver/src/layout.rs, crates/dotnet-runtime-resolver/src/methods.rs, crates/dotnet-metrics/src/lib.rs
- **Depends on**: 0c
- **Risk**: medium
- **Tasks**:
  - [x] Add optional bounded mode (size limits/eviction) for selected global caches.
  - [x] Add thread-local front-cache for hot resolver lookups (`method_info`, `vmt`, `hierarchy`).
  - [x] Add instrumentation counters for key clone count and cache memory footprint.
  - [x] Benchmark virtual dispatch and generics stress workloads with and without front-cache.
  - [x] Verify: ensure cache miss fallback preserves correctness by running full test suite.

### Step 2d: Instruction dispatch tuning
- **Status**: [x] Completed
- **Files**: crates/dotnet-vm/build.rs, crates/dotnet-vm/src/dispatch/mod.rs, crates/dotnet-vm/src/dispatch/registry.rs
- **Depends on**: 0c, 2a
- **Risk**: medium
- **Tasks**:
  - [x] Add a build-time option to generate function-pointer jump-table dispatch in parallel with current `match` dispatch.
  - [x] Add configurable safe-point polling interval for the batch loop (`32/64/128`).
  - [x] Prototype one super-instruction pair and gate it behind a feature flag.
  - [x] Benchmark arithmetic loop and virtual dispatch workloads for each dispatch variant.
  - [x] Verify: run instruction dispatch tests and confirm opcode behavior parity.

### Step 2e: Intrinsic dispatch fast path
- **Status**: [~] Deliberately deferred (missing workload signal)
- **Files**: crates/dotnet-vm/src/intrinsics/mod.rs, crates/dotnet-runtime-resolver/src/methods.rs, crates/dotnet-vm/src/dispatch/mod.rs, crates/dotnet-vm/src/stack/call_ops_impl.rs
- **Depends on**: 0c, 2c
- **Risk**: medium
- **Decision**: Defer implementation work until intrinsic-heavy benchmark coverage exists.
- **Rationale**:
  - Current instrumented runs still report `intrinsic_call_total=0` for `json` and `generics`.
  - Prior candidate implementation regressed instrumented runtime (`json +31.75%`, `generics +21.60%`) while targeting a path these workloads did not execute.
- **Reopen criteria**:
  - Add at least one intrinsic-heavy benchmark fixture that shows non-zero `intrinsic_call_total`, then re-evaluate with a scoped candidate (dispatch-path dedup only, no registry-cache redesign first).
- **Tasks**:
  - [ ] Add intrinsic-heavy fixture(s) and baseline intrinsic metrics (`intrinsic_call_total`, per-signature counts) in Criterion runs.
  - [ ] Prototype lightweight dispatch-path deduplication only (no new registry cache) and benchmark against intrinsic-heavy fixture(s).
  - [ ] Re-evaluate method-level intrinsic classification caching only if intrinsic-heavy fixture data shows repeated lookup overhead.
  - [ ] Benchmark JSON/generics plus intrinsic-heavy fixture(s) and compare intrinsic-call-path overhead.
  - [ ] Verify: run intrinsic-related fixtures and confirm identical behavior.

## Phase 3: Memory Management and GC

### Step 3a: Trace-cost and cross-arena profiling
- **Status**: [x] Completed
- **Files**: crates/dotnet-vm/src/gc/coordinator.rs, crates/dotnet-vm/src/threading/basic.rs, crates/dotnet-runtime-memory/src/access.rs, crates/dotnet-metrics/src/lib.rs
- **Depends on**: 0b
- **Risk**: high
- **Tasks**:
  - [x] Add counters for GC fixed-point iteration count and per-iteration cross-arena object volume.
  - [x] Add timing for major `Collect::trace` roots and high-cost layout traversal paths.
  - [x] Surface GC metrics snapshot API for benchmark consumption.
  - [x] Benchmark allocation-pressure workload and record STW p50/p95/p99.
  - [x] Verify: run multithreading stress fixtures and confirm no deadlocks or missing roots.

### Step 3b: Allocation-rate reduction in runtime paths
- **Status**: [x] Completed
- **Files**: crates/dotnet-vm/src/stack/context.rs, crates/dotnet-vm/src/stack/call_ops_impl.rs, crates/dotnet-vm-data/src/stack.rs, crates/dotnet-value/src/string.rs
- **Depends on**: 3a
- **Risk**: medium
- **Tasks**:
  - [x] Replace repeated per-call `Vec` allocations in local initialization with reusable buffers or `SmallVec`.
  - [x] Add optional string interning experiment for high-duplication string workloads.
  - [x] Audit and reduce unnecessary `Arc` clones in hot resolver and dispatch paths.
  - [x] Benchmark JSON and generics workloads and record allocation deltas.
  - [x] Verify: run string and generics fixture suites for functional parity.

### Step 3c: StackFrame pooling and locality
- **Status**: [x] Completed
- **Files**: crates/dotnet-vm-data/src/stack.rs, crates/dotnet-vm/src/stack/mod.rs, crates/dotnet-vm/src/stack/call_ops_impl.rs
- **Depends on**: 3b
- **Risk**: medium
- **Tasks**:
  - [x] Add `SmallVec` for `exception_stack` and `pinned_locals` in `StackFrame`.
  - [x] Prototype frame pooling per executor thread with clear lifecycle boundaries.
  - [x] Benchmark tight call/return fixture and compare allocator activity.
  - [x] Verify: run exception and recursion-related fixtures and confirm no frame reuse bugs.

## Phase 4: Compiler Optimization Enablement

### Step 4a: Hot-path inlining and monomorphization audit
- **Status**: [x] Completed
- **Files**: crates/dotnet-vm/src/dispatch/mod.rs, crates/dotnet-vm/src/stack/ops.rs, crates/dotnet-runtime-resolver/src/methods.rs, Cargo.toml
- **Depends on**: 2d, 3c (2e optional; required only for intrinsic-specific call-path tuning)
- **Risk**: medium
- **Tasks**:
  - [x] Capture profile-guided hot call graph and identify top generic call boundaries.
  - [x] Add targeted inline attributes for proven hot leaf functions only.
  - [x] Evaluate `lto = "thin"` and `lto = "fat"` on benchmark suite.
  - [x] Record binary size and runtime tradeoffs in `REVIEW.md`.
  - [x] Verify: run clippy/tests and ensure no codegen-only regressions.

### Step 4b: Branch prediction and cold-path separation
- **Status**: [x] Completed
- **Files**: crates/dotnet-vm/src/dispatch/mod.rs, crates/dotnet-vm/src/stack/call_ops_impl.rs, crates/dotnet-vm/src/intrinsics/mod.rs
- **Depends on**: 4a
- **Risk**: low
- **Tasks**:
  - [x] Isolate panic/error formatting code into `#[cold]` helpers in dispatch and call paths.
  - [x] Add `likely`/`unlikely` hints only on branches proven skewed by benchmark instrumentation.
  - [x] Benchmark arithmetic and JSON workloads before/after branch-hint changes.
  - [x] Verify: run full tests to confirm no behavior changes.

### Step 4c: SIMD and vectorization opportunities
- **Status**: [ ] Not started
- **Files**: crates/dotnet-runtime-memory/src/access.rs, crates/dotnet-vm-data/src/stack.rs, crates/dotnet-vm/src/gc/coordinator.rs
- **Depends on**: 4a
- **Risk**: medium
- **Tasks**:
  - [ ] Identify two concrete candidate loops for SIMD (reference scan and bulk copy/scan).
  - [ ] Implement an opt-in SIMD variant behind a feature gate with scalar fallback.
  - [ ] Benchmark allocation-pressure and JSON workloads with SIMD on/off.
  - [ ] Verify: run tests under default and no-default features to ensure deterministic behavior.

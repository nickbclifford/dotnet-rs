# dotnet-rs Low-Level Architecture Review

Date: 2026-04-07 (America/Chicago)
Reviewer: Codex (GPT-5)

## Executive Summary

The runtime has strong architectural foundations (build-time dispatch generation, explicit GC-safe-point protocol, split resolver/cache adapters), but current hot-path data layout and container choices are likely leaving substantial performance on the table.

Top findings:

1. `StackValue<'gc>` and `ManagedPtr<'gc>` are very large in release builds (`184` and `168` bytes, respectively), so the evaluation stack is cache-unfriendly.
2. Evaluation stack growth triggers O(n) pointer fixups on every `Vec` reallocation via `update_stack_pointers()`.
3. Core resolver/runtime caches are unbounded `DashMap`s with clone-heavy keys/values and no front-cache.
4. `HeapManager` still tracks all objects in a `BTreeMap<usize, ObjectRef>` on the hot path.
5. Intrinsic lookup still pays per-call key normalization/string formatting work before PHF lookup.
6. Baseline benchmarking infrastructure is missing (`criterion` harness and reproducible benchmark fixtures).

This review defines a phased refactor plan with dependency ordering, concrete risks, and a machine-parseable checklist for follow-up sessions.

## Scope and Method

Read and validated against:

- `AGENTS.md`
- `docs/ARCHITECTURE.md`
- `docs/GC_AND_MEMORY_SAFETY.md`
- `docs/THREADING_AND_SYNCHRONIZATION.md`
- `docs/BUILD_TIME_CODE_GENERATION.md`

Primary code paths reviewed:

- Dispatch loop: `crates/dotnet-vm/src/dispatch/mod.rs`
- Eval stack / frames: `crates/dotnet-vm-data/src/stack.rs`
- Value/pointer/object layout: `crates/dotnet-value/src/**`
- Shared caches: `crates/dotnet-vm/src/state.rs`, `crates/dotnet-vm/src/resolver/mod.rs`, `crates/dotnet-runtime-resolver/src/**`
- Heap and memory writes: `crates/dotnet-runtime-memory/src/heap.rs`, `crates/dotnet-runtime-memory/src/access.rs`
- GC orchestration: `crates/dotnet-vm/src/gc/coordinator.rs`, `crates/dotnet-vm/src/executor.rs`
- Sync and monitor contention: `crates/dotnet-vm/src/sync/**`

Ad hoc measurements (release, current code) were collected with a temporary local probe binary and existing fixture DLLs. These are preliminary baselines until Phase 0 benchmark harness lands.

## Phase 0 Baseline Status

### What exists now

- Workspace benchmark crate and fixtures are in place (`crates/dotnet-benchmarks`).
- Baseline capture script is available at `crates/dotnet-benchmarks/scripts/capture_baseline.py`.
- Persistent baseline artifacts:
  - summary + counters: `crates/dotnet-benchmarks/baselines/phase0/baseline.json`
  - raw Criterion JSON trees: `crates/dotnet-benchmarks/baselines/phase0/criterion/{default,no_default,instrumented_default}/*`

### Current measured baselines (Phase 0 capture)

Median / p95 runtime by workload (ms):

| Workload | Default median | Default p95 | No-default median | No-default p95 | Instrumented median | Instrumented p95 |
|---|---:|---:|---:|---:|---:|---:|
| `json` | 312.11 | 629.79 | 312.95 | 643.60 | 412.76 | 831.42 |
| `arithmetic` | 585.06 | 592.76 | 574.24 | 579.17 | 1034.82 | 1051.39 |
| `gc` | 433.05 | 891.60 | 428.88 | 873.36 | 608.92 | 613.58 |
| `dispatch` | 794.17 | 801.89 | 764.01 | 779.11 | 1200.72 | 1223.98 |
| `generics` | 18754.46 | 19304.49 | 17920.70 | 18185.10 | 23229.22 | 23733.31 |

Derived deltas (median):

- `no_default` vs `default`: `json +0.27%`, `arithmetic -1.85%`, `gc -0.96%`, `dispatch -3.80%`, `generics -4.45%`.
- `instrumented_default` vs `default`: `json +32.25%`, `arithmetic +76.87%`, `gc +40.61%`, `dispatch +51.19%`, `generics +23.86%`.

Instrumentation counters snapshot (`instrumented_default`, selected):

| Workload | Eval stack reallocations | Pointer fixup total (ns) | Opcode dispatch total | Intrinsic call total |
|---|---:|---:|---:|---:|
| `json` | 4 | 601 | 1,676,145 | 0 |
| `arithmetic` | 2 | 611 | 8,400,022 | 0 |
| `gc` | 3 | 571 | 2,180,020 | 0 |
| `dispatch` | 3 | 600 | 6,400,067 | 0 |
| `generics` | 4 | 841 | 78,461,722 | 0 |

Delta verification run:

- Re-ran `json` with `capture_baseline.py` and confirmed script delta output against stored baseline:
  - `default`: median `+0.41%`, p95 `+0.96%`
  - `no_default`: median `+0.00%`, p95 `+0.00%`
  - `instrumented_default`: median `+0.00%`, p95 `+0.00%`

### Gap

Phase 0 baseline capture is now reproducible and persisted. Follow-up optimization steps should compare against `crates/dotnet-benchmarks/baselines/phase0/baseline.json`.

---

## Phase 1: Data Layout and Type Size

### 1a. StackValue size reduction

Evidence:

- `StackValue` variants include heavyweight `ManagedPtr`, `ValueType(Object)`, and `TypedRef(ManagedPtr, Arc<TypeDescription>)`: `crates/dotnet-value/src/stack_value.rs:39-54`.
- Stack values are hot in evaluation stack operations: `crates/dotnet-vm-data/src/stack.rs:84-349`.
- No compile-time size assert currently exists for `StackValue` (only `ObjectRef` has one): `crates/dotnet-value/src/object/mod.rs:243`.

Current measurement:

- `size_of::<StackValue>() == 184` bytes (release, no memory-validation).

Proposed change:

1. Add compile-time layout assertions (`size_of`, `align_of`) in `dotnet-value` tests for `StackValue`, `ManagedPtr`, `PointerOrigin`, `ObjectInner`.
2. Introduce an indirection strategy for rare large variants:
   - candidate A: box `ManagedPtr` variant in `StackValue`
   - candidate B: split typed references into compact handle + side table
3. Re-evaluate `ValueType(Object)` storage strategy (handle vs inline clone) for stack-local value types.

Expected impact:

- Better cache density on eval stack and lower copy bandwidth in `Vec<StackValue>` operations.
- Likely measurable improvement in arithmetic/dispatch-heavy workloads.

Risk:

- High: serialization layout (`ManagedPtr::read/write`), GC tracing paths, and stack-address semantics are sensitive.

### 1b. ManagedPtr compaction

Update (2026-04-08): completed.

Implemented:

1. Replaced `ManagedPtr`'s `pinned: bool` with packed `flags: u8`.
2. Added `ManagedByteOffset(u32)` (`dotnet-utils`) and switched `ManagedPtr` to compact offset storage.
3. Preserved unmanaged correctness by using `_value` as authoritative full-width address fallback when offset does not fit `u32`.
4. Compacted `PointerOrigin` by moving cold metadata out of enum payload:
   - `Static(TypeDescription, GenericLookup)` -> `Static(Arc<StaticMetadata>)`
   - `Transient(Object)` -> `Transient(Box<Object>)`
5. Updated pointer serialization/deserialization and all downstream `PointerOrigin` consumers.

Measured layout impact (64-bit):

- Default release:
  - `ManagedPtr`: `168` -> `72` bytes (`-57.14%`)
  - `PointerOrigin`: `120` -> `24` bytes (`-80.00%`)
- Default debug:
  - `ManagedPtr`: `184` -> `80` bytes (`-56.52%`)
  - `PointerOrigin`: `128` -> `24` bytes (`-81.25%`)
- `--no-default-features`:
  - `ManagedPtr`: `72` debug / `64` release
  - `PointerOrigin`: `16` (debug/release)

Pointer-heavy benchmark reruns (`criterion`, sample-size 10, measurement-time 5s):

- `gc`: median `441.66 ms` vs Phase 0 baseline `433.05 ms` (`+1.99%`)
- `dispatch`: median `741.26 ms` vs Phase 0 baseline `794.17 ms` (`-6.66%`)
- `json`: median `324.99 ms` vs baseline `312.11 ms` (`+4.13%`), with Criterion reporting no significant change in the immediate A/B (`p=0.38`).

Risk notes:

- Serialization compatibility and cross-arena pointer recovery remain high-sensitivity areas; covered by pointer round-trip tests and full default/no-default validation runs.

### 1c. Object header compaction

Update (2026-04-08): completed.

Implemented:

1. Compacted `ObjectInner` header field presence:
   - `magic` is now only present when `memory-validation` or `debug_assertions` is enabled.
   - `owner_id` is now only present when `multithreading` or `memory-validation` is enabled.
2. Added `ObjectInner::new(storage, owner_id)` and `ObjectInner::owner_id()` to centralize layout/ownership behavior across configurations.
3. Audited and updated all `owner_id` consumers (`dotnet-value`, `dotnet-runtime-memory`, `dotnet-vm`) to read owner IDs through the immutable accessor contract.
4. Documented owner invariants directly on `ObjectInner`:
   - required by cross-arena write barriers and tagged serialization,
   - immutable after construction, therefore lock-free reads are valid.
5. Added a compile-time guarantee that `ValidationTag` is zero-sized in non-validation release builds.
6. Extended layout guards in `dotnet-value/tests/layout_sizes.rs` with `ObjectInner` size/alignment assertions under validation and non-validation configurations.

Measured layout impact (64-bit release):

- `default`:
  - `HeapStorage`: `128` bytes
  - `ObjectInner`: `136` bytes (`+8` header overhead, unchanged)
- `--no-default-features`:
  - `HeapStorage`: `128` bytes
  - `ObjectInner`: `128` bytes (`+0` header overhead; previously implied `+8`)
- `default + memory-validation`:
  - `HeapStorage`: `136` bytes
  - `ObjectInner`: `152` bytes (`+16` header overhead)

Per-object memory impact:

- single-threaded non-validation release path: `8` bytes saved per object (`136 -> 128`, `-5.88%`).
- multithreaded default release path: no layout delta (owner metadata remains required for cross-arena fast paths).

Allocation benchmark (`gc`, criterion, sample-size 10, measurement-time 5s):

- before: `450.04 ms`
- after: `445.56 ms`
- delta: `-0.99%` (criterion reported no statistically significant change, `p=0.11`).
- vs Phase 0 baseline (`433.05 ms`): `+2.89%` in this run.

Validation/correctness checks:

- Full default and `--no-default-features` clippy/test suites passed.
- Additional targeted validation runs passed:
  - `cargo test -p dotnet-value --features memory-validation -- --nocapture`
  - `cargo test -p dotnet-value --no-default-features --features memory-validation -- --nocapture --test-threads=1`

---

## Phase 2: Algorithms and Data Structures

### 2a. Evaluation stack reallocation and fixup

Evidence:

- Every capacity increase triggers full-stack pointer fixup: `crates/dotnet-vm-data/src/stack.rs:103-174`.
- Fixup rewrites both direct `ManagedPtr` stack origins and embedded managed pointers in value types (`visit_managed_ptrs`): `crates/dotnet-vm-data/src/stack.rs:134-163`.
- Method metadata currently does not expose/store `maxstack` in `MethodInfo`: `crates/dotnet-vm-data/src/lib.rs:46-53`, build path `crates/dotnet-vm/src/lib.rs:89-102`.

Current measurement:

- Reallocation path cost grows with stack size; 1M-push probe shows non-trivial aggregate time in reallocation/fixup path (`~9.6-11.9ms` for 19 growth events).

Proposed change:

1. Thread `body.header.max_stack` into `MethodInfo` and pre-reserve eval stack on frame entry.
2. Add growth policy tuned for deep stack methods (fewer reallocations).
3. Evaluate segmented stack design for byref stability (eliminate relocation fixup entirely).

Expected impact:

- Lower latency spikes on deep-stack workloads.
- Better worst-case behavior for pointer-heavy IL.

Risk:

- High: stack address identity semantics and `ManagedPtr::PointerOrigin::Stack` correctness.

### 2b. HeapManager structures

Evidence:

- `_all_objs: RefCell<BTreeMap<usize, ObjectRef>>` and pinned `HashSet` are always present: `crates/dotnet-runtime-memory/src/heap.rs:13-27` and initialization in `crates/dotnet-vm/src/state.rs:330-339`.
- `find_object()` performs range search over `BTreeMap`: `crates/dotnet-runtime-memory/src/heap.rs:29-46`.

Proposed change:

1. Gate `_all_objs` behind a debug/diagnostic feature if not required in production.
2. Replace `BTreeMap` with slab/indexed registry if random lookup dominates.
3. Evaluate `SmallVec` for short finalization queues and compact pinned tracking for common small cardinalities.

Expected impact:

- Reduced heap bookkeeping overhead and allocator churn.

Risk:

- Medium: affects diagnostics and conservative scan helpers.

### 2c. Cache optimization

Evidence:

- 12+ runtime caches are unbounded `DashMap::new()`: `crates/dotnet-vm/src/state.rs:38-87`.
- Cache keys include clone-heavy tuples with `GenericLookup`: `crates/dotnet-vm/src/state.rs:45,53-65`; `crates/dotnet-runtime-resolver/src/layout.rs:435-444`; `crates/dotnet-runtime-resolver/src/methods.rs:120-206`.
- `DashMap::len()` is called for all caches in `get_cache_stats`: `crates/dotnet-vm/src/state.rs:217-252`.

Current measurement sample (`cache_test_0`):

- High hit rates for `vmt`, `intrinsic`, `method_info`, but no bounding or eviction policy.

Proposed change:

1. Add optional bounded mode (LRU/admission) for heavy caches.
2. Add thread-local front-cache for hot read-mostly keys (`method_info`, `vmt`, `hierarchy`).
3. Replace default hasher for selected caches with `FxHash`-based maps where collision risk is acceptable.
4. Add per-cache clone/memory counters under instrumentation feature.

Expected impact:

- Lower contention and lower heap growth in large applications.

Risk:

- Medium: cache eviction correctness (must preserve semantic correctness when missing).

### 2d. Instruction dispatch optimization

Evidence:

- Runtime batch loop: 128 instructions with 32-instruction safe-point polls: `crates/dotnet-vm/src/dispatch/mod.rs:167-183`.
- Build-generated instruction dispatcher is `match`-based: `crates/dotnet-vm/build.rs:316-350`.

Proposed change:

1. Benchmark current `match` against indexed function-pointer table for opcode classes with dense discriminants.
2. Sweep safe-point interval (`32`, `64`, `128`) while tracking GC latency percentiles.
3. Prototype super-instruction fusion for common pairs (`ldarg.*` + field load, etc.) in build-time generation.

Expected impact:

- Reduced dispatch overhead on tight loops.

Risk:

- Medium/High: super-instruction correctness and debug trace fidelity.

### 2e. Intrinsic dispatch optimization

Evidence:

- Method dispatch path still has a hot `is_intrinsic_cached(method.clone())` route in `ExecutionEngine::dispatch_method`: `crates/dotnet-vm/src/dispatch/mod.rs:227-242`.
- Intrinsic key build performs canonicalization + `replace('/', '+')` + formatted write each lookup: `crates/dotnet-vm/src/intrinsics/mod.rs:258-300`.

Proposed change:

1. Add resolved-method flag/enum for intrinsic status to avoid repeated map lookups where possible.
2. Cache normalized intrinsic key (or pre-hashed form) per method descriptor.
3. Reconcile duplicate dispatch paths (`ExecutionEngine::dispatch_method` vs `VesContext::dispatch_method`) to avoid drift and redundant checks.

Expected impact:

- Lower overhead on call-heavy intrinsic workloads.

Risk:

- Medium: method-resolution invariants and generic specialization correctness.

---

## Phase 3: Memory Management and GC

### 3a. gc-arena tracing and cross-arena overhead

Evidence:

- Coordinated GC fixed-point cross-arena marking loop can iterate until convergence: `crates/dotnet-vm/src/gc/coordinator.rs:349-413`.
- Memory writes recursively record references based on layout descriptors: `crates/dotnet-runtime-memory/src/access.rs:1094-1180`.

Current measurement sample:

- `cache_test_0` triggered one GC pause (`52us`) in multithreaded mode.

Proposed change:

1. Add per-type/per-layout tracing counters and durations under `bench-instrumentation`.
2. Measure fixed-point iteration counts and cross-arena object counts per GC cycle.
3. Optimize/refactor hot reference-recording loops based on observed distributions.

Expected impact:

- Reduced STW pause variance and better GC scalability under cross-arena references.

Risk:

- High: GC correctness and safety invariants.

### 3b. Heap allocation patterns

Evidence:

- Local initialization allocates `Vec<StackValue>` and `Vec<bool>` per frame setup: `crates/dotnet-vm/src/stack/context.rs:98-154`, `crates/dotnet-vm/src/stack/call_ops_impl.rs:88-96`.

Proposed change:

1. Introduce pooled/small-vector strategies for common local counts.
2. Evaluate fixed-size locals containers where method metadata allows static sizing.
3. Audit high-frequency `Arc` cloning in resolver/type paths and reduce where lifetimes permit.

Expected impact:

- Lower allocation rate and improved frame setup times.

Risk:

- Medium: borrow/lifetime complexity and object ownership semantics.

### 3c. Stack frame allocation

Evidence:

- `StackFrame` carries heap-allocated vectors for exception/pin tracking: `crates/dotnet-vm-data/src/stack.rs:46-57`.

Proposed change:

1. Replace per-frame `Vec` fields with `SmallVec` where empirical distributions are small.
2. Consider frame object pooling per executor thread.

Expected impact:

- Less allocator churn during heavy call/return workloads.

Risk:

- Medium: reentrancy and frame lifecycle correctness.

---

## Phase 4: Compiler Optimization Enablement

### 4a. Monomorphization and inlining

Evidence:

- Hot APIs include layered generic trait calls across crates (`VesOps`, resolver adapters).

Proposed change:

1. Use profiling (`-Cprofile-generate/use`) to identify missing inline opportunities in dispatch/stack ops.
2. Add targeted `#[inline(always)]` only on proven hot leaf calls.
3. Measure LTO (`thin`/`fat`) impact on benchmark suite once Phase 0 harness exists.

Expected impact:

- Lower call overhead in tight interpreter loops.

Risk:

- Medium: compile-time/binary-size increase.

### 4b. Branch prediction and cold paths

Evidence:

- Dispatch loop and method-call paths contain mixed hot/error branches: `crates/dotnet-vm/src/dispatch/mod.rs:167-271`.

Proposed change:

1. Mark panic/error helpers as `#[cold]` and apply `likely/unlikely` where profiling validates strong skew.
2. Extract slow error formatting/panic paths from hot blocks.

Expected impact:

- Better i-cache and branch prediction behavior.

Risk:

- Low/Medium: modest gains unless profile confirms heavy skew.

### 4c. SIMD and vectorization

Evidence:

- Candidate loops: reference scanning and memory-copy/scan routines (`crates/dotnet-runtime-memory/src/access.rs:1156+`).

Proposed change:

1. Benchmark scalar vs SIMD for GC descriptor bitmap scans and bulk copy/scan paths.
2. Prefer compiler auto-vectorization first; add explicit SIMD only where gains are proven and portable.

Expected impact:

- Potential wins in memory-heavy workloads.

Risk:

- Medium/High: portability and maintenance burden for explicit SIMD.

---

## Risk Assessment Summary

- High risk: `1a`, `1b`, `2a`, `3a` (layout/GC correctness critical)
- Medium risk: `1c`, `2b`, `2c`, `2d`, `2e`, `3b`, `3c`, `4a`, `4c`
- Low/Medium risk: `4b`

Potential public API breakage:

- Current deliverables are documentation/tooling only: no API changes made.
- Future optimization steps likely to touch internal crate APIs and trait bounds; if any public API changes are required, document them explicitly in this file during implementation.

## Dependency Ordering (What unlocks what)

1. `0a` + `0b` (benchmark + instrumentation) unlock trustworthy measurement for all other phases.
2. `1a`/`1b` should precede `2a` because stack/fixup redesign depends on final byref/value representation.
3. `1c` can run in parallel with `2b` once validation constraints are pinned down.
4. `2c` should start before `2d`/`2e` to establish cache pressure and intrinsic call baselines.
5. `3a` should run before deeper GC algorithm changes (`3b`/`3c`).
6. `4a`/`4b`/`4c` should be last after structural changes stabilize.

## Recommended Implementation Sequence

1. Phase 0 (`0a`, `0b`, `0c`)
2. `1a` -> `1b` -> `1c`
3. `2a` -> `2b` -> `2c` -> `2e` -> `2d`
4. `3a` -> `3b` -> `3c`
5. `4a` -> `4b` -> `4c`

This order minimizes rework and ensures each optimization is benchmarked against a stable baseline.

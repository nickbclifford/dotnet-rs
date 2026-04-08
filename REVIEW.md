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

Update (2026-04-08): completed.

Implemented:

1. Added `max_stack` to `MethodInfo` and populated it from IL method headers (`body.header.maximum_stack_size`) in `build_method_info`.
2. Added `EvaluationStack::reserve_slots(total_slots)` in `dotnet-vm-data`:
   - reserves ahead of demand,
   - runs stack-origin managed pointer fixup exactly once when reserve causes reallocation,
   - reuses the same instrumentation accounting path as push-driven reallocations.
3. Updated call-frame setup paths to pre-reserve from method metadata:
   - `call_frame`: reserves for `current_top + locals + max_stack`,
   - `entrypoint_frame`: reserves for `args + locals + max_stack` before argument push.
4. Added opt-in segmented-growth prototype feature:
   - `segmented-eval-stack-prototype` in `dotnet-vm-data` (forwarded through `dotnet-vm-ops`, `dotnet-vm`, `dotnet-cli`, and `dotnet-benchmarks`),
   - reserve target rounds to 64-slot chunks when enabled.

Measured impact (`dispatch`, Criterion sample-size 10, measurement-time 5s, warm-up 1s):

- Default runtime median:
  - pre-change: `751.15 ms`
  - post-change: `725.98 ms`
  - delta: `-3.35%`
- Versus Phase 0 baseline (`794.17 ms`): `-8.59%`.

Bench instrumentation counters (`dispatch`):

- pre-change: `eval_stack_reallocations=3`, `pointer_fixup_total_ns=711`
- post-change: `eval_stack_reallocations=2`, `pointer_fixup_total_ns=331`
- delta: reallocations `-33.33%`, fixup time `-53.45%`.

Segmented prototype (`--features segmented-eval-stack-prototype`):

- Default median: `726.27 ms` (`+0.04%` vs non-prototype post-change run; effectively neutral).
- Instrumented counters: `eval_stack_reallocations=1`, `pointer_fixup_total_ns=110`.

Managed pointer/deep-stack semantics checks:

- Full default and no-default clippy/test matrices passed.
- Targeted stack/byref fixtures passed:
  - `basic_span_stack_read_0`
  - `basic_span_pinnable_stack_0`
  - `pointers_ref_struct_stress_42`
  - `unsafe_ref_struct_gc_safety_42`

Risk notes:

- Stack-slot address identity remains sensitive; reserve-before-push removes growth events but does not remove the fixup mechanism.
- Current segmented prototype is a growth-policy prototype (chunked capacity), not a fully segmented storage layout yet.

### 2f. Segmented stack architecture integration

Evidence:

- Step `2a` prototype reduced realloc/fixup events (`eval_stack_reallocations: 3 -> 2`, prototype run to `1`) but is still a growth-policy shim over contiguous `Vec` storage.
- Stack operations throughout the VM still assume contiguous backing (`stack[index]`, `split_off`, direct `truncate`) in evaluation-stack and frame/exception flows.
- Full pointer-fixup machinery remains in the hot path whenever contiguous storage moves.

Proposed change:

1. Implement a true segmented evaluation-stack backend with stable slot addresses across growth.
2. Introduce an explicit stack-storage abstraction consumed by `VesContext`/stack ops instead of direct `Vec` access.
3. Keep contiguous backend as default/fallback; select segmented backend via feature flag for rollout.
4. Rework suspend/restore and unwind/truncate flows to be segment-aware without changing externally visible stack semantics.

Expected impact:

- Eliminates relocation-driven full-stack pointer fixup in segmented mode.
- Improves worst-case latency for deep stack and byref-heavy workloads.

Risk:

- High: stack-slot identity and exception/suspension correctness are cross-cutting invariants.

### 2b. HeapManager structures

Update (2026-04-08): completed.

Implemented:

1. Added `HeapManager::new()` and moved heap-container initialization out of `dotnet-vm` state construction.
2. Added feature-gated diagnostics object registry:
   - new feature `heap-diagnostics` in `dotnet-runtime-memory` (default off),
   - `_all_objs` now exists only under `heap-diagnostics`,
   - feature forwarding added in `dotnet-vm`, `dotnet-cli`, and `dotnet-benchmarks`.
3. Replaced production object lookup path with an indexed registry:
   - `Slab<RegistryEntry>` + `start_to_id` map + bucket index map in `heap.rs`,
   - `find_object()` now routes to the slab+bucket path when diagnostics are disabled.
4. Switched small-cardinality containers in `HeapManager`:
   - `finalization_queue` and `pending_finalization` to `SmallVec<[ObjectRef; 8]>`,
   - `pinned_objects` to `SmallVec<[ObjectRef; 8]>` with deduped `pin_object`/`unpin_object` helpers.
5. Updated VM call sites to use `HeapManager` APIs:
   - object registration (`register_object`),
   - pinned tracking (`pin_object`/`unpin_object`),
   - tracer heap snapshot and object-count paths (`snapshot_objects`/`live_object_count`).

Measured impact (`gc` workload, Criterion sample-size 10, measurement-time 5s, warm-up 1s):

- Non-instrumented:
  - pre-step run: `511.39 ms`
  - post-step run: `435.33 ms`
  - delta: `-14.87%`
- Instrumented:
  - pre-step run: `721.84 ms`
  - post-step run: `629.71 ms`
  - delta: `-12.76%`

GC pause/throughput comparison:

- post-step instrumented snapshot (`target/release/dotnet-bench-metrics/gc.json`):
  - `gc_pause_total_us=4,941`, `gc_pause_count=169`
  - average pause per GC: `29.24 us`
- Phase 0 instrumented baseline snapshot:
  - `gc_pause_total_us=3,647`, `gc_pause_count=253`
  - average pause per GC: `14.42 us`
- Interpretation: post-step run performed fewer collections with higher average pause, while workload throughput improved materially in this A/B run.

Phase 0 baseline comparison:

- baseline `gc` median (`results.default`): `433.05 ms`
  - post-step non-instrumented: `435.33 ms` (`+0.53%`)
- baseline `gc` median (`results.instrumented_default`): `608.92 ms`
  - post-step instrumented: `629.71 ms` (`+3.41%`)

Risk notes:

- `heap-diagnostics` now controls full heap snapshot visibility; with feature off, `trace_dump_heap()` emits zero objects by design.
- Conservative owner lookup (`find_object`) remains active in production mode via slab index; no GC fixture regressions observed.

### 2c. Cache optimization

Update (2026-04-08): completed.

Implemented:

1. Added optional bounded mode for selected global caches:
   - `DOTNET_CACHE_LIMIT_METHOD_INFO`
   - `DOTNET_CACHE_LIMIT_VMT`
   - `DOTNET_CACHE_LIMIT_HIERARCHY`
   - insertion uses bounded eviction/fallback semantics (cache miss recomputation remains authoritative).
2. Added thread-local front-caches for:
   - `method_info` (`crates/dotnet-vm/src/state.rs`)
   - `vmt` and `hierarchy` (`crates/dotnet-vm/src/resolver/mod.rs`)
3. Updated resolver cache adapter API to use key-part lookups for hot reads, avoiding forced tuple-key construction in resolver call sites (`crates/dotnet-runtime-resolver/src/lib.rs`, `methods.rs`, `types.rs`).
4. Added bench instrumentation for:
   - key clone counters by cache (`method_info`, `vmt`, `hierarchy`)
   - front-cache hit/miss counters by cache
   - estimated cache memory footprint (`CacheSizes` bytes + per-cache maps in bench snapshot)
5. Added per-cache estimated byte accounting in shared cache sizing (`crates/dotnet-vm/src/state.rs`).
6. Added mitigation default:
   - front-cache is now opt-in (`DOTNET_FRONT_CACHE_ENABLED`, default `false`) to avoid always-on regressions on low-contention dispatch-heavy runs.

Measured impact (Criterion, sample-size 10, measurement-time 5s, warm-up 1s):

- Non-instrumented (`dispatch`, `generics`) with/without front-cache:
  - front-cache **on** (`DOTNET_FRONT_CACHE_ENABLED=1`):
    - `dispatch`: `759.91 ms`
    - `generics`: `18.908 s`
  - front-cache **off** (`DOTNET_FRONT_CACHE_ENABLED=0`):
    - `dispatch`: `736.79 ms`
    - `generics`: `19.589 s`
  - delta (on vs off):
    - `dispatch`: `+3.14%` (regression)
    - `generics`: `-3.48%` (improvement)

- Instrumented comparison (latest paired runs):
  - front-cache **on**:
    - `dispatch`: `1.1522 s`
    - `generics`: `24.044 s`
  - front-cache **off**:
    - `dispatch`: `1.1542 s`
    - `generics`: `24.670 s`
  - delta (on vs off):
    - `dispatch`: `-0.17%` (neutral)
    - `generics`: `-2.54%` (improvement)

Bench counter highlights (`target/release/dotnet-bench-metrics/*.json`):

- `dispatch`:
  - key clones: `64` (on) vs `1,000,040` (off)
  - front-cache hits (on): `method_info=200,004`, `vmt=199,994`
  - estimated cache memory bytes: `4,553` (on) vs `4,689` (off)
- `generics`:
  - key clones: `78` (on) vs `22,760,030` (off)
  - front-cache hits (on): `hierarchy=5,040,000`, `method_info=2,559,997`, `vmt=2,519,996`
  - estimated cache memory bytes: `6,605` (on) vs `6,839` (off)

Phase 0 baseline comparison (`results.default`):

- baseline `dispatch` median: `794.17 ms`
  - front-cache on run: `759.91 ms` (`-4.31%`)
  - front-cache off run: `736.79 ms` (`-7.22%`)
- baseline `generics` median: `18.75446 s`
  - front-cache on run: `18.908 s` (`+0.82%`)
  - front-cache off run: `19.589 s` (`+4.45%`)

Risk notes:

- Front-cache is workload-sensitive; it reduced clone pressure substantially and improved generics stress but regressed non-instrumented virtual dispatch in this environment.
- Mitigation in place: front-cache is opt-in via env var, keeping default behavior stable while retaining the contention optimization path for targeted workloads.

### 2d. Instruction dispatch optimization

Evidence:

- Runtime batch loop was fixed at 128 instructions with hardcoded 32-instruction safe-point polls.
- Build-generated instruction dispatcher only emitted a monomorphic `match` backend.

Implemented change:

1. Added dual dispatch generation in `crates/dotnet-vm/build.rs`:
   - retained `dispatch_monomorphic`,
   - generated `dispatch_jump_table` with opcode-indexed function-pointer selection and per-opcode wrappers,
   - added duplicate-opcode registration guard at build time.
2. Added build-time dispatch backend selection via feature flag:
   - `dotnet-vm` feature `instruction-dispatch-jump-table`,
   - `dotnet-benchmarks` feature forwarding for benchmark matrix runs.
3. Added configurable safe-point polling interval in the batch loop:
   - env var `DOTNET_SAFE_POINT_POLL_INTERVAL`,
   - accepted values: `32`, `64`, `128`,
   - invalid values fall back to `32` with a warning.
4. Added super-instruction prototype behind feature flag:
   - `dotnet-vm` feature `dispatch-super-instruction-prototype`,
   - fused pair in hot batch path: `LoadConstantInt32(1)` then `Add`,
   - preserves per-instruction tracing/ring-buffer and opcode metrics accounting.
5. Added generated-dispatch coverage assertions in `crates/dotnet-vm/src/dispatch/registry.rs` for jump-table artifacts.

Verification:

- `cargo fmt`
- `cargo clippy --all-targets -- -D warnings`
- `cargo test -- --nocapture`
- `cargo clippy --all-targets --no-default-features -- -D warnings`
- `cargo test --no-default-features -- --nocapture --test-threads=1`
- `cargo test -p dotnet-vm --features instruction-dispatch-jump-table -- --nocapture`
- `cargo test -p dotnet-vm --features "instruction-dispatch-jump-table dispatch-super-instruction-prototype" -- --nocapture`

Measured results (Criterion medians, non-instrumented):

- Match dispatch (`instruction-dispatch-jump-table` disabled)
  - poll `32`: `arithmetic=532.48 ms`, `dispatch=767.04 ms`
  - poll `64`: `arithmetic=551.07 ms`, `dispatch=768.06 ms`
  - poll `128`: `arithmetic=542.35 ms`, `dispatch=759.97 ms`
- Jump-table dispatch (`instruction-dispatch-jump-table`)
  - poll `32`: `arithmetic=553.00 ms`, `dispatch=791.16 ms`
  - poll `64`: `arithmetic=549.86 ms`, `dispatch=770.73 ms`
  - poll `128`: `arithmetic=557.45 ms`, `dispatch=769.13 ms`
- Jump-table + super-instruction (`instruction-dispatch-jump-table dispatch-super-instruction-prototype`)
  - poll `32`: `arithmetic=568.05 ms`, `dispatch=787.32 ms`
  - poll `64`: `arithmetic=579.16 ms`, `dispatch=786.49 ms`

Key deltas vs current default config (match + poll `32`):

- Safe-point interval sweep:
  - poll `64`: arithmetic `+3.49%`, dispatch `+0.13%`
  - poll `128`: arithmetic `+1.85%`, dispatch `-0.92%`
- Jump-table vs match (`poll=32`):
  - arithmetic `+3.85%`, dispatch `+3.14%` (regression)
- Jump-table+super vs match (`poll=32`):
  - arithmetic `+6.68%`, dispatch `+2.64%` (regression)

Phase 0 baseline context (`results.default` medians):

- baseline arithmetic: `585.0569755 ms`
  - current best observed arithmetic (match, poll `32`): `532.48 ms` (`-8.99%`)
- baseline dispatch: `794.1701455 ms`
  - current best observed dispatch (match, poll `128`): `759.97 ms` (`-4.31%`)

Risk notes and mitigation:

- In this environment, jump-table and super-instruction prototype both regress the targeted workloads.
- Mitigation in place:
  - both paths are feature-gated and remain disabled by default,
  - runtime default behavior remains monomorphic `match` dispatch with poll interval default `32`,
  - safe-point poll interval remains tunable via env var for workload-specific trade-offs (`dispatch` improved slightly at `128`, arithmetic regressed).

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

- High risk: `1a`, `1b`, `2a`, `2f`, `3a` (layout/GC correctness critical)
- Medium risk: `1c`, `2b`, `2c`, `2d`, `2e`, `3b`, `3c`, `4a`, `4c`
- Low/Medium risk: `4b`

Potential public API breakage:

- Current deliverables are documentation/tooling only: no API changes made.
- Future optimization steps likely to touch internal crate APIs and trait bounds; if any public API changes are required, document them explicitly in this file during implementation.

## Dependency Ordering (What unlocks what)

1. `0a` + `0b` (benchmark + instrumentation) unlock trustworthy measurement for all other phases.
2. `1a`/`1b` should precede `2a` because stack/fixup redesign depends on final byref/value representation.
3. `1c` can run in parallel with `2b` once validation constraints are pinned down.
4. `2f` should follow `2a` before dispatch tuning so segmented-stack semantics are settled.
5. `2c` should start before `2d`/`2e` to establish cache pressure and intrinsic call baselines.
6. `3a` should run before deeper GC algorithm changes (`3b`/`3c`).
7. `4a`/`4b`/`4c` should be last after structural changes stabilize.

## Recommended Implementation Sequence

1. Phase 0 (`0a`, `0b`, `0c`)
2. `1a` -> `1b` -> `1c`
3. `2a` -> `2f` -> `2b` -> `2c` -> `2e` -> `2d`
4. `3a` -> `3b` -> `3c`
5. `4a` -> `4b` -> `4c`

This order minimizes rework and ensures each optimization is benchmarked against a stable baseline.

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

Update (2026-04-08): deliberately deferred.

What was attempted and why it was reverted:

- A true segmented eval-stack backend was implemented and then reverted (`b642b40`) after integration failures across suspend/restore, truncate/unwind, and exception flows.
- Root cause: broad VM assumptions of contiguous stack storage (`stack[index]`, `split_off`, direct `truncate`) made the segmented integration high-risk and cross-cutting.

Current evidence on reverted baseline (`2a` contiguous reserve model):

- Previous `2a` counters already reduced growth/fixup substantially (`eval_stack_reallocations: 3 -> 2`, `pointer_fixup_total_ns: 711 -> 331`).
- Current confirmation run (`dispatch`, instrumented, Criterion `--sample-size 10 --measurement-time 5 --warm-up-time 1`):
  - contiguous: `eval_stack_reallocations=2`, `pointer_fixup_total_ns=490`
  - prototype (`segmented-eval-stack-prototype` growth policy): `eval_stack_reallocations=1`, `pointer_fixup_total_ns=300`
  - runtime delta (prototype vs contiguous): `-0.30%` (no meaningful throughput change).

Decision:

- Do not pursue full segmented-stack architecture in the current phase.
- Keep Step `2a` reserve-ahead approach as the production path.
- Reopen only with new evidence that pointer-fixup is a material runtime share on relevant workloads, or with a materially lower-complexity design.

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

Update (2026-04-08): deliberately deferred.

What was attempted and why it was reverted:

- A method-level intrinsic classification cache plus dispatch-path unification was implemented, benchmarked, and then reverted (`c557aef`) after regressions.
- Reverted-run Criterion change outputs (instrumented):
  - `json`: `+31.75%`
  - `generics`: `+21.60%`

Current workload signal:

- Instrumented confirmation runs on current code still show no intrinsic activity in these benchmark paths:
  - `json`: `intrinsic_call_total=0`
  - `generics`: `intrinsic_call_total=0`
- With zero intrinsic calls, these workloads are not valid for intrinsic fast-path optimization decisions.

Decision:

- Defer Step `2e` implementation work until intrinsic-heavy benchmark fixtures are added and baselined.
- Re-entry plan:
  1. Add intrinsic-heavy fixture(s) and capture non-zero intrinsic counters.
  2. Try lightweight dispatch-path deduplication first (without new registry caching).
  3. Add method-level intrinsic classification caching only if intrinsic-heavy measurements justify it.

---

## Phase 3: Memory Management and GC

### 3a. gc-arena tracing and cross-arena overhead

Status: implemented

Changes:

1. Added bench-instrumentation GC profiling counters and snapshot fields in `dotnet-metrics`:
   - STW pause sample quantiles (`p50/p95/p99/max`),
   - fixed-point cycle/iteration counters,
   - cross-arena object volume totals and per-iteration distribution,
   - named trace-root timing/count maps,
   - named layout-scan timing/count maps.
2. Instrumented fixed-point loop in `GCCoordinator::collect_all_arenas` to record:
   - iteration count per GC cycle,
   - cross-arena object volume per iteration.
3. Instrumented major mark-phase tracing roots in `threading/basic.rs`:
   - `mark_all.clear_cross_arena_roots`,
   - `mark_all.finish_marking`,
   - `mark_all.harvest_cross_arena_refs`,
   - `mark_objects.install_cross_arena_roots`,
   - `mark_objects.finish_marking`,
   - `mark_objects.harvest_cross_arena_refs`.
4. Instrumented high-cost layout traversal entry points in `dotnet-runtime-memory`:
   - `record_refs_recursive_with_recorder`,
   - `record_refs_in_range_with_recorder`.
5. Plumbed `bench-instrumentation` feature from `dotnet-vm` to `dotnet-runtime-memory` so memory access profiling is available in benchmark snapshots.

Measurements (`cargo bench -p dotnet-benchmarks --features bench-instrumentation --bench end_to_end -- gc --sample-size 10 --measurement-time 5 --warm-up-time 1`):

- Criterion time estimate: `598.04 ms` (CI `[595.89, 600.83]`).
- Phase-0 instrumented baseline median (`results.instrumented_default.gc.median_ns`): `608.923 ms`.
- Delta vs phase-0 instrumented baseline median: `-1.79%` (faster).
- New STW pause quantiles from `target/release/dotnet-bench-metrics/gc.json`:
  - `gc_pause_p50_us = 28`
  - `gc_pause_p95_us = 32`
  - `gc_pause_p99_us = 34`
  - `gc_pause_max_us = 34`
- Fixed-point/cross-arena profile:
  - `gc_fixed_point_cycle_count = 169`
  - `gc_fixed_point_iteration_total = 169`
  - `gc_fixed_point_max_iterations_per_cycle = 1`
  - `gc_fixed_point_cross_arena_objects_total = 1012`
  - `gc_fixed_point_cross_arena_objects_max_per_iteration = 8`
  - `gc_fixed_point_cross_arena_objects_by_iteration = { "1": 1012 }`
- Trace-root timing totals (ns) identified `mark_all.finish_marking` as dominant in this workload:
  - `mark_all.finish_marking = 568331ns`,
  - next highest `mark_all.harvest_cross_arena_refs = 71149ns`.
- Layout-scan timing maps were empty for this workload (`{}`), indicating these write-barrier traversal paths were not significant on this fixture.

Verification:

- `cargo clippy --all-targets -- -D warnings`
- `cargo test -- --nocapture`
- `cargo clippy --all-targets --no-default-features -- -D warnings`
- `cargo test --no-default-features -- --nocapture --test-threads=1`

Risk:

- High (unchanged): GC correctness and safety invariants. Current stress/integration suites remained green.

### 3b. Heap allocation patterns

Status: implemented

Changes:

1. Removed per-call local-value staging allocations in frame setup:
   - `VesContext::init_locals` now writes local defaults directly into evaluation stack slots and returns only `pinned_locals`.
   - `call_frame` / `entrypoint_frame` now consume this direct-initialization path, eliminating transient `Vec<StackValue>` allocation on each frame push.
2. Added reusable argument scratch buffering for hot call paths:
   - `ThreadContext` now carries `call_args_buffer: Vec<StackValue>`,
   - `constructor_frame`, tail-call dispatch, and `jmp` dispatch now reuse this buffer instead of allocating temporary argument vectors each call.
3. Reduced clone churn in local initialization context:
   - hoisted `ResolutionContext` creation out of the locals loop,
   - replaced repeated `GenericLookup` cloning in value-type-local initialization with targeted `method_generics` cloning only.
4. Added optional string interning experiment (`dotnet-value/src/string.rs`):
   - new env-gated interning path with bounded cache:
     - `DOTNET_STRING_INTERN_EXPERIMENT=1|true|yes|on` enables interning,
     - `DOTNET_STRING_INTERN_MAX_ENTRIES=<N>` controls cache size (default `4096`),
   - default behavior remains non-interned (`Owned(Vec<u16>)`) to avoid runtime overhead when the experiment is off.

Measurements (`cargo bench -p dotnet-benchmarks --features bench-instrumentation --bench end_to_end -- <workload> --sample-size 10 --measurement-time 5 --warm-up-time 1`):

- `json`:
  - Criterion median point estimate: `399.716 ms`
  - Phase-0 instrumented baseline median: `412.762 ms`
  - Delta vs phase-0 instrumented baseline: `-3.16%`
- `generics`:
  - Criterion median point estimate: `22.674 s`
  - Phase-0 instrumented baseline median: `23.229 s`
  - Delta vs phase-0 instrumented baseline: `-2.39%`

Allocation/counter deltas from bench snapshots:

- `json` (`target/release/dotnet-bench-metrics/json.json`) vs phase-0 instrumented:
  - `eval_stack_reallocations`: `4 -> 3` (`-25.0%`)
  - `eval_stack_pointer_fixup_total_ns`: `601 -> 672` (`+11.8%`)
  - `intrinsic_call_total`: still `0` (unchanged workload characteristic)
- `generics` (`target/release/dotnet-bench-metrics/generics.json`) vs phase-0 instrumented:
  - `eval_stack_reallocations`: `4 -> 3` (`-25.0%`)
  - `eval_stack_pointer_fixup_total_ns`: `841 -> 1213` (`+44.2%`)
  - `intrinsic_call_total`: still `0` (unchanged workload characteristic)

GC guardrails from Step `3a` stayed stable on these workloads:

- `json`: `gc_pause_p95_us=6`, `gc_pause_p99_us=7`
- `generics`: `gc_pause_p95_us=7`, `gc_pause_p99_us=9`

Verification:

- `cargo fmt`
- `cargo clippy --all-targets -- -D warnings`
- `cargo test -- --nocapture`
- `cargo clippy --all-targets --no-default-features -- -D warnings`
- `cargo test --no-default-features -- --nocapture --test-threads=1`

Risk:

- Medium (unchanged): call/frame setup and stack slot addressing remain sensitive; full default and no-default integration matrices passed after changes.

### 3c. Stack frame allocation

Evidence:

- `StackFrame` previously used `Vec` for both `exception_stack` and `pinned_locals`, and frame lifetimes dropped/reallocated these buffers on high-frequency call/return paths (`crates/dotnet-vm-data/src/stack.rs`, `crates/dotnet-vm/src/stack/context.rs`, `crates/dotnet-vm/src/stack/call_ops_impl.rs`).

Implemented:

1. Replaced per-frame vectors with `SmallVec`:
   - `exception_stack`: `SmallVec<[ObjectRef; 2]>`
   - `pinned_locals`: `SmallVec<[bool; 8]>`
2. Added per-thread bounded frame pooling in `FrameStack`:
   - new `pooled_frames` free-list,
   - `push_frame(...)` reuses pooled entries before allocating,
   - `recycle_frame(...)` clears GC-bearing fields and returns frames to pool (limit: `64`).
3. Wired lifecycle boundaries in all frame-pop paths:
   - normal returns/unwind (`context.rs`),
   - tail-call replacement and `jmp` frame replacement (`call_ops_impl.rs`).
4. Added bench-instrumentation counters for pool activity in `dotnet-metrics`:
   - `frame_pool_hit_count`
   - `frame_pool_miss_count`
   - `frame_pool_recycle_count`

Benchmark and allocator-activity results (`criterion`, `--features bench-instrumentation`, sample-size 10, measurement-time 5s, warm-up 1s):

- `dispatch` (call/return-heavy workload):
  - median point estimate: `1.120149 s`
  - phase-0 instrumented baseline median: `1.200718 s`
  - delta vs phase-0 instrumented baseline: `-6.71%`
  - frame-pool counters (`target/release/dotnet-bench-metrics/dispatch.json`):
    - `frame_pool_hit_count=200008`
    - `frame_pool_miss_count=3`
    - `frame_pool_recycle_count=200011`
    - hit ratio: `99.9985%`
- `generics` (high call-depth workload):
  - median point estimate: `22.432504 s`
  - phase-0 instrumented baseline median: `23.229219 s`
  - delta vs phase-0 instrumented baseline: `-3.43%`
  - frame-pool counters (`target/release/dotnet-bench-metrics/generics.json`):
    - `frame_pool_hit_count=2560008`
    - `frame_pool_miss_count=4`
    - `frame_pool_recycle_count=2560012`
    - hit ratio: `99.9998%`

Additional counters:

- `eval_stack_reallocations` reduced vs phase-0 instrumented baseline:
  - `dispatch`: `3 -> 2`
  - `generics`: `4 -> 3`
- GC pause guardrails remained in expected ranges for these runs:
  - `dispatch`: `gc_pause_p95_us=4`, `gc_pause_p99_us=4`
  - `generics`: `gc_pause_p95_us=7`, `gc_pause_p99_us=9`

Verification:

- `cargo fmt`
- `cargo clippy --all-targets -- -D warnings`
- `cargo test -- --nocapture`
- `cargo clippy --all-targets --no-default-features -- -D warnings`
- `cargo test --no-default-features -- --nocapture --test-threads=1`

Risk:

- Medium (unchanged): frame pooling adds lifecycle state transitions; validated by full default/no-default suites including exception fixtures and recursion-sensitive tail-call depth tests.

---

## Phase 4: Compiler Optimization Enablement

### 4a. Monomorphization and inlining

Implemented:

1. Captured a hot call graph using callgrind on the dispatch workload:
   - command: `valgrind --tool=callgrind target/release/deps/end_to_end-34bcb82a9827f24b dispatch --profile-time 3`
   - top VM boundaries from `callgrind_annotate`:
     - `dotnet_vm::dispatch::ExecutionEngine::step_normal` (`88.58%`)
     - `dotnet_vm::dispatch::registry::dispatch_monomorphic` (`16.72%`)
     - `dotnet_vm::stack::call_ops_impl::unified_dispatch` (`6.91%`)
     - `dotnet_vm::stack::resolution_ops_impl::current_context` (`5.51%`)
     - `dotnet_runtime_resolver::methods::find_generic_method` (`1.84%`)
     - `dotnet_runtime_resolver::methods::resolve_virtual_method` (`1.20%`)
2. Added targeted inline attributes on hot leaf wrappers only:
   - `crates/dotnet-vm/src/dispatch/mod.rs`
     - `InstructionRegistry::dispatch`: `#[inline(always)]`
     - `ExecutionEngine::step_batch_instruction`: `#[inline(always)]`
   - `crates/dotnet-runtime-resolver/src/methods.rs`
     - `is_delegate_type_in_hierarchy`, `is_intrinsic_cached`, `is_intrinsic_field_cached`, `locate_method`, `locate_field`: `#[inline]`
   - `crates/dotnet-vm/src/stack/ops.rs` was audited; no direct hot leaf methods in this file required annotation.
3. Added reproducible LTO benchmark profiles in workspace `Cargo.toml`:
   - `[profile.bench-thin]` (`lto = "thin"`)
   - `[profile.bench-fat]` (`lto = "fat"`, `codegen-units = 1`)

Benchmark results (`cargo bench -p dotnet-benchmarks --features bench-instrumentation --bench end_to_end`):

- Post-inline default profile:
  - `dispatch`: `1.1169 s` (from `[1.1111, 1.1230]`)
  - `generics`: `22.677 s` (from `[22.355, 22.998]`)
- LTO comparison versus post-inline default:
  - `bench-thin`:
    - `dispatch`: `1.0531 s` (`-5.71%`)
    - `generics`: `21.217 s` (`-6.44%`)
  - `bench-fat`:
    - `dispatch`: `1.0174 s` (`-8.91%`)
    - `generics`: `21.302 s` (`-6.06%`)
- Relative to Phase-0 instrumented baseline medians:
  - default: `dispatch -6.98%`, `generics -2.38%`
  - thin LTO: `dispatch -12.29%`, `generics -8.66%`
  - fat LTO: `dispatch -15.27%`, `generics -8.30%`

Binary size tradeoff (benchmark binary `end_to_end-*`):

- default (`target/release/deps`): `8,327,424` bytes
- thin (`target/bench-thin/deps`): `8,338,824` bytes (`+0.14%`)
- fat (`target/bench-fat/deps`): `6,107,528` bytes (`-26.66%`)

Build-time tradeoff (cold bench-profile compile from this run):

- thin: `12.20s`
- fat: `34.08s`

Verification:

- `cargo fmt`
- `cargo clippy --all-targets -- -D warnings`
- `cargo test -- --nocapture`
- `cargo clippy --all-targets --no-default-features -- -D warnings`
- `cargo test --no-default-features -- --nocapture --test-threads=1`

Risk:

- Medium (reduced): leaf inlining is bounded and validated; LTO wins are clear on runtime but increase compile cost (especially fat). Default profile behavior is unchanged.

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
- Medium risk: `1c`, `2b`, `2c`, `2d`, `2e` (deferred), `2f` (deferred), `3b`, `3c`, `4a`, `4c`
- Low/Medium risk: `4b`

Potential public API breakage:

- Current deliverables are documentation/tooling only: no API changes made.
- Future optimization steps likely to touch internal crate APIs and trait bounds; if any public API changes are required, document them explicitly in this file during implementation.

## Dependency Ordering (What unlocks what)

1. `0a` + `0b` (benchmark + instrumentation) unlock trustworthy measurement for all other phases.
2. `1a`/`1b` should precede `2a` because stack/fixup redesign depends on final byref/value representation.
3. `1c` can run in parallel with `2b` once validation constraints are pinned down.
4. `2a` is sufficient for current stack growth behavior; `2f` is deferred unless new evidence shows fixup as a meaningful bottleneck.
5. `2c` should precede any renewed `2e` attempt to keep cache-path baselines stable.
6. `3a` should run before deeper GC algorithm changes (`3b`/`3c`).
7. `4a` can proceed after `2d` + `3c`; `2e` is optional and only gates intrinsic-specific call-path tuning.
8. `4b`/`4c` should remain last after structural changes stabilize.

## Recommended Implementation Sequence

1. Phase 0 (`0a`, `0b`, `0c`)
2. `1a` -> `1b` -> `1c`
3. `2a` -> `2b` -> `2c` -> `2d` (completed); keep `2e`/`2f` deferred
4. `3a` -> `3b` -> `3c` (active next phase)
5. Optional side track: intrinsic-heavy benchmark fixture(s), then reassess `2e`
6. `4a` -> `4b` -> `4c`

This order minimizes rework and ensures each optimization is benchmarked against a stable baseline.

# Performance Trace Findings

> **Dated snapshot (2026-08-06), not a work queue.** [docs/plans/](plans/README.md) is the only
> queue. Each finding below carries its status at the current code; the method lessons are the
> durable part.

On 2026-08-06 the 17-case `end_to_end` set then in the suite was profiled with `perf` (`cycles`,
frame-pointer unwinding, 3,997 Hz, `profiling` Cargo profile, default features, so
`multithreading`). The suite now has more cases (the GC-handle trio, `heap_access`, `async_await`,
`exception_messages` and the resource cases), which this capture does not cover. The one known
warm-path regression since then, manifest-resource lookup behind `exception_messages`, is tracked
in the plans README ("Warm resource lookup").

`generics` ran roughly ten times longer per iteration than most fixtures, so a percentage win
there dominates suite totals.

## How to read the profiles

- **Self (leaf) shares** group stacks by their final frame; they are exclusive and can be added
  when categories do not overlap. **Inclusive shares** count a frame anywhere in the path; they
  overlap and must never be added to estimate a speedup.
- The folded stacks come from inline-expanded symbols, so an atomic or vector operation inlined
  into a wrapper such as `push` is attributed to the wrapper. Use source inspection to form a
  hypothesis and an A/B benchmark to assign causality.
- Do not subtract a leaf share to predict a speedup. Mechanisms interact (fewer collections and
  cheaper collections, for example); measure each change alone and together.
- Warm and cold performance are separate tracks (below). Never combine them into one score.

## Findings

| # | Finding | Status |
|---|---|---|
| 1 | Avoidable GC work in primitive-array workloads | Resolved |
| 2 | Repeated call resolution in `generics` and `linq_pipeline` | Resolved |
| 3 | Descriptor and generic `Arc` reference-count traffic | Open; re-measure first |
| 4 | Evaluation-stack push/pop is the interpreter floor | Open; re-measure first |
| 5 | Unicode-whitespace slow path in the string fixture | Open |

### 1. GC pressure and element-wise vector tracing (resolved)

Every evaluation-stack push charged allocation pressure, which drove constant collection
requests, and primitive arrays were traced element by element (GC leaves were about 70% of
`memory` and 64% of `unsafe_buffer`). Pressure is now charged only for value-type stack writes
(`account_value_type_write`), and `Vector::trace` returns early for element layouts that cannot
contain references (`is_or_contains_refs`). Under `bench-instrumentation`, allocation pressure is
counted per source (`AllocationPressureSource`).

### 2. Repeated call resolution (resolved)

`generics` (about 2.5M delegate calls per run) redid generic-method resolution for `callvirt`,
reclassified the delegate method and built two argument vectors per invocation. The `callvirt`
result now flows through the unified pipeline once, delegate classification is cached
(`delegate_dispatch_cache`), and `invoke_delegate` reuses the VM's argument buffer.
`linq_pipeline` rescanned overrides for every `constrained.` static call in the instruction
handler; `call_constrained` now goes through `resolve_static_constrained_method` with an
exact-key cache (`DOTNET_STATIC_CONSTRAINED_CACHE=0` disables it,
`DOTNET_STATIC_CONSTRAINED_SHADOW=1` compares cached and uncached results).

**Rejected design: instruction-offset inline caches.** A prototype that cached resolved
descriptors and guarded monomorphic virtual or constrained targets per instruction offset
improved some warm cases but produced wrong virtual targets, managed access violations and host
`SIGSEGV`s under EF Core. A resolved `MethodDescription` is not an executable target: dispatch
also depends on the caller lookup, method-spec and parent substitution, receiver- or
constraint-derived type arguments, per-ancestor substitution during virtual lookup, and rebinding
to the final declaring type's arity. Facade/CoreLib bridging and interface variance make
dispatch-slot compatibility broader than descriptor identity, so `Arc::ptr_eq` is not a semantic
key. Rules that follow:

- The VMT cache is the only virtual-target cache.
- Cache only exact constrained metadata outcomes, keyed by (call kind, constraint `ConcreteType`,
  base `MethodDescription`, exact source `GenericLookup`), and rebuild the dispatch lookup live.
- A cache test must compare the resolved method and the final dispatch lookup against the uncached
  path, covering interface variance, facade identity, generic method arguments, open, closed and
  multicast delegates, tail calls, constrained value and reference types, and default/static
  interface implementations.

### 3. Descriptor and generic `Arc` traffic (open)

Atomic reference-count operations on `Arc<MetadataArena>` (inside `ResolutionS`) and the
`Arc<[ConcreteType]>` slices of `GenericLookup` were 20–45% of self cycles across object,
framework and resolution-heavy fixtures (`linq_pipeline` 44.7%). EF Core, JSON and reflection
showed the same churn; the traces did not justify an EF-specific intrinsic or a
reflection-specific cache. These shares were measured before findings 1 and 2 were fixed;
re-baseline before acting.

Attack order: pass descriptors by reference where the callee does not retain them; move rather
than clone-then-drop into keys and results; avoid rebuilding equivalent `GenericLookup`s within
one instruction; reprofile with `cache_key_clone_total`. Only then consider compact identities
(loader-owned resolution IDs, interned type and lookup IDs). Any compact ID must preserve the
ownership invariant in [Type Resolution and Caching](TYPE_RESOLUTION_AND_CACHING.md): metadata
cannot be dropped while a descriptor can dereference it, so a bare ID needs an equally long-lived
owner.

Removing a cache trades clone and lock time for deep resolution; adding one does the reverse.
Evaluate total time, `Arc` self time, hit/miss rates, `cache_key_clone_total`, cache size and
cold first-use latency together.

### 4. Evaluation-stack baseline (open)

`arithmetic` spent about 93% of self time in `push` and `pop_safe`; it is the interpreter-floor
fixture. Push no longer charges allocation pressure or observes `Vec` capacity (frame homes made
pointer fixups unnecessary), so re-measure before acting. Untried ideas: split fallible and
infallible pops, and a reservation-aware push fast path (frames already reserve `max_stack`
slots). Do not add unchecked indexing until the verifier and frame-capacity invariants are
explicit and covered by malformed-IL tests. `runtime_primitives`
(`crates/dotnet-benchmarks/benches/runtime_primitives.rs`) has push/pop and vector baselines.

### 5. Unicode whitespace (open)

`String.IsNullOrWhiteSpace` (`intrinsic_string_is_null_or_white_space` and its async variant in
`crates/dotnet-intrinsics-string/src/search.rs`) has an ASCII fast path, but `all_whitespace`
falls back to `char::from_u32(..).is_whitespace()` per code unit. The `string` fixture
deliberately amplifies this (64 KiB of U+2003 checked 2,000 times, about 131M code units); do not
generalize from it.

## Warm and cold performance tracks

`end_to_end` builds one `BenchHarness` and `AssemblyLoader` per case and retains the prepared
fixture, so CoreLib and framework parsing happen once, in warmup, and are diluted in the trace and
excluded from Criterion's timing. Each run still creates a fresh `SharedGlobalState` and
`Executor`, so VM-owned caches do not persist across runs.

A small one-shot script is dominated by parsing CoreLib, process startup and first-use resolution;
hot-path caches may never repay their setup. Evaluate that with the cold track:
`cold_start/arithmetic` (the small-script floor), `cold_start/load_dominated` (framework working
set with almost no execution) and raw `metadata_load`.

Changes that affect both tracks must be measured on both: compact descriptor IDs (faster
execution, slower loader initialization), eager preparation (faster hot dispatch, slower small
scripts), larger caches (higher cold allocation and RSS) and persistent preparsed CoreLib state.
Prefer lazy, guarded preparation when a call site may run only once.

## Reproduction

Capture one fixture with the same profile shape (see [Benchmark Workflow](BENCHMARK_WORKFLOW.md)
for backends, symbol resolution and `bench.fish` corpus capture, which samples at a lower default
frequency):

```bash
./scripts/profile_perf.sh bench --name generics --sample-size 10 \
  --backend perf --event cycles --frequency 3997 --call-graph fp
```

Keep the backend, event, frequency, Cargo profile, features and call-graph mode identical across
candidates; compare unprofiled Criterion time first; then confirm the intended mechanism moved
and that time did not migrate into another resolution, allocation or GC path.

Cold-start comparisons:

```bash
cargo bench -p dotnet-benchmarks --bench cold_start -- 'cold_start/arithmetic/lazy'
cargo bench -p dotnet-benchmarks --bench cold_start -- 'cold_start/load_dominated/lazy'
cargo bench -p dotnet-benchmarks --bench metadata_load -- 'load_framework_set/lazy'
```

CoreLib parsing is a separate cold-start program; these traces deliberately warm the loader.

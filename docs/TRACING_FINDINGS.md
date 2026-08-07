# Performance Trace Findings

This document records the findings from the end-to-end performance traces captured on
2026-08-06 under:

```text
target/perf-traces/e2e-20260806-210407/
```

The capture set covers 17 Criterion fixtures and is intended to answer a specific question:
where does the runtime spend CPU cycles once a large managed workload is executing? It is not a
cold-start study. In particular, these traces do not measure the cost profile of a small one-shot
script whose elapsed time is dominated by parsing `System.Private.CoreLib`.

The short version is:

1. Primitive-array workloads are performing large amounts of avoidable GC work. Evaluation-stack
   traffic appears to create allocation pressure even for primitive values, and collection then
   scans every element of arrays whose layouts cannot contain managed references.
2. Generic, virtual, delegate, and constrained calls repeatedly redo resolution work. Some source
   and constrained metadata outcomes are stable only under an exact caller/constraint lookup;
   dynamic virtual and delegate targets are not generally call-site invariants. `generics` and
   `linq_pipeline` make the repeated work especially visible.
3. Atomic reference-count traffic for metadata arenas and generic argument slices consumes a large
   fraction of cycles across most object-, framework-, and resolution-heavy workloads.
4. Evaluation-stack push/pop bookkeeping is the dominant baseline cost for simple interpreted IL.
5. The string fixture exposes an expensive Unicode-whitespace fallback, but that opportunity is
   narrower and more fixture-sensitive than the first four.

The recommended initial work is not a descriptor representation rewrite. First validate the GC
pressure hypothesis, skip tracing for reference-free arrays, eliminate duplicate call resolution,
and centralize constrained resolution in the resolver's existing correctness-aware caches. Those
experiments are smaller, directly connected to the profiles, and will show how much descriptor
churn remains after redundant work is removed.

## Scope and capture configuration

Each fixture directory contains `perf.data`, symbolized scripts, a folded-stack file, an SVG
flamegraph, the Criterion log, a quality report, and a command manifest. The manifests consistently
report:

- backend: Linux `perf` 7.1.6-1;
- event: hardware `cycles`;
- frequency: 3,997 Hz;
- call graph: frame pointers;
- Cargo profile: `profiling`;
- Rust frame pointers forced with `-Cforce-frame-pointers=yes`;
- default crate features enabled, including the multithreading runtime path;
- kernel: Linux 7.1.5-arch1-2 x86_64.

The CPU model was not available in the manifest. Absolute times and the exact costs of atomics,
locks, allocator operations, and SIMD libc routines must therefore be treated as machine-specific.
The relative shapes within this capture set are still useful because the cases were recorded with
the same profiler configuration on the same machine.

### Trace quality

All 17 quality reports say the trace is dominated by resolved in-process work:

- empty stacks: 0%;
- foreign-process cycles: 0%;
- unresolved leaf weight: 0.0% or 0.1%;
- single-frame stacks: 0% after rounding;
- main benchmark thread: 100% of cycles after rounding.

This is a strong capture set for CPU-hot-path investigation. It does not, however, measure time
spent sleeping or blocked without consuming CPU, and it should not be used by itself to infer pause
latency, allocation volume, RSS, or scalability across cores.

### Benchmark duration and sample count

The middle value in each Criterion confidence interval and the recorded sample count were:

| Fixture | Approximate time per managed run | Perf samples |
|---|---:|---:|
| `gc_cross_arena` | 97.10 ms | 34,944 |
| `json` | 210.85 ms | 39,472 |
| `arithmetic` | 212.96 ms | 38,813 |
| `gc` | 254.41 ms | 36,695 |
| `json_dom` | 258.99 ms | 38,044 |
| `reflection` | 305.98 ms | 43,778 |
| `span_equality` | 335.41 ms | 48,163 |
| `span` | 335.76 ms | 48,033 |
| `alloc_throughput` | 355.99 ms | 50,947 |
| `dispatch` | 410.85 ms | 58,458 |
| `string` | 428.09 ms | 47,176 |
| `stack` | 455.86 ms | 49,690 |
| `unsafe_buffer` | 646.92 ms | 45,124 |
| `linq_pipeline` | 909.19 ms | 63,516 |
| `memory` | 1.024 s | 54,281 |
| `ef_inmemory` | 2.440 s | 129,144 |
| `generics` | 11.153 s | 491,004 |

`generics` is not merely the largest trace file. One managed run is more than four times slower
than `ef_inmemory` and more than ten times slower than `linq_pipeline`, so dispatch improvements in
that fixture can have substantial absolute value.

## How the percentages in this document were computed

The folded stacks are weighted by the final `cycles` value on each line.

- **Self/leaf share** groups each stack by its final frame. These shares are mutually exclusive and
  can be added when the categories do not overlap.
- **Inclusive share** counts a stack when a named frame occurs anywhere in that call path. Inclusive
  shares overlap and must never be added to estimate total speedup.
- Inline-expanded symbols were used because `perf.inline.script` is the source for the folded
  stacks. Inlining can attribute a machine instruction to an inlined wrapper such as `push` rather
  than to a lower-level atomic or vector operation. Source inspection is therefore used to form
  hypotheses, and an A/B benchmark is required before assigning causality.

The cross-workload category table below uses self/leaf shares. Its categories are:

- **stack**: `push` plus `pop_safe`;
- **metadata Arc**: clone/drop operations for `Arc<MetadataArena>` carried by `ResolutionS`;
- **generic Arc**: clone/drop operations for `Arc<[ConcreteType]>` used by `GenericLookup`;
- **GC scan**: the hot `trace<gc_arena::context::Context>` and `size` leaves; parent-path checks
  confirm that the large values in the buffer workloads are under vector tracing;
- **libc memory**: allocator and bulk-memory leaves including malloc/free/realloc internals,
  `memmove`, `memset`, `memcmp`, and their CPU-specific implementations;
- **hash**: `build_hasher` plus `probe_seq`;
- **locks**: `lock_shared` plus `unlock_shared`.

| Fixture | Stack | Metadata Arc | Generic Arc | GC scan | libc memory | Hash | Locks |
|---|---:|---:|---:|---:|---:|---:|---:|
| `alloc_throughput` | 13.6% | 24.9% | 11.8% | 0.0% | 5.5% | 2.9% | 3.5% |
| `arithmetic` | 92.9% | 0.0% | 0.0% | 0.0% | 0.1% | 0.0% | 0.0% |
| `dispatch` | 30.7% | 15.6% | 14.5% | 0.0% | 2.4% | 1.1% | 1.4% |
| `ef_inmemory` | 4.3% | 16.6% | 9.8% | 4.2% | 11.7% | 4.8% | 3.9% |
| `gc` | 13.8% | 22.6% | 12.4% | 0.0% | 6.0% | 3.3% | 3.5% |
| `gc_cross_arena` | 24.4% | 19.4% | 5.9% | 0.7% | 4.9% | 3.6% | 3.8% |
| `generics` | 16.2% | 17.2% | 10.8% | 0.0% | 12.0% | 1.7% | 3.7% |
| `json` | 14.7% | 20.2% | 16.7% | 0.0% | 4.6% | 0.8% | 2.7% |
| `json_dom` | 8.0% | 21.0% | 13.1% | 0.1% | 11.4% | 2.6% | 2.2% |
| `linq_pipeline` | 3.1% | 24.2% | 20.5% | 0.0% | 6.6% | 4.5% | 1.9% |
| `memory` | 21.9% | 0.1% | 0.1% | 69.8% | 1.2% | 0.0% | 0.9% |
| `reflection` | 11.1% | 19.6% | 14.0% | 0.0% | 12.0% | 2.0% | 3.5% |
| `span` | 31.4% | 0.7% | 0.6% | 52.9% | 1.8% | 0.2% | 2.1% |
| `span_equality` | 29.4% | 3.5% | 2.1% | 43.3% | 3.1% | 0.6% | 2.4% |
| `stack` | 59.5% | 6.9% | 5.2% | 0.0% | 2.1% | 0.3% | 0.1% |
| `string` | 20.0% | 0.6% | 0.5% | 23.4% | 0.8% | 0.0% | 3.4% |
| `unsafe_buffer` | 27.1% | 0.2% | 0.1% | 63.9% | 1.4% | 0.0% | 1.1% |

These categories do not explain every cycle. They intentionally isolate recurring mechanisms that
are both large and actionable.

## Finding 1: primitive workloads are doing avoidable GC work

### Evidence

The strongest repeated signal is vector tracing and layout-size computation:

| Fixture | GC `trace` + `size` self share | Relevant workload behavior |
|---|---:|---|
| `memory` | 69.8% | Two large byte arrays; hot loop performs copy/fill/probes |
| `unsafe_buffer` | 63.9% | Two large byte arrays; hot loop performs pointer copy/fill/probes |
| `span` | 52.9% | Large byte and char arrays; hot loop performs equality/probes |
| `span_equality` | 43.3% | Large byte and char arrays; hot loop performs equality/probes |
| `string` | 23.4% | Several large char arrays remain live during string operations |

The hot loops in `memory` and `unsafe_buffer` create no managed objects after their initial arrays.
Nevertheless, `memory` has approximately 70.9% inclusive weight under
`collect_all_arenas`, and `unsafe_buffer` has a similarly GC-dominated leaf profile. That makes a
normal allocation-rate explanation implausible and points to the runtime's pressure accounting.

### Likely trigger: every evaluation-stack push records an allocation

With default features, `EvalStackOps::push` calls:

```rust
self.gc.record_allocation(value.size_bytes());
```

for every `StackValue`, including `Int32`, `Int64`, native integers, object references already
allocated elsewhere, and transient values moved through the interpreter stack. The relevant code
is in
[`crates/dotnet-vm/src/stack/stack_ops_impl.rs`](../crates/dotnet-vm/src/stack/stack_ops_impl.rs).

`ArenaHandleInner::record_allocation` then performs an atomic increment, updates the peak with an
atomic compare/exchange loop, and requests collection when the counter crosses the configured
threshold. See
[`crates/dotnet-utils/src/gc/arena.rs`](../crates/dotnet-utils/src/gc/arena.rs).

An evaluation-stack push is not generally a managed heap allocation. A value-type clone may own
storage and some stack operations may indirectly allocate, so the call cannot be removed blindly.
The current unconditional accounting is nevertheless broad enough to explain why primitive-only
loops repeatedly request collection.

### Likely amplification: primitive arrays are traced element by element

`Vector::trace` has a specialized loop for `ObjectRef` arrays. Every other element layout takes a
generic loop:

```rust
for i in 0..self.layout.length {
    LayoutManager::trace(
        element,
        &self.storage[(element.size() * i).as_usize()..],
        cc,
    );
}
```

See
[`crates/dotnet-value/src/object/types.rs`](../crates/dotnet-value/src/object/types.rs).
For byte, char, integer, and floating-point elements, `LayoutManager::trace` reaches a no-op match
arm, but only after per-element indexing, size dispatch, slicing, and calls. Parent aggregation for
the `trace` and `size` leaves confirms that this vector path owns the dominant weights in the
buffer cases.

### Proposed experiments

Run these as separate changes so their effects remain attributable:

1. **Audit allocation-pressure ownership.** Enumerate every `record_allocation` call and define
   whether the counter means managed arena bytes, Rust backing allocations, copied value bytes, or
   a conservative mutation budget. The current comments describe bytes allocated since the last
   collection, which does not match unconditional stack traffic.
2. **Restrict stack accounting by value kind.** As an experiment, do not charge primitive values or
   already-existing object references on push. Preserve charging at actual object/vector/string
   creation sites and at value-storage clones that allocate.
3. **Add per-kind instrumentation.** Record allocation-pressure bytes and call counts by source:
   object allocation, vector allocation, string allocation, value-type clone, local/argument write,
   and evaluation-stack push. This will prevent a speedup from hiding an undercount.
4. **Skip reference-free vectors.** Return immediately from `Vector::trace` when
   `!element.has_managed_ptrs()`.
5. **Hoist stable layout values.** Compute `element.size()` once outside loops that genuinely must
   trace references or nested layouts.

### Validation and success criteria

Reprofile at least `memory`, `unsafe_buffer`, `span`, `span_equality`, `string`, `gc`, and
`gc_cross_arena`.

Success would look like:

- collection-trigger counts in allocation-free hot loops falling to zero or close to setup-only
  activity;
- the `trace`/`size` leaf share collapsing in primitive-array fixtures;
- lower `push` self time from fewer atomics;
- no reduction in collection frequency for actual `ObjectRef`, string, vector, boxed value, and
  value-type allocations;
- unchanged cross-arena reachability, finalization, resurrection, and managed-pointer tests;
- no new failures under `memory-validation` and multithreading configurations.

Do not estimate the speedup by subtracting 69.8% from `memory`. The pressure-accounting and tracing
costs interact: fewer requests prevent whole collections, while a trace fast path makes collections
cheaper. Measure them independently and together.

## Finding 2: call sites repeatedly redo generic and dispatch resolution

### `generics`: a hot delegate call is treated like a cold call millions of times

The fixture performs two 63-operation folds for each of 20,000 iterations, or approximately
2,520,000 delegate invocations per managed run. Its inclusive call-path shares include:

| Inclusive frame | Cycle share |
|---|---:|
| `dispatch_callvirt` | 75.5% |
| `try_delegate_dispatch` | 32.4% |
| `invoke_delegate` | 25.2% |
| `find_generic_method` | 18.0% |

These values overlap. They describe the same nested call paths rather than four independent
speedups.

There are three concrete sources of repeated work.

#### Duplicate generic-method resolution in `callvirt`

`dispatch_callvirt` calls `find_generic_method` to obtain the parameter count and identify the
receiver. It then calls `unified_dispatch` or `unified_dispatch_tail`, whose common path calls
`find_generic_method` again before virtual resolution. See:

- [`crates/dotnet-vm/src/instructions/calls.rs`](../crates/dotnet-vm/src/instructions/calls.rs)
- [`crates/dotnet-vm/src/stack/call_ops_impl.rs`](../crates/dotnet-vm/src/stack/call_ops_impl.rs)

The first resolved method and lookup should be passed into the common path. Phase 2 implemented
that direct handoff; no call-site target cache is required to remove this duplicate work.

#### Delegate classification walks metadata hierarchy on every invocation

`try_delegate_dispatch` checks for a body, clones the parent descriptor, creates canonical type
names, and walks ancestors to determine whether the method belongs to a delegate. See
[`crates/dotnet-intrinsics-delegates/src/helpers.rs`](../crates/dotnet-intrinsics-delegates/src/helpers.rs).

For a stable resolved `Invoke` method, this classification is invariant. It should be represented
by a cached dispatch kind or by prepared method metadata rather than recomputed for every call.
The profile supports this directly: `format_inner` self time in `generics` is mostly below
`nested_type_name`/`type_name`, and the ancestor iterator is a major parent of the hot
`TypeDescription` expectation path.

#### Delegate argument handling allocates repeatedly

`invoke_delegate` calls `pop_multiple`, which creates a new `Vec`, and the non-multicast path then
creates another vector from `args[1..].to_vec()` before constructing `PreparedCall`. See
[`crates/dotnet-intrinsics-delegates/src/invoke.rs`](../crates/dotnet-intrinsics-delegates/src/invoke.rs).

The 12.0% libc-memory self category in `generics` is consistent with significant allocation/free
traffic, although the trace alone cannot assign all of that category to argument vectors. The VM
already has reusable call-argument buffering in other call paths; delegate invocation should be
tested with the same strategy or with a small inline buffer.

### `linq_pipeline`: constrained calls repeatedly search overrides

`linq_pipeline` takes about 909 ms per managed run. Its important inclusive shares are:

| Inclusive frame | Cycle share |
|---|---:|
| `call_constrained` | 39.3% |
| `locate_method` | 32.1% |
| `dispatch_callvirt` | 21.3% |

`call_constrained` reconstructs the constraint type, resolves the source method, resolves the
concrete type, then iterates every override. For each override it locates both the implementation
and declaration before comparing the declaration with the requested method. This happens during
execution of the instruction rather than once per stable constraint/source/generic combination.
See
[`crates/dotnet-vm/src/instructions/calls.rs`](../crates/dotnet-vm/src/instructions/calls.rs).

### Prepared call sites and inline caches

`MethodInfo` currently retains the decoded metadata `Instruction` slice directly. It does not own a
prepared instruction stream or per-instruction resolution sidecar. See
[`crates/dotnet-vm/src/lib.rs`](../crates/dotnet-vm/src/lib.rs) and
[`crates/dotnet-vm-data/src/lib.rs`](../crates/dotnet-vm-data/src/lib.rs).

That architecture forces hot instructions to keep presenting metadata tokens and generic context
to general resolver APIs. Existing global caches reduce deep lookup work, but their hits still
require descriptor construction/cloning, hashing, locking, and result cloning.

A Phase 3 prototype tested caching resolved descriptors and guarded monomorphic virtual/
constrained targets at an instruction offset. It is not a viable design. Although some warm cases
improved, EF Core exposed incorrect virtual targets, managed access violations and overflows, and
host `SIGSEGV`s. Structural descriptor equality, `Arc` identity guards for generic slices, and
fresh target resolution did not make the design correct because the cached value omitted semantic
state that changes later in the dispatch pipeline.

In particular, a resolved `MethodDescription` is not a complete executable target. Correct
dispatch depends on the method together with the lookup and route produced by several distinct
stages:

1. the caller frame lookup;
2. method-spec and referenced-parent substitution in `find_generic_method`;
3. constraint- or receiver-derived type arguments;
4. per-ancestor substitution during virtual lookup;
5. rebinding to the final method's declaring-type arity.

Facade/CoreLib bridging and interface variance add a second identity domain: exact descriptor
identity is pointer/index/structural-generic identity, while dispatch-slot compatibility can be
broader and is established by canonical-name and substituted-signature comparison. `Arc::ptr_eq`
is therefore neither a semantic cache key nor a substitute for the missing lookup transformations.

The revised staged design is:

1. **Eliminate known duplicate resolution.** Pass the first `callvirt` result through the unified
   pipeline.
2. **Cache delegate dispatch classification.** Key by resolved `MethodDescription`, or store the
   classification on cached `MethodInfo`.
3. **Reuse delegate argument storage.** Avoid the two short-lived vectors per invocation.
4. **Route static constrained resolution through the resolver.** Reuse the existing override map,
   facade bridge, exact/variance signature comparison, and default-interface precedence instead of
   scanning `TypeDefinition::overrides` in the instruction handler.
5. **Keep virtual targets in the existing VMT cache.** Its semantic input is the exact base method,
   runtime receiver definition, and receiver-merged structural lookup. Do not add an instruction-
   local virtual target cache.
6. **Cache only exact constrained metadata outcomes if profiles still justify it.** A safe key is
   `(call kind, constraint ConcreteType, resolved base MethodDescription, exact source
   GenericLookup)`. The cached value may identify an explicit MethodImpl, direct implementation,
   or default interface body, but the route-specific dispatch lookup must still be constructed by
   the live pipeline.
7. **Consider source preparation only after the resolver change is measured.** Any later sidecar
   must be owned by the exact `MethodInfo` specialization keyed by caller method plus caller lookup;
   it may retain argument count and resolved source facts, but not receiver objects, managed
   pointers, delegate targets, tail-call eligibility, or dynamic virtual targets.
8. **Consider a prepared instruction representation last.** Converting the whole interpreter
   instruction format remains a larger compatibility and memory-cost decision.

Correctness tests must cover interface variance, facade/CoreLib identity bridging, generic method
arguments, open and closed delegate types, multicast delegates, tail calls, constrained value
types (including boxing fallback), constrained reference types with multiple runtime subclasses,
and default/static interface implementations. Cache tests must compare both the resolved method
and final dispatch lookup against the uncached path before the cached result is allowed to execute.

## Finding 3: descriptor and generic `Arc` traffic is a broad CPU cost

### Evidence across workloads

The combined self shares of metadata-arena and generic-slice clone/drop operations are:

| Fixture | Combined descriptor/generic Arc self share |
|---|---:|
| `linq_pipeline` | 44.7% |
| `json` | 36.9% |
| `alloc_throughput` | 36.7% |
| `gc` | 35.0% |
| `json_dom` | 34.1% |
| `reflection` | 33.6% |
| `dispatch` | 30.1% |
| `generics` | 28.0% |
| `ef_inmemory` | 26.4% |
| `gc_cross_arena` | 25.3% |

These are self shares in atomic reference-count operations, not inclusive time in all descriptor
handling. They are large enough that even cache hits can be expensive.

### Why cloning is not cheap in the current representation

The ownership model is deliberate:

- `ResolutionS` stores `(Arc<MetadataArena>, NonNull<Resolution<'static>>)`;
- `TypeDescription` contains a `ResolutionS`;
- `ConcreteType` contains a `ResolutionS` and an `Arc<BaseType<ConcreteType>>`;
- `GenericLookup` contains two `Arc<[ConcreteType]>` slices;
- method, field, layout, hierarchy, and virtual-dispatch cache keys compose these descriptors.

See
[`crates/dotnet-types/src/resolution.rs`](../crates/dotnet-types/src/resolution.rs),
[`crates/dotnet-types/src/generics.rs`](../crates/dotnet-types/src/generics.rs), and
[`docs/TYPE_RESOLUTION_AND_CACHING.md`](TYPE_RESOLUTION_AND_CACHING.md).

Memoized hashes avoid recursively hashing generic type trees, but they do not remove reference
count increments/decrements while constructing cache keys and returned descriptors. The profiles
show that clone/drop traffic, rather than deep hashing alone, is now a primary cost.

### Recommended order of attack

Start with call-site and API-level reductions before changing ownership:

1. Pass descriptors by reference where the callee does not retain them.
2. Move descriptors into keys/results when ownership is already available instead of cloning then
   dropping the original.
3. Avoid rebuilding equivalent `GenericLookup` values while dispatching one instruction.
4. Use centralized constrained resolution first; consider exact-`MethodInfo` source preparation
   only if repeated general cache-key construction remains material afterward.
5. Reprofile with the existing `cache_key_clone_total` instrumentation to distinguish fewer
   logical clones from faster clone representation.

If Arc traffic remains dominant after redundant work is removed, investigate compact identities:

- loader-owned `ResolutionId`/arena tables with one lifetime root per loader or executor;
- interned `ConcreteTypeId` and `GenericLookupId` values that are cheap to copy and hash;
- descriptor handles that separate non-owning hot-path identity from an owning boundary object;
- cache entries keyed by stable numeric identities rather than composite owner-carrying values.

Any compact-ID design must preserve the current safety invariant: metadata cannot be dropped while
a descriptor can dereference it. The existing type-resolution documentation correctly notes that
a bare ID is insufficient unless an equally long-lived owner retains its arena. This is a safety
and architecture project, not a mechanical `Arc` removal.

### Measurement caution

Removing a cache can reduce clone and lock time while increasing deep resolution work. Conversely,
adding another cache can reduce resolution but increase key clones. Evaluate:

- total benchmark time;
- self time in metadata/generic Arc clone/drop;
- cache hit/miss rates;
- `cache_key_clone_total`;
- peak cache entries and estimated cache memory;
- cold first-use latency as well as hot repeated latency.

## Finding 4: evaluation-stack operations define the interpreter baseline

### Evidence

`push` plus `pop_safe` self shares are:

| Fixture | Stack self share |
|---|---:|
| `arithmetic` | 92.9% |
| `stack` | 59.5% |
| `span` | 31.4% |
| `dispatch` | 30.7% |
| `span_equality` | 29.4% |
| `unsafe_buffer` | 27.1% |
| `gc_cross_arena` | 24.4% |
| `memory` | 21.9% |
| `string` | 20.0% |

For `arithmetic`, the top three leaves are `push` at 49.0%, `pop_safe` at 43.9%, and
`step_batch_instruction` at 4.6%. This is a useful lower-bound workload: there is little metadata,
allocation, object layout, or framework behavior available to hide basic interpreter mechanics.

### Work performed on every push/pop

The stack path currently includes several responsibilities:

- allocation-pressure accounting on push under multithreading;
- a disabled/enabled tracer branch on both push and pop;
- `Vec` capacity observation on push so managed pointers can be fixed after reallocation;
- frame `stack_height` increment/decrement;
- checked subtraction for frame underflow;
- a fallible `Vec::pop` conversion to `VmError`;
- the nominally infallible trait `pop` calling `pop_safe` and then `expect`.

The VM already reserves local slots plus the method's declared `max_stack` when a frame is entered,
which means most pushes in valid IL should not reallocate. See
[`crates/dotnet-vm-data/src/stack.rs`](../crates/dotnet-vm-data/src/stack.rs) and
[`crates/dotnet-vm/src/stack/call_ops_impl.rs`](../crates/dotnet-vm/src/stack/call_ops_impl.rs).

### Investigation sequence

The GC-accounting experiment must come first because its atomics are inlined under `push` and can
inflate both stack cost and collection frequency. After that:

1. Add a microbenchmark for primitive `push_i32`/`pop_i32` under the production feature set.
2. Split infallible and fallible pop implementations so verified instruction paths do not perform
   two layers of error construction/checking.
3. Determine whether frame height and vector length can share one validated invariant rather than
   being checked separately on every pop.
4. Consider a push path that assumes frame reservation is sufficient, with a checked slow path for
   malformed metadata, unusual runtime pushes, or underestimated `max_stack`.
5. Measure the disabled tracer branch separately. Do not remove diagnostics speculatively; the
   `enabled_emit` closure already avoids formatting when disabled.
6. Inspect `StackValue` size and movement cost. A large enum moved through a `Vec` can dominate even
   after bookkeeping is reduced.

Unsafe indexing or unchecked length manipulation should only be considered after the verifier and
frame-capacity invariants are explicit and covered by malformed-IL tests. The current runtime also
supports fuzzing and memory-validation modes, so a small branch reduction is not worth weakening
those boundaries without measured benefit.

## Finding 5: the string fixture exposes a Unicode-whitespace slow path

The largest string leaf shares are:

| Leaf | Self share |
|---|---:|
| Unicode table `lookup` | 19.1% |
| `char_try_from_u32` | 12.8% |
| vector GC `trace` | 12.1% |
| layout `size` | 11.3% |
| evaluation-stack push/pop | 20.0% combined |

The fixture creates a 64 KiB string containing U+2003 EM SPACE and calls
`String.IsNullOrWhiteSpace` 2,000 times. The ASCII SIMD probe returns no answer for that input, and
the fallback converts every UTF-16 code unit with `char::from_u32` before calling Rust's general
Unicode `is_whitespace` lookup. See
[`crates/dotnet-intrinsics-string/src/search.rs`](../crates/dotnet-intrinsics-string/src/search.rs).

The .NET whitespace property can be implemented directly over UTF-16 code units with a compact set
of exact values and ranges. A fast path for common non-ASCII whitespace, including U+2000 through
U+200A, would avoid conversion and the general Unicode-property table. A vectorized range test may
be worthwhile after a scalar direct classifier is measured.

This is lower priority than the GC, dispatch, and Arc findings because the fixture intentionally
amplifies one Unicode case over roughly 131 million code units. It is still a legitimate runtime
operation and a good focused benchmark, but its speedup should not be generalized to arbitrary
applications.

The same profile also contains primitive-char-array GC cost. Fix the reference-free vector tracing
path before judging the isolated string-intrinsic improvement.

## Framework-heavy workload notes

### `ef_inmemory`

`ef_inmemory` has no single exclusive leaf comparable to primitive-array tracing. Its shape is a
combination of:

- 26.4% metadata/generic Arc clone/drop self time;
- 11.7% libc memory/allocator self time;
- 4.8% hash probing/building self time;
- 3.9% shared-lock/unlock self time;
- approximately 23.1% inclusive `dispatch_callvirt`;
- approximately 19.5% inclusive arena collection.

This workload should benefit from general prepared dispatch, reduced key cloning, and lower
allocation churn. The trace does not support prioritizing an EF-specific intrinsic or special case.

### `json` and `json_dom`

`json` spends 36.9% of leaf cycles in the two Arc categories, while `json_dom` spends 34.1% there
and 11.4% in libc memory operations. These workloads reinforce that owner-carrying descriptors and
generic lookup cloning affect ordinary framework execution, not only the synthetic generic test.

### `reflection`

`reflection` spends 33.6% of leaf cycles in the Arc categories and 12.0% in libc memory operations.
Reflection naturally creates descriptor-rich result objects, so some churn is workload-required.
Prepared resolution and cheaper identities should be tested before adding reflection-specific
caches that could retain large metadata graphs.

### `alloc_throughput` and `gc`

The allocation-focused fixtures are not dominated by the collector's tracing leaf. Instead,
`alloc_throughput` spends 36.7% and `gc` spends 35.0% in descriptor/generic Arc clone/drop. Field
stores, object construction, method calls, and layout queries still travel through descriptor-rich
paths for each managed allocation. This suggests that preparing resolved field/layout operands may
matter as much as tuning the arena allocator itself.

## Large warm fixtures versus small corlib-dominated scripts

This distinction is essential when turning the findings into product priorities.

### What the end-to-end traces measure

The Criterion end-to-end benchmark creates one `BenchHarness` and one `AssemblyLoader` for the
case. The prepared fixture is retained by the benchmark closure. Criterion warmup invokes the
managed workload before measurement, so `System.Private.CoreLib` and framework assembly parsing
land once in warmup and the loader reuses them during the later repetitions.

There are two different warmups to distinguish:

- `profile_perf.sh` first executes an **untraced process** to build the C# fixture DLL and warm the
  filesystem cache. Its in-memory loader state cannot carry into the traced process.
- The traced Criterion process then performs its normal three-second **Criterion warmup** using the
  same loader that it will use for measurement.

Because `perf record` starts with the Criterion process, the trace technically includes process
startup, the first corlib parse, and Criterion warmup. Corlib parsing occurs only once, however,
whereas the large managed workload runs repeatedly for warmup and measurement. The one-time parse
is therefore heavily diluted in the flamegraph and excluded from Criterion's reported per-run
timing interval. A cold-start benchmark deliberately pays that parse on every measured iteration.

Each `run_prepared_with_metrics` still creates a fresh `SharedGlobalState` and `Executor`, so these
are not persistent-process throughput measurements with all VM-owned caches retained forever.
Within each managed run, however, the large fixtures execute loops ranging from thousands to
millions of operations. Setup and one-time parse work are strongly amortized, and the traces expose
interpreter, dispatch, GC, descriptor, and intrinsic costs.

The intended benchmark semantics are documented in
[`crates/dotnet-benchmarks/benches/cold_start.rs`](../crates/dotnet-benchmarks/benches/cold_start.rs)
and
[`docs/BENCHMARK_WORKFLOW.md`](BENCHMARK_WORKFLOW.md).

### What a small script measures

A small one-shot script usually starts a new process and loader, parses the approximately 12 MiB
corlib, resolves its first set of types and methods, executes relatively little IL, then exits. In
that regime:

- corlib and dependency parsing can be the majority of elapsed time;
- process startup, dynamic linking, file I/O, and teardown are visible;
- hot call-site caches may never receive enough hits to repay their initialization or memory cost;
- a 50% improvement to a 10% execution component improves end-to-end latency by only 5%;
- a modest metadata-load improvement can be more valuable than a large interpreter-loop win.

The existing framework-set metadata benchmark reports about 28.1 ms with four Rayon threads on a
24-core machine, versus 34.5 ms with one thread and 32.5 ms with 24 threads. That already shows
that metadata work has its own optimization shape: moderate parallelism helps, while excessive
fork/join and work-stealing overhead hurts.

### Maintain two performance tracks

Do not combine warm and cold numbers into one score.

#### Warm execution track

Use `end_to_end` and these trace fixtures to evaluate:

- evaluation-stack throughput;
- GC trigger accuracy and tracing cost;
- method/field/type resolution after first load;
- virtual, generic, constrained, and delegate dispatch;
- descriptor and cache-key ownership cost;
- framework intrinsics and bulk memory/string operations.

#### Cold one-shot track

Use `cold_start` and `metadata_load` to evaluate:

- corlib parse and decode time;
- lazy versus eager method-body/signature decoding;
- framework dependency discovery and loading;
- file I/O and mapping strategy;
- Rayon pool sizing and scheduling overhead;
- reusable, serialized, mapped, or process-shared metadata representations;
- the memory cost and invalidation rules of any pre-parsed corlib cache.

For cold work, profile `cold_start/arithmetic`, `cold_start/load_dominated`, and raw
`metadata_load` separately. `cold_start/arithmetic` shows the small-script floor, while
`load_dominated` isolates a broader framework working set with almost no execution.

### Cross-track changes

Some changes affect both tracks and must be evaluated twice:

- compact descriptor identities may speed execution but increase loader initialization;
- eager preparation may improve hot dispatch while making small scripts slower;
- larger caches may improve framework loops while increasing cold allocation and RSS;
- persistent/preparsed corlib state may radically improve cold start without changing any flame in
  this capture set;
- lazy preparation or first-hit inline caches can preserve cold behavior while optimizing repeated
  execution.

Prefer lazy, guarded preparation when the warm benefit is large and the small-script path may use a
call site only once.

## Prioritized investigation plan

### P0: correct allocation pressure and skip reference-free vector tracing

1. Add source/kind counters around allocation-pressure updates.
2. Stop charging primitive evaluation-stack movement in an experimental branch.
3. Add the `has_managed_ptrs` vector trace fast path.
4. Reprofile the five primitive-array/string cases plus real allocation and cross-arena cases.
5. Retain the change only if collection metrics remain correct for actual managed allocations.

This is the highest-confidence opportunity because it connects allocation-free fixture source,
the unconditional push accounting, the collection threshold, and the exact vector trace loop seen
in the profiles.

### P0: remove duplicate resolution and repeated delegate work

1. Pass the first `callvirt` resolution result into unified dispatch.
2. Cache delegate method classification.
3. Reuse delegate argument storage.
4. Reprofile `generics`, `dispatch`, `json`, `reflection`, and `ef_inmemory`.

Track absolute time carefully: `generics` is so much longer than the rest that even a modest
percentage improvement may dominate the total suite improvement.

### P1: centralize and cache constrained resolution safely

1. Add missing correctness fixtures and a shadow mode that compares cached and uncached method +
   final-lookup results.
2. Move `call_constrained` resolution behind a resolver API that reuses the existing override map,
   facade bridge, variance rules, and default-interface precedence.
3. Retain the existing VMT cache as the only virtual-target cache. If constrained direct-target
   caching remains useful, key it by call kind, concrete constraint, exact resolved base method,
   and exact structural source lookup.
4. Make first use run the current resolver path and cache only a successful result. Cache entries
   own their descriptors and are scoped to the loader-owned `SharedGlobalState`; eviction drops
   those owners safely. Do not cache resolution errors. A future unload/hot-reload design requires
   a loader epoch.
5. Measure hit rate, guard/shadow mismatches, polymorphism, key-clone count, cache size, retained
   descriptor memory, and cold first-use overhead.
6. Reprofile warm `linq_pipeline`, `generics`, `dispatch`, and `ef_inmemory`; separately run
   `cold_start/arithmetic/lazy` so warm wins are not conflated with cold-start cost.

### P1: reduce descriptor and generic lookup clones

1. Use existing clone instrumentation to identify the highest-volume API boundaries.
2. Convert non-retaining APIs to references and ownership-consuming APIs to moves.
3. Remove lookup reconstruction made redundant by centralized constrained resolution and any
   later, independently justified source preparation.
4. Reprofile before designing interned IDs.

### P1: simplify the verified evaluation-stack fast path

1. Establish a stable primitive push/pop microbenchmark.
2. Reprofile after allocation accounting is corrected.
3. Split fallible and infallible pop paths.
4. Test a reservation-aware push fast path.

### P2: optimize the Unicode whitespace classifier

1. Implement a direct UTF-16 whitespace predicate with exact .NET semantics.
2. Compare scalar direct, range-specialized, and SIMD implementations.
3. Measure ASCII, mixed ASCII, U+2003, other whitespace ranges, surrogate code units, and early
   non-whitespace exits.

## Per-fixture top exclusive leaves

The following appendix preserves the most visible leaf-level evidence. Names are shortened for
readability; percentages are self shares.

| Fixture | First leaf | Second leaf | Third leaf |
|---|---|---|---|
| `alloc_throughput` | metadata Arc clone 13.4% | metadata Arc drop 11.5% | `push` 7.4% |
| `arithmetic` | `push` 49.0% | `pop_safe` 43.9% | `step_batch_instruction` 4.6% |
| `dispatch` | `push` 16.6% | `pop_safe` 14.1% | metadata Arc clone 8.0% |
| `ef_inmemory` | metadata Arc clone 8.6% | metadata Arc drop 8.1% | generic Arc drop 5.0% |
| `gc` | metadata Arc clone 11.8% | metadata Arc drop 10.8% | `push` 7.3% |
| `gc_cross_arena` | `push` 12.6% | `pop_safe` 11.8% | metadata Arc clone 11.7% |
| `generics` | `push` 9.2% | metadata Arc clone 9.1% | metadata Arc drop 8.2% |
| `json` | metadata Arc drop 10.6% | metadata Arc clone 9.6% | generic Arc drop 9.1% |
| `json_dom` | metadata Arc drop 10.8% | metadata Arc clone 10.2% | generic Arc drop 6.7% |
| `linq_pipeline` | metadata Arc drop 13.6% | generic Arc drop 10.6% | metadata Arc clone 10.6% |
| `memory` | GC `trace` 36.4% | layout `size` 33.4% | `push` 11.8% |
| `reflection` | metadata Arc clone 10.3% | metadata Arc drop 9.3% | generic Arc drop 7.2% |
| `span` | GC `trace` 44.5% | `push` 17.4% | `pop_safe` 14.0% |
| `span_equality` | GC `trace` 36.0% | `push` 16.3% | `pop_safe` 13.1% |
| `stack` | `push` 32.6% | `pop_safe` 26.9% | metadata Arc drop 4.1% |
| `string` | Unicode `lookup` 19.1% | `char_try_from_u32` 12.8% | GC `trace` 12.1% |
| `unsafe_buffer` | GC `trace` 53.4% | `push` 14.6% | `pop_safe` 12.4% |

## Reproduction and comparison guidance

Capture one fixture with the same steady-state profile shape using:

```bash
./scripts/profile_perf.sh bench --name generics --sample-size 10 \
  --backend perf --event cycles --frequency 3997 --call-graph fp
```

For optimization comparisons:

1. Keep the profiler backend, event, frequency, Cargo profile, feature set, and call-graph mode
   identical.
2. Retain `command.txt`, `record.log`, and `quality.txt` with every candidate trace.
3. Compare Criterion time without `perf` first; sampling overhead can alter allocator and lock
   behavior.
4. Run enough unprofiled samples to distinguish a change from machine noise.
5. Then compare leaf and inclusive profile shape to confirm the intended mechanism moved.
6. Check that removed time did not migrate into a different resolution, allocation, or GC path.
7. Record runtime metrics such as collection triggers, pauses, cache hits/misses, cache sizes, and
   clone counters alongside elapsed time.

For cold-start comparisons, use the dedicated targets instead:

```bash
cargo bench -p dotnet-benchmarks --bench cold_start -- 'cold_start/arithmetic/lazy'
cargo bench -p dotnet-benchmarks --bench cold_start -- 'cold_start/load_dominated/lazy'
cargo bench -p dotnet-benchmarks --bench metadata_load -- 'load_framework_set/lazy'
```

Do not infer cold-start improvement from a reduction in these end-to-end flamegraphs. Conversely,
do not reject a warm-runtime optimization solely because it leaves the corlib-dominated small
script floor unchanged. They are different performance budgets and require different evidence.

## Conclusions

The capture set exposes two different layers of overhead.

At the mechanism level, primitive stack traffic appears to request collection, primitive vectors
are then scanned pointlessly, hot call sites redo stable resolution, and descriptors pay frequent
atomic ownership costs. At the workload level, those mechanisms combine differently: arithmetic
isolates stack mechanics, memory/span isolate GC behavior, generics isolates delegate/virtual
dispatch, LINQ isolates constrained resolution, and EF/JSON/reflection show that descriptor churn
persists in realistic framework code.

The highest-confidence sequence is:

1. correct allocation-pressure accounting;
2. skip tracing arrays that cannot contain managed references;
3. eliminate duplicate `callvirt` resolution and repeated delegate classification/allocation;
4. centralize constrained dispatch and retain the existing VMT cache as the sole virtual-target
   cache;
5. reduce descriptor clones at API/cache boundaries;
6. only then evaluate a compact interned descriptor architecture;
7. optimize the evaluation-stack and Unicode-specific paths against their focused fixtures.

In parallel, keep corlib parsing as a separate cold-start program. These traces deliberately warm
the loader and therefore cannot rank parser optimizations for the small-script case.

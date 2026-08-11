# Plan 05 — Descriptor identity interning, phase 2

**Gate:** zero `#[allow(clippy::mutable_key_type)]` suppressions remain for
`ConcreteType`/`GenericLookup`-keyed collections (interning makes the key
`Copy`, which removes the reason for the allow rather than narrowing it
further), and the `record_key_clones` counters read zero in production on the
steady-state benchmark suite.

**Status:** not started. Independent of plans 01–04 and 06–08 — this is a
performance/architecture item, not part of the assurance lineage. It is
sequenced here because it is the largest remaining item from the 2026-07-25
architecture review with the clearest acceptance signal, not because anything
above blocks it.

## Goal

Phase 1 (merged `9c61ad9b`) memoized a `u64` hash on `ConcreteType`/
`GenericLookup`, computed eagerly in `ConcreteType::new`
(`dotnet-types/src/generics.rs:71`), so equality/hashing is O(1) instead of a
full descriptor walk. It did not remove the underlying cost: every
construction pays a full `DefaultHasher` tree walk, including descriptors
never used as a hash-map key, and 8 `clippy::mutable_key_type` suppressions
plus several production `record_key_clones` call sites still exist because the
key itself is a cloned, non-`Copy` structure rather than an interned handle.

Phase 2 replaces the memoized-hash key with an interned `Copy` identifier (an
arena index, or a pointer-identity newtype in the shape of
`dotnet-types/src/resolution.rs`'s `ResolutionS`), so hashing and equality
become integer comparisons and the `Arc`-clone-per-lookup cost that phase 1's
own counters were added to *measure* goes to zero instead of staying tracked.

## Current state (verified 2026-08-10)

- 8 `clippy::mutable_key_type` allows, each with a reason string documenting
  the deferral: `dotnet-intrinsics-reflection/src/types/type_queries.rs:72`,
  `dotnet-types/src/comparer.rs:830,866`, `dotnet-value/src/layout.rs:649`,
  `dotnet-value/src/object/types.rs:631`,
  `dotnet-runtime-resolver/src/methods.rs:149,241,267`.
- `record_key_clones` production call sites: `dotnet-vm/src/state.rs:391,404`,
  `dotnet-vm/src/resolver/mod.rs:122,144,182,188,204`; the counter is defined
  at `dotnet-vm/src/cache.rs:150`.
- `StackFrame<'static>` is pinned at 520 bytes
  (`dotnet-vm-data/tests/layout_sizes.rs:30-31`), up from 496 before phase 1;
  the test's own comment attributes the growth to the memoized descriptor
  hash. Phase 2 should not grow this further, and interning to a `Copy` handle
  (likely a `u32`/`u64` index) should let it shrink back toward 496.
- Pointer-identity precedent already exists in the same codebase:
  `ResolutionS` (`dotnet-types/src/resolution.rs:231-250`).

## Steps

1. Decide the interning shape: an arena index (requires a global or
   per-load-context arena and a lookup path back to the full descriptor) vs.
   pointer identity on an already-`Arc`-owned allocation (cheaper, but only as
   stable as the `Arc`'s lifetime — audit whether any `ConcreteType` is ever
   reconstructed with different contents at the same address, which would
   make pointer identity wrong rather than merely imprecise).
2. Replace the `mutable_key_type`-suppressed map keys with the interned
   identifier at each of the 8 sites, removing the suppression outright rather
   than narrowing it further — a second narrowing pass would repeat the
   documented failure mode of re-documenting a defect instead of fixing it.
3. Replace each `record_key_clones` call with a no-clone lookup through the
   interned identifier; delete the counter infrastructure in `cache.rs` once
   every production call site is gone (keep it if a legitimate use remains —
   do not delete instrumentation that is still load-bearing).
4. Re-measure `StackFrame` size and record the before/after in
   [`docs/BENCHMARK_WORKFLOW.md`](../BENCHMARK_WORKFLOW.md), following that
   document's existing table conventions.

## Not in scope

- Any change to `ConcreteType`'s identity semantics as observed by managed
  code (`value_kind` stays excluded from identity, matching phase 1).
- The base-chain `HashSet<*const BaseType<ConcreteType>>` in `is_value_type`
  (`generics.rs:117-121`) — already pointer-keyed, not part of this gap.

## Related

- [`docs/plans/README.md`](README.md)
- `dotnet-types/src/resolution.rs` — the `ResolutionS` precedent for pointer
  identity
- [`docs/BENCHMARK_WORKFLOW.md`](../BENCHMARK_WORKFLOW.md) — where phase 1's
  benchmark rationale and this phase's remeasurement both belong

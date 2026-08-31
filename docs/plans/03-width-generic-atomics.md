# Plan 03 — Width-generic atomic access

**Gate:** `dotnet-utils/src/atomic.rs`'s nine `match size { 1 => ..., 2 => ...,
4 => ..., 8 => ..., _ => unreachable!() }` ladders are replaced by a single
width-generic implementation shaped so that a call site cannot pass a literal
width inconsistent with the type it dispatches for; existing atomic-access
tests continue to pass with no behavior change.

**Status:** complete (2026-08-31). Both former blockers are cleared: the default-build
alignment-validation soundness defects closed for real on 2026-08-01
(`208a6c8b`), and the `pop_args` sweep that was meant to supply a const-generic
precedent closed 2026-07-31 by deletion — the function had zero production
callers and was removed rather than adopted. There is no template to adapt;
this plan starts from first principles.

## Goal

This is family F4 (atomic width/alignment) from
[`ASSURANCE_BACKGROUND.md`](../ASSURANCE_BACKGROUND.md), approached the way
the plan queue rates highest-value: eliminate the obligation instead of naming
it. Nine runtime `match size` ladders, split across two cfg-forked
`impl AtomicAccess for StandardAtomicAccess` blocks (`atomic.rs:143`
multithreading, `:259` non-multithreading), dispatch on a `usize` computed
from a CTS type and then throw the width away — every accessor returns
`-> u64` regardless of whether the underlying field is 1, 2, 4, or 8 bytes. A
width-generic design makes it a type error, not a runtime match arm, for an
`Int16` dispatch to reach an 8-byte atomic primitive.

## Current state (verified 2026-08-10)

- Nine ladders: `atomic.rs:147, 166, 190, 217, 239, 265, 286, 359, 382`.
- Consumers: `AtomicAccess` is used from `dotnet-runtime-memory/src/access.rs`
  only (`load_atomic`/`store_atomic`/`compare_exchange_atomic`/
  `exchange_atomic`/`exchange_add_atomic`, ten call sites), plus
  `dotnet-value/src/stack_value.rs:833,886`
  (`StackValue::load_atomic`/`store_atomic`), plus one fuzz target. This is a
  contained goal — three files, not a workspace-wide sprawl.
- The correct alignment-fallback pattern already exists in the same file:
  `Atomic::is_atomic_field_access_supported` (`atomic.rs:347-348`) checks
  `is_ptr_aligned_to_field` and falls back to a lock-guarded memcpy; its only
  callers are `Atomic::load_field`/`store_field` (`:355,379`). The five direct
  `StandardAtomicAccess` entry points bypass it entirely. This plan reuses that
  fallback shape generically; it does not need to invent it.
- No const-generic function exists anywhere in the workspace to use as
  precedent — `pop_args<'gc, T, const N: usize>` was deleted 2026-07-31 for
  having zero production callers (see [plan 01](01-layer-invariant-specs.md)'s
  retrofit ordering, which lists `dotnet-utils`' unsafe density).

## Design question

Two shapes were costed before implementation:

1. **`const WIDTH: usize` on the trait methods**, with a `[u8; WIDTH]`-shaped
   return replacing `u64`, and the five `StandardAtomicAccess` entry points
   becoming thin dispatchers that select the monomorphized width from the
   already-computed CTS size. This establishes the workspace's first
   const-generic function — flagged in the 2026-07-25/31 architecture reviews
   as a real cost precisely because there is no existing pattern to copy, so
   review scrutiny should be heavier here than on an ordinary change.
2. **A sealed `AtomicWidth` marker trait** (`W1`/`W2`/`W4`/`W8` unit structs)
   with an associated `Repr` type. Avoids const generics at the cost of a
   small trait hierarchy and more boilerplate per call site.

**Chosen: sealed `AtomicWidth` markers.** `W1`, `W2`, `W4`, and `W8` each
have one associated integer representation, so the core `AtomicAccess` methods
take a width marker and the matching representation rather than an independent
`usize` width and `u64` value. This makes an inconsistent literal width/
representation pair unrepresentable at the typed API boundary. Dynamic CTS
sizes are converted only by narrow runtime boundary helpers, which dispatch to
the marker-typed core.

Const generics would also carry width in the type system, but would establish
the workspace's first const-generic function without a local precedent and
would still need a type-level mapping from a width to its atomic representation.
The sealed-marker design makes that mapping explicit, limits implementors to
the four supported widths, and keeps the refactor aligned with existing
associated-type idioms.

## Completion evidence

`AtomicAccess` now accepts a sealed width marker and its associated
representation. The former nine `match size` ladders are replaced by one
runtime dispatcher, which binds each dynamic supported size to exactly one
marker before executing the width-generic implementation. Runtime-memory,
`StackValue`, and the raw-memory fuzz target use the dynamic bridge; typed
tests exercise load, store, compare-exchange (success and mismatch), exchange,
and exchange-add for all four supported widths in both feature configurations.

## Not in scope

- Widening the set of supported atomic widths (still 1/2/4/8 bytes).
- Changing the alignment-fallback *behavior* in
  `Atomic::is_atomic_field_access_supported` / `load_field` / `store_field` —
  those stay as they are; only the five direct `StandardAtomicAccess` entry
  points and their internal ladders change shape.
- `Interlocked.*` misalignment throwing `DataMisalignedException` — that
  semantics question is already settled and closed; not reopened here.

## Related

- [`docs/plans/README.md`](README.md)
- [`docs/GC_AND_MEMORY_SAFETY.md`](../GC_AND_MEMORY_SAFETY.md) — records this
  as the width-generic atomic follow-up
- [`docs/plans/01-layer-invariant-specs.md`](01-layer-invariant-specs.md) —
  names F4 as the predicate this plan eliminates rather than cites
- [`docs/ASSURANCE_BACKGROUND.md`](../ASSURANCE_BACKGROUND.md) — family F4

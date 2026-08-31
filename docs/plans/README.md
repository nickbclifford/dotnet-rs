# Plan queue

This directory holds the numbered plan series for `dotnet-rs` and indexes its
active, complete, and explicitly parked work in one dependency-respecting
queue. It replaces the old standalone
`docs/ASSURANCE_ROADMAP.md`, which indexed only the unsafe-code assurance
plans (now 01, 02, 04, 06, 08 below); this file folds in the architecture and
soundness backlog that used to live only in review notes, and drops the
separate document.

The plans here come from three lineages, merged into one queue by dependency
order rather than kept as parallel lists:

- **Assurance lineage** (01, 02, 04, 06, 08) — successor to a terminated
  proof-DSL feasibility study. Keeps that study's goal — make the invariants
  behind ~580 `unsafe` blocks reviewable and mechanically checked — and drops
  its mechanism. There is no proof assistant, no DSL, no ghost tokens, and no
  dependency on any external verifier. The evidence behind these plans, the
  nine invariant families they're built on, and the process lessons from the
  abandoned attempt are in
  [`ASSURANCE_BACKGROUND.md`](../ASSURANCE_BACKGROUND.md). Read that first if
  the "why" matters; this file and the plans themselves are the "what."
- **Architecture/soundness lineage** (03, 05, 07) — items from the 2026-07-25
  and 2026-07-31 whole-codebase architecture reviews that were tracked only in
  review notes until this consolidation gave them plan files.
- **Legibility-review lineage** (09) — the accepted 2026-08-30 bottom-up and
  top-down review. It integrates new proposals with plans 01–08 without
  renaming their work or changing plan 08's parked disposition.

The premise shared across the assurance plans: **the highest-value work is
eliminating obligations, not proving them** — an invariant carried by a type
is strictly better than one carried by a comment plus a checker — and **for
what cannot be eliminated, a falsifier beats a proof on assurance per unit
effort** at this project's scale. See `ASSURANCE_BACKGROUND.md` for the full
argument (why a Rust verifier cannot reach cross-thread temporal facts or
ECMA-335 correspondence at any price, and why a `loom` leg over the
stop-the-world handshake finds real bugs this week where the matching
rely-guarantee theorem was costed at one to three months).

## The queue

Ordered by dependency, then by value. Each active plan has a hard gate — a
condition objectively met or not — so progress is not self-assessed.

| # | Plan | Gate | Status | Depends on |
| --- | --- | --- | --- | --- |
| 01 | [Layer invariant specs](01-layer-invariant-specs.md) | Every `// SAFETY:` comment in the four core crates cites a named predicate from the registry; drift-checked in CI | Complete | — |
| 02 | [Falsifier portfolio](02-falsifier-portfolio.md) | A `loom` leg exercising the STW handshake is blocking in CI; all four fuzz targets blocking; Kani harnesses for the F3/F4/F9 value facts | Not started | — |
| 03 | [Width-generic atomics](03-width-generic-atomics.md) | The nine `match size` ladders in `dotnet-utils/src/atomic.rs` are replaced by one width-generic implementation | Complete | — |
| 04 | [Model correspondence](04-model-correspondence.md) | Clause→site index covers every family-F2 site; differential fixture count ratcheted upward from 7 | Not started | 01 (predicate names) |
| 05 | [Descriptor interning, phase 2](05-descriptor-interning.md) | Zero `mutable_key_type` allows remain for `ConcreteType`/`GenericLookup` keys; `record_key_clones` reads zero in production | Not started | — |
| 06 | [Trust register](06-trust-register.md) | Every entry names a falsifier; count is CI-ceilinged | Not started | 01, 02 |
| 07 | [Fixture exit-code oracle](07-fixture-exit-code-oracle.md) | Harness-level outcomes occupy a reserved code band distinct from fixture-authored codes; opt-in exit-code-only differential mode exists | Not started | — |
| 08 | [Provenance redesign](08-provenance-redesign.md) | `-Zmiri-strict-provenance` runs green on at least the `dotnet-value` and `dotnet-runtime-memory` legs | **Parked** — implementation complete, gate closed unmet by owner-directed deferral (2026-08-05) | — |
| 09 | [Whole-codebase legibility review](09-codebase-legibility-review.md) | Every active Phase-4 row reaches its objective gate or moves to a separately tracked successor plan without duplicate scope | In progress — review accepted; priorities 1 (arena-local P/Invoke last-error cache and blocking `multithreading` isolation fixture), 3 (workspace-wide extension of Plan 01's predicate-citation coverage and drift gate), and 4 (Plan 03's sealed width markers and dynamic bridge) complete | Mixed; see plan |

Plan 01 is complete, so its dependencies no longer block plan 04 and satisfy
that half of plan 06's prerequisites. Plans 02, 03, 04, 05, and 07 therefore
have no unmet dependency and can run in any order or concurrently; plan 06
still waits on plan 02. Plan 09 is the integrated review backlog, and the
per-row dependencies in its Phase 4 table control the work it adds or extends.

08 is parked, not queued: its gate was closed by an explicit owner decision,
and reopening it requires a new, explicitly authorized task beginning with a
green local target leg — not routine progress on the rest of the queue. Do
not infer any strict-provenance coverage from the other plans' Miri legs; see
[`docs/CI.md`](../CI.md) for the current gate inventory.

## What this queue explicitly does not do

- **No proof assistant, no DSL, no proof kernel or tactic evaluator in this
  repository.** The Rust-side instruments in plans 01/02/04/06 are falsifiers
  and type refactors, not proofs.
- **No adoption of a Rust deductive verifier.** The feature census in
  [`ASSURANCE_BACKGROUND.md`](../ASSURANCE_BACKGROUND.md) eliminated
  RefinedRust, Verus, Aeneas/Anneal, and Kani-as-verifier against this
  codebase's actual feature use; VeriFast is technically viable and
  unaffordable solo.
- **No mechanized ECMA-335 model.** If that becomes attractive later, build it
  as an executable reference interpreter differentially tested against .NET —
  continuous with plan 04, with no proof obligations attached.
- **No new parallel doc tree.** Plan 01 extends the existing subsystem docs
  (`GC_AND_MEMORY_SAFETY.md`, `THREADING_AND_SYNCHRONIZATION.md`, …) rather
  than duplicating them, and plans 03/05/07/09 land their findings in this same
  `docs/plans/` tree rather than a separate backlog document.

## Re-entry conditions for a proof-assistant approach

Two developments would justify revisiting a proof-assistant-based approach to
the assurance lineage, and only these:

1. **`cargo-anneal` reaches beta with separation-logic support in Aeneas** and
   can extract a crate with trait-bound generics. At that point the F3/F4/F9
   and F2-Rust-half obligations become mechanically dischargeable by a tool
   someone else maintains, and plan 01's predicate registry is the input it
   needs. Track the tracking issue, not the release notes.
2. **A concurrency challenge in `verify-rust-std` is closed** by any tool. That
   would be the first evidence that F1/F7/F8-shaped obligations are reachable
   on real code.

Neither is a reason to build tooling now. Both are reasons to keep plan 01's
predicates named in a way a future extractor could consume — which is why the
surface tracks RFC 3842 safety tags rather than a private convention.

## Not currently queued

Two tracks are deliberately absent from this queue, for different reasons:

- **Benchmark/perf work.** Closed out 2026-08-10 (`a9436b4e`) with no open
  item. See [`docs/BENCHMARK_WORKFLOW.md`](../BENCHMARK_WORKFLOW.md).
- **Userland/stdlib compatibility breadth** (EF Core SQLite, Reflection.Emit,
  networking, real async scheduling). This isn't a queued-but-blocked plan —
  it has no plan at all. The two documents that used to track it
  (`EF_GAP_BACKLOG.md`, `USERLAND_TESTING_ROADMAP.md`) were deleted after the
  work they tracked landed and were never replaced. Reopening this track
  needs a fresh scoping decision before it can become a plan file here, not a
  reorder of the existing queue.

## Related documents

- [`docs/ASSURANCE_BACKGROUND.md`](../ASSURANCE_BACKGROUND.md) — the invariant
  families, the evidence, and links to the archived proof-DSL study in git
  history
- [`CONTRIBUTING.md` — Unsafe code policy](../../CONTRIBUTING.md#unsafe-code-policy)
  — the existing rules plan 01 extends
- [`docs/GC_AND_MEMORY_SAFETY.md`](../GC_AND_MEMORY_SAFETY.md),
  [`docs/THREADING_AND_SYNCHRONIZATION.md`](../THREADING_AND_SYNCHRONIZATION.md)
  — subsystem docs that carry invariant sections from plan 01
- [`docs/CI.md`](../CI.md) — current gate inventory, including the
  strict-provenance note at the heart of plan 08

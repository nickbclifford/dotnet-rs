# Assurance roadmap

This is the successor to a terminated proof-DSL track. It keeps that track's
goal — make the invariants behind 583 `unsafe` blocks reviewable and
mechanically checked — and drops its mechanism. There is no proof assistant, no
DSL, no ghost tokens, and no dependency on any external verifier.

The evidence behind these choices, the nine invariant families the plans are
built on, and the process lessons from the abandoned attempt are in
[`ASSURANCE_BACKGROUND.md`](ASSURANCE_BACKGROUND.md). Read that first if the
"why" matters; this file is the "what."

## The premise

The terminal finding was that "Rust semantics → premise" is not one missing
arrow but four, one per kind of invariant, and that only the cheap kinds lie
inside a Rust verifier's remit:

| Premise kind | Families | Instrument that actually reaches it |
| --- | --- | --- |
| Cross-thread temporal / protocol | F1 rooting/STW, F7 publication, F8 lock order | Systematic interleaving exploration (`loom`), not proof |
| Correspondence to ECMA-335 | F2 layout fidelity, the non-moving-GC axiom | A stated relation plus a differential oracle — *no Rust verifier can state these* |
| Structural completeness | F5 `Collect` tracing | Rust itself: exhaustive match, derive audit |
| Local value / spatial | F3 slot typing, F4 width & alignment, F9 leaked-`Box` | Refinement or bounded model checking — **or a type refactor that removes the obligation** |

Two consequences shape everything below. First, **the highest-value work is
eliminating obligations, not proving them**: an invariant carried by a type is
strictly better than one carried by a comment plus a checker. Second, **for
what cannot be eliminated, a falsifier beats a proof on assurance per unit
effort** at this project's scale — a `loom` leg over the stop-the-world
handshake finds real interleaving bugs this week; the corresponding
rely-guarantee theorem was costed at one to three months and would still rest
on a hand-written model.

## Workstreams

Ordered by dependency. Each has a hard gate — a condition that is objectively
met or not — so progress is not self-assessed.

| # | Plan | Gate | Depends on |
| --- | --- | --- | --- |
| 1 | [Provenance redesign](plans/01-provenance-redesign.md) | `-Zmiri-strict-provenance` runs green on at least the `dotnet-value` and `dotnet-runtime-memory` legs | — |
| 2 | [Layer invariant specs](plans/02-layer-invariant-specs.md) | Every `// SAFETY:` comment in the four core crates cites a named predicate from the registry; drift-checked in CI | — (runs beside 1) |
| 3 | [Falsifier portfolio](plans/03-falsifier-portfolio.md) | A `loom` leg exercising the STW handshake is blocking in CI; all four fuzz targets blocking; Kani harnesses for the F3/F4/F9 value facts | 1 (provenance) for the Miri legs |
| 4 | [Model correspondence](plans/04-model-correspondence.md) | Clause→site index covers every family-F2 site; differential fixture count ratcheted upward from 7 | 2 (predicate names) |
| 5 | [Trust register](plans/05-trust-register.md) | Every entry names a falsifier; count is CI-ceilinged | 2, 3 |

Workstream 1 is first because it is the precondition for the rest of the
assurance story *and* for any future tool: RefinedRust rejects pointer-integer
casts outright, Kani does not check provenance UB, and Charon needs MIR that
does not launder addresses. It is also the only workstream that changes
runtime code, so it should land before specs are written against that code.

## Sequencing note

Workstreams 1 and 2 are independent and can run concurrently — 1 touches
`crates/dotnet-value/src/pointer/` and its consumers, 2 touches `docs/` and
comment text. Do not start 4 before 2, or the clause index will be keyed to
predicate names that then get renamed. Do not start 5 before 3, or the register
will contain entries with no falsifier to name, which is the failure mode the
register exists to prevent.

## What this roadmap explicitly does not do

- **No proof assistant, no DSL, no proof kernel or tactic evaluator in this
  repository.** The Rust-side instruments are falsifiers and type refactors.
- **No adoption of a Rust deductive verifier.** The feature census in
  [`ASSURANCE_BACKGROUND.md`](ASSURANCE_BACKGROUND.md) eliminated RefinedRust,
  Verus, Aeneas/Anneal and Kani-as-verifier against this codebase's actual
  feature use; VeriFast is technically viable and
  unaffordable solo (+95% line growth on a *partial* std `LinkedList` proof by
  its own authors).
- **No mechanized ECMA-335 model.** If that becomes attractive later, build it
  as an executable reference interpreter differentially tested against .NET —
  continuous with workstream 4, with no proof obligations attached.
- **No new parallel doc tree.** Workstream 2 extends the existing subsystem
  docs (`GC_AND_MEMORY_SAFETY.md`, `THREADING_AND_SYNCHRONIZATION.md`, …)
  rather than duplicating them.

## Re-entry conditions

Two developments would justify revisiting the proof track, and only these:

1. **`cargo-anneal` reaches beta with separation-logic support in Aeneas** and
   can extract a crate with trait-bound generics. At that point the F3/F4/F9
   and F2-Rust-half obligations become mechanically dischargeable by a tool
   someone else maintains, and workstream 2's predicate registry is the input
   it needs. Track the tracking issue, not the release notes.
2. **A concurrency challenge in `verify-rust-std` is closed** by any tool. That
   would be the first evidence that F1/F7/F8-shaped obligations are reachable
   on real code.

Neither is a reason to build tooling now. Both are reasons to keep
workstream 2's predicates named in a way a future extractor could consume —
which is why the surface tracks RFC 3842 safety tags rather than a private
convention.

## Related documents

- [`docs/ASSURANCE_BACKGROUND.md`](ASSURANCE_BACKGROUND.md) — the invariant
  families, the evidence, and links to the archived proof-DSL study in git
  history.
- [`CONTRIBUTING.md` — Unsafe code policy](../CONTRIBUTING.md#unsafe-code-policy)
  — the existing rules workstream 2 extends.
- [`docs/GC_AND_MEMORY_SAFETY.md`](GC_AND_MEMORY_SAFETY.md),
  [`docs/THREADING_AND_SYNCHRONIZATION.md`](THREADING_AND_SYNCHRONIZATION.md) —
  the subsystem docs that will carry the invariant sections.
- [`docs/CI.md`](CI.md) — current gate inventory, including the
  strict-provenance note at the heart of workstream 1.

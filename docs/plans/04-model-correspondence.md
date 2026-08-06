# Plan 04 — Model correspondence

**Gate:** a clause→site index covering every family-F2 site and every
`unsafe` block whose premise cites the standard; plus a completed audit of the
unsafe corpus against MiniRust's UB list, recorded per site as *checked by X*,
*assumed*, or *not applicable*.

**Status:** not started. Depends on plan 02 for predicate names.

## Goal

Two correspondence claims underpin the whole codebase, and neither is currently
written down in a form anyone can audit:

1. **"This code implements ECMA-335."** The relation between runtime data
   (`FieldLayoutManager`, `GcDesc`, eval-stack slots) and the standard's
   prescriptions. The terminated proof study called this the representation relation
   and planned to state it in Lean; the useful 5% of that is a legible table.
2. **"This unsafe code is not UB under Rust's semantics."** Currently carried
   by 620 prose comments against no enumerated model.

This plan writes both down. It produces no proofs, and that is the point: for
F2 the study's decisive finding was that *no Rust verifier can state the
obligation at all*, because half the statement is not in the program. A stated
relation plus a differential oracle is the honest instrument.

## Current state (measured at `6ebda249`)

- **~35 ECMA-335 clause citations** in the entire source and docs tree, of the
  form `II.22`, `III.4.33`, `II.15.4.1.2`, `I.12.3.2`, `I.8.2.4`. The most-cited
  are `II.22` (6), `III.4.33` (4), `II.15.4.1.2` (4). For a 61k-line
  implementation of a specification, this is sparse — the knowledge is in the
  code and the author's head, not in citations.
- **7 differential fixtures** of 475 C# fixtures (see plan 03, instrument 5).
- **A rejected-candidate list** already exists in `diff_harness.rs`, naming
  known intentional divergences: an intentional ECMA alignment divergence in
  `structs/interlocked_misaligned_1`, unhandled-exception exit codes (real .NET
  exits 134, `dotnet-rs` exits 1), and GC-timing-sensitive output. **This list
  is the seed of the deviation register** — every entry is a place where the
  implementation knowingly differs from the reference.
- The workspace has an `ecma_search` MCP tool available for clause lookup, which
  makes the index cheap to build accurately rather than from memory.

## Part A — ECMA-335 clause→site index

New `docs/ECMA335_CORRESPONDENCE.md`, one row per clause: clause number, its
prescription in one line, the implementing site(s), the invariant-registry
predicate that carries it (from plan 02), and its verification status
(differentially tested / unit-tested / assumed).

Seed order, highest-value first:

| Clause | Subject | Why first |
| --- | --- | --- |
| `II.10.7`, `I.8.5` | Field layout, reference-overlap prohibition | This is F2. `LayoutManager::trace` derives GC reachability from the same descriptor, so a layout defect is simultaneously type confusion and a reachability error — the study's "uniquely consequential" family. |
| `I.12.3.2.1` | Evaluation-stack typing | F3; the theory behind it (Gordon–Syme) is already worked out on paper. |
| `I.12.6.2`, `I.12.6.6` | Alignment and atomicity guarantees, `unaligned.` prefix | F4, and the site of the intentional divergence already listed in `diff_harness.rs`. |
| `III` `cpblk`/`initblk`, `I.12.1` | Spec-licensed UB from unverifiable IL | One of the two irreducible trust classes; these rows exist to be *marked* untrusted, not resolved. |
| `II.22`, `II.23` | Metadata tables | Already the most-cited; cheap to complete. |

Do not attempt full coverage of the standard. The index covers clauses the
implementation's *unsafe* premises depend on, which is a few dozen, not the
whole of Partitions I–III.

## Part B — Rust model audit

Walk MiniRust's UB list against the unsafe corpus and record, per site, which
class of UB is at issue and what currently checks it. The output is a table with
one row per UB class:

| UB class | Sites at risk | Checked by | Residual |
| --- | --- | --- | --- |
| Invalid pointer dereference (dangling, out-of-bounds) | … | Miri tree-borrows leg, `fuzz_raw_memory_access` | … |
| Misaligned access | F4 sites | `debug_assert!` at call site, Kani harness (plan 03) | release builds |
| Data race | F1/F7 sites | `loom` leg (plan 03) | non-modelled paths |
| Pointer-integer provenance | 102 audited API sites plus 2 bare test casts → plan 01 | ordinary Miri only; Plan 01 strict-provenance CI was owner-deferred | documented managed-storage boundaries remain |
| Aliasing-model violation (Tree Borrows) | all `&mut` from raw | Miri `-Zmiri-tree-borrows` | untested paths |
| Uninitialized read | 1 `MaybeUninit` site | — | … |

The value is the **Residual** column. It is the honest version of what the
study would have written as ~30 Lean axioms, and unlike axioms each entry names
a tool that could close it.

Two facts to record explicitly, since both were established in the study and are
easy to lose: rustc optimizes under Tree Borrows, so a proof of memory safety
that violates the aliasing discipline proves the wrong theorem; and Rust has no
complete normative formal semantics as of 2026, so every claim here is against
a best-available approximation — the FLS is prose, `a-mir-formality` covers
static semantics only, and MiniRust is the leading operational candidate.

## Part C — Deviation register seed

Extract the `diff_harness.rs` rejected-candidate list plus the study's named
deviations — non-moving GC, single load context, stop-the-world-only collection
— into rows destined for plan 05's trust register. Each needs: what the standard
says, what `dotnet-rs` does, why, and what would break if the deviation changed.
The non-moving-GC entry is the one to write most carefully: it is correct today
and is invalidated the day a compacting collector lands, and a great deal of F1
reasoning depends on it silently.

## Not in scope

- A mechanized or executable model of ECMA-335 in any prover. If that becomes
  attractive, build it as a reference interpreter differentially tested against
  .NET, continuous with plan 03's fixture ratchet, with no proof obligations
  attached — not as a verification substrate.
- Auditing the standard for its own sake. Clauses enter the index because an
  unsafe premise depends on them.
- Resolving the two irreducible trust classes. They are marked, not closed.

## Related

- [`docs/ASSURANCE_ROADMAP.md`](../ASSURANCE_ROADMAP.md)
- [`docs/plans/02-layer-invariant-specs.md`](02-layer-invariant-specs.md) —
  supplies the predicate names this index keys against
- [`docs/plans/05-trust-register.md`](05-trust-register.md) — consumes Part C
- [`docs/ASSURANCE_BACKGROUND.md`](../ASSURANCE_BACKGROUND.md) — the
  families, and why no mechanized ECMA-335 model exists
- Archived study on the representation relation and the model-fidelity gap:
  [`04-theory.tex`](https://github.com/nickbclifford/dotnet-rs/blob/b5a5f65d67345b0682def83867b816ea86fa3152/docs/proof-dsl-feasibility/sections/04-theory.tex), [`08-risks.tex`](https://github.com/nickbclifford/dotnet-rs/blob/b5a5f65d67345b0682def83867b816ea86fa3152/docs/proof-dsl-feasibility/sections/08-risks.tex) §8.2

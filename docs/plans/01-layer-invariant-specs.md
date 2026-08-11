# Plan 01 — Layer invariant specifications

**Gate:** every `// SAFETY:` comment in `dotnet-value`, `dotnet-runtime-memory`,
`dotnet-utils` and `dotnet-vm` cites at least one named predicate from the
registry, and `scripts/check_doc_drift.sh` enforces that every cited predicate
name exists in the registry and every registry predicate is cited at least
once.

**Status:** complete — the citation audit was re-derived against the registry statements (including new raw-memory, raw-allocation, borrowed-storage, CLI-load, object-pointer, and P/Invoke-state predicates), and the drift gate passes. The prior “539 current comments” claim was incorrect: the four `src/` trees contain 465 `// SAFETY:` comments (153 value, 104 runtime-memory, 67 utils, 141 VM). The gate intentionally recurses into each crate and therefore also checks 8 fuzz-target comments, for 473 cited sites total.

## Goal

Turn 620 prose `// SAFETY:` comments into citations of a small, named,
reviewable set of predicates. This is the surviving output of the terminated
study: the corpus factors into nine invariant families, and each family is one
predicate stated once and instantiated per site. Naming them is worth most of
what a proof layer would have bought, because the failure mode the study found
empirically — *prose asserting a witness that no code establishes* — is a
failure of **naming**, not of proving. A comment that says "aligned" cannot be
cross-checked; a comment that cites `F4.WidthAligned` can be.

## Design decisions

**Extend the existing docs; do not build a parallel tree.** The subsystem docs
already carry this material in prose. Each gets an `## Invariants` section
listing the predicates it owns, and the registry is one new index doc pointing
into them.

| Doc | Families it owns |
| --- | --- |
| [`GC_AND_MEMORY_SAFETY.md`](../GC_AND_MEMORY_SAFETY.md) | F1 rooting/STW, F5 tracing completeness, F6 brand discipline, F9 leaked-`Box` lifetime |
| [`THREADING_AND_SYNCHRONIZATION.md`](../THREADING_AND_SYNCHRONIZATION.md) | F4 atomic width/alignment, F7 immutable-field publication, F8 lock order & safepoint discipline |
| [`ARCHITECTURE.md`](../ARCHITECTURE.md) or a new `VALUE_REPRESENTATION.md` | F2 layout faithfulness, F3 eval-stack slot typing |

**Surface syntax tracks RFC 3842 safety tags.** Predicate names are tag names.
The comment form is a citation line the drift checker can parse, and the
eventual upgrade path — if `#[safety::requires]` stabilizes, or if an extractor
like Anneal becomes usable — is a mechanical rewrite rather than a redesign.
Do not invent a private syntax.

**One naming scheme, stated once:** `F<n>.<PredicateName>`, e.g.
`F1.StwParked`, `F1.ArenaGenerationMatch`, `F2.DescriptorMatchesEcmaLayout`,
`F4.WidthAligned`, `F5.TracesEveryGcRef`, `F6.NoEscapeAcrossArena`. Family
number keeps the taxonomy legible; the name carries the content.

## Steps

1. **Write the registry.** New `docs/INVARIANT_REGISTRY.md`: one row per
   predicate — name, family, one-sentence statement, what establishes it
   (a guard, a type, a protocol step, or *nothing — assumed*), which doc
   section defines it, and its falsifier if one exists. Seed it from
   [`ASSURANCE_BACKGROUND.md`](../ASSURANCE_BACKGROUND.md), which already has
   the nine family statements and the what-Rust-carries / what-it-doesn't split
   for each.
2. **Mark the assumed ones honestly.** Any predicate whose "what establishes
   it" column is *nothing* is a trust-register candidate (plan 06). Expect this
   to be the interesting output of the whole exercise: the study found two such
   cases by manual review, and this pass is the systematic version.
3. **Add the `## Invariants` sections** to the three subsystem docs, each
   defining its predicates properly — statement, scope, and the code that
   establishes or assumes it.
4. **Retrofit the comments, crate by crate**, in the order
   `dotnet-value` → `dotnet-runtime-memory` → `dotnet-utils` → `dotnet-vm`
   (ascending unsafe density: 163, 96, 121, 175 occurrences). A comment keeps
   its prose and gains a citation line. Do not delete the prose — the citation
   says *which* invariant, the prose says *why it holds here*.
5. **Extend `check_doc_drift.sh`** with a new check class: for every
   `F<n>.<Name>` cited in source, the name must appear in
   `INVARIANT_REGISTRY.md`; for every registry row, the name must be cited at
   least once in source. This is the same doc↔code identifier check the script
   already does, applied to predicate names, and it is what makes the registry
   unable to drift.
6. **Update `CONTRIBUTING.md`'s unsafe-code policy** to require a citation
   alongside the existing `// SAFETY:` requirement, and to say that adding a new
   predicate means adding a registry row in the same commit.

## Why this is not the ghost-token layer again

The terminated proof study built a Rust crate whose zero-sized types were the
citations, so that `rustc` checked their existence and `cfg`-liveness. That
mechanism worked and is not being rebuilt, for two reasons. It required a proc
macro to expand inside every unsafe site, which put macro-expansion bugs on the
path to miscompilation; and its value was entirely bookkeeping, which a
grep-based drift check in CI delivers at a fraction of the cost. What the token
layer had and this does not is `cfg`-liveness checking — citing a predicate
established by a guard that is compiled out is not a compile error here. That
gap is covered instead by step 2 (the registry records what establishes each
predicate, and a feature-gated guard is recorded as such) and by plan 02's
guard-off CI leg.

## Not in scope

- Any machine-checked semantics for the predicates. They are named and
  cross-referenced, not proved.
- Rewriting the 55% of comments the study found to be duplicated boilerplate
  (one template repeated 67 times in `raw_memory_ops_impl/mod.rs`).
  Deduplicating those is a separate, purely editorial cleanup — though this
  plan makes it much easier, since after citation they differ only in prose.
- The F4 width-generic refactor ([plan 03](03-width-generic-atomics.md)). That
  *eliminates* a predicate rather than naming it, which is better, but it is a
  code change with its own design question and should not be bundled into a
  documentation pass.

## Related

- [`docs/plans/README.md`](README.md)
- [`CONTRIBUTING.md` — Unsafe code policy](../../CONTRIBUTING.md#unsafe-code-policy)
- [`docs/ASSURANCE_BACKGROUND.md`](../ASSURANCE_BACKGROUND.md) — the nine
  family statements this registry is seeded from
- [`docs/plans/06-trust-register.md`](06-trust-register.md) — where step 2's
  unestablished predicates land
